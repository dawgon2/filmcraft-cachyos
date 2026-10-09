//! The built-in F-Log2 / F-Gamut transform against Fujifilm's official X-M5 3D LUT (fork Phase 4
//! acceptance). Fujifilm's `FLog2_FGamut_to_FLog2_BT.709` keeps the F-Log2 tone and converts
//! only the gamut (F-Gamut → BT.709), so it checks exactly our F-Log2 curve and F-Gamut primaries:
//! decode F-Log2 → linear F-Gamut → BT.709 → encode F-Log2. Its other LUTs (WDR, ETERNA) are
//! Fujifilm's own tone renderings and are not a colour-science reference.
//!
//! Finding (fork Phase 4 acceptance, 2026-10-09): the tone matches to a twentieth of a 10-bit
//! code, but Fujifilm's LUT changes the gamut **on the F-Log2 signal** (the 3×3 matrix applied to
//! log code values, approximately), not in linear light as the built-in transform (and
//! colour science) does, so saturated colours differ by tens of codes. The test asserts the tone
//! and reports both colour models; the official `.cube` stays usable as an Input LUT for that
//! look.
//!
//! The LUT is not in the repository (vendor file): download
//! `https://dl.fujifilm-x.com/lut/x-m5-3d-lut-v100.zip` into
//! `target/fixtures/fujifilm-lut/x-m5/` (unzipped); the test skips without it.

use filmcraft_color::log::LogCurve;
use filmcraft_color::ops::{ColorOp, apply_chain, gamut_op};
use filmcraft_color::spaces::Gamut;
use filmcraft_color::{Lut, Range};

fn lut() -> Option<Lut> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures/fujifilm-lut/x-m5/x-m5-3d-lut-v100/F-Log2/XM5_FLog2_FGamut_to_FLog2_BT.709_33grid_V.1.00.cube");
    let text = std::fs::read_to_string(p).ok()?;
    Lut::parse_cube(&text).ok()
}

#[test]
fn f_log2_matches_fujifilms_official_lut() {
    let Some(lut) = lut() else {
        eprintln!("Fujifilm X-M5 LUT not in target/fixtures; skipping");
        return;
    };
    let decode = [ColorOp::decode_log(LogCurve::FLog2, Range::Full), gamut_op(Gamut::Bt2020, Gamut::Bt709)];
    let encode = ColorOp::encode_log(LogCurve::FLog2, Range::Full);
    // the gamut matrix on the log signal (how the LUT appears to be made)
    let on_log = gamut_op(Gamut::Bt2020, Gamut::Bt709);
    let (mut lin_max, mut lin_sum, mut log_max, mut log_sum, mut n) = (0.0f32, 0.0f64, 0.0f32, 0.0f64, 0usize);
    let (mut grey_worst, steps) = (0.0f32, 33usize);
    for b in 0..steps {
        for g in 0..steps {
            for r in 0..steps {
                let c = [r, g, b].map(|v| v as f32 / (steps - 1) as f32);
                let theirs = lut.apply(c);
                let diff = |o: [f32; 3]| o.iter().zip(theirs).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                if r == g && g == b && c[0] <= 0.9 {
                    grey_worst = grey_worst.max(diff(apply_chain(&encode, apply_chain(&decode, c))));
                }
                let lin = apply_chain(&decode, c);
                let log = on_log.apply(c);
                // colours in gamut either way, below F-Log2's clip
                if lin.iter().any(|v| *v < 0.0) || log.iter().any(|v| !(0.0..=1.0).contains(v)) || c.iter().any(|v| *v > 0.9) {
                    continue;
                }
                let (dl, dg) = (diff(apply_chain(&encode, lin)), diff(log));
                (lin_max, log_max) = (lin_max.max(dl), log_max.max(dg));
                lin_sum += f64::from(dl);
                log_sum += f64::from(dg);
                n += 1;
            }
        }
    }
    let k = 1023.0;
    eprintln!(
        "greys (tone): max {:.3} codes; {n} colours: gamut in linear light (built-in) max {:.1} / mean {:.1} codes, gamut on the log signal max {:.1} / mean {:.1} codes",
        grey_worst * k,
        lin_max * k,
        lin_sum / n.max(1) as f64 * f64::from(k),
        log_max * k,
        log_sum / n.max(1) as f64 * f64::from(k)
    );
    assert!(n > 1000);
    assert!(grey_worst * k <= 0.5, "F-Log2 tone differs from Fujifilm's LUT by {} codes", grey_worst * k);
}
