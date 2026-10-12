//! Temporal noise reduction ([`crate::gpufx::FxOp::TemporalDenoise`], fork Phase 6.7): the CPU
//! reference. `filmcraft_gpu` runs the same formula in WGSL (`fx.wgsl`: `OP_TEMPORAL_INIT`
//! converts the current frame and every neighbour once to (Y, Cb, Cr, α); `temporal_accumulate`
//! sums the neighbours and finishes the pixel in one pass).
//!
//! Own code from published methods: motion-compensated temporal filtering (Dubois & Sabri 1984)
//! with per-pixel range weights, i.e. a temporal bilateral (Bennett & McMillan 2005), motion from
//! dense pyramidal Lucas–Kanade ([`crate::flow`]; Lucas & Kanade 1981, Bouguet 2001) and the
//! forward–backward consistency check of Kalal et al. (2010).
//!
//! # Formula
//!
//! Work happens in the encoded (Y, Cb, Cr) space of spatial noise reduction
//! ([`crate::denoise::pixel_ycc`]; skipped pixels — transparent or non-finite — are zeros). For
//! every pixel `p` (centre `(x + 0.5, y + 0.5)`) whose own α > 0, and every neighbour `k` (frame
//! `t + offset`, its forward flow `f` = t → t+offset and backward flow `b` = t+offset → t, both on
//! the same grid of scale `s`):
//!
//! 1. `g = p / s − 0.5` (grid coordinates), `f = flow(g)` (bilinear, clamped), the matched image
//!    point `q = p + f·s`. `inside` = `0 ≤ q ≤ (w, h)`; outside, the neighbour gives nothing.
//! 2. `e = f + b(g + f)`, `agree = 1 / (1 + |e|² · ka)` with `ka = 1 / τ²` and the tolerance
//!    `τ = 0.25 + 1.5 · motion` grid pixels (motion clamped 0…1, NaN → 0.5).
//! 3. The neighbour's value `c_k` is its (Y, Cb, Cr) bilinearly sampled at `q` (centres at +0.5,
//!    clamped); when any of the four texels is a skipped pixel, the neighbour gives nothing.
//! 4. `Δ = (1/9) Σ_{d ∈ 3×3} |Y_t(p + d) − Y_k(q + d)|` (current: integer pixels, clamped;
//!    neighbour: bilinear as in 3). Luma weight `wl = exp(−Δ² · kl) · agree`, chroma weight
//!    `wc = exp(−((Cb_k − Cb_t)² + (Cr_k − Cr_t)²) · kc) · agree`, where `k = 1 / (2σ²)`,
//!    `σ = max(0.05 · strength, 1e-4)`; a component whose strength is 0 is not filtered.
//! 5. `Y = (Y_t + Σ wl Y_k) / (1 + Σ wl)`, `Cb, Cr = (C_t + Σ wc C_k) / (1 + Σ wc)`; back to RGB,
//!    times the pixel's own α (alpha unchanged). A non-finite result leaves the pixel unchanged.
//!
//! Neighbours whose image size differs from the frame's, or whose flows are invalid or of
//! different grids, are skipped; at most [`MAX_NEIGHBOURS`] are used (in the order given). No
//! usable neighbour, or both strengths 0, leaves the image untouched (bit-exact).

use rayon::prelude::*;

use crate::denoise::{RANGE_SIGMA_AT_FULL, from_ycc, pixel_ycc, strength};
use crate::flow::FlowField;
use crate::gpufx::{FxOp, NeighbourPicture, TemporalInputs};
use crate::image::Image;

/// The largest temporal radius (frames each side).
pub const MAX_RADIUS: u8 = 2;
/// The most neighbours one pixel blends (the GPU binds this many).
pub const MAX_NEIGHBOURS: usize = 2 * MAX_RADIUS as usize;
/// The smallest range sigma of an active component (keeps `1 / 2σ²` finite).
const MIN_SIGMA: f32 = 1e-4;

/// The evaluated constants both the CPU and the GPU use (always finite).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TnrParams {
    /// `1 / (2 σl²)`, or None when luma is not filtered.
    pub luma_k: Option<f32>,
    /// `1 / (2 σc²)`, or None when chroma is not filtered.
    pub chroma_k: Option<f32>,
    /// `1 / τ²` of the forward–backward agreement.
    pub agree_k: f32,
}

/// The motion tolerance τ (grid pixels) for a hostile `motion` (0…1, NaN → 0.5).
pub fn motion_tolerance(motion: f32) -> f32 {
    let m = if motion.is_nan() { 0.5 } else { motion.clamp(0.0, 1.0) };
    0.25 + 1.5 * m
}

/// The constants for `luma` / `chroma` strengths and `motion`; None: an exact no-op.
pub fn params(luma: f32, chroma: f32, motion: f32) -> Option<TnrParams> {
    let (l, c) = (strength(luma), strength(chroma));
    if l <= 0.0 && c <= 0.0 {
        return None;
    }
    let k = |s: f32| (s > 0.0).then(|| 1.0 / (2.0 * (s * RANGE_SIGMA_AT_FULL).max(MIN_SIGMA).powi(2)));
    let tau = motion_tolerance(motion);
    Some(TnrParams { luma_k: k(l), chroma_k: k(c), agree_k: 1.0 / (tau * tau) })
}

/// A hostile radius as 0…=[`MAX_RADIUS`].
pub fn radius(r: u8) -> u8 {
    r.min(MAX_RADIUS)
}

/// Whether a neighbour's flows can be used: both valid, on the same grid and scale.
pub fn flows_ok(flow: &FlowField, back: &FlowField) -> bool {
    flow.is_valid() && back.is_valid() && (flow.w, flow.h) == (back.w, back.h) && flow.scale == back.scale
}

/// The neighbours of `inputs` the op uses at `radius`, as indices into `inputs.neighbours`, in
/// order: offset within ±radius (not 0), usable flows, and (for source frames) the size of the
/// current frame. At most [`MAX_NEIGHBOURS`]. CPU and GPU select with this one function.
pub fn select(inputs: &TemporalInputs, radius_: u8) -> Vec<usize> {
    let r = i16::from(radius(radius_));
    inputs
        .neighbours
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            let o = i16::from(n.offset);
            o != 0
                && o.abs() <= r
                && flows_ok(&n.flow, &n.back)
                && match &n.picture {
                    NeighbourPicture::Frame { frame, .. } => (frame.width, frame.height) == inputs.source_size,
                    NeighbourPicture::Image(_) => true,
                }
        })
        .map(|(i, _)| i)
        .take(MAX_NEIGHBOURS)
        .collect()
}

/// One neighbour for [`apply_with_flows`].
#[derive(Clone, Copy, Debug)]
pub struct Neighbour<'a> {
    /// The neighbouring frame in the same state as the image being filtered.
    pub image: &'a Image,
    /// Current → neighbour.
    pub flow: &'a FlowField,
    /// Neighbour → current.
    pub back: &'a FlowField,
}

/// (Y, Cb, Cr, α) of every pixel (zeros: skipped).
fn ycc_plane(img: &Image) -> Vec<[f32; 4]> {
    img.px.par_chunks(4).map(|q| if let &[r, g, b, a] = q { pixel_ycc([r, g, b, a]) } else { [0.0; 4] }).collect()
}

#[inline]
fn texel(p: &[[f32; 4]], w: usize, x: usize, y: usize) -> [f32; 4] {
    p.get(y.saturating_mul(w).saturating_add(x)).copied().unwrap_or([0.0; 4])
}

/// Bilinear (Y, Cb, Cr) at image point `(x, y)` (centres at +0.5, clamped), None when one of
/// the four texels is skipped (α ≤ 0). `w`, `h` ≥ 1.
#[inline]
fn bilinear(p: &[[f32; 4]], w: usize, h: usize, x: f32, y: f32) -> Option<[f32; 3]> {
    let fx = (x - 0.5).clamp(0.0, w as f32 - 1.0);
    let fy = (y - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let (a, b, c, d) = (texel(p, w, x0, y0), texel(p, w, x1, y0), texel(p, w, x0, y1), texel(p, w, x1, y1));
    if a[3] <= 0.0 || b[3] <= 0.0 || c[3] <= 0.0 || d[3] <= 0.0 {
        return None;
    }
    let mut o = [0.0; 3];
    for k in 0..3 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        o[k] = top + (bot - top) * ty;
    }
    Some(o)
}

/// Bilinear Y only (skipped texels read as their zeros), as the 3×3 difference uses it.
#[inline]
fn bilinear_y(p: &[[f32; 4]], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let fx = (x - 0.5).clamp(0.0, w as f32 - 1.0);
    let fy = (y - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let (a, b, c, d) = (texel(p, w, x0, y0)[0], texel(p, w, x1, y0)[0], texel(p, w, x0, y1)[0], texel(p, w, x1, y1)[0]);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

struct Prepared {
    ycc: Vec<[f32; 4]>,
    flow: FlowField,
    back: FlowField,
}

/// One neighbour's weights and value at pixel `(x, y)` (formula steps 1–4).
#[inline]
fn contribution(cur: &[[f32; 4]], w: usize, h: usize, x: usize, y: usize, c0: [f32; 4], n: &Prepared, p: &TnrParams) -> Option<(f32, f32, [f32; 3])> {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let [sx, sy] = n.flow.scale;
    let (gx, gy) = (px / sx - 0.5, py / sy - 0.5);
    let f = n.flow.sample(gx, gy);
    let (qx, qy) = (px + f[0] * sx, py + f[1] * sy);
    if !(qx >= 0.0 && qx <= w as f32 && qy >= 0.0 && qy <= h as f32) {
        return None;
    }
    let r = n.back.sample(gx + f[0], gy + f[1]);
    let (ex, ey) = (f[0] + r[0], f[1] + r[1]);
    let fb2 = ex * ex + ey * ey;
    let agree = 1.0 / (1.0 + fb2 * p.agree_k);
    let c = bilinear(&n.ycc, w, h, qx, qy)?;
    let mut sad = 0.0f32;
    for dy in -1i64..=1 {
        let yy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
        for dx in -1i64..=1 {
            let xx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
            let yt = texel(cur, w, xx, yy)[0];
            let yk = bilinear_y(&n.ycc, w, h, qx + dx as f32, qy + dy as f32);
            sad += (yt - yk).abs();
        }
    }
    let d = sad / 9.0;
    let wl = p.luma_k.map_or(0.0, |k| (-(d * d) * k).exp() * agree);
    let wc = p.chroma_k.map_or(0.0, |k| {
        let (d1, d2) = (c[1] - c0[1], c[2] - c0[2]);
        (-(d1 * d1 + d2 * d2) * k).exp() * agree
    });
    Some((wl, wc, c))
}

/// Filter `img` in place with its `neighbours` (see the module docs).
pub fn apply_with_flows(img: &mut Image, neighbours: &[Neighbour<'_>], luma: f32, chroma: f32, motion: f32) {
    let Some(p) = params(luma, chroma, motion) else { return };
    let (w, h) = (img.w, img.h);
    let n = w.saturating_mul(h);
    if w == 0 || h == 0 || img.px.len() != n.saturating_mul(4) {
        return;
    }
    let nb: Vec<Prepared> = neighbours
        .iter()
        .filter(|k| (k.image.w, k.image.h) == (w, h) && k.image.px.len() == img.px.len() && flows_ok(k.flow, k.back))
        .take(MAX_NEIGHBOURS)
        .map(|k| Prepared { ycc: ycc_plane(k.image), flow: k.flow.sanitized(), back: k.back.sanitized() })
        .collect();
    if nb.is_empty() {
        return;
    }
    let cur = ycc_plane(img);
    let (cur, nb, p) = (&cur, &nb, &p);
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for (x, q) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let c0 = texel(cur, w, x, y);
            if c0[3] <= 0.0 {
                continue;
            }
            let (mut sl, mut wl, mut scb, mut scr, mut wc) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for k in nb {
                if let Some((a, b, c)) = contribution(cur, w, h, x, y, c0, k, p) {
                    sl += a * c[0];
                    wl += a;
                    scb += b * c[1];
                    scr += b * c[2];
                    wc += b;
                }
            }
            let yy = if p.luma_k.is_some() { (c0[0] + sl) / (1.0 + wl) } else { c0[0] };
            let (cb, cr) = if p.chroma_k.is_some() { ((c0[1] + scb) / (1.0 + wc), (c0[2] + scr) / (1.0 + wc)) } else { (c0[1], c0[2]) };
            let a = c0[3];
            let out = from_ycc([yy, cb, cr]).map(|v| v * a);
            if out.iter().all(|v| v.is_finite()) {
                q[..3].copy_from_slice(&out);
            }
        }
    });
}

/// [`FxOp::TemporalDenoise`] on `img` with `inputs`: the selected neighbours (see [`select`]),
/// source frames decoded like the layer's own (`gpufx::working_image`) and then run through
/// `before` — the ops preceding this one in the layer (temporal ones skipped) — so they are in
/// the same state as `img`. Ready images are used as they are (size must match).
pub fn apply_inputs(img: &mut Image, inputs: &TemporalInputs, luma: f32, chroma: f32, radius_: u8, motion: f32, before: &[FxOp]) {
    if params(luma, chroma, motion).is_none() || img.w == 0 || img.h == 0 {
        return;
    }
    let idx = select(inputs, radius_);
    if idx.is_empty() {
        return;
    }
    let mut owned: Vec<(Image, usize)> = Vec::with_capacity(idx.len());
    let mut shared: Vec<(&Image, usize)> = Vec::new();
    for &i in &idx {
        let Some(n) = inputs.neighbours.get(i) else { continue };
        match &n.picture {
            NeighbourPicture::Image(im) => shared.push((im.as_ref(), i)),
            NeighbourPicture::Frame { frame, decimation } => {
                let mut im = crate::gpufx::working_image(frame, *decimation, (img.w, img.h));
                // (temporal ops, also inside masked effects, are identity on a neighbour)
                crate::gpufx::apply_ops_with(before, &mut im, &[], false);
                owned.push((im, i));
            }
        }
    }
    // (keep the selection order: the sums run in the same order on the GPU)
    let mut all: Vec<(&Image, usize)> = owned.iter().map(|(im, i)| (im, *i)).chain(shared).collect();
    all.sort_by_key(|(_, i)| *i);
    let list: Vec<Neighbour<'_>> =
        all.iter().filter_map(|(im, i)| inputs.neighbours.get(*i).map(|n| Neighbour { image: im, flow: n.flow.as_ref(), back: n.back.as_ref() })).collect();
    apply_with_flows(img, &list, luma, chroma, motion);
}

/// The synthetic test sequence of the 6.7 design (shared by this crate's tests and the GPU
/// parity tests in `filmcraft_gpu`): 320×240, 7 frames, a textured disc moving (3.2, −1.7)
/// px/frame over a static textured background, encoded-domain Gaussian noise from a fixed-seed
/// generator. Test support only (the `test-support` feature, enabled by `filmcraft-gpu`'s
/// dev-dependency); panic-free like everything else.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod fixture {
    use crate::denoise::decode;
    use crate::image::Image;

    pub const W: usize = 320;
    pub const H: usize = 240;
    pub const FRAMES: usize = 7;
    /// The frame being filtered.
    pub const MID: usize = 3;
    pub const RADIUS: f32 = 40.0;
    pub const MOTION: [f32; 2] = [3.2, -1.7];
    pub const NOISE: f32 = 0.02;

    /// The disc centre in frame `i`.
    pub fn centre(i: usize) -> [f32; 2] {
        let k = i as f32 - MID as f32;
        [160.0 + MOTION[0] * k, 120.0 + MOTION[1] * k]
    }

    /// Clean encoded colour of pixel `(x, y)` in frame `i`, and the disc coverage there.
    pub fn clean_encoded(i: usize, x: usize, y: usize) -> ([f32; 3], f32) {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let bl = 0.30 + 0.06 * (0.21 * px).sin() * (0.17 * py).cos() + 0.04 * (0.11 * (px + py)).sin();
        let bg = [bl, bl * 0.95 + 0.02, bl * 0.85];
        let c = centre(i);
        let (u, v) = (px - c[0], py - c[1]);
        let dl = 0.70 + 0.06 * (0.23 * u).sin() * (0.19 * v).cos() + 0.04 * (0.13 * (u - v)).cos();
        let disc = [dl * 0.95, dl, dl * 0.8 + 0.05];
        let cov = (RADIUS + 0.5 - (u * u + v * v).sqrt()).clamp(0.0, 1.0);
        ([0, 1, 2].map(|k| bg[k] + (disc[k] - bg[k]) * cov), cov)
    }

    /// xorshift64 + Box–Muller.
    struct Rng(u64);
    impl Rng {
        fn uniform(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
        }
        fn gauss(&mut self) -> f32 {
            let (a, b) = (self.uniform(), self.uniform());
            (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
        }
    }

    /// Frame `i` as a linear premultiplied opaque image, with encoded-domain noise of `sigma`
    /// (0: clean). The seed depends on `seed` and `i`.
    pub fn frame(i: usize, sigma: f32, seed: u64) -> Image {
        let mut rng = Rng((seed ^ 0x9e37_79b9_7f4a_7c15u64.wrapping_mul(i as u64 + 1)) | 1);
        let mut img = Image::new(W, H);
        for (j, p) in img.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (c, _) = clean_encoded(i, j % W, j / W);
            for k in 0..3 {
                let n = if sigma > 0.0 { sigma * rng.gauss() } else { 0.0 };
                p[k] = decode(c[k] + n);
            }
            p[3] = 1.0;
        }
        img
    }

    /// All frames.
    pub fn frames(sigma: f32, seed: u64) -> Vec<Image> {
        (0..FRAMES).map(|i| frame(i, sigma, seed)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::denoise::to_ycc;
    use crate::flow::{chain, estimate_pair};
    use crate::gpufx::TemporalNeighbour;
    use std::sync::Arc;

    /// Flows of the fixture around MID: for offsets −2…2 (not 0), (forward, backward), by
    /// chaining adjacent pairs.
    fn flows(frames: &[Image]) -> Vec<(i8, FlowField, FlowField)> {
        let pair = |a: usize, b: usize| estimate_pair(&frames[a], &frames[b]).expect("pair");
        let (m1, m2) = (pair(MID, MID - 1), pair(MID - 1, MID - 2));
        let (p1, p2) = (pair(MID, MID + 1), pair(MID + 1, MID + 2));
        let fm2 = chain(&m1.0, &m2.0).expect("chain");
        let bm2 = chain(&m2.1, &m1.1).expect("chain");
        let fp2 = chain(&p1.0, &p2.0).expect("chain");
        let bp2 = chain(&p2.1, &p1.1).expect("chain");
        vec![(-1, m1.0, m1.1), (1, p1.0, p1.1), (-2, fm2, bm2), (2, fp2, bp2)]
    }

    fn run(frames: &[Image], fl: &[(i8, FlowField, FlowField)], r: i8, luma: f32, chroma: f32, motion: f32) -> Image {
        let mut out = frames[MID].clone();
        let list: Vec<Neighbour<'_>> = fl
            .iter()
            .filter(|(o, _, _)| o.abs() <= r)
            .map(|(o, f, b)| Neighbour { image: &frames[(MID as i64 + *o as i64) as usize], flow: f, back: b })
            .collect();
        apply_with_flows(&mut out, &list, luma, chroma, motion);
        out
    }

    fn y_at(img: &Image, x: usize, y: usize) -> f32 {
        let p = img.get(x, y);
        to_ycc([p[0], p[1], p[2]])[0]
    }

    fn clean_y(i: usize, x: usize, y: usize) -> f32 {
        let (c, _) = clean_encoded(i, x, y);
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    fn dist(i: usize, x: usize, y: usize) -> f32 {
        let c = centre(i);
        ((x as f32 + 0.5 - c[0]).powi(2) + (y as f32 + 0.5 - c[1]).powi(2)).sqrt()
    }

    /// Residual noise σ (encoded Y, against the clean frame) over pixels where `region` holds.
    fn sigma(img: &Image, region: &dyn Fn(usize, usize) -> bool) -> f32 {
        let (mut s, mut s2, mut n) = (0.0f64, 0.0f64, 0usize);
        for y in 8..H - 8 {
            for x in 8..W - 8 {
                if region(x, y) {
                    let d = (y_at(img, x, y) - clean_y(MID, x, y)) as f64;
                    s += d;
                    s2 += d * d;
                    n += 1;
                }
            }
        }
        let m = s / n as f64;
        ((s2 / n as f64 - m * m).max(0.0)).sqrt() as f32
    }

    fn background(x: usize, y: usize) -> bool {
        (MID - 2..=MID + 2).all(|i| dist(i, x, y) > RADIUS + 12.0)
    }
    fn interior(x: usize, y: usize) -> bool {
        dist(MID, x, y) < RADIUS - 6.0
    }

    /// The design's acceptance metrics on the noisy fixture.
    #[test]
    fn noise_drops_without_ghosts_or_soft_edges() {
        let noisy = frames(NOISE, 0x5eed);
        let t = std::time::Instant::now();
        let fl = flows(&noisy);
        eprintln!("flows for ±2 (6 estimates at 320×240, synthetic): {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let src = &noisy[MID];
        let (bg0, in0) = (sigma(src, &background), sigma(src, &interior));
        for (r, limit) in [(1i8, 0.70f32), (2, 0.60)] {
            let out = run(&noisy, &fl, r, 1.0, 1.0, 0.5);
            let (bg, inn) = (sigma(&out, &background) / bg0, sigma(&out, &interior) / in0);
            eprintln!("radius {r}: σ ratio background {bg:.3}, disc interior {inn:.3} (limit {limit})");
            assert!(bg <= limit && inn <= limit, "radius {r}: {bg} {inn}");
            // ghost index: in disc(t±k) \ disc(t), mean |out − clean| over the naive mean's
            let naive = {
                let mut m = Image::new(W, H);
                let ks: Vec<usize> = (MID - r as usize..=MID + r as usize).collect();
                for (j, p) in m.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    for k in 0..4 {
                        p[k] = ks.iter().map(|&i| noisy[i].px[j * 4 + k]).sum::<f32>() / ks.len() as f32;
                    }
                }
                m
            };
            let (mut eo, mut en, mut n) = (0.0f64, 0.0f64, 0usize);
            for y in 0..H {
                for x in 0..W {
                    let ghost = (1..=r as usize).any(|k| dist(MID - k, x, y) < RADIUS - 1.0 || dist(MID + k, x, y) < RADIUS - 1.0);
                    if ghost && dist(MID, x, y) > RADIUS + 1.0 {
                        let c = clean_y(MID, x, y);
                        eo += (y_at(&out, x, y) - c).abs() as f64;
                        en += (y_at(&naive, x, y) - c).abs() as f64;
                        n += 1;
                    }
                }
            }
            let gi = eo / en.max(1e-9);
            eprintln!("radius {r}: ghost index {gi:.3} over {n} px");
            assert!(gi <= 0.25, "ghost index {gi}");
            // edge: 10–90 % width of the radial profile (averaged over angles), out vs clean
            let clean = frame(MID, 0.0, 0);
            let (wc, wo) = (edge_width(&clean), edge_width(&out));
            eprintln!("radius {r}: disc edge 10–90 % {wc:.3} px → {wo:.3} px (growth {:.3})", wo - wc);
            assert!(wo - wc <= 0.5, "edge grew {wc} → {wo}");
        }
    }

    /// 10–90 % width of the disc edge in `img` at frame MID: the radial profile averaged over 256
    /// angles (texture and noise average out).
    fn edge_width(img: &Image) -> f32 {
        let c = centre(MID);
        let rs: Vec<f32> = (0..=64).map(|i| RADIUS - 8.0 + i as f32 * 0.25).collect();
        let prof: Vec<f32> = rs
            .iter()
            .map(|&r| {
                (0..256)
                    .map(|a| {
                        let t = a as f32 / 256.0 * std::f32::consts::TAU;
                        let p = img.sample_bilinear_clamped(c[0] + r * t.cos(), c[1] + r * t.sin());
                        to_ycc([p[0], p[1], p[2]])[0]
                    })
                    .sum::<f32>()
                    / 256.0
            })
            .collect();
        // falling profile: inside (bright) to outside
        let (hi, lo) = (prof[..8].iter().sum::<f32>() / 8.0, prof[prof.len() - 8..].iter().sum::<f32>() / 8.0);
        let cross = |t: f32| {
            let level = hi - (hi - lo) * t;
            for i in 1..prof.len() {
                if prof[i] <= level {
                    let (a, b) = (prof[i - 1], prof[i]);
                    return rs[i - 1] + if a > b { (a - level) / (a - b) * 0.25 } else { 0.0 };
                }
            }
            *rs.last().unwrap_or(&0.0)
        };
        cross(0.9) - cross(0.1)
    }

    #[test]
    fn static_input_is_the_plain_mean() {
        // identical frames with zero motion: the output is the frame (the mean of equal frames)
        let f = frame(MID, NOISE, 9);
        let z = FlowField::zero(W, H, 1.0);
        let list = vec![Neighbour { image: &f, flow: &z, back: &z }; 4];
        let mut out = f.clone();
        apply_with_flows(&mut out, &list, 1.0, 1.0, 0.5);
        let worst = out.px.iter().zip(&f.px).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst <= 1e-5, "{worst}");
        // static frames differing by tiny amounts (weights ≈ 1): the plain per-pixel mean
        let others: Vec<Image> = (1..=4u32)
            .map(|k| {
                let mut g = f.clone();
                for (j, v) in g.px.iter_mut().enumerate() {
                    if j % 4 != 3 {
                        *v += 2e-5 * ((j as u32 * 7 + k * 13) % 5) as f32;
                    }
                }
                g
            })
            .collect();
        let list: Vec<Neighbour<'_>> = others.iter().map(|g| Neighbour { image: g, flow: &z, back: &z }).collect();
        let mut out = f.clone();
        apply_with_flows(&mut out, &list, 1.0, 1.0, 0.5);
        let mut worst = 0.0f32;
        for j in 0..out.px.len() {
            let mean = (f.px[j] + others.iter().map(|g| g.px[j]).sum::<f32>()) / 5.0;
            worst = worst.max((out.px[j] - mean).abs());
        }
        eprintln!("static plain mean: max diff {worst:.2e}");
        assert!(worst <= 1e-5, "{worst}");
    }

    /// Static frames whose differences are of the order of the range sigma (so the weights
    /// really vary, far from the plain mean) against a direct evaluation of the formula with zero
    /// motion (the neighbour is sampled at the pixel itself).
    #[test]
    fn static_input_with_varying_weights_matches_the_formula() {
        let f = frame(MID, NOISE, 21);
        let z = FlowField::zero(W, H, 1.0);
        let others: Vec<Image> = (1..=4u64).map(|k| frame(MID, NOISE * 2.0, 100 + k)).collect();
        let list: Vec<Neighbour<'_>> = others.iter().map(|g| Neighbour { image: g, flow: &z, back: &z }).collect();
        let (luma, chroma) = (0.6, 0.8);
        let mut out = f.clone();
        apply_with_flows(&mut out, &list, luma, chroma, 0.5);
        let p = params(luma, chroma, 0.5).expect("active");
        let (kl, kc) = (p.luma_k.expect("luma"), p.chroma_k.expect("chroma"));
        let ycc = |img: &Image, x: i64, y: i64| {
            let q = img.get(x.clamp(0, W as i64 - 1) as usize, y.clamp(0, H as i64 - 1) as usize);
            to_ycc([q[0], q[1], q[2]])
        };
        let (mut worst, mut off_mean, mut low_w) = (0.0f32, 0.0f32, 0usize);
        for y in 0..H as i64 {
            for x in 0..W as i64 {
                let c0 = ycc(&f, x, y);
                let (mut sl, mut wl, mut sb, mut sr, mut wc) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
                let mut plain = c0;
                for g in &others {
                    let mut sad = 0.0f32;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            sad += (ycc(&f, x + dx, y + dy)[0] - ycc(g, x + dx, y + dy)[0]).abs();
                        }
                    }
                    let c = ycc(g, x, y);
                    let a = (-(sad / 9.0).powi(2) * kl).exp();
                    let b = (-((c[1] - c0[1]).powi(2) + (c[2] - c0[2]).powi(2)) * kc).exp();
                    low_w += usize::from(a < 0.8);
                    (sl, wl, sb, sr, wc) = (sl + a * c[0], wl + a, sb + b * c[1], sr + b * c[2], wc + b);
                    for k in 0..3 {
                        plain[k] += c[k];
                    }
                }
                let want = crate::denoise::from_ycc([(c0[0] + sl) / (1.0 + wl), (c0[1] + sb) / (1.0 + wc), (c0[2] + sr) / (1.0 + wc)]);
                let mean = crate::denoise::from_ycc(plain.map(|v| v / 5.0));
                let got = out.get(x as usize, y as usize);
                for k in 0..3 {
                    worst = worst.max((got[k] - want[k]).abs());
                    off_mean = off_mean.max((got[k] - mean[k]).abs());
                }
            }
        }
        eprintln!("static, varying weights: max diff to the formula {worst:.2e}, to the plain mean {off_mean:.2e}, {low_w} weights < 0.8");
        assert!(worst <= 1e-5, "{worst}");
        assert!(off_mean > 1e-3 && low_w > (W * H) / 10, "the weights should vary: {off_mean} {low_w}");
    }

    /// Source-frame and ready-image neighbours mix: the same frames given either way (and in a
    /// mixed list) give the same result, in the selection order.
    #[test]
    fn source_frames_and_ready_images_mix() {
        use filmcraft_frame::VideoFrame;
        let noisy = frames(NOISE, 31);
        let fl = flows(&noisy);
        let vf: Vec<Arc<VideoFrame>> = noisy.iter().map(|im| Arc::new(VideoFrame::rgba_f32(W as u32, H as u32, im.px.clone()))).collect();
        let pic = |o: i8, as_frame: bool| {
            let i = (MID as i64 + o as i64) as usize;
            if as_frame {
                NeighbourPicture::Frame { frame: vf[i].clone(), decimation: 1 }
            } else {
                NeighbourPicture::Image(Arc::new(crate::gpufx::working_image(&vf[i], 1, (W, H))))
            }
        };
        let run_with = |mask: [bool; 4]| {
            let inputs = TemporalInputs {
                source_size: (W as u32, H as u32),
                neighbours: fl
                    .iter()
                    .zip(mask)
                    .map(|((o, f, b), m)| TemporalNeighbour { offset: *o, picture: pic(*o, m), flow: Arc::new(f.clone()), back: Arc::new(b.clone()) })
                    .collect(),
            };
            let mut img = crate::gpufx::working_image(&vf[MID], 1, (W, H));
            apply_inputs(&mut img, &inputs, 1.0, 1.0, 2, 0.5, &[]);
            img
        };
        let images = run_with([false; 4]);
        assert_ne!(images, crate::gpufx::working_image(&vf[MID], 1, (W, H)));
        for mask in [[true; 4], [true, false, true, false], [false, true, false, true]] {
            assert_eq!(run_with(mask), images, "{mask:?}");
        }
    }

    fn inputs_of(frames: &[Image], fl: &[(i8, FlowField, FlowField)]) -> TemporalInputs {
        TemporalInputs {
            source_size: (W as u32, H as u32),
            neighbours: fl
                .iter()
                .map(|(o, f, b)| TemporalNeighbour {
                    offset: *o,
                    picture: NeighbourPicture::Image(Arc::new(frames[(MID as i64 + *o as i64) as usize].clone())),
                    flow: Arc::new(f.clone()),
                    back: Arc::new(b.clone()),
                })
                .collect(),
        }
    }

    #[test]
    fn the_op_is_identity_without_inputs_or_strength_and_bounded_on_hostile_params() {
        let noisy = frames(NOISE, 3);
        let fl = flows(&noisy);
        let inputs = Arc::new(inputs_of(&noisy, &fl));
        let src = noisy[MID].clone();
        let same = |op: FxOp| {
            let mut img = src.clone();
            op.apply(&mut img);
            img.px.iter().zip(&src.px).all(|(a, b)| a.to_bits() == b.to_bits())
        };
        let op = |luma: f32, chroma: f32, radius: u8, motion: f32, inputs: Option<Arc<TemporalInputs>>| FxOp::TemporalDenoise {
            luma,
            chroma,
            radius,
            motion,
            inputs,
        };
        assert!(same(op(1.0, 1.0, 2, 0.5, None)));
        assert!(same(op(1.0, 1.0, 2, 0.5, Some(Arc::new(TemporalInputs { source_size: (W as u32, H as u32), neighbours: vec![] })))));
        assert!(same(op(0.0, 0.0, 2, 0.5, Some(inputs.clone()))));
        assert!(same(op(f32::NAN, -1.0, 2, 0.5, Some(inputs.clone()))));
        assert!(same(op(1.0, 1.0, 0, 0.5, Some(inputs.clone()))));
        assert!(!same(op(1.0, 1.0, 1, 0.5, Some(inputs.clone()))));
        // radius beyond 2 is 2
        let (mut a, mut b) = (src.clone(), src.clone());
        op(1.0, 1.0, 255, 0.5, Some(inputs.clone())).apply(&mut a);
        op(1.0, 1.0, 2, 0.5, Some(inputs.clone())).apply(&mut b);
        assert_eq!(a, b);
        let vals = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 0.5];
        for &l in &vals {
            for &m in &vals {
                let mut img = src.clone();
                op(l, m, 2, m, Some(inputs.clone())).apply(&mut img);
                assert!(img.px.iter().all(|v| v.is_finite()), "{l} {m}");
                assert!(!op(l, m, 9, m, Some(inputs.clone())).gpu_ok(), "image pictures stay on the CPU");
            }
        }
    }

    #[test]
    fn hostile_neighbours_and_flows_are_skipped_or_bounded() {
        let noisy = frames(NOISE, 4);
        let fl = flows(&noisy);
        let src = noisy[MID].clone();
        let small = Image::new(W / 2, H);
        let (f, b) = (&fl[0].1, &fl[0].2);
        // size-mismatched neighbour: skipped (no change at all)
        let mut out = src.clone();
        apply_with_flows(&mut out, &[Neighbour { image: &small, flow: f, back: b }], 1.0, 1.0, 0.5);
        assert_eq!(out, src);
        // broken flows: skipped
        let mut short = f.clone();
        short.uv.truncate(3);
        let other_grid = FlowField::zero(10, 10, 32.0);
        for (ff, bb) in [(&short, b), (f, &short), (f, &other_grid)] {
            let mut out = src.clone();
            apply_with_flows(&mut out, &[Neighbour { image: &noisy[MID - 1], flow: ff, back: bb }], 1.0, 1.0, 0.5);
            assert_eq!(out, src);
        }
        // NaN / infinite flows: those pixels get no contribution, everything stays finite
        let mut nan = f.clone();
        for (i, v) in nan.uv.iter_mut().enumerate() {
            if i % 3 == 0 {
                *v = [f32::NAN, f32::INFINITY];
            }
        }
        let mut out = src.clone();
        apply_with_flows(&mut out, &[Neighbour { image: &noisy[MID - 1], flow: &nan, back: &nan }], 1.0, 1.0, 0.5);
        assert!(out.px.iter().all(|v| v.is_finite()));
        // hostile pixels, empty / tiny / broken images
        let mut bad = src.clone();
        bad.px[0] = f32::NAN;
        bad.px[44] = f32::INFINITY;
        bad.px[47] = 0.0;
        let mut out = bad.clone();
        apply_with_flows(&mut out, &[Neighbour { image: &bad, flow: f, back: b }], 1.0, 1.0, 0.5);
        assert!(out.px[0].is_nan() && out.px[47] == 0.0);
        let z = FlowField::zero(1, 1, 1.0);
        for (w, h) in [(0usize, 0usize), (1, 1), (2, 1)] {
            let im = Image::new(w, h);
            let mut o = im.clone();
            apply_with_flows(&mut o, &[Neighbour { image: &im, flow: &z, back: &z }], 1.0, 1.0, 0.5);
        }
        let broken = Image { w: 5, h: 5, px: vec![0.5; 7] };
        let mut o = broken.clone();
        apply_with_flows(&mut o, &[Neighbour { image: &broken, flow: &z, back: &z }], 1.0, 1.0, 0.5);
        assert_eq!(o, broken);
        // more than four neighbours: the first four are used
        let five = vec![Neighbour { image: &noisy[MID - 1], flow: f, back: b }; 5];
        let (mut a, mut c) = (src.clone(), src.clone());
        apply_with_flows(&mut a, &five, 1.0, 1.0, 0.5);
        apply_with_flows(&mut c, &five[..4], 1.0, 1.0, 0.5);
        assert_eq!(a, c);
    }

    /// Single-run timing of the CPU flow for one 640-px-wide pair (synthetic texture).
    #[test]
    fn flow_time_640_synthetic() {
        let (w, h) = (640usize, 427usize);
        let mk = |dx: f32, dy: f32| {
            let mut img = Image::new(w, h);
            for (j, p) in img.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = ((j % w) as f32 - dx, (j / w) as f32 - dy);
                let l = 0.3 + 0.1 * (0.21 * x).sin() * (0.17 * y).cos() + 0.05 * (0.11 * (x + y)).sin();
                *p = [l, l, l, 1.0];
            }
            img
        };
        let (a, b) = (mk(0.0, 0.0), mk(3.2, -1.7));
        let t = std::time::Instant::now();
        let f = crate::flow::estimate(&a, &b);
        let one = t.elapsed().as_secs_f64() * 1e3;
        let t = std::time::Instant::now();
        let pair = estimate_pair(&a, &b);
        let both = t.elapsed().as_secs_f64() * 1e3;
        eprintln!("CPU flow 640×427 (synthetic, single run): one direction {one:.1} ms, both directions {both:.1} ms");
        assert!(f.is_some() && pair.is_some());
    }
}
