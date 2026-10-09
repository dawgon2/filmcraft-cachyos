//! Scope samples from the composited picture (fork Phase 4.4): the Lumetri Scopes read the
//! Program monitor's frame where it already is, in VRAM. A compute pass picks the samples
//! `filmcraft_scopes::Signal::from_rgba8_max` would pick (nearest sample, every `sx`-th column
//! and `sy`-th row, at most `max_w` × `max_h`) into a small buffer that is read back
//! asynchronously (≈ 0.5 MB for 480 × 270), instead of rendering the frame a second time for the
//! scopes.

use std::sync::mpsc;

const SHADER: &str = r"
struct U { w: u32, h: u32, sx: u32, sy: u32, ow: u32, oh: u32, p0: u32, p1: u32 }
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.ow || id.y >= u.oh) {
        return;
    }
    let p = vec2<i32>(i32(min(id.x * u.sx, u.w - 1u)), i32(min(id.y * u.sy, u.h - 1u)));
    out[id.y * u.ow + id.x] = pack4x8unorm(textureLoad(src, p, 0));
}
";

pub(crate) struct ScopeSampler {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
}

/// A scope sample read-back in flight; [`try_take`](Self::try_take) returns it once the GPU is
/// done, without waiting.
pub struct ScopeReadback {
    device: wgpu::Device,
    buf: wgpu::Buffer,
    /// Samples per row and rows.
    pub w: u32,
    pub h: u32,
    rx: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl ScopeReadback {
    /// The samples as straight RGBA8 (`w` × `h`), when the read-back has finished; None while it
    /// is in flight (or when it failed).
    pub fn try_take(&self) -> Option<Vec<u8>> {
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.rx.try_recv().ok()?.ok()?;
        let n = (self.w as usize).checked_mul(self.h as usize)?.checked_mul(4)?;
        let data = self.buf.slice(..).get_mapped_range().ok()?;
        let out = data.get(..n)?.to_vec();
        drop(data);
        self.buf.unmap();
        Some(out)
    }
}

/// The sample grid for a `w`×`h` picture: steps and sample counts (`Signal`'s `steps`).
pub fn sample_grid(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32, u32, u32) {
    let sx = w.div_ceil(max_w.max(1)).max(1);
    let sy = h.div_ceil(max_h.max(1)).max(1);
    (sx, sy, w.div_ceil(sx), h.div_ceil(sy))
}

impl ScopeSampler {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("filmcraft-scopes"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scopes"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scopes"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("scopes"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, bgl }
    }

    /// Sample `src` (`w`×`h`, RGBA8) and start reading the samples back.
    pub(crate) fn request(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::TextureView,
        (w, h): (u32, u32),
        max: (u32, u32),
    ) -> Option<ScopeReadback> {
        if w == 0 || h == 0 {
            return None;
        }
        let (sx, sy, ow, oh) = sample_grid(w, h, max.0, max.1);
        let size = u64::from(ow).checked_mul(u64::from(oh))?.checked_mul(4)?;
        let mut u = Vec::with_capacity(32);
        for v in [w, h, sx, sy, ow, oh, 0, 0] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scopes-u"),
            size: u.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, &u);
        let samples = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scopes-samples"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scopes-readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scopes"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: samples.as_entire_binding() },
            ],
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("scopes"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(ow.div_ceil(16), oh.div_ceil(16), 1);
        }
        enc.copy_buffer_to_buffer(&samples, 0, &buf, 0, size);
        queue.submit([enc.finish()]);
        let (tx, rx) = mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        Some(ScopeReadback { device: device.clone(), buf, w: ow, h: oh, rx })
    }
}
