//! Turning a picture into the vision encoder's input (AI-PLAN AI4 item 2, Qwen3-VL 4B), as the
//! model's published processor settings describe it (`preprocessor_config.json`: patch 16,
//! temporal patch 2, 2×2 merge, mean / std 0.5, 65 536 – 16 777 216 pixels): pick a size whose
//! sides are multiples of 32 near the picture's own, normalise, and cut it into patches in the
//! order the encoder's 2×2 merger expects. Resizing to that size is the caller's job.
//! Provisional until the encoder's parity check against the reference processor (owed, STEADY+).

/// Pixels per patch side.
pub const PATCH: usize = 16;
/// Patches per merged token side.
pub const MERGE: usize = 2;
/// Frames per temporal patch (a still picture is repeated).
pub const TEMPORAL: usize = 2;
/// Fewest and most pixels the processor allows.
pub const MIN_PIXELS: u64 = 65_536;
pub const MAX_PIXELS: u64 = 16_777_216;
/// Values per patch: 3 channels × temporal × patch².
pub const PATCH_VALUES: usize = 3 * TEMPORAL * PATCH * PATCH;

/// The size (width, height) to resize a `w` × `h` picture to: both sides multiples of 32
/// (patch × merge), the pixel count within the processor's bounds, the aspect kept as far as
/// those allow. None for an empty or absurd input (zero, or a side over 1 M pixels).
/// Longest side over shortest side accepted (as the reference processor's limit).
pub const MAX_ASPECT: f64 = 200.0;

pub fn target_size(w: u32, h: u32, min_pixels: u64, max_pixels: u64) -> Option<(u32, u32)> {
    const LIMIT: u32 = 1 << 20;
    let factor = (PATCH * MERGE) as f64;
    if w == 0 || h == 0 || w > LIMIT || h > LIMIT || min_pixels > max_pixels {
        return None;
    }
    let (wf, hf) = (f64::from(w), f64::from(h));
    let round = |x: f64| (x / factor).round().max(1.0) * factor;
    let (mut tw, mut th) = (round(wf), round(hf));
    if tw * th > max_pixels as f64 {
        let beta = (wf * hf / max_pixels as f64).sqrt();
        tw = (wf / beta / factor).floor().max(1.0) * factor;
        th = (hf / beta / factor).floor().max(1.0) * factor;
    } else if tw * th < min_pixels as f64 {
        let beta = (min_pixels as f64 / (wf * hf)).sqrt();
        tw = (wf * beta / factor).ceil() * factor;
        th = (hf * beta / factor).ceil() * factor;
    }
    let (tw, th) = (tw as u64, th as u64);
    // (a very thin picture can't shrink below one block on its short side: refuse it rather than
    // go over the cap, like the reference processor's aspect limit)
    let fits = tw.checked_mul(th).is_some_and(|p| p <= max_pixels.max(min_pixels));
    (tw > 0 && th > 0 && tw <= u64::from(LIMIT) && th <= u64::from(LIMIT) && fits && wf.max(hf) / wf.min(hf) <= MAX_ASPECT).then_some((tw as u32, th as u32))
}

/// Patches of an RGB picture already resized to `w` × `h` (from [`target_size`]), `rgb` holding
/// `w * h * 3` values in 0..=1, row by row. Returns the patch grid (rows, columns, in patches) and
/// one row of [`PATCH_VALUES`] normalised values per patch: channel-major, then the two (identical)
/// frames, then the 16 × 16 pixels; patches ordered so each 2 × 2 block of them (one merged token)
/// is consecutive, blocks in reading order. None when the sizes don't fit.
pub fn patches(rgb: &[f32], w: usize, h: usize) -> Option<((usize, usize), Vec<f32>)> {
    let side = PATCH * MERGE;
    if w == 0 || h == 0 || !w.is_multiple_of(side) || !h.is_multiple_of(side) || rgb.len() != w.checked_mul(h)?.checked_mul(3)? {
        return None;
    }
    let (gh, gw) = (h / PATCH, w / PATCH);
    let mut out = Vec::with_capacity(gh.checked_mul(gw)?.checked_mul(PATCH_VALUES)?);
    for by in 0..gh / MERGE {
        for bx in 0..gw / MERGE {
            for my in 0..MERGE {
                for mx in 0..MERGE {
                    let (py, px) = (by * MERGE + my, bx * MERGE + mx);
                    for c in 0..3 {
                        for _frame in 0..TEMPORAL {
                            for y in 0..PATCH {
                                for x in 0..PATCH {
                                    let i = ((py * PATCH + y) * w + px * PATCH + x) * 3 + c;
                                    // (v − 0.5) / 0.5
                                    out.push(rgb.get(i).map_or(0.0, |v| v * 2.0 - 1.0));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Some(((gh, gw), out))
}

/// Resizes interleaved RGB (`w * h * 3` values) to `tw` × `th` with a separable bicubic filter
/// (a = -0.5, widened by the scale when shrinking so every source pixel counts, edge pixels
/// repeated): the usual image-library bicubic the model's processor resizes with. Provisional
/// (the parity check against the reference processor decides between this and Lanczos).
/// None when a size is zero, the buffer doesn't match, or the result would exceed
/// [`MAX_PIXELS`].
pub fn resize_bicubic(rgb: &[f32], w: usize, h: usize, tw: usize, th: usize) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || tw == 0 || th == 0 || rgb.len() != w.checked_mul(h)?.checked_mul(3)? || (tw as u64).checked_mul(th as u64)? > MAX_PIXELS {
        return None;
    }
    // horizontal pass: h rows of tw, then vertical: th rows of tw
    let xw = taps(w, tw)?;
    let yw = taps(h, th)?;
    // (the intermediate is source rows × target width: capped like the result)
    if (h as u64).checked_mul(tw as u64)? > MAX_PIXELS {
        return None;
    }
    let mut mid = vec![0.0f32; h.checked_mul(tw)?.checked_mul(3)?];
    for (src, dst) in rgb.chunks_exact(w * 3).zip(mid.chunks_exact_mut(tw * 3)) {
        for (o, (first, ws)) in dst.as_chunks_mut::<3>().0.iter_mut().zip(&xw) {
            for (k, wt) in ws.iter().enumerate() {
                if let Some(p) = src.get((first + k) * 3..(first + k) * 3 + 3) {
                    for c in 0..3 {
                        o[c] += wt * p[c];
                    }
                }
            }
        }
    }
    let mut out = vec![0.0f32; th * tw * 3];
    for (dst, (first, ws)) in out.chunks_exact_mut(tw * 3).zip(&yw) {
        for (k, wt) in ws.iter().enumerate() {
            if let Some(row) = mid.get((first + k) * tw * 3..(first + k + 1) * tw * 3) {
                for (d, v) in dst.iter_mut().zip(row) {
                    *d += wt * v;
                }
            }
        }
    }
    Some(out)
}

/// Per output sample: the first source index and the normalised weights from there on.
fn taps(src: usize, dst: usize) -> Option<Vec<(usize, Vec<f32>)>> {
    let cubic = |x: f64| {
        let (a, x) = (-0.5, x.abs());
        if x < 1.0 {
            ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0
        } else if x < 2.0 {
            (((x - 5.0) * x + 8.0) * x - 4.0) * a
        } else {
            0.0
        }
    };
    let scale = src as f64 / dst as f64;
    let support = 2.0 * scale.max(1.0);
    let inv = 1.0 / scale.max(1.0);
    (0..dst)
        .map(|i| {
            let centre = (i as f64 + 0.5) * scale;
            let lo = (centre - support).floor().max(0.0) as usize;
            let hi = ((centre + support).ceil() as usize).min(src);
            let mut ws: Vec<f64> = (lo..hi).map(|j| cubic((j as f64 + 0.5 - centre) * inv)).collect();
            let sum: f64 = ws.iter().sum();
            if sum.abs() < 1e-12 {
                return None;
            }
            ws.iter_mut().for_each(|w| *w /= sum);
            Some((lo, ws.into_iter().map(|w| w as f32).collect()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_multiples_of_32_within_the_pixel_bounds() {
        // a 6.2K 3:2 frame comes down under the cap, keeping its aspect closely
        let (w, h) = target_size(6240, 4160, MIN_PIXELS, MAX_PIXELS).unwrap();
        assert!(w % 32 == 0 && h % 32 == 0 && u64::from(w) * u64::from(h) <= MAX_PIXELS, "{w}×{h}");
        assert!((f64::from(w) / f64::from(h) - 1.5).abs() < 0.02, "{w}×{h}");
        // an ordinary HD frame only snaps to multiples of 32
        assert_eq!(target_size(1920, 1080, MIN_PIXELS, MAX_PIXELS), Some((1920, 1088)));
        // a tiny picture grows to the minimum
        let (w, h) = target_size(64, 64, MIN_PIXELS, MAX_PIXELS).unwrap();
        assert!(u64::from(w) * u64::from(h) >= MIN_PIXELS && w == h, "{w}×{h}");
        // a smaller cap (what FilmCraft will use for speed) is honoured too
        let (w, h) = target_size(3840, 2160, MIN_PIXELS, 1 << 20).unwrap();
        assert!(u64::from(w) * u64::from(h) <= 1 << 20, "{w}×{h}");
        // hostile input
        for (w, h) in [(0, 10), (10, 0), (u32::MAX, 10), (10, u32::MAX)] {
            assert_eq!(target_size(w, h, MIN_PIXELS, MAX_PIXELS), None);
        }
        assert_eq!(target_size(100, 100, 10, 5), None, "min above max");
        // a thin strip up to the reference's aspect limit gives a size within the cap; a thinner
        // one is refused (its short side can't shrink below one block, so it would go over)
        let (w, h) = target_size(20, 4000, MIN_PIXELS, MAX_PIXELS).expect("aspect 200");
        assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS);
        assert_eq!(target_size(1, 4000, MIN_PIXELS, MAX_PIXELS), None);
        // the review's case: 16 × 60 000 under a 2^20 cap used to come back twice the cap
        assert_eq!(target_size(16, 60_000, MIN_PIXELS, 1 << 20), None);
        for (w, h) in [(300, 60_000), (32, 6_000), (1000, 10)] {
            if let Some((tw, th)) = target_size(w, h, MIN_PIXELS, 1 << 18) {
                assert!(u64::from(tw) * u64::from(th) <= 1 << 18, "{w}×{h} → {tw}×{th}");
            }
        }
    }

    #[test]
    fn patches_come_in_merge_blocks_normalised() {
        // 64 × 32: a 2 × 4 patch grid, two merge blocks side by side; each pixel's red value says
        // which patch it is in
        let (w, h) = (64, 32);
        let mut rgb = vec![0.0f32; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let patch = (y / PATCH) * (w / PATCH) + x / PATCH;
                rgb[(y * w + x) * 3] = patch as f32 / 10.0;
                rgb[(y * w + x) * 3 + 1] = 1.0;
            }
        }
        let ((gh, gw), p) = patches(&rgb, w, h).unwrap();
        assert_eq!((gh, gw, p.len()), (2, 4, 8 * PATCH_VALUES));
        // block order: (0,0) (0,1) (1,0) (1,1) of the first block, then the second block
        let red_of = |k: usize| (p[k * PATCH_VALUES] + 1.0) / 2.0 * 10.0;
        let order: Vec<i32> = (0..8).map(|k| red_of(k).round() as i32).collect();
        assert_eq!(order, vec![0, 1, 4, 5, 2, 3, 6, 7]);
        // green (channel 1) starts after red's two frames, normalised 1.0 → 1.0, 0.0 → −1.0
        assert_eq!(p[2 * PATCH * PATCH], 1.0);
        assert_eq!(p[PATCH * PATCH], p[0], "the still frame is repeated");
        // sizes that don't fit
        assert!(patches(&rgb, 48, 32).is_none(), "not a multiple of 32");
        assert!(patches(&rgb[1..], w, h).is_none(), "wrong length");
        assert!(patches(&[], 0, 0).is_none());
    }

    #[test]
    fn bicubic_keeps_flat_colour_and_refuses_bad_sizes() {
        let flat: Vec<f32> = [0.2f32, 0.5, 0.9].repeat(40 * 30);
        for (tw, th) in [(64, 32), (17, 9), (40, 30), (128, 96)] {
            let out = resize_bicubic(&flat, 40, 30, tw, th).unwrap();
            assert_eq!(out.len(), tw * th * 3);
            assert!(out.as_chunks::<3>().0.iter().all(|p| (p[0] - 0.2).abs() < 1e-5 && (p[1] - 0.5).abs() < 1e-5 && (p[2] - 0.9).abs() < 1e-5), "{tw}x{th}");
        }
        // same size is the identity; a horizontal ramp stays monotonic when shrunk
        let ramp: Vec<f32> = (0..16 * 4)
            .flat_map(|i| {
                let v = (i % 16) as f32 / 15.0;
                [v, v, v]
            })
            .collect();
        let same = resize_bicubic(&ramp, 16, 4, 16, 4).unwrap();
        assert!(same.iter().zip(&ramp).all(|(a, b)| (a - b).abs() < 1e-5));
        let small = resize_bicubic(&ramp, 16, 4, 5, 2).unwrap();
        assert!(small[..15].as_chunks::<3>().0.windows(2).all(|p| p[0][0] < p[1][0]));
        assert!(resize_bicubic(&ramp, 16, 4, 0, 4).is_none());
        assert!(resize_bicubic(&ramp, 15, 4, 8, 4).is_none());
        assert!(resize_bicubic(&ramp, 16, 4, 1 << 20, 1 << 20).is_none());
    }
}
