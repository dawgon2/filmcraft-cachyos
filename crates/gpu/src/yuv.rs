//! Export frames as Y'CbCr 4:2:0 straight from the compositor's accumulator (fork Phase 5.1).
//!
//! The composited picture (linear, premultiplied, over black) is encoded like the final display
//! pass (clamped, sRGB curve) and converted per 2×2 block into limited-range Y'CbCr with the
//! matrix (Kr, Kb) — the maths of `filmcraft_export::rgb_to_yuv420p10le` (10-bit) and
//! `rgbf_to_yuv420_8` (8-bit), but from the float accumulator instead of an 8-bit RGBA read-back,
//! so a 10-bit export keeps 10 bits. A second pass packs the samples (one `u32` each) into
//! `yuv420p10le` / `yuv420p` bytes, so only those come back and the CPU copies them as they are
//! (fork Phase 5.2b: the read-back was half of a 4K export frame's time). The buffers are kept
//! for the next frame of the same size.
//!
//! Zero-copy export (fork Phase 5.2c): instead of the pack pass and the read-back, a render pass
//! writes the same samples into a video encoder's input surface (imported as two UNORM render
//! targets, `interop::Importer::import_target`): luma, then interleaved chroma, P010 (sample in
//! the top 10 bits) or NV12. Texels past the picture (the surface is padded to whole coding
//! blocks) repeat the last row / column. Nothing waits: the caller gets the submission to wait
//! for before the encoder reads the surface.

const SHADER: &str = r"
struct U { w: u32, h: u32, ten: u32, pad: u32, kr: f32, kb: f32, p0: f32, p1: f32 }
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

fn enc(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3(0.0), vec3(1.0));
    let lo = v * 12.92;
    let hi = 1.055 * pow(v, vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, v <= vec3(0.0031308));
}

fn q(x: f32, lo: f32, hi: f32) -> u32 {
    return u32(clamp(floor(x + 0.5), lo, hi));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let cw = (u.w + 1u) / 2u;
    let ch = (u.h + 1u) / 2u;
    if (id.x >= cw || id.y >= ch) {
        return;
    }
    let kg = 1.0 - u.kr - u.kb;
    let sb = 2.0 * (1.0 - u.kb);
    let sr = 2.0 * (1.0 - u.kr);
    var us = 0.0;
    var vs = 0.0;
    var n = 0.0;
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let x = id.x * 2u + dx;
            let y = id.y * 2u + dy;
            if (x >= u.w || y >= u.h) {
                continue;
            }
            let c = enc(textureLoad(src, vec2<i32>(i32(x), i32(y)), 0).rgb);
            let yy = u.kr * c.r + kg * c.g + u.kb * c.b;
            if (u.ten != 0u) {
                out[y * u.w + x] = q(64.0 + 876.0 * yy, 4.0, 1019.0);
            } else {
                out[y * u.w + x] = q(16.0 + 219.0 * yy, 1.0, 254.0);
            }
            us += (c.b - yy) / sb;
            vs += (c.r - yy) / sr;
            n += 1.0;
        }
    }
    let base = u.w * u.h;
    let i = id.y * cw + id.x;
    if (u.ten != 0u) {
        out[base + i] = q(512.0 + 896.0 * us / n, 4.0, 1019.0);
        out[base + cw * ch + i] = q(512.0 + 896.0 * vs / n, 4.0, 1019.0);
    } else {
        out[base + i] = q(128.0 + 224.0 * us / n, 1.0, 254.0);
        out[base + cw * ch + i] = q(128.0 + 224.0 * vs / n, 1.0, 254.0);
    }
}
";

/// Packs one `u32` per sample into bytes: two 16-bit (`ten`) or four 8-bit samples per word, in
/// order. Workgroups are laid out `gx` wide (dispatch limits).
const PACK: &str = r"
struct P { words: u32, samples: u32, ten: u32, gx: u32 }
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> s: array<u32>;
@group(0) @binding(2) var<storage, read_write> o: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.y * p.gx * 256u + id.x;
    if (k >= p.words) {
        return;
    }
    var w = 0u;
    if (p.ten != 0u) {
        for (var j = 0u; j < 2u; j++) {
            let i = 2u * k + j;
            if (i < p.samples) {
                w |= (s[i] & 0xffffu) << (16u * j);
            }
        }
    } else {
        for (var j = 0u; j < 4u; j++) {
            let i = 4u * k + j;
            if (i < p.samples) {
                w |= (s[i] & 0xffu) << (8u * j);
            }
        }
    }
    o[k] = w;
}
";

/// Writes the converted samples into the encoder surface's planes (render targets of the surface
/// size): `luma` one sample per texel, `chroma` (half size) Cb and Cr.
const TARGET: &str = r"
struct T { w: u32, h: u32, ten: u32, pad: u32 }
@group(0) @binding(0) var<uniform> t: T;
@group(0) @binding(1) var<storage, read> s: array<u32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn shift() -> u32 {
    return select(0u, 6u, t.ten != 0u);
}

// a stored code as UNORM: exactly representable, converted back to the same integer
fn as_unorm(v: u32) -> f32 {
    return f32(v << shift()) / select(255.0, 65535.0, t.ten != 0u);
}

@fragment
fn luma(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let x = min(u32(p.x), t.w - 1u);
    let y = min(u32(p.y), t.h - 1u);
    return vec4<f32>(as_unorm(s[y * t.w + x]), 0.0, 0.0, 0.0);
}

@fragment
fn chroma(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let cw = (t.w + 1u) / 2u;
    let ch = (t.h + 1u) / 2u;
    let x = min(u32(p.x), cw - 1u);
    let y = min(u32(p.y), ch - 1u);
    let i = t.w * t.h + y * cw + x;
    return vec4<f32>(as_unorm(s[i]), as_unorm(s[i + cw * ch]), 0.0, 0.0);
}
";

/// The planes of an encoder surface to render into (`interop::Importer::import_target`).
pub struct YuvTargets<'a> {
    pub luma: &'a wgpu::TextureView,
    pub chroma: &'a wgpu::TextureView,
    /// The surface's size (at least the picture's; even).
    pub size: (u32, u32),
}

/// Most workgroups in a row of the pack dispatch.
const PACK_GX: u32 = 32768;

/// Buffers of the last frame size (reused while it stays the same).
struct Buffers {
    key: (u32, u32, bool),
    ubuf: wgpu::Buffer,
    pbuf: wgpu::Buffer,
    planes: wgpu::Buffer,
    packed: wgpu::Buffer,
    back: wgpu::Buffer,
    pack_bg: wgpu::BindGroup,
    tbuf: wgpu::Buffer,
    target_bg: wgpu::BindGroup,
}

/// The render pipelines writing luma and chroma targets of one format pair.
struct TargetPipelines {
    luma: wgpu::RenderPipeline,
    chroma: wgpu::RenderPipeline,
}

pub(crate) struct YuvPass {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    pack: wgpu::ComputePipeline,
    pack_bgl: wgpu::BindGroupLayout,
    buffers: std::sync::Mutex<Option<Buffers>>,
    target_bgl: wgpu::BindGroupLayout,
    target_shader: wgpu::ShaderModule,
    /// [8-bit, 10-bit] target pipelines, made on first use.
    targets: std::sync::Mutex<[Option<std::sync::Arc<TargetPipelines>>; 2]>,
}

impl YuvPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("filmcraft-yuv"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                ),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("yuv"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("yuv"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let pshader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("filmcraft-yuv-pack"), source: wgpu::ShaderSource::Wgsl(PACK.into()) });
        let storage =
            |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let pack_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv-pack"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(1, storage(true)),
                entry(2, storage(false)),
            ],
        });
        let ppl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv-pack"),
            bind_group_layouts: &[Some(&pack_bgl)],
            immediate_size: 0,
        });
        let pack = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("yuv-pack"),
            layout: Some(&ppl),
            module: &pshader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let target_shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("filmcraft-yuv-target"), source: wgpu::ShaderSource::Wgsl(TARGET.into()) });
        let fentry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
        let target_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv-target"),
            entries: &[
                fentry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                fentry(1, storage(true)),
            ],
        });
        Self { pipeline, bgl, pack, pack_bgl, buffers: std::sync::Mutex::new(None), target_bgl, target_shader, targets: Default::default() }
    }

    fn target_pipelines(&self, device: &wgpu::Device, ten_bit: bool) -> std::sync::Arc<TargetPipelines> {
        let mut slot = self.targets.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let i = usize::from(ten_bit);
        if let Some(Some(p)) = slot.get(i) {
            return p.clone();
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv-target"),
            bind_group_layouts: &[Some(&self.target_bgl)],
            immediate_size: 0,
        });
        let make = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("yuv-target"),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &self.target_shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &self.target_shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::TextureFormat as F;
        let (fy, fc) = if ten_bit { (F::R16Unorm, F::Rg16Unorm) } else { (F::R8Unorm, F::Rg8Unorm) };
        let p = std::sync::Arc::new(TargetPipelines { luma: make("luma", fy), chroma: make("chroma", fc) });
        if let Some(s) = slot.get_mut(i) {
            *s = Some(p.clone());
        }
        p
    }

    /// The buffers for a `w`×`h` frame of `samples` samples packed into `words` words.
    fn buffers(&self, device: &wgpu::Device, key: (u32, u32, bool), samples: u64, words: u64) -> Buffers {
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-u"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-pack-u"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let planes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-planes"),
            size: samples * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-packed"),
            size: words * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-readback"),
            size: words * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let pack_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv-pack"),
            layout: &self.pack_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: pbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: planes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: packed.as_entire_binding() },
            ],
        });
        let tbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-target-u"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let target_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv-target"),
            layout: &self.target_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: tbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: planes.as_entire_binding() },
            ],
        });
        Buffers { key, ubuf, pbuf, planes, packed, back, pack_bg, tbuf, target_bg }
    }

    /// Record the conversion of `src` (`w`×`h`, the accumulator) into the planes buffer: the
    /// buffers (locked, for the passes after it), the encoder, and (samples, packed words,
    /// packed bytes).
    #[allow(clippy::too_many_arguments)]
    fn convert(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::TextureView,
        (w, h): (u32, u32),
        kr: f32,
        kb: f32,
        ten_bit: bool,
    ) -> Option<(std::sync::MutexGuard<'_, Option<Buffers>>, wgpu::CommandEncoder, (u64, u64, u64))> {
        if w == 0 || h == 0 {
            return None;
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let samples = u64::from(w).checked_mul(u64::from(h))?.checked_add(2 * u64::from(cw) * u64::from(ch))?;
        let size = samples.checked_mul(4)?;
        if size > device.limits().max_storage_buffer_binding_size || size > device.limits().max_buffer_size {
            return None;
        }
        // packed bytes: two per sample (10-bit) or one, padded to whole words
        let bytes = if ten_bit { samples.checked_mul(2)? } else { samples };
        let words = bytes.div_ceil(4);
        let mut u = Vec::with_capacity(32);
        for v in [w, h, u32::from(ten_bit), 0] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        for v in [kr, kb, 0.0, 0.0] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        let mut slot = self.buffers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (w, h, ten_bit);
        if slot.as_ref().is_none_or(|b| b.key != key) {
            *slot = None;
            *slot = Some(self.buffers(device, key, samples, words));
        }
        let b = slot.as_ref()?;
        queue.write_buffer(&b.ubuf, 0, &u);
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: b.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: b.planes.as_entire_binding() },
            ],
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("yuv"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(cw.div_ceil(16), ch.div_ceil(16), 1);
        }
        Some((slot, enc, (samples, words, bytes)))
    }

    /// Convert `src` (`w`×`h`, the accumulator) and read the planes back as `yuv420p10le`
    /// (`ten_bit`) or `yuv420p` bytes. Blocks until the GPU is done.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::TextureView,
        size: (u32, u32),
        kr: f32,
        kb: f32,
        ten_bit: bool,
    ) -> Option<Vec<u8>> {
        let (slot, mut enc, (samples, words, bytes)) = self.convert(device, queue, src, size, kr, kb, ten_bit)?;
        let b = slot.as_ref()?;
        let groups = u32::try_from(words.div_ceil(256)).ok()?;
        let (gx, gy) = (groups.clamp(1, PACK_GX), groups.div_ceil(PACK_GX).max(1));
        let mut pu = Vec::with_capacity(16);
        for v in [u32::try_from(words).ok()?, u32::try_from(samples).ok()?, u32::from(ten_bit), gx] {
            pu.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&b.pbuf, 0, &pu);
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("yuv-pack"), timestamp_writes: None });
            pass.set_pipeline(&self.pack);
            pass.set_bind_group(0, &b.pack_bg, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        enc.copy_buffer_to_buffer(&b.packed, 0, &b.back, 0, words * 4);
        queue.submit([enc.finish()]);
        let slice = b.back.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let out = {
            let data = slice.get_mapped_range().ok()?;
            data.get(..usize::try_from(bytes).ok()?).map(|d| d.to_vec())
        };
        b.back.unmap();
        out
    }

    /// Convert `src` (`w`×`h`) and render the samples into `targets` (an encoder surface at
    /// least `w`×`h`). Does not wait: returns the submission that finishes the write.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_into(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::TextureView,
        (w, h): (u32, u32),
        kr: f32,
        kb: f32,
        ten_bit: bool,
        targets: &YuvTargets<'_>,
    ) -> Option<wgpu::SubmissionIndex> {
        let (sw, sh) = targets.size;
        if sw < w || sh < h || sw == 0 || sh == 0 {
            return None;
        }
        let pipes = self.target_pipelines(device, ten_bit);
        let (slot, mut enc, _) = self.convert(device, queue, src, (w, h), kr, kb, ten_bit)?;
        let b = slot.as_ref()?;
        let mut tu = Vec::with_capacity(16);
        for v in [w, h, u32::from(ten_bit), 0] {
            tu.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&b.tbuf, 0, &tu);
        for (view, pipe) in [(targets.luma, &pipes.luma), (targets.chroma, &pipes.chroma)] {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yuv-target"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &b.target_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        Some(queue.submit([enc.finish()]))
    }
}
