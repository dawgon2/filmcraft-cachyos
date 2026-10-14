//! The GPU effect stage: standard effects of a plan layer ([`LayerFx`]) run in WGSL (`fx.wgsl`).
//!
//! A layer with effects is first drawn from its source (YUV planes or RGBA, through the
//! compositor's own `fs` shader, so decoding is the same as for a plain layer) into a working
//! texture of the size the CPU decodes it at; each [`FxOp`] then runs as compute passes that
//! ping-pong between two working textures (three for Unsharp Mask, which keeps the original);
//! the last one is drawn into the accumulator with the layer's matrix, opacity and blend mode
//! like any RGBA layer. Working textures are `Rgba32Float` (the CPU reference's precision) and
//! pooled per size; layers without effects never touch any of this. Preview may use `Rgba16Float`
//! instead ([`fx_format_for`], fork U0.2.3b: half the bandwidth; opt-in for now, owner question 18);
//! export always works in `Rgba32Float` (`GpuCompositor::for_export`, FORK-PLAN §1a.64).
//!
//! Box blurs (Gaussian Blur, Camera Blur, the blur inside Unsharp / Sharpen) run per pixel for
//! radii up to [`BOX_PER_PIXEL_MAX`] and as a running sum per row / column above, so a huge
//! radius costs O(1) per pixel like on the CPU (radii are capped by `gaussian_boxes`).
//!
//! Temporal noise reduction (`FxOp::TemporalDenoise`, fork 6.7) also draws each neighbouring
//! source frame into a scratch texture, replays the passes of the ops before it there (so the
//! neighbour is in the same state as the layer's image), converts it to (Y, Cb, Cr, α)
//! (`OP_TEMPORAL_INIT`), uploads the flows as `Rg32Float` textures and blends everything in one
//! `temporal_accumulate` pass (accumulation and finish fused: the luma and chroma sums and their
//! two weights are five values, more than one storage texture holds).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

use filmcraft_render::gpufx::FxOp;
use filmcraft_render::plan::LayerFx;

/// Working texture format (full precision; [`FX_FORMAT_HALF`] on integrated GPUs).
pub(crate) const FX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Working texture format on integrated GPUs ([`fx_format_for`]).
pub(crate) const FX_FORMAT_HALF: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Whether preview on integrated GPUs uses [`FX_FORMAT_HALF`]. Off, provisional (owner question 18):
/// the owner's bar for half floats is under one 10-bit code value from f32 (FORK-PLAN §1a.62 /
/// §1a.64); colour ops meet it (~0.3), Sharpen / Unsharp don't (~4, `fx_tests::half_float_preview_*`),
/// and on the Intel UHD 750 the colour chain is ALU-bound, so half floats barely speed preview there.
const HALF_ON_INTEGRATED: bool = false;

/// The working texture format for preview on a device: [`FX_FORMAT_HALF`] on integrated GPUs when
/// [`HALF_ON_INTEGRATED`], [`FX_FORMAT`] otherwise. `FILMCRAFT_FX_FORMAT=f16` / `f32` overrides it
/// (A/B runs). Export never comes here (`GpuCompositor::for_export` is always f32).
pub(crate) fn fx_format_for(device_type: wgpu::DeviceType) -> wgpu::TextureFormat {
    fx_format_pick(device_type, std::env::var("FILMCRAFT_FX_FORMAT").ok().as_deref())
}

fn fx_format_pick(device_type: wgpu::DeviceType, forced: Option<&str>) -> wgpu::TextureFormat {
    match forced.map(str::trim) {
        Some("f16") => FX_FORMAT_HALF,
        Some("f32") => FX_FORMAT,
        _ if HALF_ON_INTEGRATED && device_type == wgpu::DeviceType::IntegratedGpu => FX_FORMAT_HALF,
        _ => FX_FORMAT,
    }
}

/// `src` (a WGSL source writing `rgba32float` storage textures) for working textures of `format`.
fn wgsl_for(src: &str, format: wgpu::TextureFormat) -> String {
    if format == FX_FORMAT_HALF { src.replace("texture_storage_2d<rgba32float, write>", "texture_storage_2d<rgba16float, write>") } else { src.to_string() }
}

/// Bytes per working-texture texel.
fn texel_bytes(format: wgpu::TextureFormat) -> u64 {
    if format == FX_FORMAT_HALF { 8 } else { 16 }
}
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
const OP_TEMPORAL_INIT: u32 = 34;
/// Neighbour slots of the temporal pass (`tnr::MAX_NEIGHBOURS`).
const TEMPORAL_SLOTS: usize = 4;
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
    /// The temporal blend (`temporal_accumulate`) and its second bind group's layout.
    temporal: wgpu::ComputePipeline,
    tbgl: wgpu::BindGroupLayout,
    pool: HashMap<(u32, u32), Vec<Target>>,
    used: HashSet<(u32, u32)>,
    /// GPU memory one layer's working textures may take ([`FX_VRAM_BUDGET`]).
    budget: u64,
    /// Working texture format ([`fx_format_for`]).
    format: wgpu::TextureFormat,
    /// The chain shader's source for `format` and its pipeline layout, for chains specialised to
    /// their op codes ([`colorops::specialise`](crate::colorops::specialise)).
    chain_src: String,
    pl: wgpu::PipelineLayout,
    /// Specialised chain pipelines by entry and op codes (None: failed to build; the interpreter
    /// runs that chain).
    special: Mutex<HashMap<(Pipe, Vec<u32>), Option<wgpu::ComputePipeline>>>,
    /// The effect shader and per-op `fx_px` pipelines (`PX_OP` set; None: failed to build).
    shader: wgpu::ShaderModule,
    px_special: Mutex<HashMap<u32, Option<wgpu::ComputePipeline>>>,
    /// Run every pass on the generic shaders (tests compare the two).
    interpret_only: bool,
}

/// Specialised chain pipelines kept at most (the cache is cleared when full).
const MAX_SPECIAL_PIPELINES: usize = 48;

/// Whether colour chains run as shaders specialised to their op codes (`FILMCRAFT_CHAIN_SPECIALISE=0`
/// turns it off: the interpreter, for A/B checks).
pub(crate) fn specialise_chains() -> bool {
    !cfg!(target_arch = "wasm32") && std::env::var("FILMCRAFT_CHAIN_SPECIALISE").map_or(true, |v| v != "0")
}

/// Which compute pipeline a pass runs.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Pipe {
    Px,
    Run,
    Chain,
    ChainKey,
    ChainMasked,
    Mask,
    Temporal,
}

/// One compute pass of a job.
struct Dispatch {
    pipe: Pipe,
    bg: wgpu::BindGroup,
    /// Group 1 (the temporal pass only).
    bg1: Option<wgpu::BindGroup>,
    groups: (u32, u32),
    /// A chain pipeline specialised to the step's op codes, run instead of `pipe`'s.
    special: Option<wgpu::ComputePipeline>,
}

/// One recorded piece of a job, in order.
enum Work {
    /// Draw a source frame (layer bind group) into a working texture.
    Source {
        bg: wgpu::BindGroup,
        target: wgpu::TextureView,
    },
    Compute(Dispatch),
}

/// A temporal op of a job: its drawable neighbours (source bind group and sanitised flows, in
/// `tnr::select` order), the passes to replay on them, and the constants.
struct TemporalSpec {
    nbrs: Vec<(wgpu::BindGroup, filmcraft_render::flow::FlowField, filmcraft_render::flow::FlowField)>,
    pre: Vec<Step>,
    params: filmcraft_render::tnr::TnrParams,
}

/// The recorded work of one layer's effects (bind groups made; recorded into the frame's encoder
/// right before the layer is drawn, so layers can share pooled textures).
pub(crate) struct FxJob {
    work: Vec<Work>,
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
    /// The chain's op codes (empty: not a chain, or run it on the interpreter).
    codes: Vec<u32>,
    /// No pass: remember the current image for a masked effect's mix.
    save: bool,
    /// The mask mix: reads the saved image.
    restore: bool,
    /// The temporal blend of `TemporalSpec` n (a combine: src = current YCC, aux = original).
    temporal: Option<usize>,
}

fn step(code: u32, i1: [u32; 4], p: &[f32]) -> Step {
    let mut q = [0.0; 24];
    for (d, s) in q.iter_mut().zip(p) {
        *d = *s;
    }
    Step { run: false, code, i1, p: q, combine: false, unsharp_blur: false, program: None, codes: Vec::new(), save: false, restore: false, temporal: None }
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
            let (Some((kp, kc)), Some((cp, cc))) = (crate::colorops::program_and_codes(key), crate::colorops::program_and_codes(correct)) else {
                log::warn!("HSL Secondary chains too large for the GPU program; skipped");
                return out;
            };
            let mut k = step(CHAIN_KEY, [kp.len() as u32, 0, 0, 0], &[]);
            k.program = Some(kp);
            k.codes = kc;
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
            c.codes = cc;
            c.combine = true;
            out.push(c);
        }
        FxOp::ColorChain(ops) => match crate::colorops::program_and_codes(ops) {
            Some((p, codes)) if !p.is_empty() => {
                let mut s = step(0, [p.len() as u32, 0, 0, 0], &[]);
                s.program = Some(p);
                s.codes = codes;
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
        // (built by `FxStage::job`, which has the neighbours' bind groups)
        FxOp::TemporalDenoise { .. } => {}
    }
    out
}

/// Default GPU memory the effect stage's working textures of one layer may take (bytes): above
/// it, temporal noise reduction degrades to ±1 frame, then is skipped. 4 GiB fits ±2 at 6.2K
/// (8 textures of 413 MB). Override with the `FILMCRAFT_FX_VRAM_MB` environment variable or
/// `GpuCompositor::set_fx_vram_budget`.
pub const FX_VRAM_BUDGET: u64 = 4 << 30;

/// [`FX_VRAM_BUDGET`], or `FILMCRAFT_FX_VRAM_MB` (megabytes) when set to a number.
pub fn fx_vram_budget_default() -> u64 {
    std::env::var("FILMCRAFT_FX_VRAM_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()).map_or(FX_VRAM_BUDGET, |mb| mb.saturating_mul(1 << 20))
}

/// The main passes of `fx` and its temporal ops, with temporal radii capped at `cap` (0: the
/// temporal ops are left out). `neighbours[j]` is aligned with `tnr::select` at the op's own
/// radius; capping drops the farther neighbours from that list.
fn plan_steps(fx: &LayerFx, neighbours: &[Vec<Option<wgpu::BindGroup>>], max: u32, cap: u8) -> (Vec<Step>, Vec<TemporalSpec>) {
    let mut all: Vec<Step> = Vec::new();
    let mut specs: Vec<TemporalSpec> = Vec::new();
    for (j, op) in fx.ops.iter().enumerate() {
        let FxOp::TemporalDenoise { luma, chroma, radius, motion, inputs } = op else {
            all.extend(steps(op));
            continue;
        };
        let (Some(inp), Some(params)) = (inputs, filmcraft_render::tnr::params(*luma, *chroma, *motion)) else { continue };
        let bgs = neighbours.get(j);
        let nbrs: Vec<_> = filmcraft_render::tnr::select(inp, *radius)
            .iter()
            .enumerate()
            .filter_map(|(slot, &i)| {
                let n = inp.neighbours.get(i)?;
                let bg = bgs?.get(slot)?.as_ref()?;
                if n.offset.unsigned_abs() > cap {
                    return None;
                }
                let (f, b) = (n.flow.sanitized(), n.back.sanitized());
                (f.w as u32 <= max && f.h as u32 <= max).then(|| (bg.clone(), f, b))
            })
            .take(TEMPORAL_SLOTS)
            .collect();
        if nbrs.is_empty() {
            continue;
        }
        let pre: Vec<Step> = fx.ops.get(..j).unwrap_or_default().iter().filter(|o| !matches!(o, FxOp::TemporalDenoise { .. })).flat_map(steps).collect();
        specs.push(TemporalSpec { nbrs, pre, params });
        let mut init = step(OP_TEMPORAL_INIT, [0; 4], &[]);
        init.unsharp_blur = true;
        all.push(init);
        let mut acc = step(0, [0; 4], &[]);
        acc.combine = true;
        acc.temporal = Some(specs.len() - 1);
        all.push(acc);
    }
    (all, specs)
}

/// Working textures a list of passes needs (ping-pong, Unsharp's original, a masked save).
fn textures_for(steps: &[Step]) -> usize {
    2 + usize::from(steps.iter().any(|s| s.combine)) + usize::from(steps.iter().any(|s| s.save))
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

    pub(crate) fn with_format(device: &wgpu::Device, composite: &wgpu::ShaderModule, layer_bgl: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filmcraft-fx"),
            source: wgpu::ShaderSource::Wgsl(wgsl_for(include_str!("fx.wgsl"), format).into()),
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
                        format,
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
        let chain_src = wgsl_for(include_str!("colorops.wgsl"), format);
        let chain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filmcraft-colorops"),
            source: wgpu::ShaderSource::Wgsl(chain_src.as_str().into()),
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
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // the temporal pass: group 1 = its constants, the neighbours' (Y, Cb, Cr, α) and their
        // forward / backward flows (`Rg32Float`, read with textureLoad)
        let mut tentries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }];
        tentries.extend((1..=(3 * TEMPORAL_SLOTS) as u32).map(tex));
        let tbgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fx-temporal"), entries: &tentries });
        let tpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-temporal"),
            bind_group_layouts: &[Some(&bgl), Some(&tbgl)],
            immediate_size: 0,
        });
        let temporal = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fx-temporal"),
            layout: Some(&tpl),
            module: &shader,
            entry_point: Some("temporal_accumulate"),
            compilation_options: Default::default(),
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
            temporal,
            tbgl,
            pool: HashMap::new(),
            budget: fx_vram_budget_default(),
            used: HashSet::new(),
            format,
            chain_src,
            pl,
            special: Mutex::new(HashMap::new()),
            shader,
            px_special: Mutex::new(HashMap::new()),
            interpret_only: false,
        }
    }

    /// The working texture format.
    pub(crate) fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Set the working-texture budget (bytes; see [`FX_VRAM_BUDGET`]).
    pub(crate) fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
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

    /// Run colour chains on the interpreter only (false: specialised shaders where they build).
    #[cfg(test)]
    pub(crate) fn set_interpret_only(&mut self, on: bool) {
        self.interpret_only = on;
    }

    /// Specialised chain pipelines built so far, and builds that failed.
    #[cfg(test)]
    pub(crate) fn specialised_count(&self) -> (usize, usize) {
        let cache = self.special.lock().unwrap_or_else(PoisonError::into_inner);
        let built = cache.values().filter(|p| p.is_some()).count();
        (built, cache.len() - built)
    }

    /// Per-op effect pipelines built so far, and builds that failed.
    #[cfg(test)]
    pub(crate) fn px_specialised_count(&self) -> (usize, usize) {
        let cache = self.px_special.lock().unwrap_or_else(PoisonError::into_inner);
        let built = cache.values().filter(|p| p.is_some()).count();
        (built, cache.len() - built)
    }

    /// No working textures held.
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.pool.is_empty()
    }

    fn targets(&mut self, device: &wgpu::Device, size: (u32, u32), n: usize) -> Vec<Target> {
        let format = self.format;
        self.used.insert(size);
        let v = self.pool.entry(size).or_default();
        while v.len() < n {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filmcraft-fx"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
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
    /// uniforms map the frame onto the working image). `neighbours[j]`: for a temporal op at
    /// `fx.ops[j]`, the source bind groups of its `tnr::select`ed neighbours (same mapping as
    /// `source_bg`; None where a neighbour can't be drawn, which skips it). None when the working
    /// image does not fit in a texture (the layer is then drawn without its effects — never
    /// larger than the source frame, which would not have uploaded either).
    pub(crate) fn job(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fx: &LayerFx,
        source_bg: wgpu::BindGroup,
        neighbours: &[Vec<Option<wgpu::BindGroup>>],
        dummy: &wgpu::TextureView,
    ) -> Option<FxJob> {
        let (w, h) = (fx.size.0.max(1), fx.size.1.max(1));
        let max = device.limits().max_texture_dimension_2d;
        if w > max || h > max {
            return None;
        }
        // the working textures must fit the budget: temporal ops degrade to ±1, then are skipped
        let px_bytes = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(texel_bytes(self.format));
        let wanted = fx.ops.iter().any(|o| matches!(o, FxOp::TemporalDenoise { inputs: Some(_), .. }));
        let mut chosen = None;
        for cap in [filmcraft_render::tnr::MAX_RADIUS, 1, 0] {
            let (all, specs) = plan_steps(fx, neighbours, max, cap);
            let n_main = textures_for(&all);
            // per temporal op: a (Y, Cb, Cr, α) texture per neighbour plus scratch for its replay
            let extra = specs.iter().map(|t| t.nbrs.len() + if t.pre.is_empty() { 1 } else { textures_for(&t.pre) }).max().unwrap_or(0);
            let bytes = px_bytes.saturating_mul((n_main + extra) as u64);
            if bytes <= self.budget || cap == 0 {
                if wanted && cap < filmcraft_render::tnr::MAX_RADIUS {
                    log::warn!(
                        "temporal noise reduction at {w}×{h}: working textures over the {} MB GPU budget; {}",
                        self.budget >> 20,
                        if cap == 0 { "skipped (identity)" } else { "limited to ±1 frame" }
                    );
                }
                chosen = Some((all, specs, n_main, extra));
                break;
            }
        }
        let (all, specs, n_main, extra) = chosen?;
        let targets = self.targets(device, (w, h), n_main + extra);
        let views: Vec<wgpu::TextureView> = targets.iter().map(|t| t.1.clone()).collect();
        let main: Vec<usize> = (0..n_main).collect();
        let spare: Vec<usize> = (n_main..n_main + extra).collect();
        let mut work = vec![Work::Source { bg: source_bg, target: views.first()?.clone() }];
        let cx = RouteCx { device, queue, views: &views, size: (w, h), dummy, specs: &specs, spare: &spare };
        let cur = self.route(&cx, &all, &main, 0, &mut work)?;
        Some(FxJob { work, result: views.get(cur)?.clone(), result_texture: targets.get(cur)?.0.clone() })
    }

    /// Route `steps` through the working textures `slots` (indices into `cx.views`) starting
    /// from the image in `start`; the passes are appended to `work`. The index holding the
    /// result.
    fn route(&self, cx: &RouteCx<'_>, steps: &[Step], slots: &[usize], start: usize, work: &mut Vec<Work>) -> Option<usize> {
        let mut cur = start;
        // Unsharp: the image its blur started from (kept while its blur passes run)
        let mut orig: Option<usize> = None;
        // a masked effect: the image before it (kept until its mask mix)
        let mut saved: Option<usize> = None;
        for s in steps {
            if s.save {
                saved = Some(cur);
                continue;
            }
            // (an Unsharp without blur passes, radius 0, combines the original with itself)
            if (s.unsharp_blur || s.combine) && orig.is_none() {
                orig = Some(cur);
            }
            let keep = if s.unsharp_blur || s.combine { orig } else { None };
            let dst = slots.iter().copied().find(|k| *k != cur && Some(*k) != keep && Some(*k) != saved)?;
            let aux = match (s.combine, orig, s.restore, saved) {
                (true, Some(o), _, _) => cx.views.get(o)?,
                (_, _, true, Some(v)) => cx.views.get(v)?,
                _ => cx.dummy,
            };
            let (src_v, dst_v) = (cx.views.get(cur)?, cx.views.get(dst)?);
            match s.temporal {
                Some(t) => {
                    let spec = cx.specs.get(t)?;
                    let nbr = self.temporal_neighbours(cx, spec, work)?;
                    work.push(Work::Compute(self.temporal_dispatch(cx, spec, src_v, dst_v, aux, &nbr)?));
                }
                None => work.push(Work::Compute(self.dispatch(cx, s, src_v, dst_v, aux))),
            }
            cur = dst;
            if s.combine {
                orig = None;
            }
            if s.restore {
                saved = None;
            }
        }
        Some(cur)
    }

    /// One pass of `s` from `src` into `dst` (`aux`: the image a combine or mask mix reads).
    fn dispatch(&self, cx: &RouteCx<'_>, s: &Step, src: &wgpu::TextureView, dst: &wgpu::TextureView, aux: &wgpu::TextureView) -> Dispatch {
        let (device, queue, (w, h)) = (cx.device, cx.queue, cx.size);
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
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(dst) },
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
        let special = match pipe {
            Pipe::Chain | Pipe::ChainKey | Pipe::ChainMasked => self.special_pipeline(device, pipe, &s.codes),
            Pipe::Px => self.px_pipeline(device, s.code),
            _ => None,
        };
        Dispatch { pipe, bg, bg1: None, groups, special }
    }

    /// The chain pipeline for `pipe` specialised to `codes`, built on first use and cached.
    /// None: specialisation off, no codes, or the build failed (the interpreter runs the chain).
    fn special_pipeline(&self, device: &wgpu::Device, pipe: Pipe, codes: &[u32]) -> Option<wgpu::ComputePipeline> {
        if codes.is_empty() || codes.len() > crate::colorops::MAX_SPECIALISED_OPS || self.interpret_only || !specialise_chains() {
            return None;
        }
        let key = (pipe, codes.to_vec());
        let mut cache = self.special.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = cache.get(&key) {
            return p.clone();
        }
        let built = self.build_special(device, pipe, codes);
        if cache.len() >= MAX_SPECIAL_PIPELINES {
            cache.clear();
        }
        cache.insert(key, built.clone());
        built
    }

    /// The `fx_px` pipeline built for op `code` alone, built on first use and cached. None:
    /// specialisation off, code 0, or the build failed (the generic pipeline runs the pass).
    fn px_pipeline(&self, device: &wgpu::Device, code: u32) -> Option<wgpu::ComputePipeline> {
        if code == 0 || self.interpret_only || !specialise_chains() {
            return None;
        }
        let mut cache = self.px_special.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = cache.get(&code) {
            return p.clone();
        }
        let built = self.build_px(device, code);
        if cache.len() >= MAX_SPECIAL_PIPELINES {
            cache.clear();
        }
        cache.insert(code, built.clone());
        built
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn build_px(&self, device: &wgpu::Device, code: u32) -> Option<wgpu::ComputePipeline> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fx-px-specialised"),
            layout: Some(&self.pl),
            module: &self.shader,
            entry_point: Some("fx_px"),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &[("PX_OP", f64::from(code))], ..Default::default() },
            cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            log::warn!("effect pass {code} failed to build alone, using the generic pass: {e}");
            return None;
        }
        Some(pipeline)
    }

    #[cfg(target_arch = "wasm32")]
    fn build_px(&self, _device: &wgpu::Device, _code: u32) -> Option<wgpu::ComputePipeline> {
        None
    }

    fn build_special(&self, device: &wgpu::Device, pipe: Pipe, codes: &[u32]) -> Option<wgpu::ComputePipeline> {
        let entry = match pipe {
            Pipe::ChainKey => "chain_key",
            Pipe::ChainMasked => "chain_masked",
            _ => "chain_px",
        };
        crate::colorops::build_specialised(device, &self.pl, &self.chain_src, codes, entry)
    }

    /// Draw each neighbour of `spec` into scratch, replay the passes before the temporal op on
    /// it and convert it to (Y, Cb, Cr, α) into its own texture. Those textures' indices.
    fn temporal_neighbours(&self, cx: &RouteCx<'_>, spec: &TemporalSpec, work: &mut Vec<Work>) -> Option<Vec<usize>> {
        let k = spec.nbrs.len();
        let (slots, scratch) = (cx.spare.get(..k)?, cx.spare.get(k..)?);
        let first = *scratch.first()?;
        for ((bg, _, _), &slot) in spec.nbrs.iter().zip(slots) {
            work.push(Work::Source { bg: bg.clone(), target: cx.views.get(first)?.clone() });
            let end = self.route(cx, &spec.pre, scratch, first, work)?;
            let init = step(OP_TEMPORAL_INIT, [0; 4], &[]);
            work.push(Work::Compute(self.dispatch(cx, &init, cx.views.get(end)?, cx.views.get(slot)?, cx.dummy)));
        }
        Some(slots.to_vec())
    }

    /// The temporal blend: `src` = the current image as (Y, Cb, Cr, α), `aux` = the original,
    /// `nbr` = the neighbours' converted textures (indices into `cx.views`).
    fn temporal_dispatch(
        &self,
        cx: &RouteCx<'_>,
        spec: &TemporalSpec,
        src: &wgpu::TextureView,
        dst: &wgpu::TextureView,
        aux: &wgpu::TextureView,
        nbr: &[usize],
    ) -> Option<Dispatch> {
        let (device, queue) = (cx.device, cx.queue);
        let base = self.dispatch(cx, &step(0, [0; 4], &[]), src, dst, aux);
        let p = &spec.params;
        // TU: n = (count, luma on, chroma on, 0), k = (luma k, chroma k, agree k, 0),
        // gi[4] = (grid w, grid h, 0, 0), gs[4] = (scale x, scale y, 0, 0)
        let mut bytes: Vec<u8> = Vec::with_capacity(160);
        for v in [spec.nbrs.len() as u32, u32::from(p.luma_k.is_some()), u32::from(p.chroma_k.is_some()), 0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for v in [p.luma_k.unwrap_or(0.0), p.chroma_k.unwrap_or(0.0), p.agree_k, 0.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for i in 0..TEMPORAL_SLOTS {
            let (gw, gh) = spec.nbrs.get(i).map_or((1, 1), |n| (n.1.w as u32, n.1.h as u32));
            for v in [gw, gh, 0, 0] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        for i in 0..TEMPORAL_SLOTS {
            let [sx, sy] = spec.nbrs.get(i).map_or([1.0; 2], |n| n.1.scale);
            for v in [sx, sy, 0.0, 0.0] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-temporal-u"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buf, 0, &bytes);
        let flow_view = |f: &filmcraft_render::flow::FlowField| {
            let (fw, fh) = (f.w.max(1) as u32, f.h.max(1) as u32);
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fx-flow"),
                size: wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let data: Vec<u8> = f.uv.iter().flat_map(|v| v.iter().flat_map(|c| c.to_le_bytes())).collect();
            if data.len() == (fw as usize) * (fh as usize) * 8 {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(fw * 8), rows_per_image: Some(fh) },
                    wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
                );
            }
            t.create_view(&Default::default())
        };
        let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(3 * TEMPORAL_SLOTS);
        for i in 0..TEMPORAL_SLOTS {
            views.push(match nbr.get(i) {
                Some(&v) => cx.views.get(v)?.clone(),
                None => cx.dummy.clone(),
            });
        }
        for i in 0..TEMPORAL_SLOTS {
            match spec.nbrs.get(i) {
                Some((_, f, b)) => {
                    views.push(flow_view(f));
                    views.push(flow_view(b));
                }
                None => {
                    views.push(cx.dummy.clone());
                    views.push(cx.dummy.clone());
                }
            }
        }
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }];
        entries.extend(views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry { binding: i as u32 + 1, resource: wgpu::BindingResource::TextureView(v) }));
        let bg1 = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fx-temporal"), layout: &self.tbgl, entries: &entries });
        Some(Dispatch { pipe: Pipe::Temporal, bg1: Some(bg1), ..base })
    }

    /// Record a job (source draws and compute passes, in order) into `enc`.
    pub(crate) fn record(&self, enc: &mut wgpu::CommandEncoder, job: &FxJob) {
        let mut i = 0;
        while let Some(w) = job.work.get(i) {
            match w {
                Work::Source { bg, target } => {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("fx-source"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target,
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
                    pass.set_bind_group(0, bg, &[]);
                    pass.draw(0..6, 0..1);
                    i += 1;
                }
                Work::Compute(_) => {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fx"), timestamp_writes: None });
                    while let Some(Work::Compute(d)) = job.work.get(i) {
                        pass.set_pipeline(match (&d.special, d.pipe) {
                            (Some(p), _) => p,
                            (None, pipe) => match pipe {
                                Pipe::Px => &self.px,
                                Pipe::Run => &self.run,
                                Pipe::Chain => &self.chain,
                                Pipe::ChainKey => &self.chain_key,
                                Pipe::ChainMasked => &self.chain_masked,
                                Pipe::Mask => &self.mask,
                                Pipe::Temporal => &self.temporal,
                            },
                        });
                        pass.set_bind_group(0, &d.bg, &[]);
                        if let Some(b) = &d.bg1 {
                            pass.set_bind_group(1, b, &[]);
                        }
                        pass.dispatch_workgroups(d.groups.0, d.groups.1, 1);
                        i += 1;
                    }
                }
            }
        }
    }
}

/// What routing passes needs besides the steps.
struct RouteCx<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    views: &'a [wgpu::TextureView],
    size: (u32, u32),
    dummy: &'a wgpu::TextureView,
    specs: &'a [TemporalSpec],
    /// Textures beyond the main chain's (the temporal neighbours' and their scratch).
    spare: &'a [usize],
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn the_preview_format_follows_the_device_rule_and_the_override() {
        let integrated = if HALF_ON_INTEGRATED { FX_FORMAT_HALF } else { FX_FORMAT };
        assert_eq!(fx_format_pick(wgpu::DeviceType::IntegratedGpu, None), integrated);
        assert_eq!(fx_format_pick(wgpu::DeviceType::DiscreteGpu, None), FX_FORMAT);
        assert_eq!(fx_format_pick(wgpu::DeviceType::Cpu, None), FX_FORMAT);
        assert_eq!(fx_format_pick(wgpu::DeviceType::DiscreteGpu, Some(" f16 ")), FX_FORMAT_HALF);
        assert_eq!(fx_format_pick(wgpu::DeviceType::IntegratedGpu, Some("f32")), FX_FORMAT);
        // anything else: the device's default
        assert_eq!(fx_format_pick(wgpu::DeviceType::IntegratedGpu, Some("bogus")), integrated);
        assert_eq!(texel_bytes(FX_FORMAT_HALF), 8);
        assert_eq!(texel_bytes(FX_FORMAT), 16);
    }

    #[test]
    fn half_float_shaders_write_half_float_storage() {
        for src in [include_str!("fx.wgsl"), include_str!("colorops.wgsl")] {
            assert!(src.contains("texture_storage_2d<rgba32float, write>"));
            let half = wgsl_for(src, FX_FORMAT_HALF);
            assert!(!half.contains("rgba32float") && half.contains("texture_storage_2d<rgba16float, write>"));
            assert_eq!(wgsl_for(src, FX_FORMAT), src);
        }
    }
}
