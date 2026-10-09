//! Hardware-decoded pictures sampled where the decoder wrote them (Linux, Vulkan; ADR 0002).
//!
//! [`create_device`] makes the wgpu device on a Vulkan adapter with the extensions that import
//! Linux DMA-BUFs (`VK_KHR_external_memory_fd`, `VK_EXT_external_memory_dma_buf`,
//! `VK_EXT_image_drm_format_modifier`). [`Importer`] turns a decoded picture's DMA-BUF layers
//! ([`filmcraft_frame::DmaBuf`]: luma and interleaved chroma) into two wgpu textures over the
//! same memory, which the compositor samples with its biplanar path, with no copy. Imports are
//! cached per decoder surface (decoders reuse a pool of surfaces).
//!
//! This is the only module of `filmcraft-gpu` with `unsafe` (Vulkan through `ash` and wgpu-hal),
//! under the rules of ADR 0001: a `// SAFETY:` comment on every block, a safe `Option` / `Result`
//! API, no panics. Without the extensions (other drivers or backends), callers fall back to
//! reading the picture back to memory.

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::fd::IntoRawFd;

use ash::vk;
use filmcraft_frame::{DmaBuf, GpuFormat, GpuSurface};
use wgpu::hal::api::Vulkan;

const EXTENSIONS: [&CStr; 3] = [ash::khr::external_memory_fd::NAME, ash::ext::external_memory_dma_buf::NAME, ash::ext::image_drm_format_modifier::NAME];

/// Imported pictures kept (each holds a reference on its DMA-BUF, which keeps the decoder's
/// memory alive even after the decoder released the surface).
const MAX_IMPORTS: usize = 64;
/// Imports not drawn for this many imports are dropped.
const STALE_AFTER: u64 = 300;

/// A wgpu device on `adapter` with DMA-BUF import enabled, when the adapter is Vulkan and has the
/// extensions (None: it is not, use an ordinary device). `desc` as for `Adapter::request_device`.
pub fn create_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor<'_>) -> Option<Result<(wgpu::Device, wgpu::Queue), String>> {
    // SAFETY: the hal adapter is only used within this function, while `adapter` is alive, and we
    // do not destroy anything it owns.
    let hal = unsafe { adapter.as_hal::<Vulkan>() }?;
    // the import's format queries are Vulkan 1.1 functions (instance and physical device)
    if hal.shared_instance().instance_api_version() < vk::API_VERSION_1_1 || hal.physical_device_capabilities().properties().api_version < vk::API_VERSION_1_1 {
        return None;
    }
    if !EXTENSIONS.iter().all(|e| hal.physical_device_capabilities().supports_extension(e)) {
        return None;
    }
    let cb = Box::new(|args: wgpu::hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>| {
        for e in EXTENSIONS {
            if !args.extensions.contains(&e) {
                args.extensions.push(e);
            }
        }
    });
    // SAFETY: the callback only adds extensions the physical device supports (checked above); it
    // removes no extension or feature. Features and limits are the ones wgpu would request.
    let open = unsafe { hal.open_with_callback(desc.required_features, &desc.required_limits, &desc.memory_hints, Some(cb)) };
    let open = match open {
        Ok(o) => o,
        Err(e) => return Some(Err(format!("Vulkan device with DMA-BUF import: {e}"))),
    };
    drop(hal);
    // SAFETY: `open` was created from this adapter's hal adapter, with exactly `desc`'s features.
    Some(unsafe { adapter.create_device_from_hal(open, desc) }.map_err(|e| e.to_string()))
}

/// Whether `device` was made by [`create_device`] (has the import extensions).
pub fn can_import(device: &wgpu::Device) -> bool {
    // SAFETY: only reads the enabled-extension list of the live hal device.
    let Some(hal) = (unsafe { device.as_hal::<Vulkan>() }) else { return false };
    EXTENSIONS.iter().all(|e| hal.enabled_device_extensions().contains(e))
}

/// One imported picture: the luma and chroma planes as textures over the decoder's memory.
pub struct Imported {
    /// Dead once the decoder freed the memory: the import is dropped then.
    alive: std::sync::Weak<()>,
    pub luma: wgpu::TextureView,
    pub chroma: wgpu::TextureView,
    last_used: u64,
    _textures: [wgpu::Texture; 2],
}

/// Imports decoded pictures into a device made by [`create_device`].
pub struct Importer {
    device: wgpu::Device,
    cache: HashMap<u64, Imported>,
    clock: u64,
    /// Imports that failed (they fall back to reading back).
    pub failures: u64,
}

impl Importer {
    /// None when `device` cannot import DMA-BUFs.
    pub fn new(device: &wgpu::Device) -> Option<Importer> {
        can_import(device).then(|| Importer { device: device.clone(), cache: HashMap::new(), clock: 0, failures: 0 })
    }

    /// The textures of `surface` (imported on first use, then cached by its memory).
    pub fn import(&mut self, surface: &dyn GpuSurface) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.clock += 1;
        let key = surface.memory_id();
        if !self.cache.contains_key(&key) {
            let made = surface
                .dmabuf()
                .ok_or_else(|| "the decoder did not export the picture".to_string())
                .and_then(|d| self.import_dmabuf(&d, surface.format()))
                .map(|mut i| {
                    i.alive = surface.liveness();
                    i
                });
            match made {
                Ok(i) => {
                    let clock = self.clock;
                    self.cache.retain(|_, v| v.last_used + STALE_AFTER > clock && v.alive.strong_count() > 0);
                    if self.cache.len() >= MAX_IMPORTS
                        && let Some(old) = self.cache.iter().min_by_key(|(_, v)| v.last_used).map(|(k, _)| *k)
                    {
                        self.cache.remove(&old);
                    }
                    self.cache.insert(key, i);
                }
                Err(e) => {
                    self.failures += 1;
                    if self.failures <= 3 {
                        log::warn!("importing a decoded picture into Vulkan failed: {e} (reading it back instead)");
                    }
                    return None;
                }
            }
        }
        let clock = self.clock;
        let i = self.cache.get_mut(&key)?;
        i.last_used = clock;
        Some((&i.luma, &i.chroma))
    }

    fn import_dmabuf(&self, d: &DmaBuf, format: GpuFormat) -> Result<Imported, String> {
        let (fy, fc, wy, wc) = match format {
            GpuFormat::P010 => (vk::Format::R16_UNORM, vk::Format::R16G16_UNORM, wgpu::TextureFormat::R16Unorm, wgpu::TextureFormat::Rg16Unorm),
            GpuFormat::Nv12 => (vk::Format::R8_UNORM, vk::Format::R8G8_UNORM, wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
        };
        let (y_fourcc, c_fourcc) = layer_fourccs(format);
        if d.layer_formats != [y_fourcc, c_fourcc] {
            return Err(format!("layer formats {:#x} / {:#x} do not match {format:?}", d.layer_formats[0], d.layer_formats[1]));
        }
        let (w, h) = (d.width.max(1), d.height.max(1));
        let luma = self.import_plane(d, 0, fy, wy, (w, h), "decoded luma")?;
        let chroma = self.import_plane(d, 1, fc, wc, (w.div_ceil(2), h.div_ceil(2)), "decoded chroma")?;
        Ok(Imported {
            alive: std::sync::Weak::new(),
            luma: luma.create_view(&Default::default()),
            chroma: chroma.create_view(&Default::default()),
            last_used: 0,
            _textures: [luma, chroma],
        })
    }

    fn import_plane(
        &self,
        d: &DmaBuf,
        layer: usize,
        vk_format: vk::Format,
        format: wgpu::TextureFormat,
        (w, h): (u32, u32),
        label: &'static str,
    ) -> Result<wgpu::Texture, String> {
        let (planes, modifier) = (d.layers.get(layer).ok_or("layer index")?, d.modifier);
        // One memory plane only: a modifier with metadata planes (compression) would need the
        // metadata kept across the layout transition below, which only a foreign-queue acquire
        // guarantees; such memory is read back instead.
        let [first] = planes.as_slice() else {
            return Err(format!("{} memory planes in a layer (only single-plane modifiers are imported)", planes.len()));
        };
        let object = d.objects.get(first.object).ok_or("plane of a missing buffer object")?;
        // SAFETY: the hal device is used only within this call, while `self.device` is alive.
        let hal = unsafe { self.device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
        let dev = hal.raw_device().clone();
        // SAFETY: the instance outlives the device; only used for function pointers here.
        let instance = hal.shared_instance().raw_instance().clone();
        if hal.shared_instance().instance_api_version() < vk::API_VERSION_1_1 {
            return Err("Vulkan 1.1 is needed to import".into());
        }
        // creating an image with a modifier the device does not list for the format is invalid usage
        check_modifier(&instance, hal.raw_physical_device(), vk_format, modifier, planes.len(), (w, h))?;
        let fd_api = ash::khr::external_memory_fd::Device::new(&instance, &dev);
        let layouts: Vec<vk::SubresourceLayout> = planes
            .iter()
            .map(|p| vk::SubresourceLayout { offset: p.offset as u64, size: 0, row_pitch: p.pitch as u64, array_pitch: 0, depth_pitch: 0 })
            .collect();
        let mut modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default().drm_format_modifier(modifier).plane_layouts(&layouts);
        let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk_format)
            .extent(vk::Extent3D { width: w, height: h, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external)
            .push_next(&mut modifier_info);
        // SAFETY: a valid create info (single-plane colour format, DRM modifier tiling with one
        // explicit layout per memory plane) on a live device that enabled the extensions.
        let image = unsafe { dev.create_image(&info, None) }.map_err(|e| format!("vkCreateImage: {e}"))?;
        let destroy_image = |dev: &ash::Device| {
            // SAFETY: the image was created above, is unbound / unused and destroyed once.
            unsafe { dev.destroy_image(image, None) }
        };
        // SAFETY: valid image.
        let req = unsafe { dev.get_image_memory_requirements(image) };
        // our own reference on the buffer (Vulkan takes ownership of the fd only on success), and
        // the buffer's real size from the kernel (DMA-BUFs report it through seeking to the end)
        let fd = match object.try_clone().and_then(|fd| {
            let mut f = std::fs::File::from(fd);
            let size = std::io::Seek::seek(&mut f, std::io::SeekFrom::End(0))?;
            Ok((std::os::fd::OwnedFd::from(f), size))
        }) {
            Ok((fd, size)) if req.size <= size => fd,
            Ok((_, size)) => {
                destroy_image(&dev);
                return Err(format!("the image needs {} bytes, the buffer has {size}", req.size));
            }
            Err(e) => {
                destroy_image(&dev);
                return Err(format!("dup: {e}"));
            }
        };
        let mut fd_props = vk::MemoryFdPropertiesKHR::default();
        let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
        // SAFETY: a valid DMA-BUF fd and out-struct.
        let props = unsafe { fd_api.get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT, raw, &mut fd_props) };
        if let Err(e) = props {
            destroy_image(&dev);
            return Err(format!("vkGetMemoryFdPropertiesKHR: {e}"));
        }
        let bits = req.memory_type_bits & fd_props.memory_type_bits;
        let Some(type_index) = (0..32).find(|i| bits & (1 << i) != 0) else {
            destroy_image(&dev);
            return Err("no memory type fits the image and the buffer".into());
        };
        let raw_fd = fd.into_raw_fd();
        let mut import = vk::ImportMemoryFdInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT).fd(raw_fd);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let alloc = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index).push_next(&mut import).push_next(&mut dedicated);
        // SAFETY: imports the DMA-BUF (ownership of `raw_fd` passes to Vulkan on success) as
        // dedicated memory of `image`, with a type both allow.
        let memory = match unsafe { dev.allocate_memory(&alloc, None) } {
            Ok(m) => m,
            Err(e) => {
                // SAFETY: Vulkan did not take the fd; close our duplicate.
                drop(unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw_fd) });
                destroy_image(&dev);
                return Err(format!("vkAllocateMemory (DMA-BUF import): {e}"));
            }
        };
        // SAFETY: binds the dedicated memory to its image at offset 0, once.
        if let Err(e) = unsafe { dev.bind_image_memory(image, memory, 0) } {
            // SAFETY: free the memory and image made above, unused.
            unsafe { dev.free_memory(memory, None) };
            destroy_image(&dev);
            return Err(format!("vkBindImageMemory: {e}"));
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let hal_desc = wgpu::hal::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUses::RESOURCE,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        // SAFETY: `image` was created on this device to match `hal_desc` and has memory bound;
        // with no drop callback and `Dedicated` memory, wgpu-hal destroys both when the texture goes.
        let hal_tex = unsafe { hal.texture_from_raw(image, &hal_desc, None, wgpu::hal::vulkan::TextureMemory::Dedicated(memory)) };
        drop(hal);
        let desc = wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        // SAFETY: the hal texture was made on this device for `desc`; its memory already holds the
        // decoded picture (written by the video decoder and synced before export). UNINITIALIZED
        // lets wgpu move it from UNDEFINED to the sampled layout, which keeps the contents of a
        // DMA-BUF image without compression metadata (the decoder's surfaces).
        Ok(unsafe { self.device.create_texture_from_hal::<Vulkan>(hal_tex, &desc, wgpu::TextureUses::UNINITIALIZED) })
    }
}

/// The DRM fourccs of the luma and chroma layers of `format` (`R16` / `GR32`, `R8` / `GR88`).
fn layer_fourccs(format: GpuFormat) -> (u32, u32) {
    let fourcc = |c: &[u8; 4]| u32::from_le_bytes(*c);
    match format {
        GpuFormat::P010 => (fourcc(b"R16 "), fourcc(b"GR32")),
        GpuFormat::Nv12 => (fourcc(b"R8  "), fourcc(b"GR88")),
    }
}

/// Whether the device can make a sampled `format` image with DRM `modifier` in `planes` memory
/// planes over imported DMA-BUF memory, `w`×`h` in size (vkCreateImage requires it; the queries
/// are the ones the Vulkan spec prescribes for `VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT`).
fn check_modifier(
    instance: &ash::Instance,
    pd: vk::PhysicalDevice,
    format: vk::Format,
    modifier: u64,
    planes: usize,
    (w, h): (u32, u32),
) -> Result<(), String> {
    let count = {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut props = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: a live instance (Vulkan ≥ 1.1, checked by the caller) and physical device; the
        // list without an array only receives the count.
        unsafe { instance.get_physical_device_format_properties2(pd, format, &mut props) };
        (list.drm_format_modifier_count as usize).min(1024)
    };
    let mut mods = vec![vk::DrmFormatModifierPropertiesEXT::default(); count];
    let listed = {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut mods);
        let mut props = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: as above; the list points to `count` writable entries and says so.
        unsafe { instance.get_physical_device_format_properties2(pd, format, &mut props) };
        (list.drm_format_modifier_count as usize).min(count)
    };
    let m = mods
        .get(..listed)
        .and_then(|l| l.iter().find(|m| m.drm_format_modifier == modifier))
        .ok_or_else(|| format!("format modifier {modifier:#x} not supported for {format:?}"))?;
    if m.drm_format_modifier_plane_count as usize != planes {
        return Err(format!("modifier {modifier:#x} has {} memory planes, the export {planes}", m.drm_format_modifier_plane_count));
    }
    if !m.drm_format_modifier_tiling_features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE) {
        return Err(format!("modifier {modifier:#x} cannot be sampled as {format:?}"));
    }
    let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default().drm_format_modifier(modifier).sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .push_next(&mut external)
        .push_next(&mut drm);
    let mut external_props = vk::ExternalImageFormatProperties::default();
    let mut out = vk::ImageFormatProperties2::default().push_next(&mut external_props);
    // SAFETY: valid chained in / out structs on a live instance and physical device.
    unsafe { instance.get_physical_device_image_format_properties2(pd, &info, &mut out) }.map_err(|e| format!("modifier {modifier:#x} for {format:?}: {e}"))?;
    let max = out.image_format_properties.max_extent;
    if w > max.width || h > max.height {
        return Err(format!("{w}x{h} exceeds the device's {}x{} for this format", max.width, max.height));
    }
    if !external_props.external_memory_properties.external_memory_features.contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE) {
        return Err("DMA-BUF memory is not importable for this image".into());
    }
    Ok(())
}
