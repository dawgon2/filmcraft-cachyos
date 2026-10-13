//! White balance: the Temperature / Tint gains a grade node and Lumetri ▸ Basic Correction apply
//! in working linear light, and their inverse: the Temperature / Tint that make a picked colour
//! neutral (the white-balance picker, GRADE-PLAN G2c), plus a grey-world estimate of the neutral
//! for Auto balance. Colour difference for the acceptance tests is CIEDE2000 ([`delta_e2000`]).
//!
//! The solver doesn't add grading maths: it inverts [`gains`] exactly. Two controls (blue / amber
//! and green / magenta) set two channel ratios, so any cast inside the slider ranges is removed
//! exactly; beyond them the values clamp and [`WbSolve::clamped`] says so.

use crate::spaces::{Gamut, Mat3, mul_vec};

/// Blue / amber strength of Temperature at ±100 (red and blue gains 1 ± 0.35).
const TEMP_K: f32 = 0.35;
/// Green / magenta strength of Tint at ±100 (green gain 1 ∓ 0.3).
const TINT_K: f32 = 0.3;
/// Slider range of Temperature and Tint.
pub const RANGE: f32 = 100.0;

/// Per-channel gains for `temperature` / `tint` slider values (−100 … 100), in linear light.
pub fn gains(temperature: f32, tint: f32) -> [f32; 3] {
    let (t, n) = (temperature / RANGE, tint / RANGE);
    [1.0 + TEMP_K * t, 1.0 - TINT_K * n, 1.0 - TEMP_K * t]
}

/// The result of [`solve`]: slider values that make the sample neutral.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WbSolve {
    pub temperature: f32,
    pub tint: f32,
    /// The cast needed more than the slider range: the values are clamped and the sample is only
    /// partly neutral.
    pub clamped: bool,
}

/// Why a sample can't be used as a neutral reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WbReject {
    /// NaN / infinite, or no picture there.
    Invalid,
    /// Too dark to tell its colour (noise decides).
    TooDark,
    /// A channel is at or above the clip level: its true colour is lost.
    Clipped,
    /// Far too colourful to be something grey or white.
    TooSaturated,
}

impl WbReject {
    /// A plain-language reason for the user.
    pub fn message(self) -> &'static str {
        match self {
            WbReject::Invalid => "there is no usable picture there: pick something grey or white",
            WbReject::TooDark => "that spot is too dark to judge its colour: pick something grey or white that is brighter",
            WbReject::Clipped => "that spot is clipped (overexposed), so its colour is lost: pick something grey or white that isn't blown out",
            WbReject::TooSaturated => "that spot is too colourful to be neutral: pick something grey or white",
        }
    }
}

/// Darkest usable sample (largest channel, linear light).
const MIN_LEVEL: f32 = 0.004;
/// Most colourful usable sample: (max − min) / max of the channels. The sliders reach about
/// 0.55 at their ends, so a stronger cast than that is clamped, and beyond this it's not grey.
const MAX_SAT: f32 = 0.75;

/// Temperature / Tint (slider units) that make `rgb` (linear light at the white balance's
/// input) neutral, R = G = B. `clip` is the clip level of the picture in the same units (1.0 for
/// display-referred SDR; `f32::INFINITY` when nothing clips). Exact within the ranges.
pub fn solve(rgb: [f32; 3], clip: f32) -> Result<WbSolve, WbReject> {
    let hi = rgb[0].max(rgb[1]).max(rgb[2]);
    solve_judged(rgb, clip.is_finite() && hi >= clip * 0.985)
}

/// [`solve`] with the clipping judged by the caller (`clipped`: e.g. [`ClipLevel::is_clipped`]
/// on the source under the sample), refused in the same order of reasons.
pub fn solve_judged(rgb: [f32; 3], clipped: bool) -> Result<WbSolve, WbReject> {
    let [r, g, b] = rgb;
    if !(r.is_finite() && g.is_finite() && b.is_finite()) {
        return Err(WbReject::Invalid);
    }
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    if hi < MIN_LEVEL || lo <= 0.0 {
        return Err(if hi < MIN_LEVEL { WbReject::TooDark } else { WbReject::TooSaturated });
    }
    if clipped {
        return Err(WbReject::Clipped);
    }
    if (hi - lo) / hi > MAX_SAT {
        return Err(WbReject::TooSaturated);
    }
    // (1 + k t) r = (1 − k t) b  →  t = (b − r) / (k (r + b))
    let t_raw = (b - r) / (TEMP_K * (r + b));
    let t = t_raw.clamp(-1.0, 1.0);
    // green meets the mean of the balanced red and blue: (1 − k n) g = level
    let level = 0.5 * ((1.0 + TEMP_K * t) * r + (1.0 - TEMP_K * t) * b);
    let n_raw = (1.0 - level / g) / TINT_K;
    let n = n_raw.clamp(-1.0, 1.0);
    let clamped = (t_raw - t).abs() > 1e-6 || (n_raw - n).abs() > 1e-6;
    if !(t.is_finite() && n.is_finite()) {
        return Err(WbReject::Invalid);
    }
    Ok(WbSolve { temperature: t * RANGE, tint: n * RANGE, clamped })
}

/// Where a source clips, for judging working linear light that came out of the source's input
/// transform alone (no grade, no effects). A colour is clipped when a channel reached the source's
/// encoded maximum: code value 1.0 of SDR and HLG signals, the top a camera log file can hold (less
/// a ~2 % margin; F-Log2 included, carried through the log curve into scene linear), or the content peak of PQ media (its
/// MaxCLL / mastering peak, else 1000 cd/m²). The check undoes the gamut matrix (and the HLG
/// OOTF) of the input transform, so one blown channel is caught in DaVinci Wide Gamut and the HDR
/// working spaces too, not only a blown white.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipLevel {
    /// Working gamut → source gamut.
    to_source: [[f32; 3]; 3],
    /// HLG: the OOTF's luma weights (BT.2020) and the scale of scene 1.0 in working units.
    hlg: Option<([f32; 3], f32)>,
    /// A channel at or above this (source-gamut linear) is clipped.
    level: f32,
}

/// HLG system gamma of the input transform (BT.2100 at 1000 cd/m²).
const HLG_GAMMA: f32 = 1.2;

impl ClipLevel {
    /// Nothing clips (linear float media: values above 1 are real).
    pub const NONE: ClipLevel = ClipLevel { to_source: IDENTITY3, hlg: None, level: f32::INFINITY };

    /// A plain clip level in working units (no gamut or OOTF to undo), e.g. 1.0 for
    /// display-referred SDR when the source isn't known.
    pub fn working(level: f32) -> ClipLevel {
        ClipLevel { level: if level.is_finite() && level > 0.0 { level } else { f32::INFINITY }, ..ClipLevel::NONE }
    }

    /// The clip level of media in colour space `src` (signal range `range`; `peak_nits`: the
    /// content peak of PQ media from its HDR metadata) entering pipeline `pipe`.
    pub fn for_source(src: crate::spaces::ColorSpace, range: crate::Range, pipe: &crate::spaces::ColorPipeline, peak_nits: Option<f64>) -> ClipLevel {
        use crate::spaces::Curve;
        use crate::transform::{HDR_PEAK_NITS, InputTransform, REFERENCE_WHITE_NITS};
        let curve = src.curve();
        let Some(code) = top_code(src, range, peak_nits) else { return ClipLevel::NONE };
        let t = InputTransform::new(src, range, pipe, peak_nits);
        // the top code value on every channel, through the real input transform
        let white = t.convert([code; 3]);
        let hlg = (curve == Curve::Hlg).then(|| {
            let l = Gamut::Bt2020.luma();
            ([l[0] as f32, l[1] as f32, l[2] as f32], (HDR_PEAK_NITS / REFERENCE_WHITE_NITS) as f32)
        });
        let m = crate::spaces::gamut_matrix(pipe.working_gamut(), src.gamut());
        let mut c = ClipLevel { to_source: m.map(|r| r.map(|v| v as f32)), hlg, level: f32::INFINITY };
        let level = c.source_linear(white).into_iter().fold(f32::INFINITY, f32::min);
        c.level = if level.is_finite() && level > 0.0 { level } else { f32::INFINITY };
        c
    }

    /// Whether nothing can be judged clipped.
    pub fn is_none(&self) -> bool {
        !self.level.is_finite()
    }

    /// Working linear (straight colour) → the source's linear light, before its OOTF and matrix.
    fn source_linear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut c = crate::spaces::apply3(&self.to_source, rgb);
        if let Some((l, k)) = self.hlg {
            // display = k · Ys^0.2 · scene, so Yd = k · Ys^1.2
            let yd = (l[0] * c[0] + l[1] * c[1] + l[2] * c[2]).max(0.0);
            let ys = (yd / k).powf(1.0 / HLG_GAMMA);
            let g = k * ys.powf(HLG_GAMMA - 1.0);
            c = if g > 1e-9 { c.map(|v| v / g) } else { [0.0; 3] };
        }
        c
    }

    /// Whether `rgb` (working linear, straight colour, from the input transform alone) has a
    /// clipped channel. Non-finite values count as clipped (nothing can be told about them).
    pub fn is_clipped(&self, rgb: [f32; 3]) -> bool {
        if self.is_none() {
            return false;
        }
        if !rgb.iter().all(|v| v.is_finite()) {
            return true;
        }
        let c = self.source_linear(rgb);
        c[0].max(c[1]).max(c[2]) >= self.level * 0.985
    }
}

/// The decoder-normalised code value where `src` clips (see [`ClipLevel`]); `None` for linear
/// media, which has no top.
pub fn top_code(src: crate::spaces::ColorSpace, range: crate::Range, peak_nits: Option<f64>) -> Option<f32> {
    use crate::spaces::Curve;
    use crate::transform::{HDR_PEAK_NITS, REFERENCE_WHITE_NITS, encode_channel};
    let curve = src.curve();
    // the source's top in its own linear light
    let top = match curve {
        Curve::Linear => return None,
        Curve::Sdr | Curve::Srgb | Curve::Hlg => 1.0,
        Curve::Pq => peak_nits.filter(|p| p.is_finite()).unwrap_or(HDR_PEAK_NITS).clamp(REFERENCE_WHITE_NITS, 10_000.0) / REFERENCE_WHITE_NITS,
        Curve::Log(c) => {
            // camera log: the top a file can hold (code value 1023, or the limited range's legal
            // maximum, 1019 / 254 → 1.086 normalised), less a margin of ~2 % of the signal (about
            // 20 10-bit code values) because sensors saturate a little below the top code
            let top = (encode_channel(c.peak_linear(), curve, range) as f32).min(match range {
                crate::Range::Limited => LOG_LEGAL_MAX_LIMITED,
                crate::Range::Full => 1.0,
            });
            return Some(top - LOG_CLIP_MARGIN);
        }
    };
    Some(encode_channel(top, curve, range) as f32)
}

/// The largest legal limited-range code value, normalised (10-bit 1019, 8-bit 254: both ≈ 1.087).
const LOG_LEGAL_MAX_LIMITED: f32 = 1.086;
/// How far below its top a camera log signal counts as clipped (normalised code value).
const LOG_CLIP_MARGIN: f32 = 0.02;

const IDENTITY3: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Box average of an RGBA f32 image (`w`×`h`, row-major) over the rectangle `uv0`…`uv1` (0…1 of
/// the frame, either order), at least `min_px` × `min_px` pixels around its centre. Non-finite
/// pixels are skipped; `None` when nothing usable is inside.
pub fn box_average(px: &[f32], w: usize, h: usize, uv0: [f32; 2], uv1: [f32; 2], min_px: usize) -> Option<[f32; 3]> {
    if w == 0 || h == 0 || px.len() < w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let ok = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
    if !(ok(uv0[0]) && ok(uv0[1]) && ok(uv1[0]) && ok(uv1[1])) {
        return None;
    }
    let to_px = |u: f32, n: usize| ((u * n as f32) as usize).min(n - 1);
    let (mut x0, mut x1) = (to_px(uv0[0].min(uv1[0]), w), to_px(uv0[0].max(uv1[0]), w));
    let (mut y0, mut y1) = (to_px(uv0[1].min(uv1[1]), h), to_px(uv0[1].max(uv1[1]), h));
    let grow = |a: &mut usize, b: &mut usize, n: usize| {
        let want = min_px.max(1).min(n);
        if *b + 1 - *a < want {
            let c = (*a + *b) / 2;
            *a = c.saturating_sub(want / 2).min(n - want);
            *b = *a + want - 1;
        }
    };
    grow(&mut x0, &mut x1, w);
    grow(&mut y0, &mut y1, h);
    let (mut acc, mut cnt) = ([0f64; 3], 0usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = (y * w + x) * 4;
            let Some(p) = px.get(i..i + 4) else { continue };
            if p.iter().all(|v| v.is_finite()) && p[3] > 1e-4 {
                // straight (un-premultiplied) colour
                for (a, v) in acc.iter_mut().zip(p) {
                    *a += f64::from(*v / p[3]);
                }
                cnt += 1;
            }
        }
    }
    (cnt > 0).then(|| [(acc[0] / cnt as f64) as f32, (acc[1] / cnt as f64) as f32, (acc[2] / cnt as f64) as f32])
}

/// Auto balance's neutral estimate: a robust grey-world average of an RGBA f32 image (linear
/// light at the white balance's input) over mid-tone, low-saturation, unclipped pixels. Pixels
/// are judged *after* the current estimate's correction, so a strong cast doesn't hide the greys;
/// three rounds. `None` when too little of the frame qualifies.
pub fn auto_neutral(px: &[f32], w: usize, h: usize, luma: [f32; 3], clip: f32) -> Option<[f32; 3]> {
    let n = w.checked_mul(h)?;
    let pixels = px.get(..n.checked_mul(4)?)?;
    // at most ~64k samples: a regular stride over the frame
    let step = (n / 65_536).max(1);
    let mut gain = [1.0f32; 3];
    let mut est = None;
    for round in 0..3 {
        // looser saturation the first round (the cast itself is colour), tighter after
        let max_sat = [0.6, 0.25, 0.15][round];
        let (mut acc, mut cnt) = ([0f64; 3], 0usize);
        for p in pixels.as_chunks::<4>().0.iter().step_by(step) {
            let (a, c) = (p[3], [p[0], p[1], p[2]]);
            if !(a.is_finite() && a > 1e-4 && c.iter().all(|v| v.is_finite())) {
                continue;
            }
            let c = [c[0] / a, c[1] / a, c[2] / a];
            let hi = c[0].max(c[1]).max(c[2]);
            if clip.is_finite() && hi >= clip * 0.985 {
                continue;
            }
            let k = [c[0] * gain[0], c[1] * gain[1], c[2] * gain[2]];
            let (khi, klo) = (k[0].max(k[1]).max(k[2]), k[0].min(k[1]).min(k[2]));
            let y = luma[0] * k[0] + luma[1] * k[1] + luma[2] * k[2];
            // mid-tones: between deep shadow and near white of the (balanced) picture
            if !(0.02..=0.8).contains(&y) || klo <= 0.0 || (khi - klo) / khi > max_sat {
                continue;
            }
            for (s, v) in acc.iter_mut().zip(c) {
                *s += f64::from(v);
            }
            cnt += 1;
        }
        // at least ~0.5 % of the samples (and 16 pixels) must qualify
        if cnt < 16 || cnt * 200 < n / step {
            return est;
        }
        let m = [(acc[0] / cnt as f64) as f32, (acc[1] / cnt as f64) as f32, (acc[2] / cnt as f64) as f32];
        let mean = (m[0] + m[1] + m[2]) / 3.0;
        if m.iter().any(|v| *v <= 0.0) || !mean.is_finite() {
            return est;
        }
        gain = [mean / m[0], mean / m[1], mean / m[2]];
        est = Some(m);
    }
    est
}

/// CIE XYZ (D65, white Y = 1) → CIELAB with a D65 reference white.
pub fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    const WHITE: [f64; 3] = [0.950_47, 1.0, 1.088_83];
    let f = |t: f64| if t > 216.0 / 24_389.0 { t.cbrt() } else { (24_389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(xyz[0] / WHITE[0]), f(xyz[1] / WHITE[1]), f(xyz[2] / WHITE[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Linear RGB in `gamut` (D65) → CIELAB.
pub fn rgb_to_lab(rgb: [f32; 3], gamut: Gamut) -> [f64; 3] {
    let m: Mat3 = gamut.to_xyz();
    xyz_to_lab(mul_vec(&m, [f64::from(rgb[0]), f64::from(rgb[1]), f64::from(rgb[2])]))
}

/// CIEDE2000 colour difference between two CIELAB colours (kL = kC = kH = 1; Sharma, Wu and
/// Dalal, "The CIEDE2000 Color-Difference Formula", 2005).
pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c_bar = 0.5 * ((a1 * a1 + b1 * b1).sqrt() + (a2 * a2 + b2 * b2).sqrt());
    let c7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c7 / (c7 + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = ((a1p * a1p + b1 * b1).sqrt(), (a2p * a2p + b2 * b2).sqrt());
    let hue = |bb: f64, aa: f64| {
        if bb == 0.0 && aa == 0.0 {
            0.0
        } else {
            let h = bb.atan2(aa).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let l_bar = 0.5 * (l1 + l2);
    let cp_bar = 0.5 * (c1p + c2p);
    let hp_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        0.5 * (h1p + h2p)
    } else if h1p + h2p < 360.0 {
        0.5 * (h1p + h2p + 360.0)
    } else {
        0.5 * (h1p + h2p - 360.0)
    };
    let t = 1.0 - 0.17 * (hp_bar - 30.0).to_radians().cos() + 0.24 * (2.0 * hp_bar).to_radians().cos() + 0.32 * (3.0 * hp_bar + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hp_bar - 63.0).to_radians().cos();
    let d_theta = 30.0 * (-((hp_bar - 275.0) / 25.0).powi(2)).exp();
    let cp7 = cp_bar.powi(7);
    let rc = 2.0 * (cp7 / (cp7 + 25f64.powi(7))).sqrt();
    let l50 = (l_bar - 50.0).powi(2);
    let sl = 1.0 + 0.015 * l50 / (20.0 + l50).sqrt();
    let sc = 1.0 + 0.045 * cp_bar;
    let sh = 1.0 + 0.015 * cp_bar * t;
    let rt = -(2.0 * d_theta * PI / 180.0).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(rgb: [f32; 3], s: WbSolve) -> [f32; 3] {
        let g = gains(s.temperature, s.tint);
        [rgb[0] * g[0], rgb[1] * g[1], rgb[2] * g[2]]
    }

    #[test]
    fn delta_e2000_matches_published_pairs() {
        // Sharma, Wu and Dalal 2005, Table 1 (pairs 1, 7, 17, 25)
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
        ];
        for (a, b, want) in cases {
            let got = delta_e2000(a, b);
            assert!((got - want).abs() < 1e-3, "{a:?} {b:?}: {got} vs {want}");
            assert!((delta_e2000(b, a) - want).abs() < 1e-3);
        }
        assert_eq!(delta_e2000([40.0, 3.0, -2.0], [40.0, 3.0, -2.0]), 0.0);
        // across the 0° / 360° hue wrap a tiny change stays tiny
        let d = delta_e2000([50.0, 2.0, -0.001], [50.0, 2.0, 0.001]);
        assert!(d < 0.01, "{d}");
        let d = delta_e2000([50.0, -2.0, -0.001], [50.0, -2.0, 0.001]);
        assert!(d < 0.01, "{d}");
    }

    #[test]
    fn solve_inverts_the_gains_exactly() {
        for (t, n) in [(0.0, 0.0), (40.0, 0.0), (-55.0, 0.0), (0.0, 30.0), (0.0, -70.0), (25.0, -40.0), (-90.0, 90.0)] {
            // a grey that the gains (t, n) would neutralise
            let g = gains(t, n);
            let grey = 0.18;
            let rgb = [grey / g[0], grey / g[1], grey / g[2]];
            let s = solve(rgb, 1.0).unwrap();
            assert!((s.temperature - t).abs() < 1e-3 && (s.tint - n).abs() < 1e-3, "{t} {n} → {s:?}");
            assert!(!s.clamped);
            let o = apply(rgb, s);
            assert!((o[0] - o[1]).abs() < 1e-5 && (o[1] - o[2]).abs() < 1e-5, "{o:?}");
        }
    }

    #[test]
    fn solving_a_neutral_sample_changes_nothing() {
        let s = solve([0.3, 0.3, 0.3], 1.0).unwrap();
        assert_eq!((s.temperature, s.tint, s.clamped), (0.0, 0.0, false));
    }

    #[test]
    fn casts_beyond_the_range_clamp_and_say_so() {
        // red / blue ratio 2.5 needs more than Temperature −100 can give
        let s = solve([0.5, 0.3, 0.2], 1.0).unwrap();
        assert!(s.clamped);
        assert_eq!(s.temperature, -100.0);
        assert!(s.tint.abs() <= 100.0);
    }

    #[test]
    fn bad_samples_are_refused_with_a_reason() {
        assert_eq!(solve([f32::NAN, 0.2, 0.2], 1.0), Err(WbReject::Invalid));
        assert_eq!(solve([0.2, f32::INFINITY, 0.2], 1.0), Err(WbReject::Invalid));
        assert_eq!(solve([0.001, 0.001, 0.002], 1.0), Err(WbReject::TooDark));
        assert_eq!(solve([0.0, 0.0, 0.0], 1.0), Err(WbReject::TooDark));
        assert_eq!(solve([-1.0, -1.0, -1.0], 1.0), Err(WbReject::TooDark));
        assert_eq!(solve([1.0, 0.99, 0.98], 1.0), Err(WbReject::Clipped));
        assert_eq!(solve([0.8, 0.1, 0.05], 1.0), Err(WbReject::TooSaturated));
        assert_eq!(solve([0.5, 0.0, 0.5], 1.0), Err(WbReject::TooSaturated));
        assert_eq!(solve([0.5, -0.1, 0.5], 1.0), Err(WbReject::TooSaturated));
        // scene-referred: nothing clips at 1.0
        assert!(solve([2.0, 2.0, 1.8], f32::INFINITY).is_ok());
        for r in [WbReject::Invalid, WbReject::TooDark, WbReject::Clipped, WbReject::TooSaturated] {
            assert!(r.message().contains("pick"));
        }
    }

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Vec<f32> {
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let c = f(x, y);
                v.extend_from_slice(&[c[0], c[1], c[2], 1.0]);
            }
        }
        v
    }

    #[test]
    fn box_average_covers_at_least_the_minimum_and_survives_hostile_input() {
        let img = image(20, 10, |x, _| [x as f32, 0.0, 1.0]);
        // a click at the left edge: 5 px wide starting at 0 → mean x = 2
        let c = box_average(&img, 20, 10, [0.0, 0.5], [0.0, 0.5], 5).unwrap();
        assert!((c[0] - 2.0).abs() < 1e-6, "{c:?}");
        // a box over the whole width → mean x = 9.5
        let c = box_average(&img, 20, 10, [1.0, 0.0], [0.0, 1.0], 5).unwrap();
        assert!((c[0] - 9.5).abs() < 1e-6);
        assert!(box_average(&img, 20, 10, [f32::NAN, 0.5], [0.5, 0.5], 5).is_none());
        assert!(box_average(&img, 20, 10, [1.5, 0.5], [0.5, 0.5], 5).is_none());
        assert!(box_average(&img, 20, 11, [0.5, 0.5], [0.5, 0.5], 5).is_none());
        assert!(box_average(&[], 0, 0, [0.5, 0.5], [0.5, 0.5], 5).is_none());
        assert!(box_average(&img, usize::MAX, 2, [0.5, 0.5], [0.5, 0.5], 5).is_none());
        let nan = image(4, 4, |_, _| [f32::NAN, 0.0, 0.0]);
        assert!(box_average(&nan, 4, 4, [0.5, 0.5], [0.5, 0.5], 99).is_none());
    }

    use crate::spaces::{ColorPipeline, ColorSpace, WorkingSpace};
    use crate::transform::{encode_channel, input_ops};

    /// Casts in the camera's own linear RGB: warm, cool, green, magenta and combinations.
    const CASTS: [[f32; 3]; 7] =
        [[1.25, 1.0, 0.8], [0.8, 1.0, 1.25], [0.92, 1.12, 0.92], [1.08, 0.88, 1.08], [1.2, 1.1, 0.75], [0.85, 0.9, 1.2], [1.1, 0.95, 0.7]];

    /// `lin` (scene / display linear in `src`'s gamut) encoded with `src`'s curve, through the
    /// input transform into `working`: what the white balance sees.
    fn through_input(lin: [f32; 3], src: ColorSpace, working: WorkingSpace) -> [f32; 3] {
        let curve = src.curve();
        let sig = lin.map(|v| encode_channel(f64::from(v), curve, crate::Range::Full) as f32);
        let pipe = ColorPipeline { working, ..Default::default() };
        crate::ops::apply_chain(&input_ops(src, crate::Range::Full, &pipe, None), sig)
    }

    fn chroma_error(rgb: [f32; 3], working: WorkingSpace) -> f64 {
        let pipe = ColorPipeline { working, ..Default::default() };
        let lab = rgb_to_lab(rgb, pipe.working_gamut());
        delta_e2000(lab, [lab[0], 0.0, 0.0])
    }

    fn chart_cases() -> [(ColorSpace, WorkingSpace, f32); 2] {
        [(ColorSpace::Rec709, WorkingSpace::Rec709, 1.0), (ColorSpace::FLog2FGamut, WorkingSpace::DaVinciWide, f32::INFINITY)]
    }

    #[test]
    fn picked_grey_patch_is_neutral_within_delta_e_1_and_picking_twice_changes_nothing() {
        for (src, working, clip) in chart_cases() {
            for cast in CASTS {
                for grey in [0.09f32, 0.18, 0.45] {
                    let sample = through_input(cast.map(|c| grey * c), src, working);
                    let before = chroma_error(sample, working);
                    let s = solve(sample, clip).unwrap_or_else(|e| panic!("{src:?} {cast:?} {grey}: {e:?}"));
                    assert!(!s.clamped, "{src:?} {cast:?}: {s:?}");
                    let de = chroma_error(apply(sample, s), working);
                    assert!(de < 1.0, "{src:?} {cast:?} {grey}: ΔE {de} (was {before})");
                    assert!(before > 2.0, "the cast should be visible: {before}");
                    // the picker samples the node's input, so a second pick sees the same colour
                    assert_eq!(solve(sample, clip), Ok(s));
                }
            }
        }
    }

    #[test]
    fn auto_balance_lands_within_delta_e_3_on_the_charts() {
        for (src, working, clip) in chart_cases() {
            for cast in CASTS {
                // a chart: six grey steps, plus colour patches (a quarter of the frame) that
                // aren't balanced around grey
                let patches = [[0.35f32, 0.08, 0.06], [0.07, 0.25, 0.08], [0.3, 0.25, 0.04], [0.05, 0.1, 0.32]];
                let img = image(96, 48, |x, y| {
                    let lin = if x >= 72 { patches[(y / 12).min(3)] } else { [0.03 + 0.1 * (x / 12) as f32; 3] };
                    through_input([lin[0] * cast[0], lin[1] * cast[1], lin[2] * cast[2]], src, working)
                });
                let pipe = ColorPipeline { working, ..Default::default() };
                let luma = pipe.working_gamut().luma().map(|v| v as f32);
                let m = auto_neutral(&img, 96, 48, luma, clip).unwrap();
                let s = solve(m, clip).unwrap();
                // judge a grey patch of the chart, corrected
                let patch = through_input(cast.map(|c| 0.23 * c), src, working);
                let de = chroma_error(apply(patch, s), working);
                assert!(de < 3.0, "{src:?} {cast:?}: ΔE {de}");
            }
        }
    }

    #[test]
    fn auto_neutral_ignores_colourful_and_clipped_pixels() {
        let cast = [1.2f32, 1.0, 0.75];
        // grey steps (with the cast) + a saturated red block + a clipped white block
        let img = image(64, 32, |x, y| {
            if x >= 48 {
                [0.6, 0.05, 0.05]
            } else if y < 4 {
                [1.0, 1.0, 1.0]
            } else {
                let v = 0.05 + 0.4 * (x / 8) as f32 / 6.0;
                [v * cast[0], v * cast[1], v * cast[2]]
            }
        });
        let m = auto_neutral(&img, 64, 32, [0.2126, 0.7152, 0.0722], 1.0).unwrap();
        let (rg, bg) = (m[0] / m[1], m[2] / m[1]);
        assert!((rg - 1.2).abs() < 0.01 && (bg - 0.75).abs() < 0.01, "{m:?}");
        assert!(auto_neutral(&[], 0, 0, [0.2126, 0.7152, 0.0722], 1.0).is_none());
        let black = image(8, 8, |_, _| [0.0; 3]);
        assert!(auto_neutral(&black, 8, 8, [0.2126, 0.7152, 0.0722], 1.0).is_none());
        let nan = image(8, 8, |_, _| [f32::NAN; 3]);
        assert!(auto_neutral(&nan, 8, 8, [0.2126, 0.7152, 0.0722], 1.0).is_none());
        assert!(auto_neutral(&img, usize::MAX, 3, [0.2126, 0.7152, 0.0722], 1.0).is_none());
    }

    #[test]
    fn clip_level_catches_blown_channels_through_every_input_transform() {
        use crate::Range;
        use crate::spaces::{ColorPipeline, ColorSpace, Curve, WorkingSpace};
        use crate::transform::{InputTransform, encode_channel};
        let pipes = [
            ColorPipeline { working: WorkingSpace::Rec709, ..Default::default() },
            ColorPipeline { working: WorkingSpace::Rec709, wide_gamut: true, ..Default::default() },
            ColorPipeline { working: WorkingSpace::Rec2100Pq, ..Default::default() },
            ColorPipeline { working: WorkingSpace::Rec2100Hlg, ..Default::default() },
            ColorPipeline { working: WorkingSpace::DaVinciWide, ..Default::default() },
        ];
        for pipe in pipes {
            for src in ColorSpace::ALL {
                for range in [Range::Full, Range::Limited] {
                    let clip = ClipLevel::for_source(src, range, &pipe, None);
                    if src.curve() == Curve::Linear {
                        assert!(clip.is_none() && !clip.is_clipped([50.0; 3]));
                        continue;
                    }
                    let t = InputTransform::new(src, range, &pipe, None);
                    let lin = |l: f64| encode_channel(l, src.curve(), range) as f32;
                    let top = top_code(src, range, None).unwrap();
                    let why = format!("{src:?} {range:?} into {:?} wide {}", pipe.working, pipe.wide_gamut);
                    // a blown white, and a blown channel (each one) on a bright but unclipped colour
                    assert!(clip.is_clipped(t.convert([top; 3])), "white: {why}");
                    let tone = src.is_hdr() && pipe.working == WorkingSpace::Rec709;
                    if !tone {
                        for ch in 0..3 {
                            let mut c = [lin(0.4); 3];
                            c[ch] = top;
                            assert!(clip.is_clipped(t.convert(c)), "channel {ch}: {why}");
                        }
                    }
                    // mid grey, a light grey and a coloured patch are fine
                    for c in [[lin(0.18); 3], [lin(0.6); 3], [lin(0.3), lin(0.2), lin(0.12)]] {
                        assert!(!clip.is_clipped(t.convert(c)), "{c:?}: {why}");
                    }
                    assert!(clip.is_clipped([f32::NAN, 0.1, 0.1]));
                }
            }
        }
        // camera log: the legal top of a limited-range file (10-bit 1019) and a sensor that
        // saturates ~15 code values under it count as clipped; ~60 code values under it doesn't
        let dwg = ColorPipeline { working: WorkingSpace::DaVinciWide, ..Default::default() };
        for src in [ColorSpace::FLog2FGamut, ColorSpace::SLog3SGamut3Cine] {
            let c = ClipLevel::for_source(src, Range::Limited, &dwg, None);
            let t = InputTransform::new(src, Range::Limited, &dwg, None);
            let code = |cv: f32| (cv - 64.0) / 876.0;
            assert!(c.is_clipped(t.convert([code(1019.0); 3])), "{src:?}");
            assert!(c.is_clipped(t.convert([code(1004.0), code(700.0), code(700.0)])), "{src:?}");
            assert!(!c.is_clipped(t.convert([code(960.0); 3])), "{src:?}");
        }
        // the content peak of PQ media is its clip level
        let pq = ColorPipeline { working: WorkingSpace::Rec2100Pq, ..Default::default() };
        let c = ClipLevel::for_source(ColorSpace::Rec2100Pq, Range::Limited, &pq, Some(1000.0));
        let nits = |n: f32| [n / 203.0; 3];
        assert!(c.is_clipped(nits(1000.0)) && !c.is_clipped(nits(900.0)) && !c.is_clipped(nits(203.0)));
        let c4 = ClipLevel::for_source(ColorSpace::Rec2100Pq, Range::Limited, &pq, Some(4000.0));
        assert!(!c4.is_clipped(nits(1000.0)) && c4.is_clipped(nits(4000.0)));
        // hostile peaks fall back to the default
        for p in [f64::NAN, f64::INFINITY, -5.0, 0.0] {
            let c = ClipLevel::for_source(ColorSpace::Rec2100Pq, Range::Limited, &pq, Some(p));
            assert!(c.is_clipped(nits(10_000.0)) && !c.is_clipped(nits(100.0)), "{p}");
        }
        assert!(ClipLevel::working(1.0).is_clipped([1.0, 0.2, 0.2]) && !ClipLevel::working(1.0).is_clipped([0.9; 3]));
        for bad in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert!(ClipLevel::working(bad).is_none());
        }
    }
}
