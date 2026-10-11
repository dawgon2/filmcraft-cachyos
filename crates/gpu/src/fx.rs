//! The GPU effect stage: standard effects of a plan layer ([`LayerFx`]) run in WGSL (`fx.wgsl`).
//!
//! A layer with effects is first drawn from its source (YUV planes or RGBA, through the
//! compositor's own `fs` shader, so decoding is the same as for a plain layer) into a working
//! texture of the size the CPU decodes it at; each [`FxOp`] then runs as compute passes that
//! ping-pong between two working textures (three for Unsharp Mask, which keeps the original);
//! the last one is drawn into the accumulator with the layer's matrix, opacity and blend mode
//! like any RGBA layer. Working textures are `Rgba32Float` (the CPU reference's precision) and
//! pooled per size; layers without effects never touch any of this.
//!
//! Box blurs (Gaussian Blur, Camera Blur, the blur inside Unsharp / Sharpen) run per pixel for
//! radii up to [`BOX_PER_PIXEL_MAX`] and as a running sum per row / column above, so a huge
//! radius costs O(1) per pixel like on the CPU (radii are capped by `gaussian_boxes`).

use std::collections::{HashMap, HashSet};

use filmcraft_render::gpufx::FxOp;
use filmcraft_render::plan::LayerFx;

/// Working texture format.
pub(crate) const FX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Box radii up to this many pixels are summed per output pixel; larger ones as a running sum.
pub const BOX_PER_PIXEL_MAX: u32 = 32;

const OP_BRIGHTNESS_CONTRAST: u32 = 1;
const OP_PROC_AMP: u32 = 2;
const OP_TINT: u32 = 3;
const OP_BLACK_WHITE: u32 = 4;
const OP_COLOR_BALANCE: u32 = 5;
const OP_LEAVE_COLOR: u32 = 6;
const OP_CHANGE_TO_COLOR: u32 = 7;
const OP_COLOR_PASS: u32 = 8;
const OP_GAMMA: u32 = 9;
const OP_LEVELS: u32 = 10;
const OP_EXTRACT: u32 = 11;
const OP_INVERT: u32 = 12;
const OP_INVERT_ALPHA: u32 = 13;
const OP_POSTERIZE: u32 = 14;
const OP_ASC_CDL: u32 = 15;
const OP_CHANNEL_MIX: u32 = 16;
const OP_COLOR_REPLACE: u32 = 17;
const OP_ALPHA_ADJUST: u32 = 18;
const OP_VIGNETTE: u32 = 19;
const OP_BOX: u32 = 20;
const OP_DIRECTIONAL: u32 = 22;
const OP_UNSHARP: u32 = 23;
const OP_CROP: u32 = 24;
const OP_RESAMPLE: u32 = 25;
const OP_HFLIP: u32 = 26;
const OP_VFLIP: u32 = 27;
const OP_MIRROR: u32 = 28;
const OP_OFFSET: u32 = 29;
const OP_MEDIAN_MASK: u32 = 30;
const OP_VIDEO_LIMITER: u32 = 31;
const OP_DENOISE: u32 = 32;
const OP_DENOISE_YCC: u32 = 33;
// (Lumetri is not one op: it is a colour chain, `FxOp::ColorChain`, fork ADR 0003)
/// Step codes of colour-chain passes: plain, key (writes the mask), masked (reads it).
const CHAIN_KEY: u32 = 1;
const CHAIN_MASKED: u32 = 2;
/// Mask pass codes (`colorops.wgsl` `mask_pass`): mix with the saved image, scale.
const MASK_MIX: u32 = 3;
const MASK_SCALE: u32 = 4;

type Target = (wgpu::Texture, wgpu::TextureView);

pub(crate) struct FxStage {
    px: wgpu::ComputePipeline,
    run: wgpu::ComputePipeline,
    /// A colour-operation chain (`colorops.wgsl`), one pass.
    chain: wgpu::ComputePipeline,
    /// A key chain writing its mask, and a chain correcting `aux` inside the mask in `src`.
    chain_key: wgpu::ComputePipeline,
    chain_masked: wgpu::ComputePipeline,
    /// Mask coverage applied to `src` (mix with the saved image in `aux`, or scale).
    mask: wgpu::ComputePipeline,
    /// Bound as the program of the passes that have none.
    no_program: wgpu::Buffer,
    bgl: wgpu::BindGroupLayout,
    /// Draws a layer's source into a working texture (`composite.wgsl` `fs`, no blending).
    source: wgpu::RenderPipeline,
    pool: HashMap<(u32, u32), Vec<Target>>,
    used: HashSet<(u32, u32)>,
}

/// Which compute pipeline a pass runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pipe {
    Px,
    Run,
    Chain,
    ChainKey,
    ChainMasked,
    Mask,
}

/// One compute pass of a job.
struct Dispatch {
    pipe: Pipe,
    bg: wgpu::BindGroup,
    groups: (u32, u32),
}

/// The recorded work of one layer's effects (bind groups made; recorded into the frame's encoder
/// right before the layer is drawn, so layers can share pooled textures).
pub(crate) struct FxJob {
    source_bg: wgpu::BindGroup,
    source_target: wgpu::TextureView,
    dispatches: Vec<Dispatch>,
    /// The texture holding the result.
    pub(crate) result: wgpu::TextureView,
    pub(crate) result_texture: wgpu::Texture,
}

/// One pass before texture routing: entry point, op code, integer and float parameters.
struct Step {
    run: bool,
    code: u32,
    i1: [u32; 4],
    p: [f32; 24],
    /// Unsharp combine: reads the blurred image and the original.
    combine: bool,
    /// A pass before a combine (Unsharp Mask's blur, HSL Secondary's key and refine): the image
    /// it started from stays untouched until the combine.
    unsharp_blur: bool,
    /// A colour-operation chain's program (`colorops.rs`); the pass runs `colorops.wgsl`.
    program: Option<Vec<f32>>,
    /// No pass: remember the current image for a masked effect's mix.
    save: bool,
    /// The mask mix: reads the saved image.
    restore: bool,
}

fn step(code: u32, i1: [u32; 4], p: &[f32]) -> Step {
    let mut q = [0.0; 24];
    for (d, s) in q.iter_mut().zip(p) {
        *d = *s;
    }
    Step { run: false, code, i1, p: q, combine: false, unsharp_blur: false, program: None, save: false, restore: false }
}

/// Box passes along x (vertical = false) or y for the radii (radius 0 is a no-op on the CPU).
fn boxes(radii: &[u32], vertical: bool, repeat: bool, out: &mut Vec<Step>) {
    for &r in radii.iter().filter(|r| **r > 0) {
        let mut s = step(OP_BOX, [r, repeat as u32, vertical as u32, 0], &[]);
        s.run = r > BOX_PER_PIXEL_MAX;
        out.push(s);
    }
}

/// Active masks as `count, (mode, inverted, feather, expansion, opacity, n, x, y…)…`.
fn mask_program(masks: &[filmcraft_render::mask::FlatMask]) -> Vec<f32> {
    let active: Vec<_> = masks.iter().filter(|m| m.mode.index() != 0).collect();
    let mut p = vec![active.len() as f32];
    for m in active {
        p.extend([m.mode.index() as f32, f32::from(u8::from(m.inverted)), m.feather, m.expansion, m.opacity, m.pts.len() as f32]);
        p.extend(m.pts.iter().flatten());
    }
    p
}

/// The passes of an op (empty: no-op). Unsharp's blur passes come first, then the combine.
fn steps(op: &FxOp) -> Vec<Step> {
    let mut out = Vec::new();
    match op {
        FxOp::BrightnessContrast { br, co } => out.push(step(OP_BRIGHTNESS_CONTRAST, [0; 4], &[*br, *co])),
        FxOp::ProcAmp { br, co, hue, sat } => out.push(step(OP_PROC_AMP, [0; 4], &[*br, *co, *hue, *sat])),
        FxOp::Tint { black, white, amount } => {
            out.push(step(OP_TINT, [0; 4], &[black[0], black[1], black[2], *amount, white[0], white[1], white[2]]));
        }
        FxOp::BlackWhite => out.push(step(OP_BLACK_WHITE, [0; 4], &[])),
        FxOp::ColorBalance { sh, md, hi, preserve } => {
            out.push(step(OP_COLOR_BALANCE, [*preserve as u32, 0, 0, 0], &[sh[0], sh[1], sh[2], 0.0, md[0], md[1], md[2], 0.0, hi[0], hi[1], hi[2]]))
        }
        FxOp::LeaveColor { amount, key_hue, tol, soft } => out.push(step(OP_LEAVE_COLOR, [0; 4], &[*amount, *key_hue, *tol, *soft])),
        FxOp::ChangeToColor { from_hue, to_hue, tol, soft } => out.push(step(OP_CHANGE_TO_COLOR, [0; 4], &[*from_hue, *to_hue, *tol, *soft])),
        FxOp::ColorPass { key, sim, reverse } => out.push(step(OP_COLOR_PASS, [*reverse as u32, 0, 0, 0], &[key[0], key[1], key[2], *sim])),
        FxOp::Gamma { g } => out.push(step(OP_GAMMA, [0; 4], &[*g])),
        FxOp::Levels { ib, iw, ob, ow, g } => out.push(step(OP_LEVELS, [0; 4], &[*ib, *iw, *ob, *ow, *g])),
        FxOp::Extract { lo, hi, soft, invert } => out.push(step(OP_EXTRACT, [*invert as u32, 0, 0, 0], &[*lo, *hi, *soft])),
        FxOp::Invert { channel: 4, blend } => out.push(step(OP_INVERT_ALPHA, [0; 4], &[*blend])),
        FxOp::Invert { channel, blend } => out.push(step(OP_INVERT, [*channel, 0, 0, 0], &[*blend])),
        FxOp::Posterize { n } => out.push(step(OP_POSTERIZE, [0; 4], &[*n])),
        FxOp::Gaussian { rx, ry, repeat } => {
            boxes(rx, false, *repeat, &mut out);
            boxes(ry, true, *repeat, &mut out);
        }
        FxOp::DirectionalBlur { dx, dy, steps } => {
            if *steps >= 2 {
                out.push(step(OP_DIRECTIONAL, [*steps, 0, 0, 0], &[*dx, *dy]));
            }
        }
        FxOp::Unsharp { rx, ry, amount, threshold } => {
            boxes(rx, false, true, &mut out);
            boxes(ry, true, true, &mut out);
            for s in &mut out {
                s.unsharp_blur = true;
            }
            let mut s = step(OP_UNSHARP, [0; 4], &[*amount, *threshold]);
            s.combine = true;
            out.push(s);
        }
        FxOp::Crop { x0, x1, y0, y1, feather } => out.push(step(OP_CROP, [0; 4], &[*x0, *x1, *y0, *y1, *feather])),
        FxOp::Resample(r) => {
            let op = if (r.opacity - 1.0).abs() < 1e-6 { 1.0 } else { r.opacity };
            match &r.inv {
                Some(m) => out.push(step(OP_RESAMPLE, r.rect, &[m.a as f32, m.b as f32, m.c as f32, m.d as f32, m.e as f32, m.f as f32, op, 1.0])),
                None => out.push(step(OP_RESAMPLE, [0; 4], &[0.0; 8])),
            }
        }
        FxOp::HFlip => out.push(step(OP_HFLIP, [0; 4], &[])),
        FxOp::VFlip => out.push(step(OP_VFLIP, [0; 4], &[])),
        FxOp::Mirror { cx, cy, nx, ny } => out.push(step(OP_MIRROR, [0; 4], &[*cx, *cy, *nx, *ny])),
        FxOp::Offset { dx, dy, blend } => out.push(step(OP_OFFSET, [0; 4], &[*dx, *dy, *blend])),
        FxOp::AscCdl { slope: s, offset: o, power: p, sat } => {
            out.push(step(OP_ASC_CDL, [0; 4], &[s[0], s[1], s[2], *sat, o[0], o[1], o[2], 0.0, p[0], p[1], p[2]]));
        }
        FxOp::ChannelMix { m } => {
            let p: Vec<f32> = m.iter().flatten().copied().collect();
            out.push(step(OP_CHANNEL_MIX, [0; 4], &p));
        }
        FxOp::ColorReplace { sim, solid, target: t, replace: r, replace_hsl: h } => {
            out.push(step(OP_COLOR_REPLACE, [*solid as u32, 0, 0, 0], &[t[0], t[1], t[2], *sim, r[0], r[1], r[2], 0.0, h[0], h[1], h[2]]))
        }
        FxOp::AlphaAdjust { opacity, ignore, invert, mask_only } => {
            out.push(step(OP_ALPHA_ADJUST, [*ignore as u32, *invert as u32, *mask_only as u32, 0], &[*opacity]));
        }
        // (no active mask: the effect applies everywhere, as `mask::coverage` → None)
        FxOp::Masked { ops, masks } if mask_program(masks).len() <= 1 => out.extend(ops.iter().flat_map(steps)),
        FxOp::OpacityMask { masks } if mask_program(masks).len() <= 1 => {}
        FxOp::Masked { ops, masks } => {
            let mut save = step(0, [0; 4], &[]);
            save.save = true;
            out.push(save);
            out.extend(ops.iter().flat_map(steps));
            let prog = mask_program(masks);
            let mut mix = step(MASK_MIX, [prog.len() as u32, 0, 0, 0], &[]);
            mix.program = Some(prog);
            mix.restore = true;
            out.push(mix);
        }
        FxOp::OpacityMask { masks } => {
            let prog = mask_program(masks);
            let mut s = step(MASK_SCALE, [prog.len() as u32, 0, 0, 0], &[]);
            s.program = Some(prog);
            out.push(s);
        }
        FxOp::HslSecondary { key, median_r, denoise, blur, correct } => {
            let (Some(kp), Some(cp)) = (crate::colorops::program(key), crate::colorops::program(correct)) else {
                log::warn!("HSL Secondary chains too large for the GPU program; skipped");
                return out;
            };
            let mut k = step(CHAIN_KEY, [kp.len() as u32, 0, 0, 0], &[]);
            k.program = Some(kp);
            out.push(k);
            if *denoise > 0.0 && *median_r > 0 {
                out.push(step(OP_MEDIAN_MASK, [*median_r, 0, 0, 0], &[*denoise]));
            }
            boxes(blur, false, true, &mut out);
            boxes(blur, true, true, &mut out);
            for s in &mut out {
                s.unsharp_blur = true;
            }
            let mut c = step(CHAIN_MASKED, [cp.len() as u32, 0, 0, 0], &[]);
            c.program = Some(cp);
            c.combine = true;
            out.push(c);
        }
        FxOp::ColorChain(ops) => match crate::colorops::program(ops) {
            Some(p) if !p.is_empty() => {
                let mut s = step(0, [p.len() as u32, 0, 0, 0], &[]);
                s.program = Some(p);
                out.push(s);
            }
            Some(_) => {}
            None => log::warn!("colour chain of {} ops too large for the GPU program; skipped", ops.len()),
        },
        FxOp::Vignette { amount, midpoint, roundness, feather, target } => {
            if amount.abs() >= 1e-5 {
                out.push(step(OP_VIGNETTE, [0; 4], &[*amount, *midpoint, *roundness, *feather, target[0], target[1], target[2]]));
            }
        }
        FxOp::VideoLimiter { max, comp, axis, warn, warning_color } => {
            out.push(step(OP_VIDEO_LIMITER, [*axis, *warn as u32, 0, 0], &[*max, *comp, warning_color[0], warning_color[1], warning_color[2]]));
        }
        FxOp::Denoise { luma, chroma, sigma } => {
            // (both strengths 0: an exact no-op, no pass; the constants are the CPU's.) A pre-pass
            // converts every pixel once to (Y, Cb, Cr, α); the filter reads it and keeps the
            // original (`aux`) for the pixels it leaves unchanged, like Unsharp's combine.
            if let Some(p) = filmcraft_render::denoise::params(*luma, *chroma, *sigma) {
                let mut pre = step(OP_DENOISE_YCC, [0; 4], &[]);
                pre.unsharp_blur = true;
                out.push(pre);
                let flags = [p.radius, u32::from(p.luma_k.is_some()), u32::from(p.chroma_k.is_some()), 0];
                let mut q = vec![p.luma_k.unwrap_or(0.0), p.chroma_k.unwrap_or(0.0), 0.0, 0.0];
                q.extend_from_slice(&p.spatial);
                let mut f = step(OP_DENOISE, flags, &q);
                f.combine = true;
                out.push(f);
            }
        }
    }
    out
}

impl FxStage {
    /// Whether `device` can run the stage: compute shaders with 16×16 workgroups and a storage
    /// texture (WebGL2-class devices can't; they render effects on the CPU).
    pub(crate) fn supported(device: &wgpu::Device) -> bool {
        let l = device.limits();
        l.max_storage_textures_per_shader_stage >= 1
            && l.max_compute_invocations_per_workgroup >= 256
            && l.max_compute_workgroup_size_x >= 64
            && l.max_compute_workgroup_size_y >= 16
            && l.max_compute_workgroups_per_dimension >= 1
    }

    pub(crate) fn new(device: &wgpu::Device, composite: &wgpu::ShaderModule, layer_bgl: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filmcraft-fx"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fx.wgsl").into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: FX_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                tex(3),
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
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fx"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let chain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filmcraft-colorops"),
            source: wgpu::ShaderSource::Wgsl(include_str!("colorops.wgsl").into()),
        });
        let compute_in = |module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx"),
                layout: Some(&pl),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let compute = |entry: &str| compute_in(&shader, entry);
        let no_program = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-no-program"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let spl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-source"),
            bind_group_layouts: &[Some(layer_bgl)],
            immediate_size: 0,
        });
        let source = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx-source"),
            layout: Some(&spl),
            vertex: wgpu::VertexState { module: composite, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: composite,
                entry_point: Some("fs_fx_source"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: FX_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            px: compute("fx_px"),
            run: compute("fx_run"),
            chain: compute_in(&chain_shader, "chain_px"),
            chain_key: compute_in(&chain_shader, "chain_key"),
            chain_masked: compute_in(&chain_shader, "chain_masked"),
            mask: compute_in(&chain_shader, "mask_pass"),
            no_program,
            bgl,
            source,
            pool: HashMap::new(),
            used: HashSet::new(),
        }
    }

    /// Start a frame: pooled textures of sizes no layer uses by [`end_frame`](Self::end_frame)
    /// are released then.
    pub(crate) fn begin_frame(&mut self) {
        self.used.clear();
    }

    pub(crate) fn end_frame(&mut self) {
        let used = &self.used;
        self.pool.retain(|k, _| used.contains(k));
    }

    /// No working textures held.
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.pool.is_empty()
    }

    fn targets(&mut self, device: &wgpu::Device, size: (u32, u32), n: usize) -> Vec<Target> {
        self.used.insert(size);
        let v = self.pool.entry(size).or_default();
        while v.len() < n {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filmcraft-fx"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FX_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = t.create_view(&Default::default());
            v.push((t, view));
        }
        v.iter().take(n).cloned().collect()
    }

    /// Prepare the passes of `fx`, whose source draw uses `source_bg` (a layer bind group whose
    /// uniforms map the frame onto the working image). None when the working image does not fit
    /// in a texture (the layer is then drawn without its effects — never larger than the source
    /// frame, which would not have uploaded either).
    pub(crate) fn job(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fx: &LayerFx,
        source_bg: wgpu::BindGroup,
        dummy: &wgpu::TextureView,
    ) -> Option<FxJob> {
        let (w, h) = (fx.size.0.max(1), fx.size.1.max(1));
        let max = device.limits().max_texture_dimension_2d;
        if w > max || h > max {
            return None;
        }
        let all: Vec<Step> = fx.ops.iter().flat_map(steps).collect();
        let n = 2 + usize::from(all.iter().any(|s| s.combine)) + usize::from(all.iter().any(|s| s.save));
        let targets = self.targets(device, (w, h), n);
        let views: Vec<wgpu::TextureView> = targets.iter().map(|t| t.1.clone()).collect();
        let mut dispatches = Vec::with_capacity(all.len());
        let mut cur = 0usize;
        // Unsharp: the image its blur started from (kept while its blur passes run)
        let mut orig: Option<usize> = None;
        // a masked effect: the image before it (kept until its mask mix)
        let mut saved: Option<usize> = None;
        for s in &all {
            if s.save {
                saved = Some(cur);
                continue;
            }
            // (an Unsharp without blur passes, radius 0, combines the original with itself)
            if (s.unsharp_blur || s.combine) && orig.is_none() {
                orig = Some(cur);
            }
            let keep = if s.unsharp_blur || s.combine { orig } else { None };
            let dst = (0..n).find(|k| *k != cur && Some(*k) != keep && Some(*k) != saved)?;
            let aux = match (s.combine, orig, s.restore, saved) {
                (true, Some(o), _, _) => views.get(o)?,
                (_, _, true, Some(v)) => views.get(v)?,
                _ => dummy,
            };
            let mut bytes = Vec::with_capacity(128);
            for v in [s.code, w, h, 0].iter().chain(&s.i1) {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for v in &s.p {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fx-u"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &bytes);
            let program = s.program.as_ref().map(|p| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("fx-program"),
                    size: (p.len().max(4) * 4) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bytes: Vec<u8> = p.iter().flat_map(|v| v.to_le_bytes()).collect();
                queue.write_buffer(&b, 0, &bytes);
                b
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(views.get(cur)?) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(views.get(dst)?) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(aux) },
                    wgpu::BindGroupEntry { binding: 4, resource: program.as_ref().unwrap_or(&self.no_program).as_entire_binding() },
                ],
            });
            let groups = if s.run {
                let lines = if s.i1[2] != 0 { w } else { h };
                (lines.div_ceil(64), 1)
            } else {
                (w.div_ceil(16), h.div_ceil(16))
            };
            let pipe = if s.program.is_some() {
                match s.code {
                    CHAIN_KEY => Pipe::ChainKey,
                    CHAIN_MASKED => Pipe::ChainMasked,
                    MASK_MIX | MASK_SCALE => Pipe::Mask,
                    _ => Pipe::Chain,
                }
            } else if s.run {
                Pipe::Run
            } else {
                Pipe::Px
            };
            dispatches.push(Dispatch { pipe, bg, groups });
            cur = dst;
            if s.combine {
                orig = None;
            }
            if s.restore {
                saved = None;
            }
        }
        Some(FxJob {
            source_bg,
            source_target: views.first()?.clone(),
            dispatches,
            result: views.get(cur)?.clone(),
            result_texture: targets.get(cur)?.0.clone(),
        })
    }

    /// Record a job (source draw, then its compute passes) into `enc`.
    pub(crate) fn record(&self, enc: &mut wgpu::CommandEncoder, job: &FxJob) {
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx-source"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &job.source_target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.source);
            pass.set_bind_group(0, &job.source_bg, &[]);
            pass.draw(0..6, 0..1);
        }
        if job.dispatches.is_empty() {
            return;
        }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fx"), timestamp_writes: None });
        for d in &job.dispatches {
            pass.set_pipeline(match d.pipe {
                Pipe::Px => &self.px,
                Pipe::Run => &self.run,
                Pipe::Chain => &self.chain,
                Pipe::ChainKey => &self.chain_key,
                Pipe::ChainMasked => &self.chain_masked,
                Pipe::Mask => &self.mask,
            });
            pass.set_bind_group(0, &d.bg, &[]);
            pass.dispatch_workgroups(d.groups.0, d.groups.1, 1);
        }
    }
}
