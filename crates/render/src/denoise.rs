//! Spatial noise reduction ([`crate::gpufx::FxOp::Denoise`]): an edge-preserving bilateral filter
//! with separate luma and chroma strengths. This is the CPU reference; `filmcraft_gpu` runs the
//! same formula in WGSL (`fx.wgsl`: `OP_DENOISE_YCC` converts every pixel once, `OP_DENOISE`
//! filters).
//!
//! # Formula
//!
//! Per pixel, on straight (unpremultiplied) colour of the premultiplied linear-light working image:
//!
//! 1. Encode each channel with `e(v)`: the sRGB curve on 0…1, its linear toe below 0
//!    (`12.92 v`) and a linear extension above 1 (`1 + (v − 1) / 2.4`), so super-whites and
//!    negatives survive and decode exactly back with `e⁻¹`.
//! 2. Decorrelate: `Y = 0.2126 R′ + 0.7152 G′ + 0.0722 B′`, `Cb = B′ − Y`, `Cr = R′ − Y`.
//!    Pixels whose alpha is ≤ 1e-6 or non-finite, or whose (Y, Cb, Cr) is not finite, are
//!    *skipped*: never a neighbour, and left unchanged themselves.
//! 3. Over the `(2r+1)²` window clipped to the image, with the spatial sigma `σs` (image pixels,
//!    clamped to 0.25…3.5; NaN → 0.25) and `r = ceil(2 σs)` (1…=[`DENOISE_MAX_RADIUS`]):
//!    `ws = g(|dx|) · g(|dy|) · αn`, `g(d) = exp(−d² / (2 σs²))` (a table of 8 values), and
//!    - luma: `wl = ws · exp(−(Yn − Y)² / (2 σl²))`, `Y ← Σ wl Yn / Σ wl`;
//!    - chroma: `wc = ws · exp(−((Cbn − Cb)² + (Crn − Cr)²) / (2 σc²))`, likewise for Cb and Cr.
//!
//!    The range sigmas are `σ = max(strength · 0.05, 1e-4)` in encoded units; a component whose
//!    strength is 0 is left untouched. Differences far above σ (edges) get ~0 weight, so edges
//!    stay sharp while noise of the order of σ is averaged out.
//! 4. Invert step 2 and `e`, multiply by the pixel's own alpha (alpha itself is unchanged). A
//!    non-finite result leaves the pixel unchanged.
//!
//! Strengths are clamped to 0…1 (NaN → 0). Both strengths 0 is an exact no-op (the image is not
//! touched).
//!
//! **Limitation (by design of a range filter):** edges whose contrast is of the order of the range
//! sigma or below are treated as noise. At strength 1 (σ = 0.05 encoded) a faint chroma edge
//! (e.g. a slight tint change on a grey wall, ΔCb/ΔCr ≈ 0.01–0.03) bleeds over roughly the window
//! radius; hard edges (contrast ≫ σ) stay sharp. See `faint_chroma_edges_bleed_at_full_strength`.

use rayon::prelude::*;

use crate::image::Image;

/// The largest window radius (the GPU's per-pixel loop bound).
pub const DENOISE_MAX_RADIUS: u32 = 7;
/// The spatial sigma range (image pixels): `r = ceil(2σ)` spans 1…=7.
pub const SPATIAL_SIGMA_MIN: f32 = 0.25;
pub const SPATIAL_SIGMA_MAX: f32 = 3.5;
/// The range sigma (encoded units) at strength 1.
pub const RANGE_SIGMA_AT_FULL: f32 = 0.05;
/// The smallest range sigma of an active component (keeps `1 / 2σ²` finite).
const MIN_SIGMA: f32 = 1e-4;

const KR: f32 = 0.2126;
const KG: f32 = 0.7152;
const KB: f32 = 0.0722;

/// The evaluated filter: the constants both the CPU and the GPU use (always finite).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiseParams {
    /// `1 / (2 σl²)`, or None when luma is not filtered.
    pub luma_k: Option<f32>,
    /// `1 / (2 σc²)`, or None when chroma is not filtered.
    pub chroma_k: Option<f32>,
    /// `g(d)` for d = 0…7 (0 beyond the radius).
    pub spatial: [f32; 8],
    /// Window radius, 1…=[`DENOISE_MAX_RADIUS`].
    pub radius: u32,
}

/// A hostile strength as 0…1 (NaN → 0, +inf → 1).
pub fn strength(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// A hostile spatial sigma as [`SPATIAL_SIGMA_MIN`]…[`SPATIAL_SIGMA_MAX`] (NaN → the minimum).
pub fn spatial_sigma(v: f32) -> f32 {
    if v.is_nan() { SPATIAL_SIGMA_MIN } else { v.clamp(SPATIAL_SIGMA_MIN, SPATIAL_SIGMA_MAX) }
}

/// The filter constants for `luma` / `chroma` strengths and spatial `sigma` (pixels); None: an
/// exact no-op.
pub fn params(luma: f32, chroma: f32, sigma: f32) -> Option<DenoiseParams> {
    let (l, c) = (strength(luma), strength(chroma));
    if l <= 0.0 && c <= 0.0 {
        return None;
    }
    let k = |s: f32| (s > 0.0).then(|| 1.0 / (2.0 * (s * RANGE_SIGMA_AT_FULL).max(MIN_SIGMA).powi(2)));
    let ss = spatial_sigma(sigma);
    // (ss ≤ 3.5: 2·ss ≤ 7, the cast is exact and in range)
    let radius = ((2.0 * ss).ceil() as u32).clamp(1, DENOISE_MAX_RADIUS);
    let ks = 1.0 / (2.0 * ss * ss);
    let mut spatial = [0.0f32; 8];
    for (d, g) in spatial.iter_mut().enumerate().take(radius as usize + 1) {
        let d = d as f32;
        *g = (-(d * d) * ks).exp();
    }
    Some(DenoiseParams { luma_k: k(l), chroma_k: k(c), spatial, radius })
}

/// `e(v)` of the module docs.
#[inline]
pub fn encode(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else if v <= 1.0 {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    } else {
        1.0 + (v - 1.0) / 2.4
    }
}

/// `e⁻¹`.
#[inline]
pub fn decode(e: f32) -> f32 {
    if e <= 0.04045 {
        e / 12.92
    } else if e <= 1.0 {
        ((e + 0.055) / 1.055).powf(2.4)
    } else {
        1.0 + (e - 1.0) * 2.4
    }
}

/// Straight linear RGB → (Y, Cb, Cr) of the encoded signal.
#[inline]
pub fn to_ycc(c: [f32; 3]) -> [f32; 3] {
    let e = c.map(encode);
    let y = KR * e[0] + KG * e[1] + KB * e[2];
    [y, e[2] - y, e[0] - y]
}

/// The inverse of [`to_ycc`].
#[inline]
pub fn from_ycc(v: [f32; 3]) -> [f32; 3] {
    let r = v[2] + v[0];
    let b = v[1] + v[0];
    let g = (v[0] - KR * r - KB * b) / KG;
    [r, g, b].map(decode)
}

/// Step 2 for one premultiplied pixel: (Y, Cb, Cr, α), or zeros for a skipped pixel.
#[inline]
pub fn pixel_ycc(p: [f32; 4]) -> [f32; 4] {
    let [r, g, b, a] = p;
    if !a.is_finite() || a <= 1e-6 {
        return [0.0; 4];
    }
    let c = to_ycc([r / a, g / a, b / a]);
    if c.iter().all(|v| v.is_finite()) { [c[0], c[1], c[2], a] } else { [0.0; 4] }
}

/// Filter `img` in place (see the module docs).
pub fn apply(img: &mut Image, luma: f32, chroma: f32, sigma: f32) {
    let Some(p) = params(luma, chroma, sigma) else { return };
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() < w.saturating_mul(h).saturating_mul(4) {
        return;
    }
    let ycc: Vec<[f32; 4]> = img.px.par_chunks(4).map(|q| if let &[r, g, b, a] = q { pixel_ycc([r, g, b, a]) } else { [0.0; 4] }).collect();
    let r = p.radius as usize;
    let g = |d: usize| p.spatial.get(d).copied().unwrap_or(0.0);
    let ycc = &ycc;
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for (x, q) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let Some(c0) = ycc.get(y * w + x).copied() else { continue };
            if c0[3] <= 0.0 {
                continue;
            }
            let (mut sl, mut wl, mut scb, mut scr, mut wc) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for ny in y.saturating_sub(r)..=(y + r).min(h - 1) {
                let gy = g(ny.abs_diff(y));
                for nx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    let Some(c) = ycc.get(ny * w + nx) else { continue };
                    if c[3] <= 0.0 {
                        continue;
                    }
                    let ws = g(nx.abs_diff(x)) * gy * c[3];
                    if let Some(k) = p.luma_k {
                        let d = c[0] - c0[0];
                        let wt = ws * (-(d * d) * k).exp();
                        sl += wt * c[0];
                        wl += wt;
                    }
                    if let Some(k) = p.chroma_k {
                        let (d1, d2) = (c[1] - c0[1], c[2] - c0[2]);
                        let wt = ws * (-(d1 * d1 + d2 * d2) * k).exp();
                        scb += wt * c[1];
                        scr += wt * c[2];
                        wc += wt;
                    }
                }
            }
            // (the centre contributes ws = α > 1e-6, so the sums are never empty)
            let yy = if p.luma_k.is_some() && wl > 0.0 { sl / wl } else { c0[0] };
            let (cb, cr) = if p.chroma_k.is_some() && wc > 0.0 { (scb / wc, scr / wc) } else { (c0[1], c0[2]) };
            let out = from_ycc([yy, cb, cr]);
            let a = c0[3];
            let out = out.map(|v| v * a);
            if out.iter().all(|v| v.is_finite()) {
                q[..3].copy_from_slice(&out);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpufx::FxOp;

    /// Deterministic Gaussian-ish noise (xorshift + sum of 12 uniforms − 6).
    struct Rng(u64);
    impl Rng {
        fn uniform(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
        fn gauss(&mut self) -> f32 {
            (0..12).map(|_| self.uniform()).sum::<f32>() - 6.0
        }
    }

    fn noisy_flat(w: usize, h: usize, level: [f32; 3], sigma: f32, seed: u64) -> Image {
        let mut rng = Rng(seed);
        let mut img = Image::new(w, h);
        for p in img.px.as_chunks_mut::<4>().0.iter_mut() {
            for (v, l) in p.iter_mut().zip(level) {
                *v = l + sigma * rng.gauss();
            }
            p[3] = 1.0;
        }
        img
    }

    fn std_dev(img: &Image, k: usize, margin: usize) -> f32 {
        let mut v = Vec::new();
        for y in margin..img.h - margin {
            for x in margin..img.w - margin {
                v.push(img.px[(y * img.w + x) * 4 + k]);
            }
        }
        let n = v.len() as f32;
        let m = v.iter().sum::<f32>() / n;
        (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / n).sqrt()
    }

    #[test]
    fn zero_strength_is_bit_exact() {
        let src = noisy_flat(33, 21, [0.2, 0.3, 0.1], 0.02, 7);
        for (l, c) in [(0.0, 0.0), (-1.0, 0.0), (f32::NAN, -0.0), (f32::NEG_INFINITY, f32::NAN)] {
            let mut img = src.clone();
            FxOp::Denoise { luma: l, chroma: c, sigma: 1.5 }.apply(&mut img);
            assert!(img.px.iter().zip(&src.px).all(|(a, b)| a.to_bits() == b.to_bits()), "{l} {c}");
        }
    }

    #[test]
    fn round_trip_is_close() {
        for c in [[0.0, 0.0, 0.0], [0.2, 0.5, 0.9], [1.6, 1.2, 0.3], [-0.01, 0.002, 1.0]] {
            let o = from_ycc(to_ycc(c));
            assert!(o.iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-5), "{c:?} → {o:?}");
        }
    }

    #[test]
    fn flat_noise_drops_by_at_least_40_percent() {
        let src = noisy_flat(64, 64, [0.2, 0.18, 0.15], 0.01, 12345);
        let mut img = src.clone();
        FxOp::Denoise { luma: 0.5, chroma: 0.5, sigma: 1.5 }.apply(&mut img);
        for k in 0..3 {
            let (before, after) = (std_dev(&src, k, 4), std_dev(&img, k, 4));
            eprintln!("channel {k}: σ {before:.5} → {after:.5}");
            assert!(after <= before * 0.6, "channel {k}: {before} → {after}");
        }
        // luma only / chroma only still reduce the noise
        for (l, c) in [(0.5, 0.0), (0.0, 0.5)] {
            let mut img = src.clone();
            FxOp::Denoise { luma: l, chroma: c, sigma: 1.5 }.apply(&mut img);
            assert!(std_dev(&img, 1, 4) < std_dev(&src, 1, 4), "{l} {c}");
        }
    }

    /// 10–90 % rise width of a monotone row profile (linear interpolation between pixels).
    fn edge_width(row: &[f32]) -> f32 {
        let (lo, hi) = (row[0], row[row.len() - 1]);
        let cross = |t: f32| {
            let level = lo + (hi - lo) * t;
            for i in 1..row.len() {
                if row[i] >= level {
                    let (a, b) = (row[i - 1], row[i]);
                    return (i - 1) as f32 + if b > a { (level - a) / (b - a) } else { 0.0 };
                }
            }
            row.len() as f32
        };
        cross(0.9) - cross(0.1)
    }

    #[test]
    fn hard_edges_stay_sharp() {
        let (w, h) = (40usize, 16usize);
        for (luma, chroma, sigma) in [(0.5, 0.5, 1.5), (1.0, 1.0, 3.5), (1.0, 0.0, 3.5), (1.0, 1.0, 0.25)] {
            for (lo, hi) in [([0.1, 0.1, 0.1], [0.6, 0.6, 0.6]), ([0.05, 0.2, 0.1], [0.5, 0.3, 0.6])] {
                let mut img = Image::new(w, h);
                for y in 0..h {
                    for x in 0..w {
                        let c = if x < w / 2 { lo } else { hi };
                        img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
                    }
                }
                let before: Vec<f32> = (0..w).map(|x| img.get(x, h / 2)[1]).collect();
                FxOp::Denoise { luma, chroma, sigma }.apply(&mut img);
                let after: Vec<f32> = (0..w).map(|x| img.get(x, h / 2)[1]).collect();
                let (wb, wa) = (edge_width(&before), edge_width(&after));
                eprintln!("{luma} {chroma} σ{sigma}: edge {wb:.3} → {wa:.3} px");
                assert!(wa <= wb + 1.0, "edge widened {wb} → {wa}");
            }
        }
    }

    #[test]
    fn hostile_params_never_panic_and_stay_finite() {
        let src = noisy_flat(19, 13, [0.3, 0.4, 0.5], 0.05, 99);
        let vals = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 1e-30, f32::MIN_POSITIVE, 0.5];
        for &l in &vals {
            for &c in &vals {
                for sigma in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0, 0.25, 3.5, 4.0, 1e30] {
                    let mut img = src.clone();
                    FxOp::Denoise { luma: l, chroma: c, sigma }.apply(&mut img);
                    assert!(img.px.iter().all(|v| v.is_finite()), "{l} {c} {sigma}");
                }
            }
        }
        // hostile pixels (non-finite, huge, transparent, empty images) never panic
        let mut img = src.clone();
        img.px[0] = f32::NAN;
        img.px[5] = f32::INFINITY;
        img.px[10] = 1e30;
        img.px[15] = 0.0;
        FxOp::Denoise { luma: 1.0, chroma: 1.0, sigma: 3.5 }.apply(&mut img);
        FxOp::Denoise { luma: 1.0, chroma: 1.0, sigma: 3.5 }.apply(&mut Image::new(0, 0));
        FxOp::Denoise { luma: 1.0, chroma: 1.0, sigma: 3.5 }.apply(&mut Image::new(1, 1));
        FxOp::Denoise { luma: 1.0, chroma: 1.0, sigma: 3.5 }.apply(&mut Image { w: 5, h: 5, px: vec![0.5; 7] });
    }

    #[test]
    fn transparent_pixels_and_alpha_are_preserved() {
        let mut img = noisy_flat(16, 16, [0.3, 0.3, 0.3], 0.02, 5);
        for (i, p) in img.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let a = if i % 5 == 0 { 0.0 } else { 0.5 };
            for v in p.iter_mut().take(3) {
                *v *= a;
            }
            p[3] = a;
        }
        let src = img.clone();
        FxOp::Denoise { luma: 0.7, chroma: 0.7, sigma: 1.0 }.apply(&mut img);
        for (a, b) in img.px.as_chunks::<4>().0.iter().zip(src.px.as_chunks::<4>().0) {
            assert_eq!(a[3].to_bits(), b[3].to_bits());
            if b[3] == 0.0 {
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn gpu_ok_whenever_the_constants_are_finite() {
        // (the GPU gets `params`' constants, always finite: hostile values clamp like on the CPU)
        for v in [0.5, 0.0, f32::NAN, f32::INFINITY, -1e30] {
            assert!(FxOp::Denoise { luma: v, chroma: 0.2, sigma: v }.gpu_ok());
            if let Some(p) = params(v, 0.2, v) {
                assert!(p.spatial.iter().chain(p.luma_k.iter()).chain(p.chroma_k.iter()).all(|x| x.is_finite()));
                assert!((1..=DENOISE_MAX_RADIUS).contains(&p.radius));
            }
        }
        assert_eq!(params(1.0, 1.0, 1.5).map(|p| p.radius), Some(3));
        assert_eq!(params(1.0, 1.0, 3.5).map(|p| p.radius), Some(7));
        assert_eq!(params(1.0, 1.0, 0.1).map(|p| p.radius), Some(1));
        assert_eq!(params(1.0, 1.0, 100.0).map(|p| p.radius), Some(7));
    }

    /// The documented limitation: a faint chroma edge (below the range sigma) bleeds at strength
    /// 1, while a mild chroma strength keeps it.
    #[test]
    fn faint_chroma_edges_bleed_at_full_strength() {
        let (w, h) = (40usize, 16usize);
        let (lo, hi) = ([0.3, 0.3, 0.3], [0.31, 0.3, 0.29]);
        let mk = || {
            let mut img = Image::new(w, h);
            for (i, p) in img.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let c = if i % w < w / 2 { lo } else { hi };
                *p = [c[0], c[1], c[2], 1.0];
            }
            img
        };
        let profile = |img: &Image| -> Vec<f32> { (0..w).map(|x| img.get(x, h / 2)[0]).collect() };
        let wb = edge_width(&profile(&mk()));
        let mut full = mk();
        FxOp::Denoise { luma: 0.0, chroma: 1.0, sigma: 3.5 }.apply(&mut full);
        let mut mild = mk();
        FxOp::Denoise { luma: 0.0, chroma: 0.1, sigma: 3.5 }.apply(&mut mild);
        let (wf, wm) = (edge_width(&profile(&full)), edge_width(&profile(&mild)));
        eprintln!("faint chroma edge: {wb:.3} px → {wf:.3} px at strength 1, {wm:.3} px at 0.1");
        assert!(wf > wb + 1.0, "expected bleed at strength 1: {wb} → {wf}");
        assert!(wm <= wb + 1.0, "strength 0.1 should keep the edge: {wb} → {wm}");
    }
}
