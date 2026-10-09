//! Colour-operation chains on the GPU (fork ADR 0003): a [`ColorOp`] list becomes a flat `f32`
//! program that `colorops.wgsl` runs per pixel in one compute pass (the FX stage's
//! [`FxOp::ColorChain`](filmcraft_render::gpufx::FxOp::ColorChain) step).

use filmcraft_color::ops::{ColorOp, Tf};

const CO_MATRIX: f32 = 1.0;
const CO_DECODE: f32 = 2.0;
const CO_ENCODE: f32 = 3.0;
const CO_TABLE: f32 = 4.0;
const CO_LGGO: f32 = 5.0;
const CO_SATURATION: f32 = 6.0;
const CO_HLG_OOTF: f32 = 7.0;
const CO_TONE_MAP: f32 = 8.0;
const CO_GAMUT_COMPRESS: f32 = 9.0;
const CO_DESATURATE: f32 = 10.0;
const CO_CLAMP: f32 = 11.0;
const CO_TONE_REGIONS: f32 = 12.0;
const CO_CONTRAST: f32 = 13.0;
const CO_SPECULAR: f32 = 14.0;
const CO_SPLIT_TONE: f32 = 15.0;
const CO_VIGNETTE: f32 = 16.0;
const CO_CURVE: f32 = 17.0;
const CO_WHEELS: f32 = 18.0;
const CO_HSL_CURVES: f32 = 19.0;
const CO_LUT: f32 = 20.0;
const CO_LOOK: f32 = 21.0;
const CO_HSL_KEY: f32 = 22.0;
const CO_HSL_CORRECT: f32 = 23.0;

/// Values a program may hold (tables of a few thousand samples each; a chain is a handful of ops).
pub const MAX_PROGRAM: usize = filmcraft_color::ops::MAX_GPU_PROGRAM;

fn tf(t: Tf) -> f32 {
    t.code() as f32
}

/// A table as `lo, hi, n, values…` (None: too long to count in `f32`).
fn table(p: &mut Vec<f32>, t: &filmcraft_color::ops::Table) -> Option<()> {
    let n = t.values.len();
    if n >= 1 << 24 {
        return None;
    }
    p.extend([t.lo, t.hi, n as f32]);
    p.extend(t.values.iter());
    Some(())
}

fn flag(b: bool) -> f32 {
    f32::from(u8::from(b))
}

/// A LUT as `n1, dmin, dmax, shaper…, size, dmin, dmax, cube…` (0 sizes: absent). None: too
/// large to index in `f32`.
fn lut(p: &mut Vec<f32>, l: &filmcraft_color::Lut) -> Option<()> {
    match &l.shaper {
        Some(s) if s.data.len() < 1 << 22 => {
            p.push(s.data.len() as f32);
            p.extend(s.domain_min.iter().chain(&s.domain_max));
            p.extend(s.data.iter().flatten());
        }
        Some(_) => return None,
        None => p.extend([0.0; 7]),
    }
    match &l.cube {
        Some(c) if c.size < 1 << 8 => {
            p.push(c.size as f32);
            p.extend(c.domain_min.iter().chain(&c.domain_max));
            p.extend(c.data.iter().flatten());
        }
        Some(_) => return None,
        None => p.extend([0.0; 7]),
    }
    Some(())
}

/// The program of a chain (None: too large, or a table too long to index in `f32`).
pub fn program(ops: &[ColorOp]) -> Option<Vec<f32>> {
    let mut p = Vec::new();
    for op in ops {
        match op {
            ColorOp::Matrix { m, offset } => {
                p.push(CO_MATRIX);
                p.extend(m.iter().flatten());
                p.extend(offset);
            }
            ColorOp::Decode(t) => p.extend([CO_DECODE, tf(*t)]),
            ColorOp::Encode(t) => p.extend([CO_ENCODE, tf(*t)]),
            ColorOp::Table(t) => {
                p.push(CO_TABLE);
                table(&mut p, t)?;
            }
            ColorOp::Lggo { lift, gamma, gain, offset } => {
                p.push(CO_LGGO);
                for v in [lift, gamma, gain, offset] {
                    p.extend(v);
                }
            }
            ColorOp::Saturation { sat, vibrance, weights } => {
                p.extend([CO_SATURATION, *sat, *vibrance]);
                p.extend(weights);
            }
            ColorOp::HlgOotf { weights, k, exponent } => {
                p.push(CO_HLG_OOTF);
                p.extend(weights);
                p.extend([*k, *exponent]);
            }
            ColorOp::ToneMap { src_peak, dst_peak, ratio } => {
                p.extend([CO_TONE_MAP, *src_peak, *dst_peak]);
                table(&mut p, ratio)?;
            }
            ColorOp::GamutCompress => p.push(CO_GAMUT_COMPRESS),
            ColorOp::DesaturateIntoGamut { weights } => {
                p.push(CO_DESATURATE);
                p.extend(weights);
            }
            ColorOp::Clamp { lo, hi } => p.extend([CO_CLAMP, *lo, *hi]),
            ColorOp::ToneRegions { shadows, highlights, weights } => {
                p.extend([CO_TONE_REGIONS, *shadows, *highlights]);
                p.extend(weights);
            }
            ColorOp::Contrast { amount, keep_above_one } => p.extend([CO_CONTRAST, *amount, f32::from(u8::from(*keep_above_one))]),
            ColorOp::Specular { gain } => p.extend([CO_SPECULAR, *gain]),
            ColorOp::SplitTone { shadows, highlights, weights } => {
                p.push(CO_SPLIT_TONE);
                for v in [shadows, highlights, weights] {
                    p.extend(v);
                }
            }
            ColorOp::Vignette { amount, midpoint, feather, sx, size } => {
                p.extend([CO_VIGNETTE, *amount, *midpoint, *feather, *sx]);
                p.extend(size);
            }
            ColorOp::Curve { table: t, channels, hdr } => {
                p.extend([CO_CURVE, flag(*hdr), flag(channels[0]), flag(channels[1]), flag(channels[2])]);
                table(&mut p, t)?;
            }
            ColorOp::Wheels { shadows, midtones, highlights, weights } => {
                p.push(CO_WHEELS);
                for v in [shadows, midtones, highlights, weights] {
                    p.extend(v);
                }
            }
            ColorOp::HslCurves(h) => {
                p.extend([CO_HSL_CURVES, flag(h.hdr)]);
                for t in [&h.hue_vs_hue, &h.hue_vs_sat, &h.hue_vs_luma, &h.luma_vs_sat, &h.sat_vs_sat] {
                    match t {
                        Some(t) => table(&mut p, t)?,
                        None => p.extend([0.0; 3]),
                    }
                }
            }
            ColorOp::Lut { lut: l, intensity, hdr } => {
                p.extend([CO_LUT, *intensity, flag(*hdr)]);
                lut(&mut p, l)?;
            }
            ColorOp::HslKey { hue, range, sat_min, luma_min, luma_max, soft, hdr } => {
                p.extend([CO_HSL_KEY, *hue, *range, *sat_min, *luma_min, *luma_max, *soft, flag(*hdr)]);
            }
            ColorOp::HslCorrect { hue_shift, sat, temp, tint, show_mask, hdr } => {
                p.extend([CO_HSL_CORRECT, *hue_shift, *sat, *temp, *tint, (*show_mask).min(16) as f32, flag(*hdr)]);
            }
            ColorOp::Look { look, intensity, hdr } => p.extend([CO_LOOK, (*look).min(1 << 20) as f32, *intensity, flag(*hdr)]),
        }
        if p.len() > MAX_PROGRAM {
            return None;
        }
    }
    Some(p)
}

/// Format of the compositor's accumulator, which the output pass reads and writes.
const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The output transform of a colour-managed sequence on the composited picture
/// ([`OutputStage`](filmcraft_render::plan::OutputStage)): the chain shader, built for the
/// accumulator's format, run from the accumulator into a texture of the same format.
pub(crate) struct OutputPass {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
}

impl OutputPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let src = include_str!("colorops.wgsl").replace("rgba32float", "rgba16float");
        let shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("filmcraft-output"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output"),
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
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: OUT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("output"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("output"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("chain_px"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, bgl }
    }

    /// Record `ops` from `src` into `dst` (both `w`×`h`, the accumulator's format). None: the
    /// program does not fit (the picture stays unconverted).
    pub(crate) fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        dst: &wgpu::TextureView,
        (w, h): (u32, u32),
        ops: &[ColorOp],
    ) -> Option<()> {
        let prog = program(ops)?;
        let mut u = Vec::with_capacity(96);
        for v in [0u32, w, h, 0, prog.len() as u32, 0, 0, 0] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        u.resize(96, 0);
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output-u"),
            size: u.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, &u);
        let pbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output-program"),
            size: (prog.len().max(4) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes: Vec<u8> = prog.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&pbuf, 0, &bytes);
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("output"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(dst) },
                wgpu::BindGroupEntry { binding: 4, resource: pbuf.as_entire_binding() },
            ],
        });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("output"), timestamp_writes: None });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);
        Some(())
    }
}
