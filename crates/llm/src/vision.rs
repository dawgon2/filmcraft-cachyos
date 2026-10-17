//! The Qwen3-VL vision encoder on the CPU (AI-PLAN AI4 item 2): patches from
//! [`crate::vision_prep::patches`] → merged picture tokens for the language model, plus the
//! DeepStack taps. Structure as the model's published architecture (patch embed as one matmul,
//! learned position table interpolated bilinearly to the grid, pre-norm blocks with 2-D rotary
//! attention, 2 × 2 merger); checked against candle's encoder in `tests/qwen3vl_vision.rs`.
//! Plain f32, rows in parallel with the `infer` feature; `gpu_vision` (feature `gpu`) runs the
//! same encoder on the GPU and is checked against this one.

use std::collections::BTreeMap;
use std::path::Path;

use crate::LlmError;
use crate::safetensors::{self, Header};
use crate::vision_config::VisionConfig;

/// Most patches one picture may have (the processor's 16 777 216-pixel cap at 16 × 16 patches).
pub const MAX_PATCHES: usize = 65_536;
/// Largest single tensor read (elements).
const MAX_TENSOR: u64 = 1 << 26;
const NORM_EPS: f32 = 1e-6;
const ROPE_THETA: f32 = 10_000.0;
const PREFIX: &str = "model.visual.";

pub(crate) struct Linear {
    /// `out × in`, row-major.
    pub(crate) w: Vec<f32>,
    pub(crate) b: Vec<f32>,
    pub(crate) inp: usize,
    pub(crate) out: usize,
}

pub(crate) struct Norm {
    pub(crate) w: Vec<f32>,
    pub(crate) b: Vec<f32>,
}

pub(crate) struct Block {
    pub(crate) norm1: Norm,
    pub(crate) norm2: Norm,
    pub(crate) qkv: Linear,
    pub(crate) proj: Linear,
    pub(crate) fc1: Linear,
    pub(crate) fc2: Linear,
}

pub(crate) struct Merger {
    /// Normalise after grouping 2 × 2 tokens (DeepStack mergers) or before (the final one).
    pub(crate) post_shuffle: bool,
    pub(crate) norm: Norm,
    pub(crate) fc1: Linear,
    pub(crate) fc2: Linear,
}

/// The encoder's output for one picture.
#[derive(Clone, Debug)]
pub struct VisionOutput {
    /// Merged tokens: `count × out_hidden`, in the patches' 2 × 2 block order.
    pub tokens: Vec<f32>,
    /// One `count × out_hidden` tensor per DeepStack tap, in `VisionConfig::deepstack` order.
    pub deepstack: Vec<Vec<f32>>,
    pub count: usize,
    /// The merged token grid: `rows × cols = count`, in reading order (M-RoPE needs it).
    pub rows: usize,
    pub cols: usize,
}

/// A loaded encoder.
pub struct VisionEncoder {
    pub(crate) cfg: VisionConfig,
    pub(crate) patch: Linear,
    /// The learned position table, `positions × hidden`.
    pub(crate) pos: Vec<f32>,
    /// Side of the (square) position table.
    pub(crate) side: usize,
    pub(crate) blocks: Vec<Block>,
    pub(crate) merger: Merger,
    pub(crate) deepstack: Vec<Merger>,
}

fn bad(reason: impl Into<String>) -> LlmError {
    LlmError::BadModel { file: "vision encoder".into(), reason: reason.into() }
}

/// Reads the vision tensors of a sharded safetensors model folder.
struct Loader<'a> {
    dir: &'a Path,
    index: BTreeMap<String, String>,
    headers: BTreeMap<String, Header>,
}

impl Loader<'_> {
    fn get(&mut self, name: &str, shape: &[usize]) -> Result<Vec<f32>, LlmError> {
        let full = format!("{PREFIX}{name}");
        let file = self.index.get(&full).ok_or_else(|| bad(format!("missing tensor {full}")))?.clone();
        if !self.headers.contains_key(&file) {
            let h = safetensors::read_header(&self.dir.join(&file))?;
            self.headers.insert(file.clone(), h);
        }
        let info = self.headers.get(&file).and_then(|h| h.tensors.get(&full)).ok_or_else(|| bad(format!("{full} not in {file}")))?;
        let want: Vec<u64> = shape.iter().map(|&d| d as u64).collect();
        if info.shape != want {
            return Err(bad(format!("{full} has shape {:?}, expected {want:?}", info.shape)));
        }
        safetensors::read_f32(&self.dir.join(&file), info, MAX_TENSOR)
    }

    fn linear(&mut self, name: &str, inp: usize, out: usize) -> Result<Linear, LlmError> {
        Ok(Linear { w: self.get(&format!("{name}.weight"), &[out, inp])?, b: self.get(&format!("{name}.bias"), &[out])?, inp, out })
    }

    fn norm(&mut self, name: &str, dim: usize) -> Result<Norm, LlmError> {
        Ok(Norm { w: self.get(&format!("{name}.weight"), &[dim])?, b: self.get(&format!("{name}.bias"), &[dim])? })
    }

    fn merger(&mut self, name: &str, cfg: &VisionConfig, post_shuffle: bool) -> Result<Merger, LlmError> {
        let merged = cfg.hidden * cfg.merge * cfg.merge;
        Ok(Merger {
            post_shuffle,
            norm: self.norm(&format!("{name}.norm"), if post_shuffle { merged } else { cfg.hidden })?,
            fc1: self.linear(&format!("{name}.linear_fc1"), merged, merged)?,
            fc2: self.linear(&format!("{name}.linear_fc2"), merged, cfg.out_hidden)?,
        })
    }
}

impl VisionEncoder {
    /// Loads `model.visual.*` from a model folder with `model.safetensors.index.json`, every
    /// tensor's shape checked against `cfg`.
    pub fn load(dir: &Path, cfg: &VisionConfig) -> Result<Self, LlmError> {
        let in_values = 3usize.saturating_mul(cfg.temporal_patch).saturating_mul(cfg.patch).saturating_mul(cfg.patch);
        let side = (1..=4096usize).find(|s| s.saturating_mul(*s) >= cfg.positions).unwrap_or(0);
        if cfg.hidden == 0
            || cfg.heads == 0
            || !cfg.hidden.is_multiple_of(cfg.heads)
            || !cfg.head_dim().is_multiple_of(4)
            || cfg.merge == 0
            || side == 0
            || side * side != cfg.positions
            || in_values == 0
        {
            return Err(bad("unsupported vision configuration"));
        }
        if cfg.deepstack.iter().any(|&l| l >= cfg.depth) {
            return Err(bad("a DeepStack tap past the last block"));
        }
        let index_path = dir.join("model.safetensors.index.json");
        let index = safetensors::parse_index(&std::fs::read(&index_path)?, "model.safetensors.index.json")?;
        let mut l = Loader { dir, index, headers: BTreeMap::new() };
        let h = cfg.hidden;
        let patch_w = l.get("patch_embed.proj.weight", &[h, 3, cfg.temporal_patch, cfg.patch, cfg.patch])?;
        let patch = Linear { w: patch_w, b: l.get("patch_embed.proj.bias", &[h])?, inp: in_values, out: h };
        let pos = l.get("pos_embed.weight", &[cfg.positions, h])?;
        let mut blocks = Vec::with_capacity(cfg.depth.min(256));
        for i in 0..cfg.depth {
            let p = format!("blocks.{i}");
            blocks.push(Block {
                norm1: l.norm(&format!("{p}.norm1"), h)?,
                norm2: l.norm(&format!("{p}.norm2"), h)?,
                qkv: l.linear(&format!("{p}.attn.qkv"), h, h * 3)?,
                proj: l.linear(&format!("{p}.attn.proj"), h, h)?,
                fc1: l.linear(&format!("{p}.mlp.linear_fc1"), h, cfg.intermediate)?,
                fc2: l.linear(&format!("{p}.mlp.linear_fc2"), cfg.intermediate, h)?,
            });
        }
        let merger = l.merger("merger", cfg, false)?;
        let deepstack = (0..cfg.deepstack.len()).map(|i| l.merger(&format!("deepstack_merger_list.{i}"), cfg, true)).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { cfg: cfg.clone(), patch, pos, side, blocks, merger, deepstack })
    }

    /// Encodes one still picture: `patches` as [`crate::vision_prep::patches`] returns them for a
    /// grid of `grid.0` rows × `grid.1` columns of patches (both multiples of the merge size).
    pub fn encode(&self, patches: &[f32], grid: (usize, usize)) -> Result<VisionOutput, LlmError> {
        let (gh, gw) = grid;
        let m = self.cfg.merge;
        let n = gh
            .checked_mul(gw)
            .filter(|&n| n > 0 && n <= MAX_PATCHES)
            .ok_or_else(|| LlmError::Inference(format!("{gh} × {gw} patches is not a picture the encoder takes")))?;
        if !gh.is_multiple_of(m) || !gw.is_multiple_of(m) || Some(patches.len()) != n.checked_mul(self.patch.inp) {
            return Err(LlmError::Inference("patches don't match the grid".into()));
        }
        let h = self.cfg.hidden;
        let mut x = self.patch.forward(patches, n);
        let pos = self.positions(gh, gw);
        for (a, b) in x.iter_mut().zip(&pos) {
            *a += b;
        }
        let (cos, sin) = self.rotary(gh, gw);
        let mut taps = Vec::with_capacity(self.deepstack.len());
        for (i, block) in self.blocks.iter().enumerate() {
            block.forward(&mut x, n, h, self.cfg.heads, &cos, &sin);
            if let Some(mg) = self.cfg.deepstack.iter().position(|&l| l == i).and_then(|k| self.deepstack.get(k)) {
                taps.push(mg.forward(&x, n, h, m * m));
            }
        }
        let tokens = self.merger.forward(&x, n, h, m * m);
        Ok(VisionOutput { tokens, deepstack: taps, count: n / (m * m), rows: gh / m, cols: gw / m })
    }

    /// Encodes an RGB picture (`w * h * 3` values in 0..=1, row by row): resized to the
    /// processor's size ([`crate::vision_prep::target_size`]), normalised, cut into patches.
    pub fn encode_image(&self, rgb: &[f32], w: usize, h: usize) -> Result<VisionOutput, LlmError> {
        self.encode_image_within(rgb, w, h, crate::vision_prep::MAX_PIXELS)
    }

    /// [`Self::encode_image`] resized to at most `max_pixels` (clamped to the processor's bounds):
    /// fewer pixels, fewer tokens, a faster answer.
    pub fn encode_image_within(&self, rgb: &[f32], w: usize, h: usize, max_pixels: u64) -> Result<VisionOutput, LlmError> {
        let (values, grid) = prepare_image(rgb, w, h, max_pixels)?;
        self.encode(&values, grid)
    }

    /// The position table sampled bilinearly at `gh` × `gw` evenly spaced points spanning it,
    /// reordered into the merger's 2 × 2 block order.
    fn positions(&self, gh: usize, gw: usize) -> Vec<f32> {
        positions(&self.pos, self.side, self.cfg.hidden, self.cfg.merge, gh, gw)
    }

    /// Per-token rotary angles (cos, sin), `head_dim` each.
    fn rotary(&self, gh: usize, gw: usize) -> (Vec<f32>, Vec<f32>) {
        rotary(self.cfg.head_dim(), self.cfg.merge, gh, gw)
    }

    /// An encoder of shape `cfg` with deterministic pseudo-random weights, for parity tests of
    /// other implementations (the GPU one) against this one.
    #[cfg(all(test, feature = "gpu"))]
    pub(crate) fn synthetic(cfg: &VisionConfig, seed: u32) -> Result<Self, LlmError> {
        let side = cfg.positions.isqrt();
        if cfg.hidden == 0 || cfg.heads == 0 || !cfg.hidden.is_multiple_of(cfg.heads) || side * side != cfg.positions || cfg.merge == 0 {
            return Err(bad("unsupported synthetic configuration"));
        }
        /// A linear congruential generator of values in −amp/2 ..= amp/2.
        struct Gen(u32);
        impl Gen {
            fn vals(&mut self, n: usize, amp: f32) -> Vec<f32> {
                (0..n)
                    .map(|_| {
                        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        ((self.0 >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * amp
                    })
                    .collect()
            }
            fn linear(&mut self, inp: usize, out: usize) -> Linear {
                Linear { w: self.vals(inp * out, 2.0 / (inp as f32).sqrt()), b: self.vals(out, 0.2), inp, out }
            }
            fn norm(&mut self, dim: usize) -> Norm {
                Norm { w: self.vals(dim, 0.4).iter().map(|v| 1.0 + v).collect(), b: self.vals(dim, 0.2) }
            }
            fn merger(&mut self, post_shuffle: bool, h: usize, unit: usize, out_hidden: usize) -> Merger {
                Merger {
                    post_shuffle,
                    norm: self.norm(if post_shuffle { h * unit } else { h }),
                    fc1: self.linear(h * unit, h * unit),
                    fc2: self.linear(h * unit, out_hidden),
                }
            }
        }
        let mut g = Gen(seed);
        let h = cfg.hidden;
        let in_values = 3 * cfg.temporal_patch * cfg.patch * cfg.patch;
        let unit = cfg.merge * cfg.merge;
        let blocks = (0..cfg.depth)
            .map(|_| Block {
                norm1: g.norm(h),
                norm2: g.norm(h),
                qkv: g.linear(h, 3 * h),
                proj: g.linear(h, h),
                fc1: g.linear(h, cfg.intermediate),
                fc2: g.linear(cfg.intermediate, h),
            })
            .collect();
        let deepstack = (0..cfg.deepstack.len()).map(|_| g.merger(true, h, unit, cfg.out_hidden)).collect();
        let merger = g.merger(false, h, unit, cfg.out_hidden);
        Ok(Self { cfg: cfg.clone(), patch: g.linear(in_values, h), pos: g.vals(cfg.positions * h, 0.5), side, blocks, merger, deepstack })
    }
}

/// Resizes an RGB picture (`w * h * 3` values in 0..=1) to the processor's size within
/// `max_pixels` (clamped to its bounds), normalises it and cuts it into patches: the values
/// [`VisionEncoder::encode`] takes and their grid.
pub(crate) fn prepare_image(rgb: &[f32], w: usize, h: usize, max_pixels: u64) -> Result<(Vec<f32>, (usize, usize)), LlmError> {
    use crate::vision_prep::{MAX_PIXELS, MIN_PIXELS, patches, resize_bicubic, target_size};
    let bad_picture = || LlmError::Inference(format!("a {w} × {h} picture can't be encoded"));
    let max_pixels = max_pixels.clamp(MIN_PIXELS, MAX_PIXELS);
    let (tw, th) = target_size(u32::try_from(w).map_err(|_| bad_picture())?, u32::try_from(h).map_err(|_| bad_picture())?, MIN_PIXELS, max_pixels)
        .ok_or_else(bad_picture)?;
    let (tw, th) = (tw as usize, th as usize);
    let resized = if (tw, th) == (w, h) { rgb.to_vec() } else { resize_bicubic(rgb, w, h, tw, th).ok_or_else(bad_picture)? };
    let (grid, values) = patches(&resized, tw, th).ok_or_else(bad_picture)?;
    Ok((values, grid))
}

/// The position table `pos` (`side × side` rows of `h`) sampled bilinearly at `gh` × `gw`
/// evenly spaced points spanning it, reordered into the merger's `m × m` block order.
pub(crate) fn positions(pos: &[f32], side: usize, h: usize, m: usize, gh: usize, gw: usize) -> Vec<f32> {
    let last = side.saturating_sub(1);
    let points = |steps: usize| -> Vec<(usize, usize, f32)> {
        (0..steps)
            .map(|i| {
                let v = if steps == 1 { 0.0 } else { i as f32 * (last as f32 / (steps - 1) as f32) };
                let f = (v.floor() as usize).min(last);
                ((f), (v.ceil() as usize).min(last), v - f as f32)
            })
            .collect()
    };
    let (rows, cols) = (points(gh), points(gw));
    let m = m.max(1);
    let mut out = vec![0.0f32; gh * gw * h];
    let mut t = 0;
    for by in 0..gh / m {
        for bx in 0..gw / m {
            for iy in 0..m {
                for ix in 0..m {
                    let (Some(&(rf, rc, dh)), Some(&(cf, cc, dw)), Some(dst)) = (rows.get(by * m + iy), cols.get(bx * m + ix), out.get_mut(t * h..(t + 1) * h))
                    else {
                        t += 1;
                        continue;
                    };
                    for (idx, wgt) in [
                        (rf * side + cf, (1.0 - dh) * (1.0 - dw)),
                        (rf * side + cc, (1.0 - dh) * dw),
                        (rc * side + cf, dh * (1.0 - dw)),
                        (rc * side + cc, dh * dw),
                    ] {
                        if let Some(src) = pos.get(idx * h..(idx + 1) * h) {
                            for (d, s) in dst.iter_mut().zip(src) {
                                *d += wgt * s;
                            }
                        }
                    }
                    t += 1;
                }
            }
        }
    }
    out
}

/// Per-token rotary angles (cos, sin), `hd` each, tokens in `m × m` block order: a quarter of
/// the head turns with the patch row, a quarter with the column, and that half is repeated.
pub(crate) fn rotary(hd: usize, m: usize, gh: usize, gw: usize) -> (Vec<f32>, Vec<f32>) {
    let quarter = hd / 4;
    let inv: Vec<f32> = (0..quarter).map(|j| 1.0 / ROPE_THETA.powf((2 * j) as f32 / (hd / 2).max(1) as f32)).collect();
    let m = m.max(1);
    let n = gh * gw;
    let (mut cos, mut sin) = (Vec::with_capacity(n * hd), Vec::with_capacity(n * hd));
    for by in 0..gh / m {
        for bx in 0..gw / m {
            for iy in 0..m {
                for ix in 0..m {
                    let (r, c) = ((by * m + iy) as f32, (bx * m + ix) as f32);
                    let half: Vec<f32> = inv.iter().map(|f| r * f).chain(inv.iter().map(|f| c * f)).collect();
                    for _ in 0..2 {
                        cos.extend(half.iter().map(|a| a.cos()));
                        sin.extend(half.iter().map(|a| a.sin()));
                    }
                }
            }
        }
    }
    (cos, sin)
}

/// Rows handled together, so each weight row (or key / value row) is read once per block
/// instead of once per row.
const TILE: usize = 16;

impl Linear {
    /// `rows × out` from `rows × inp`.
    pub(crate) fn forward(&self, x: &[f32], rows: usize) -> Vec<f32> {
        let mut y = vec![0.0f32; rows * self.out];
        #[cfg(feature = "infer")]
        {
            use rayon::prelude::*;
            let arch = crate::quant::arch(crate::quant::simd_level());
            x.par_chunks(self.inp * TILE).zip(y.par_chunks_mut(self.out * TILE)).for_each(|(xb, yb)| arch.dispatch(LinearTile { lin: self, xb, yb }));
        }
        #[cfg(not(feature = "infer"))]
        x.chunks(self.inp * TILE).zip(y.chunks_mut(self.out * TILE)).for_each(|(xb, yb)| linear_tile(self, xb, yb, dot));
        y
    }
}

#[inline(always)]
fn linear_tile(lin: &Linear, xb: &[f32], yb: &mut [f32], dot: impl Fn(&[f32], &[f32]) -> f32) {
    let n = xb.len() / lin.inp;
    for (c, (w, b)) in lin.w.chunks_exact(lin.inp).zip(&lin.b).enumerate() {
        for r in 0..n {
            if let (Some(xr), Some(o)) = (xb.get(r * lin.inp..(r + 1) * lin.inp), yb.get_mut(r * lin.out + c)) {
                *o = dot(xr, w) + b;
            }
        }
    }
}

/// One tile of [`Linear::forward`], compiled for the CPU's SIMD level.
#[cfg(feature = "infer")]
struct LinearTile<'a> {
    lin: &'a Linear,
    xb: &'a [f32],
    yb: &'a mut [f32],
}

#[cfg(feature = "infer")]
impl pulp::WithSimd for LinearTile<'_> {
    type Output = ();
    #[inline(always)]
    fn with_simd<S: pulp::Simd>(self, simd: S) {
        linear_tile(self.lin, self.xb, self.yb, |a, b| dot_simd(simd, a, b));
    }
}

/// [`dot`] on SIMD registers (two accumulators, fused multiply-add where the CPU has it).
#[cfg(feature = "infer")]
#[inline(always)]
fn dot_simd<S: pulp::Simd>(simd: S, a: &[f32], b: &[f32]) -> f32 {
    let (av, at) = S::as_simd_f32s(a);
    let (bv, bt) = S::as_simd_f32s(b);
    let (mut acc0, mut acc1) = (simd.splat_f32s(0.0), simd.splat_f32s(0.0));
    let (ap, ar) = av.as_chunks::<2>();
    let (bp, br) = bv.as_chunks::<2>();
    for (x, y) in ap.iter().zip(bp) {
        acc0 = simd.mul_add_e_f32s(x[0], y[0], acc0);
        acc1 = simd.mul_add_e_f32s(x[1], y[1], acc1);
    }
    for (x, y) in ar.iter().zip(br) {
        acc0 = simd.mul_add_e_f32s(*x, *y, acc0);
    }
    simd.reduce_sum_f32s(simd.add_f32s(acc0, acc1)) + at.iter().zip(bt).map(|(x, y)| x * y).sum::<f32>()
}

#[inline(always)]
#[cfg_attr(feature = "infer", allow(dead_code))] // the portable path: builds without `infer`, and the tests
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ac, bc) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    let tail: f32 = ac.1.iter().zip(bc.1).map(|(x, y)| x * y).sum();
    for (x, y) in ac.0.iter().zip(bc.0) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    acc.iter().sum::<f32>() + tail
}

impl Norm {
    /// Layer norm of each `dim`-wide row.
    fn forward(&self, x: &[f32], dim: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(x.len());
        for r in x.chunks_exact(dim) {
            let mean = r.iter().sum::<f32>() / dim as f32;
            let var = r.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / dim as f32;
            let inv = 1.0 / (var + NORM_EPS).sqrt();
            out.extend(r.iter().zip(&self.w).zip(&self.b).map(|((v, w), b)| (v - mean) * inv * w + b));
        }
        out
    }
}

fn gelu_tanh(x: f32) -> f32 {
    0.5 * x * (1.0 + (0.797_884_6 * (x + 0.044_715 * x * x * x)).tanh())
}

/// GELU with the exact error function (the merger's activation in the reference model).
fn gelu_erf(x: f32) -> f32 {
    0.5 * x * (1.0 + erf(x / std::f32::consts::SQRT_2))
}

/// Abramowitz & Stegun 7.1.26 (|error| < 1.5e-7).
fn erf(x: f32) -> f32 {
    let s = x.signum();
    let x = x.abs() as f64;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0 - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t * (-x * x).exp();
    s * y as f32
}

impl Block {
    fn forward(&self, x: &mut [f32], n: usize, h: usize, heads: usize, cos: &[f32], sin: &[f32]) {
        let hd = h / heads;
        let normed = self.norm1.forward(x, h);
        let mut qkv = self.qkv.forward(&normed, n);
        // rotate q and k of every head: x·cos + rotate_half(x)·sin
        for (t, row) in qkv.chunks_exact_mut(3 * h).enumerate() {
            let (Some(c), Some(s), Some(qk)) = (cos.get(t * hd..(t + 1) * hd), sin.get(t * hd..(t + 1) * hd), row.get_mut(..2 * h)) else { continue };
            for head in qk.chunks_exact_mut(hd) {
                let orig: Vec<f32> = head.to_vec();
                let (lo, hi) = orig.split_at(hd / 2);
                // rotate_half(x) = [-x_hi, x_lo]
                let rotated = hi.iter().map(|v| -v).chain(lo.iter().copied());
                for ((((v, o), r), c), s) in head.iter_mut().zip(&orig).zip(rotated).zip(c).zip(s) {
                    *v = o * c + r * s;
                }
            }
        }
        let attn = attention(&qkv, n, h, heads);
        let out = self.proj.forward(&attn, n);
        for (a, b) in x.iter_mut().zip(&out) {
            *a += b;
        }
        let normed = self.norm2.forward(x, h);
        let mut mid = self.fc1.forward(&normed, n);
        mid.iter_mut().for_each(|v| *v = gelu_tanh(*v));
        let out = self.fc2.forward(&mid, n);
        for (a, b) in x.iter_mut().zip(&out) {
            *a += b;
        }
    }
}

/// Full (bidirectional) attention over one picture's tokens, [`TILE`] queries at a time so
/// memory stays O(TILE × n) per block and each key / value row is read once per block.
/// `qkv` rows are `[q | k | v]`, each `h` wide.
fn attention(qkv: &[f32], n: usize, h: usize, heads: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; n * h];
    #[cfg(feature = "infer")]
    {
        use rayon::prelude::*;
        let arch = crate::quant::arch(crate::quant::simd_level());
        out.par_chunks_mut(h * TILE).enumerate().for_each(|(bi, o)| arch.dispatch(AttentionTile { qkv, n, h, heads, first: bi * TILE, o }));
    }
    #[cfg(not(feature = "infer"))]
    out.chunks_mut(h * TILE).enumerate().for_each(|(bi, o)| attention_tile(qkv, n, h, heads, bi * TILE, o, dot));
    out
}

#[inline(always)]
fn attention_tile(qkv: &[f32], n: usize, h: usize, heads: usize, first: usize, o: &mut [f32], dot: impl Fn(&[f32], &[f32]) -> f32) {
    let hd = h / heads;
    let scale = 1.0 / (hd as f32).sqrt();
    let seg = |t: usize, part: usize, head: usize| qkv.get(t * 3 * h + part * h + head * hd..t * 3 * h + part * h + (head + 1) * hd).unwrap_or(&[]);
    let rows = o.len() / h;
    let mut scores = vec![0.0f32; rows * n];
    for head in 0..heads {
        for j in 0..n {
            let k = seg(j, 1, head);
            for (r, row) in scores.chunks_exact_mut(n).enumerate() {
                if let Some(s) = row.get_mut(j) {
                    *s = dot(seg(first + r, 0, head), k) * scale;
                }
            }
        }
        for row in scores.chunks_exact_mut(n) {
            let max = row.iter().fold(f32::NEG_INFINITY, |m, v| m.max(*v));
            let mut sum = 0.0f32;
            for s in row.iter_mut() {
                *s = (*s - max).exp();
                sum += *s;
            }
            row.iter_mut().for_each(|s| *s /= sum);
        }
        for j in 0..n {
            let v = seg(j, 2, head);
            for (row, orow) in scores.chunks_exact(n).zip(o.chunks_exact_mut(h)) {
                let w = row.get(j).copied().unwrap_or(0.0);
                if let Some(dst) = orow.get_mut(head * hd..(head + 1) * hd) {
                    for (d, vv) in dst.iter_mut().zip(v) {
                        *d += w * vv;
                    }
                }
            }
        }
    }
}

/// One query tile of [`attention`], compiled for the CPU's SIMD level.
#[cfg(feature = "infer")]
struct AttentionTile<'a> {
    qkv: &'a [f32],
    n: usize,
    h: usize,
    heads: usize,
    first: usize,
    o: &'a mut [f32],
}

#[cfg(feature = "infer")]
impl pulp::WithSimd for AttentionTile<'_> {
    type Output = ();
    #[inline(always)]
    fn with_simd<S: pulp::Simd>(self, simd: S) {
        attention_tile(self.qkv, self.n, self.h, self.heads, self.first, self.o, |a, b| dot_simd(simd, a, b));
    }
}

impl Merger {
    fn forward(&self, x: &[f32], n: usize, h: usize, unit: usize) -> Vec<f32> {
        let groups = n / unit;
        let normed = if self.post_shuffle { self.norm.forward(x, h * unit) } else { self.norm.forward(x, h) };
        let mut mid = self.fc1.forward(&normed, groups);
        mid.iter_mut().for_each(|v| *v = gelu_erf(*v));
        self.fc2.forward(&mid, groups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erf_matches_known_values() {
        for (x, want) in [(0.0f32, 0.0f32), (0.5, 0.520_499_9), (1.0, 0.842_700_8), (-2.0, -0.995_322_3), (4.0, 0.999_999_98)] {
            assert!((erf(x) - want).abs() < 1e-6, "erf({x}) = {}", erf(x));
        }
    }

    #[test]
    fn attention_of_identical_keys_averages_values() {
        // two tokens, one head of width 2, equal keys → each output is the mean of the values
        let qkv = [1.0, 0.0, 1.0, 1.0, 2.0, 4.0, 0.0, 1.0, 1.0, 1.0, 6.0, 8.0];
        let out = attention(&qkv, 2, 2, 1);
        assert_eq!(out, vec![4.0, 6.0, 4.0, 6.0]);
    }

    #[test]
    fn missing_model_and_hostile_configs_are_errors() {
        let cfg = VisionConfig {
            depth: 1,
            hidden: 8,
            heads: 2,
            intermediate: 8,
            patch: 2,
            temporal_patch: 2,
            merge: 2,
            out_hidden: 4,
            positions: 4,
            deepstack: vec![],
        };
        let dir = std::env::temp_dir().join("filmcraft-vision-missing");
        assert!(VisionEncoder::load(&dir, &cfg).is_err());
        for bad_cfg in [
            VisionConfig { heads: 3, ..cfg.clone() },
            VisionConfig { positions: 5, ..cfg.clone() },
            VisionConfig { merge: 0, ..cfg.clone() },
            VisionConfig { deepstack: vec![1], ..cfg.clone() },
            VisionConfig { heads: 0, ..cfg.clone() },
        ] {
            assert!(matches!(VisionEncoder::load(&dir, &bad_cfg), Err(LlmError::BadModel { .. })), "{bad_cfg:?}");
        }
    }

    #[cfg(feature = "infer")]
    #[test]
    fn simd_dot_matches_the_portable_one() {
        let arch = pulp::Arch::new();
        for len in [0usize, 1, 7, 8, 9, 15, 16, 17, 31, 64, 100, 257, 300] {
            let a: Vec<f32> = (0..len).map(|i| ((i * 37 % 101) as f32 - 50.0) / 13.0).collect();
            let b: Vec<f32> = (0..len).map(|i| ((i * 53 % 89) as f32 - 44.0) / 7.0).collect();
            let want = dot(&a, &b);
            struct D<'a>(&'a [f32], &'a [f32]);
            impl pulp::WithSimd for D<'_> {
                type Output = f32;
                fn with_simd<S: pulp::Simd>(self, simd: S) -> f32 {
                    dot_simd(simd, self.0, self.1)
                }
            }
            let got = arch.dispatch(D(&a, &b));
            assert!((got - want).abs() <= 1e-5 * want.abs().max(1.0), "{len}: {got} vs {want}");
        }
    }

    #[cfg(feature = "infer")]
    #[test]
    fn simd_and_portable_levels_agree() {
        use crate::quant::{SimdLevel, set_simd_level};
        let (inp, out, rows) = (40usize, 24usize, 37usize);
        let lin = Linear { w: (0..inp * out).map(|i| ((i * 31 % 17) as f32 - 8.0) / 9.0).collect(), b: (0..out).map(|i| i as f32 / 10.0).collect(), inp, out };
        let x: Vec<f32> = (0..inp * rows).map(|i| ((i * 13 % 23) as f32 - 11.0) / 5.0).collect();
        let qkv: Vec<f32> = (0..rows * 3 * 16).map(|i| ((i * 7 % 29) as f32 - 14.0) / 20.0).collect();
        set_simd_level(SimdLevel::Baseline);
        let (y0, a0) = (lin.forward(&x, rows), attention(&qkv, rows, 16, 4));
        set_simd_level(SimdLevel::Auto);
        let (y1, a1) = (lin.forward(&x, rows), attention(&qkv, rows, 16, 4));
        for (p, q) in y0.iter().zip(&y1).chain(a0.iter().zip(&a1)) {
            assert!((p - q).abs() <= 1e-4 * p.abs().max(1.0), "{p} vs {q}");
        }
    }
}
