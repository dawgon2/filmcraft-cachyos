//! The wgpu compositor.
//!
//! Executes a [`FramePlan`](filmcraft_render::plan::FramePlan) on the GPU: every layer's source is
//! uploaded as textures (Y/Cb/Cr planes stay YUV — conversion to linear RGB happens per pixel in
//! the shader), drawn as a transformed quad with premultiplied "over" blending into a Rgba16Float
//! accumulator, then resolved over black into an sRGB `Rgba8UnormSrgb` texture that the UI
//! registers as a native texture. Uploads are cached by pixel-buffer identity, so a paused frame or
//! a still costs nothing.
//!
//! Every blend mode of [`filmcraft_render::Blend`] runs here. Normal and Dissolve (whose per-pixel
//! pattern is the CPU's 64-bit hash, reproduced in WGSL) use fixed-function "over" blending. The
//! other 25 modes need the colour under the layer: before such a layer is drawn, the accumulator
//! region under its quad is copied into a backdrop texture, and the fragment shader composites
//! with the CPU reference's math (`filmcraft_render::blend::composite`: sRGB-encoded straight
//! colour, W3C formulas, back to linear) and writes the result. Only those layers pay the copy.
//!
//! A layer carrying standard effects ([`filmcraft_render::plan::LayerFx`]) goes through the
//! effect stage ([`fx`]) first: its source is drawn into an `Rgba32Float` working image, the
//! effects run as compute passes in WGSL with the CPU reference's math
//! (`filmcraft_render::gpufx::FxOp::apply`), and the result is drawn like any RGBA layer. Layers
//! without effects take the single draw above.
//!
//! Layers that need converting before upload (linear f32 RGBA from CPU-rendered layers, 16-bit
//! YUV such as ProRes) are converted to half floats by [`prepare`], which frame workers run off the
//! UI thread; [`GpuCompositor::composite_prepared`] then only copies bytes into textures.
//!
//! The CPU plan executor (`filmcraft_render::plan::execute_cpu`) is the oracle; tests compare.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::collections::HashMap;
use std::sync::Arc;

use rayon::prelude::*;

use filmcraft_color::{Matrix, Range, Transfer};
use filmcraft_frame::{PixelData, VideoFrame};
use filmcraft_render::Blend;
use filmcraft_render::plan::{FramePlan, PlanLayer};

pub mod colorops;
pub mod fx;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod interop;
pub mod lut;
pub mod mask;
pub mod scopes;
pub use lut::GpuLut;
pub use mask::GpuMask;

/// Output texture format: gamma-encoded RGBA8 (what egui expects of native textures); the resolve
/// shader applies the sRGB encoding.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ACCUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

struct Uploaded {
    /// Y, Cb, Cr (or the RGBA texture) and the alpha plane (the dummy texture when there is none).
    views: [wgpu::TextureView; 4],
    kind: u32,
    code_scale: f32,
    /// Multiplier from a loaded Y'CbCr texel to its code (see [`SrcInfo::texel_scale`]).
    texel_scale: f32,
    /// Multiplier from the alpha texture's sample to alpha 0..1; 0 when the frame has no alpha plane.
    alpha_scale: f32,
    chroma: (u32, u32),
    last_used: u64,
    /// The uploaded pixel buffers, kept alive while cached: the cache key is the buffer address,
    /// and a freed buffer's address can be reused by a different frame (stale texture).
    _pixels: PixelData,
}

pub struct GpuCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layer_pipeline: wgpu::RenderPipeline,
    /// Blend modes that read the destination: `fs_blend`, no fixed-function blending.
    blend_pipeline: wgpu::RenderPipeline,
    final_pipeline: wgpu::RenderPipeline,
    layer_bgl: wgpu::BindGroupLayout,
    blend_bgl: wgpu::BindGroupLayout,
    final_bgl: wgpu::BindGroupLayout,
    accum: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    /// Copy of the accumulator under a blend-mode layer (allocated on first use).
    backdrop: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    output: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    uploads: HashMap<(usize, u32, u32), Uploaded>,
    clock: u64,
    dummy: wgpu::TextureView,
    /// The effect stage (None on devices without compute shaders or storage textures: layers
    /// with effects are then rendered on the CPU here).
    fx: Option<fx::FxStage>,
    /// Maximum Render Quality: minified layers are resampled through a Lanczos-3 filter.
    high_quality: bool,
    /// Scope samples of the output (compute devices only).
    scope_sampler: Option<scopes::ScopeSampler>,
    /// The output transform of colour-managed sequences (with the effect stage).
    out_pass: Option<colorops::OutputPass>,
    /// Where the output pass writes (then copied back into the accumulator).
    post: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    /// Total bytes uploaded (stats).
    pub uploaded_bytes: u64,
    /// The device has `TEXTURE_FORMAT_16BIT_NORM`: 9–16-bit planes upload as they are
    /// (`R16Unorm` / `Rg16Unorm`) instead of being converted to half floats on the CPU.
    unorm16: bool,
    /// Imports hardware-decoded pictures in place (Linux, a device from `interop::create_device`).
    #[cfg(target_os = "linux")]
    importer: Option<interop::Importer>,
    /// GPU pictures drawn in place / read back to memory first (stats).
    pub gpu_frames_in_place: u64,
    pub gpu_frames_read_back: u64,
}

/// Whether the last compositor made has 16-bit normalized textures: [`prepare`] (run on frame
/// workers, away from the device) then leaves 9–16-bit planes alone. Only a hint for speed: a
/// compositor ignores half-float planes it was handed when it uploads 16-bit ones as they are,
/// and converts planes it was not handed.
static UNORM16: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The device features a [`GpuCompositor`] uses when the adapter has them: request these when
/// creating the device (`adapter.features() & filmcraft_gpu::wanted_features()`).
pub fn wanted_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// f32 → IEEE half (round to nearest even), no dependency needed.
pub fn f32_to_f16(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (mant | 0x80_0000) >> (1 - e);
        let round = (m >> 12) & 1;
        return sign | (((m >> 13) + round) as u16);
    }
    let m = mant >> 13;
    let round_bits = mant & 0x1fff;
    let mut h = (sign as u32) | ((e as u32) << 10) | m;
    if round_bits > 0x1000 || (round_bits == 0x1000 && (m & 1) == 1) {
        h += 1;
    }
    h as u16
}

/// Half-float texel data for one frame (one byte vector per plane), converted off the UI thread.
/// 8-bit frames need no conversion and have none.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    planes: Vec<Vec<u8>>,
}

impl Prepared {
    pub fn bytes(&self) -> usize {
        self.planes.iter().map(Vec::len).sum()
    }
}

/// [`Prepared`] data for every layer of a plan (index-aligned with its layers), or for the
/// fallback image of a [`FramePlan::Image`].
#[derive(Clone, Debug, Default)]
pub struct PreparedPlan {
    layers: Vec<Option<Prepared>>,
    image: Option<Prepared>,
}

impl PreparedPlan {
    pub fn bytes(&self) -> usize {
        self.layers.iter().flatten().chain(&self.image).map(Prepared::bytes).sum()
    }
}

/// Values per parallel chunk when converting.
const CHUNK: usize = 1 << 15;

fn f32_to_f16_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; v.len() * 2];
    out.par_chunks_mut(CHUNK * 2).zip(v.par_chunks(CHUNK)).for_each(|(o, s)| {
        for (o, x) in o.as_chunks_mut::<2>().0.iter_mut().zip(s) {
            o.copy_from_slice(&f32_to_f16(*x).to_le_bytes());
        }
    });
    out
}

/// Half floats of `code / 2^bits` for every 16-bit code (built once per bit depth).
fn code_table(bits: u32) -> &'static [[u8; 2]] {
    static TABLES: [std::sync::OnceLock<Vec<[u8; 2]>>; 17] = [const { std::sync::OnceLock::new() }; 17];
    let bits = bits.min(16);
    TABLES[bits as usize].get_or_init(|| {
        let scale = (1u32 << bits) as f32;
        (0..=u16::MAX as u32).map(|c| f32_to_f16(c as f32 / scale).to_le_bytes()).collect()
    })
}

/// `code / 2^bits` as half floats, through a per-code table (same values as converting each sample).
fn codes_to_f16_bytes(v: &[u16], bits: u32) -> Vec<u8> {
    let table = code_table(bits);
    let mut out = vec![0u8; v.len() * 2];
    out.par_chunks_mut(CHUNK * 2).zip(v.par_chunks(CHUNK)).for_each(|(o, s)| {
        for (o, c) in o.as_chunks_mut::<2>().0.iter_mut().zip(s) {
            o.copy_from_slice(&table[*c as usize]);
        }
    });
    out
}

/// Convert a frame's texels for upload (None when it uploads as it is).
pub fn prepare_frame(f: &VideoFrame) -> Option<Prepared> {
    prepare_frame_for(f, UNORM16.load(std::sync::atomic::Ordering::Relaxed))
}

/// [`prepare_frame`] for a compositor with (`unorm16`) or without 16-bit normalized textures.
fn prepare_frame_for(f: &VideoFrame, unorm16: bool) -> Option<Prepared> {
    match &f.data {
        PixelData::RgbaF32(d) => Some(Prepared { planes: vec![f32_to_f16_bytes(d)] }),
        PixelData::Yuv16 { .. } | PixelData::Yuv16Bi { .. } if unorm16 => None,
        PixelData::Yuv16 { planes, bits, alpha, .. } => {
            Some(Prepared { planes: planes.iter().chain(alpha.as_ref()).map(|p| codes_to_f16_bytes(p, *bits)).collect() })
        }
        // MSB-aligned samples: sample / 65536 = code / 2^bits, exact in a half float
        PixelData::Yuv16Bi { y, uv, .. } => Some(Prepared { planes: vec![codes_to_f16_bytes(y, 16), codes_to_f16_bytes(uv, 16)] }),
        PixelData::Rgba8(_) | PixelData::Yuv8 { .. } | PixelData::Gpu(_) => None,
    }
}

/// Convert every layer of a plan for upload. Thread-safe and GPU-free: run it on a worker.
pub fn prepare(plan: &FramePlan) -> PreparedPlan {
    match plan {
        FramePlan::Layers { layers, .. } => PreparedPlan { layers: layers.iter().map(|l| prepare_frame(&l.frame)).collect(), image: None },
        FramePlan::Image(img) => PreparedPlan { layers: Vec::new(), image: Some(Prepared { planes: vec![f32_to_f16_bytes(&img.px)] }) },
    }
}

impl GpuCompositor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let unorm16 = device.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM);
        UNORM16.store(unorm16, std::sync::atomic::Ordering::Relaxed);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filmcraft-composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });
        let layer_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(5),
            ],
        });
        let blend_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend-layer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
            ],
        });
        let final_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("final"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, ..tex_entry(0) }],
        });
        let pl =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("layer"), bind_group_layouts: &[Some(&layer_bgl)], immediate_size: 0 });
        let layer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("layer"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ACCUM_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blend-layer"),
            bind_group_layouts: &[Some(&blend_bgl)],
            immediate_size: 0,
        });
        let blend_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blend-layer"),
            layout: Some(&bpl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_blend"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: ACCUM_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let fpl =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("final"), bind_group_layouts: &[Some(&final_bgl)], immediate_size: 0 });
        let final_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("final"),
            layout: Some(&fpl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_full"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: OUTPUT_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let dummy_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy = dummy_tex.create_view(&Default::default());
        let fx = fx::FxStage::supported(device).then(|| fx::FxStage::new(device, &shader, &layer_bgl));
        let out_pass = fx.is_some().then(|| colorops::OutputPass::new(device));
        let scope_sampler = fx.is_some().then(|| scopes::ScopeSampler::new(device));
        Self {
            device: device.clone(),
            queue: queue.clone(),
            layer_pipeline,
            blend_pipeline,
            final_pipeline,
            layer_bgl,
            blend_bgl,
            final_bgl,
            accum: None,
            backdrop: None,
            output: None,
            uploads: HashMap::new(),
            clock: 0,
            dummy,
            fx,
            high_quality: false,
            scope_sampler,
            out_pass,
            post: None,
            uploaded_bytes: 0,
            unorm16,
            #[cfg(target_os = "linux")]
            importer: interop::Importer::new(device),
            gpu_frames_in_place: 0,
            gpu_frames_read_back: 0,
        }
    }

    fn target(
        device: &wgpu::Device,
        slot: &mut Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
        w: u32,
        h: u32,
        format: wgpu::TextureFormat,
        extra: wgpu::TextureUsages,
    ) -> wgpu::TextureView {
        if let Some(s) = slot.as_ref().filter(|s| s.2 == (w, h)) {
            return s.1.clone();
        }
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filmcraft-target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | extra,
            view_formats: &[],
        });
        let v = t.create_view(&Default::default());
        *slot = Some((t, v.clone(), (w, h)));
        v
    }

    fn plane_texture(&mut self, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8], bpp: u32) -> wgpu::TextureView {
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filmcraft-plane"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.uploaded_bytes += bytes.len() as u64;
        t.create_view(&Default::default())
    }

    /// Upload (or reuse) the textures of a frame, using `prep` when it was converted beforehand.
    fn upload(&mut self, f: &VideoFrame, prep: Option<&Prepared>) -> (usize, u32, u32) {
        let id = match &f.data {
            PixelData::Rgba8(d) => Arc::as_ptr(d) as *const u8 as usize,
            PixelData::RgbaF32(d) => Arc::as_ptr(d) as *const u8 as usize,
            PixelData::Yuv8 { planes, .. } => Arc::as_ptr(&planes[0]) as *const u8 as usize,
            PixelData::Yuv16 { planes, .. } => Arc::as_ptr(&planes[0]) as *const u8 as usize,
            PixelData::Yuv16Bi { y, .. } => Arc::as_ptr(y) as *const u8 as usize,
            PixelData::Gpu(s) => Arc::as_ptr(s) as *const u8 as usize,
        };
        let key = (id, f.width, f.height);
        self.clock += 1;
        if let Some(u) = self.uploads.get_mut(&key) {
            u.last_used = self.clock;
            return key;
        }
        let (w, h) = (f.width, f.height);
        let d0 = self.dummy.clone();
        let dummy = || d0.clone();
        let up = match &f.data {
            PixelData::Gpu(s) => {
                let (bits, texel) = match s.format() {
                    filmcraft_frame::GpuFormat::P010 => (10u32, 65535.0 / 64.0),
                    filmcraft_frame::GpuFormat::Nv12 => (8, 255.0),
                };
                #[cfg(target_os = "linux")]
                let imported = match self.importer.as_mut() {
                    Some(i) if s.crop_origin() == (0, 0) => i.import(s.as_ref()).map(|(a, b)| (a.clone(), b.clone())),
                    _ => None,
                };
                #[cfg(not(target_os = "linux"))]
                let imported: Option<(wgpu::TextureView, wgpu::TextureView)> = None;
                match imported {
                    Some((y, uv)) => {
                        self.gpu_frames_in_place += 1;
                        Uploaded {
                            views: [y, uv, dummy(), dummy()],
                            kind: 3,
                            code_scale: (1u32 << bits) as f32,
                            texel_scale: texel,
                            alpha_scale: 0.0,
                            chroma: (w.div_ceil(2), h.div_ceil(2)),
                            last_used: self.clock,
                            _pixels: f.data.clone(),
                        }
                    }
                    None => {
                        // read back and upload as a biplanar / planar frame
                        self.gpu_frames_read_back += 1;
                        let data = s.download(w, h).unwrap_or_else(|e| {
                            log::warn!("reading a decoded picture back failed: {e}");
                            let c = (w as usize).div_ceil(2) * (h as usize).div_ceil(2);
                            PixelData::Yuv8 {
                                planes: [Arc::new(vec![16; (w * h) as usize]), Arc::new(vec![128; c]), Arc::new(vec![128; c])],
                                chroma: filmcraft_frame::Chroma::C420,
                                alpha: None,
                            }
                        });
                        let tmp = VideoFrame { data, ..f.clone() };
                        let k = self.upload(&tmp, None);
                        match self.uploads.remove(&k) {
                            Some(mut u) => {
                                // keep the surface alive with the textures, as for other frames
                                u._pixels = f.data.clone();
                                u
                            }
                            None => return key,
                        }
                    }
                }
            }
            PixelData::Rgba8(d) => {
                let v = self.plane_texture(w, h, wgpu::TextureFormat::Rgba8UnormSrgb, d, 4);
                Uploaded {
                    views: [v, dummy(), dummy(), dummy()],
                    kind: 0,
                    code_scale: 1.0,
                    texel_scale: 1.0,
                    alpha_scale: 0.0,
                    chroma: (w, h),
                    last_used: self.clock,
                    _pixels: f.data.clone(),
                }
            }
            PixelData::RgbaF32(d) => {
                let owned;
                let half = match prep {
                    Some(p) => &p.planes[0],
                    None => {
                        owned = f32_to_f16_bytes(d);
                        &owned
                    }
                };
                let v = self.plane_texture(w, h, wgpu::TextureFormat::Rgba16Float, half, 8);
                Uploaded {
                    views: [v, dummy(), dummy(), dummy()],
                    kind: 1,
                    code_scale: 1.0,
                    texel_scale: 1.0,
                    alpha_scale: 0.0,
                    chroma: (w, h),
                    last_used: self.clock,
                    _pixels: f.data.clone(),
                }
            }
            PixelData::Yuv8 { planes, chroma, alpha } => {
                let (sx, sy) = chroma.shifts();
                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));
                let y = self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, &planes[0], 1);
                let u = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[1], 1);
                let v = self.plane_texture(cw, ch, wgpu::TextureFormat::R8Unorm, &planes[2], 1);
                let a = alpha.as_ref().map(|a| self.plane_texture(w, h, wgpu::TextureFormat::R8Unorm, a, 1));
                let alpha_scale = if a.is_some() { 1.0 } else { 0.0 };
                Uploaded {
                    views: [y, u, v, a.unwrap_or_else(dummy)],
                    kind: 2,
                    code_scale: 256.0,
                    texel_scale: 255.0,
                    alpha_scale,
                    chroma: (cw, ch),
                    last_used: self.clock,
                    _pixels: f.data.clone(),
                }
            }
            PixelData::Yuv16 { planes, chroma, bits, alpha } => {
                let (sx, sy) = chroma.shifts();
                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));
                let bits = (*bits).clamp(1, 16);
                let scale = (1u32 << bits) as f32;
                // codes in the low bits: R16Unorm loads code / 65535, half floats code / 2^bits
                let (views, a, texel_scale) = if self.unorm16 {
                    let f = wgpu::TextureFormat::R16Unorm;
                    let y = self.plane_texture(w, h, f, bytemuck::cast_slice(&planes[0]), 2);
                    let u = self.plane_texture(cw, ch, f, bytemuck::cast_slice(&planes[1]), 2);
                    let v = self.plane_texture(cw, ch, f, bytemuck::cast_slice(&planes[2]), 2);
                    let a = alpha.as_ref().map(|a| self.plane_texture(w, h, f, bytemuck::cast_slice(a), 2));
                    ([y, u, v], a, 65535.0)
                } else {
                    let owned;
                    let p = match prep.filter(|p| p.planes.len() == 3 + alpha.is_some() as usize) {
                        Some(p) => p,
                        None => {
                            owned = Prepared { planes: planes.iter().chain(alpha.as_ref()).map(|p| codes_to_f16_bytes(p, bits)).collect() };
                            &owned
                        }
                    };
                    let f = wgpu::TextureFormat::R16Float;
                    let y = self.plane_texture(w, h, f, &p.planes[0], 2);
                    let u = self.plane_texture(cw, ch, f, &p.planes[1], 2);
                    let v = self.plane_texture(cw, ch, f, &p.planes[2], 2);
                    let a = alpha.as_ref().and_then(|_| p.planes.get(3)).map(|b| self.plane_texture(w, h, f, b, 2));
                    ([y, u, v], a, scale)
                };
                // (full-resolution alpha plane: its codes span 0..2^bits - 1, hence the scale; the
                // shader multiplies its texel by alpha_scale: texel × texel_scale / (2^bits - 1))
                let alpha_scale = if a.is_some() { texel_scale / (scale - 1.0) } else { 0.0 };
                let [y, u, v] = views;
                Uploaded {
                    views: [y, u, v, a.unwrap_or_else(dummy)],
                    kind: 2,
                    code_scale: scale,
                    texel_scale,
                    alpha_scale,
                    chroma: (cw, ch),
                    last_used: self.clock,
                    _pixels: f.data.clone(),
                }
            }
            PixelData::Yuv16Bi { y, uv, chroma, bits } => {
                let (sx, sy) = chroma.shifts();
                let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));
                let bits = (*bits).clamp(1, 16);
                let scale = (1u32 << bits) as f32;
                // MSB-aligned: R16Unorm loads (code << (16 - bits)) / 65535, half floats
                // (code << (16 - bits)) / 65536
                let lsb = (1u32 << (16 - bits)) as f32;
                let (yv, uvv, texel_scale) = if self.unorm16 {
                    let yv = self.plane_texture(w, h, wgpu::TextureFormat::R16Unorm, bytemuck::cast_slice(y), 2);
                    let uvv = self.plane_texture(cw, ch, wgpu::TextureFormat::Rg16Unorm, bytemuck::cast_slice(uv), 4);
                    (yv, uvv, 65535.0 / lsb)
                } else {
                    let owned;
                    let p = match prep.filter(|p| p.planes.len() == 2) {
                        Some(p) => p,
                        None => {
                            owned = Prepared { planes: vec![codes_to_f16_bytes(y, 16), codes_to_f16_bytes(uv, 16)] };
                            &owned
                        }
                    };
                    let yv = self.plane_texture(w, h, wgpu::TextureFormat::R16Float, &p.planes[0], 2);
                    let uvv = self.plane_texture(cw, ch, wgpu::TextureFormat::Rg16Float, &p.planes[1], 4);
                    (yv, uvv, 65536.0 / lsb)
                };
                Uploaded {
                    views: [yv, uvv, dummy(), dummy()],
                    kind: 3,
                    code_scale: scale,
                    texel_scale,
                    alpha_scale: 0.0,
                    chroma: (cw, ch),
                    last_used: self.clock,
                    _pixels: f.data.clone(),
                }
            }
        };
        self.uploads.insert(key, up);
        // keep the most recent uploads (enough for several stacked layers + playback lookahead)
        if self.uploads.len() > 24 {
            let mut v: Vec<(u64, (usize, u32, u32))> = self.uploads.iter().map(|(k, u)| (u.last_used, *k)).collect();
            v.sort_unstable();
            for (_, k) in v.into_iter().take(self.uploads.len() - 24) {
                self.uploads.remove(&k);
            }
        }
        key
    }

    /// The source description of an uploaded frame.
    fn src_info(&self, f: &VideoFrame, key: (usize, u32, u32)) -> Option<SrcInfo> {
        let up = self.uploads.get(&key)?;
        Some(SrcInfo {
            kind: up.kind,
            code_scale: up.code_scale,
            texel_scale: up.texel_scale,
            alpha: up.alpha_scale,
            chroma: up.chroma,
            size: (f.width, f.height),
            color: f.color,
        })
    }

    /// Maximum Render Quality on or off (the Program monitor sets it from the sequence while
    /// paused; playback keeps the cheaper supersampling).
    pub fn set_high_quality(&mut self, on: bool) {
        self.high_quality = on;
    }

    /// A layer's uniforms with the resample quality: Lanczos-3 when minifying with Maximum Render
    /// Quality (`p3.w`; `p2.w` is the footprint).
    fn with_quality(&self, mut un: [f32; 28]) -> [f32; 28] {
        if self.high_quality && un[23] > filmcraft_render::image::HQ_MIN_FOOTPRINT {
            un[27] = 1.0;
        }
        un
    }

    fn uniforms(src: &SrcInfo, m: &filmcraft_geom::Affine, opacity: f32, blend: Blend, out: (u32, u32)) -> [f32; 28] {
        let (kr, kb) = src.color.matrix.kr_kb();
        // code units relative to 8-bit (2^(bits - 8)), as `filmcraft_color::normalize_y` / `_c`
        let bits_scale = src.code_scale / 256.0;
        let (yo, ys, co, cs) = match (src.color.range, src.kind) {
            (Range::Limited, 2 | 3) => (16.0 * bits_scale, 219.0 * bits_scale, 128.0 * bits_scale, 224.0 * bits_scale),
            (Range::Full, 2 | 3) => (0.0, src.code_scale - 1.0, src.code_scale / 2.0, src.code_scale - 1.0),
            _ => (0.0, 1.0, 0.0, 1.0),
        };
        let transfer = match src.color.transfer {
            Transfer::Linear => 1.0,
            Transfer::Pq => 2.0,
            Transfer::Hlg => 3.0,
            _ => 0.0,
        };
        // source pixels per output pixel → supersampling taps
        let sx = (m.a * m.a + m.b * m.b).sqrt();
        let sy = (m.c * m.c + m.d * m.d).sqrt();
        let footprint = (1.0 / sx.min(sy).max(1e-6)) as f32;
        let taps = if footprint > 1.25 { footprint.ceil().min(4.0) } else { 1.0 };
        let _ = Matrix::Bt709;
        [
            m.a as f32,
            m.b as f32,
            m.c as f32,
            m.d as f32,
            m.e as f32,
            m.f as f32,
            out.0 as f32,
            out.1 as f32,
            src.size.0 as f32,
            src.size.1 as f32,
            src.chroma.0 as f32,
            src.chroma.1 as f32,
            opacity,
            src.kind as f32,
            taps,
            transfer,
            yo,
            ys,
            co,
            cs,
            kr,
            kb,
            src.code_scale,
            footprint.max(1.0),
            blend.index() as f32,
            src.alpha,
            src.texel_scale,
            0.0,
        ]
    }

    fn uniform_buffer(&self, u: &[f32; 28]) -> wgpu::Buffer {
        let bytes: Vec<u8> = u.iter().flat_map(|v| v.to_le_bytes()).collect();
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer-u"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf, 0, &bytes);
        buf
    }

    /// Composite a plan; returns the output view (sRGB, over black) and its size.
    pub fn composite(&mut self, plan: &FramePlan) -> (wgpu::TextureView, (u32, u32)) {
        self.composite_prepared(plan, None)
    }

    /// [`composite`](Self::composite) with texel conversions already done by [`prepare`] (the
    /// result is identical; only the upload work on this thread differs).
    pub fn composite_prepared(&mut self, plan: &FramePlan, prep: Option<&PreparedPlan>) -> (wgpu::TextureView, (u32, u32)) {
        // a colour-managed sequence's output transform needs the compute stage: without it, the
        // whole frame is made on the CPU
        if let FramePlan::Layers { output: Some(_), .. } = plan
            && self.out_pass.is_none()
        {
            return self.composite_prepared(&FramePlan::Image(filmcraft_render::plan::execute_cpu(plan)), None);
        }
        let output = match plan {
            FramePlan::Layers { output, .. } => output.clone(),
            FramePlan::Image(_) => None,
        };
        let owned;
        let (w, h, layers): (u32, u32, &[PlanLayer]) = match plan {
            FramePlan::Layers { width, height, layers, .. } => (*width as u32, *height as u32, layers.as_slice()),
            FramePlan::Image(img) => {
                let frame = match prep.and_then(|p| p.image.as_ref()) {
                    // The texels come from `prep`; the frame only carries size and colour.
                    Some(_) => VideoFrame {
                        width: img.w as u32,
                        height: img.h as u32,
                        data: PixelData::RgbaF32(Arc::new(Vec::new())),
                        ..VideoFrame::rgba_f32(1, 1, vec![0.0; 4])
                    },
                    None => VideoFrame::rgba_f32(img.w as u32, img.h as u32, img.px.clone()),
                };
                owned = [PlanLayer { frame: Arc::new(frame), matrix: filmcraft_geom::Affine::IDENTITY, opacity: 1.0, blend: Blend::Normal, fx: None }];
                (img.w as u32, img.h as u32, &owned[..])
            }
        };
        let (w, h) = (w.max(1), h.max(1));
        let accum_view = Self::target(&self.device, &mut self.accum, w, h, ACCUM_FORMAT, wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST);
        let post_view = output
            .as_ref()
            .map(|_| Self::target(&self.device, &mut self.post, w, h, ACCUM_FORMAT, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC));
        let backdrop_view = if layers.iter().any(|l| l.blend.reads_destination()) {
            Some(Self::target(&self.device, &mut self.backdrop, w, h, ACCUM_FORMAT, wgpu::TextureUsages::COPY_DST))
        } else {
            None
        };
        let out_view = Self::target(&self.device, &mut self.output, w, h, OUTPUT_FORMAT, wgpu::TextureUsages::COPY_SRC);
        // Layers whose effects the GPU stage can't run (no stage on this device, or a working image
        // larger than a texture) are rendered on the CPU here and drawn as plain RGBA layers.
        let max_side = self.device.limits().max_texture_dimension_2d;
        let cpu_fx: Vec<Option<PlanLayer>> = layers
            .iter()
            .map(|l| {
                let lfx = l.fx.as_ref()?;
                if self.fx.is_some() && lfx.size.0.max(lfx.size.1) <= max_side {
                    return None;
                }
                let img = filmcraft_render::plan::effect_image(&l.frame, lfx);
                Some(PlanLayer::new(Arc::new(VideoFrame::rgba_f32(img.w as u32, img.h as u32, img.px)), l.matrix, l.opacity, l.blend))
            })
            .collect();
        let resolved: Vec<&PlanLayer> = layers.iter().zip(&cpu_fx).map(|(l, c)| c.as_ref().unwrap_or(l)).collect();
        let layers = resolved;
        let keys: Vec<(usize, u32, u32)> = layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let p = prep.and_then(|p| if matches!(plan, FramePlan::Image(_)) { p.image.as_ref() } else { p.layers.get(i).and_then(Option::as_ref) });
                // (texels converted beforehand belong to the plan's own frame)
                let p = if cpu_fx.get(i).is_some_and(Option::is_some) { None } else { p };
                self.upload(&l.frame, p)
            })
            .collect();
        // (bind group, None for a fixed-function layer or Some(region of the accumulator to copy
        // into the backdrop) for a layer that reads the destination; Some(None): off the output)
        let mut bind_groups = Vec::with_capacity(layers.len());
        // the effect stage of each layer with effects (recorded right before its draw)
        let mut jobs: Vec<Option<fx::FxJob>> = Vec::with_capacity(layers.len());
        if let Some(f) = self.fx.as_mut() {
            f.begin_frame();
        }
        for (l, k) in layers.iter().zip(&keys) {
            // (every frame was just uploaded; a missing upload draws transparent texels)
            let src = self.src_info(&l.frame, *k).unwrap_or(SrcInfo {
                kind: 1,
                code_scale: 1.0,
                texel_scale: 1.0,
                alpha: 0.0,
                chroma: (1, 1),
                size: (l.frame.width, l.frame.height),
                color: l.frame.color,
            });
            let views = self.uploads.get(k).map_or_else(|| std::array::from_fn(|_| self.dummy.clone()), |up| up.views.clone());
            let job = match &l.fx {
                Some(lfx) => {
                    // the source drawn onto the working image (box-decimated by n, as the CPU decodes)
                    let n = lfx.decimation.max(1) as f64;
                    let su = Self::uniforms(&src, &filmcraft_geom::Affine::scale(1.0 / n, 1.0 / n), 1.0, Blend::Normal, (lfx.size.0.max(1), lfx.size.1.max(1)));
                    let sbuf = self.uniform_buffer(&su);
                    let sbg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("fx-source"),
                        layout: &self.layer_bgl,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: sbuf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[1]) },
                            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[2]) },
                            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&views[3]) },
                        ],
                    });
                    match self.fx.as_mut() {
                        Some(f) => f.job(&self.device, &self.queue, lfx, sbg, &self.dummy),
                        None => None,
                    }
                }
                None => None,
            };
            // what the layer draws: the effect result (linear premultiplied RGBA) or the frame
            let (u, tex) = match &job {
                Some(j) => {
                    let s = SrcInfo { kind: 1, code_scale: 1.0, texel_scale: 1.0, alpha: 0.0, chroma: l.size(), size: l.size(), color: l.frame.color };
                    (
                        self.with_quality(Self::uniforms(&s, &l.matrix, l.opacity, l.blend, (w, h))),
                        [j.result.clone(), self.dummy.clone(), self.dummy.clone(), self.dummy.clone()],
                    )
                }
                None => {
                    // (a layer with effects only gets here if its job failed, which the CPU
                    // fallback above rules out; it is then drawn without them, where the working
                    // image would be)
                    let m = match &l.fx {
                        Some(lfx) => {
                            let n = 1.0 / lfx.decimation.max(1) as f64;
                            l.matrix.then_apply(&filmcraft_geom::Affine::scale(n, n))
                        }
                        None => l.matrix,
                    };
                    (self.with_quality(Self::uniforms(&src, &m, l.opacity, l.blend, (w, h))), views)
                }
            };
            jobs.push(job);
            let buf = self.uniform_buffer(&u);
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&tex[0]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&tex[1]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&tex[2]) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&tex[3]) },
            ];
            let (layout, region) = match (&backdrop_view, l.blend.reads_destination()) {
                (Some(b), true) => {
                    entries.push(wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(b) });
                    (&self.blend_bgl, Some(quad_bounds(l, w, h)))
                }
                _ => (&self.layer_bgl, None),
            };
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("layer"), layout, entries: &entries });
            bind_groups.push((bg, region));
        }
        if let Some(f) = self.fx.as_mut() {
            f.end_frame();
        }
        let final_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("final"),
            layout: &self.final_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&accum_view) }],
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filmcraft-composite") });
        // Runs of fixed-function layers draw in one pass. A layer that reads the destination
        // ends it, copies the accumulator under its quad into the backdrop and draws on its own.
        let accum_tex = self.accum.as_ref().map(|a| a.0.clone());
        let backdrop_tex = self.backdrop.as_ref().map(|b| b.0.clone());
        let post_tex = self.post.as_ref().map(|p| p.0.clone());
        let mut cleared = false;
        let mut converted = output.is_none();
        let mut i = 0;
        while i < bind_groups.len() || !cleared {
            // the output transform, before the first display-referred layer (captions)
            if let (false, true, Some(o)) = (converted, cleared, output.as_ref())
                && o.at <= i
            {
                converted = true;
                self.record_output(&mut enc, &accum_view, post_view.as_ref(), (accum_tex.as_ref(), post_tex.as_ref()), (w, h), &o.ops);
            }
            // a layer's effects run right before it is drawn (pooled working textures are reused
            // by the next layer with effects)
            if let (Some(Some(job)), Some(f)) = (jobs.get(i), self.fx.as_ref()) {
                f.record(&mut enc, job);
            }
            if let Some((bg, Some(region))) = bind_groups.get(i) {
                i += 1;
                let (Some(rect), Some(src), Some(dst)) = (region, &accum_tex, &backdrop_tex) else { continue };
                if !cleared {
                    accum_pass(&mut enc, &accum_view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
                    cleared = true;
                }
                let origin = wgpu::Origin3d { x: rect.0, y: rect.1, z: 0 };
                enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { texture: src, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyTextureInfo { texture: dst, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
                    wgpu::Extent3d { width: rect.2, height: rect.3, depth_or_array_layers: 1 },
                );
                let mut pass = accum_pass(&mut enc, &accum_view, wgpu::LoadOp::Load);
                pass.set_pipeline(&self.blend_pipeline);
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..6, 0..1);
                continue;
            }
            // a run of fixed-function layers, up to the next one that reads the destination or
            // has effects to run first
            let breaks = |j: &usize| bind_groups.get(*j).is_some_and(|(_, r)| r.is_some()) || jobs.get(*j).is_some_and(Option::is_some);
            let end = (i + 1..bind_groups.len()).find(breaks).unwrap_or(bind_groups.len()).max(i);
            let load = if cleared { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) };
            let mut pass = accum_pass(&mut enc, &accum_view, load);
            cleared = true;
            pass.set_pipeline(&self.layer_pipeline);
            for (bg, _) in bind_groups.get(i..end).unwrap_or_default() {
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..6, 0..1);
            }
            i = end;
        }
        if let (false, Some(o)) = (converted, output.as_ref()) {
            self.record_output(&mut enc, &accum_view, post_view.as_ref(), (accum_tex.as_ref(), post_tex.as_ref()), (w, h), &o.ops);
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.final_pipeline);
            pass.set_bind_group(0, &final_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        (out_view, (w, h))
    }

    /// The output transform on the accumulator: into `post`, then copied back.
    fn record_output(
        &self,
        enc: &mut wgpu::CommandEncoder,
        accum: &wgpu::TextureView,
        post: Option<&wgpu::TextureView>,
        (accum_tex, post_tex): (Option<&wgpu::Texture>, Option<&wgpu::Texture>),
        (w, h): (u32, u32),
        ops: &[filmcraft_color::ops::ColorOp],
    ) {
        let (Some(pass), Some(post), Some(src), Some(dst)) = (self.out_pass.as_ref(), post, post_tex, accum_tex) else { return };
        if pass.record(&self.device, &self.queue, enc, accum, post, (w, h), ops).is_none() {
            log::warn!("output transform of {} ops too large for the GPU program; shown unconverted", ops.len());
            return;
        }
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: dst, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// Run the effect stage of a layer alone: `frame` decoded into its working image and `fx`
    /// applied, read back as linear premultiplied RGBA f32 (what `LayerFx` ops produce on the
    /// CPU from the same frame). None when the working image does not fit in a texture.
    pub fn effect_image(&mut self, frame: &VideoFrame, fx: &filmcraft_render::plan::LayerFx) -> Option<(u32, u32, Vec<f32>)> {
        let key = self.upload(frame, None);
        let src = self.src_info(frame, key)?;
        let views = self.uploads.get(&key)?.views.clone();
        let n = fx.decimation.max(1) as f64;
        let (w, h) = (fx.size.0.max(1), fx.size.1.max(1));
        let su = Self::uniforms(&src, &filmcraft_geom::Affine::scale(1.0 / n, 1.0 / n), 1.0, Blend::Normal, (w, h));
        let sbuf = self.uniform_buffer(&su);
        let sbg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-source"),
            layout: &self.layer_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: sbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[1]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[2]) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&views[3]) },
            ],
        });
        let stage = self.fx.as_mut()?;
        stage.begin_frame();
        let job = stage.job(&self.device, &self.queue, fx, sbg, &self.dummy)?;
        let row = (w * 16).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        stage.record(&mut enc, &job);
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &job.result_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        stage.end_frame();
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let r = data.get((y * row) as usize..(y * row + w * 16) as usize)?;
            out.extend(r.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
        }
        drop(data);
        buf.unmap();
        Some((w, h, out))
    }

    /// Start reading back the scope samples of the last composited picture (at most `max_w` ×
    /// `max_h`, picked as `filmcraft_scopes::Signal` picks them), without waiting. None without
    /// an output or on devices without compute shaders.
    pub fn request_scope_samples(&self, max_w: u32, max_h: u32) -> Option<scopes::ScopeReadback> {
        let (_, view, size) = self.output.as_ref()?;
        self.scope_sampler.as_ref()?.request(&self.device, &self.queue, view, *size, (max_w, max_h))
    }

    /// Read the output back as RGBA8 (tests / screenshots / thumbnails).
    pub fn read_output(&self) -> Option<(u32, u32, Vec<u8>)> {
        let (tex, _, (w, h)) = self.output.as_ref()?;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(*h) } },
            wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..*h {
            out.extend_from_slice(&data[(y * row) as usize..(y * row + w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some((*w, *h, out))
    }
}

/// What a layer's shader samples: the texture kind (0 RGBA8 sRGB, 1 linear premultiplied RGBA
/// float, 2 YUV planes), code scale, chroma plane size, picture size and colour description.
struct SrcInfo {
    kind: u32,
    /// 2^bits of a Y'CbCr source (256 for 8-bit; code units for the range offsets).
    code_scale: f32,
    /// Loaded texel × this = code (255 for R8Unorm, 65535 for R16Unorm, 2^bits for half floats
    /// of code / 2^bits, less for MSB-aligned planes).
    texel_scale: f32,
    /// Alpha texture sample → alpha (0: the frame has no alpha plane).
    alpha: f32,
    chroma: (u32, u32),
    size: (u32, u32),
    color: filmcraft_color::ColorInfo,
}

/// A render pass drawing into the accumulator.
fn accum_pass<'e>(enc: &'e mut wgpu::CommandEncoder, view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("layers"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// The output pixels a layer's quad can touch, as (x, y, width, height) clamped to the `w`×`h`
/// output with a one-pixel margin; None when it touches none. A non-finite matrix covers it all.
fn quad_bounds(l: &PlanLayer, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let m = &l.matrix;
    let (fw, fh) = (l.size().0 as f64, l.size().1 as f64);
    let pts = [(0.0, 0.0), (fw, 0.0), (0.0, fh), (fw, fh)].map(|(x, y)| (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f));
    if pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Some((0, 0, w, h));
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    // in range before the cast (`as` saturates anyway; NaN is excluded above)
    let px = |v: f64, hi: u32| v.max(0.0).min(hi as f64) as u32;
    let (x0, y0) = (px(x0.floor() - 1.0, w), px(y0.floor() - 1.0, h));
    let (x1, y1) = (px(x1.ceil() + 1.0, w), px(y1.ceil() + 1.0, h));
    (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod blend_tests;
#[cfg(test)]
mod colorops_tests;
#[cfg(test)]
mod fx_tests;
#[cfg(test)]
mod tests;
