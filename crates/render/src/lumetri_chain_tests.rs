//! Lumetri Basic + Creative as a chain of colour operations (fork ADR 0003) against the per-pixel
//! implementation it replaced (kept here as the oracle), and which Lumetri settings the GPU can
//! draw.

use filmcraft_color::{GradeSpace, WorkingSpace};
use filmcraft_project::{EffectInstance, ParamValue, find_effect};
use filmcraft_time::Tick;

use crate::Image;
use crate::effects::{FxCtx, apply, lumetri_gpu};
use crate::gpufx::FxOp;

fn cx(working: WorkingSpace) -> FxCtx<'static> {
    FxCtx { t: Tick::ZERO, px_scale: 1.0, seconds: 0.0, timecode: "", clip_name: "", project: None, env: None, working }
}

fn lumetri(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect("lumetri").unwrap().instance();
    for (k, v) in params {
        e.params.get_mut(*k).unwrap_or_else(|| panic!("no param {k}")).value = v.clone();
    }
    e
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}

fn on(e: &EffectInstance, id: &str) -> bool {
    e.param(id).and_then(|p| p.value.as_bool()).unwrap_or(true)
}

/// The Basic + Creative + Vignette maths before the colour-operation chain (session 4's
/// `effects::lumetri`, without the Input LUT, Look, Curves, Wheels, HSL and Sharpen).
fn reference(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let f = |id: &str| e.f64_at(id, cx.t) as f32;
    let color = |id: &str| e.param(id).and_then(|p| p.value_at(cx.t).as_color()).unwrap_or([1.0; 4]);
    let (basic_on, creative_on, vignette_on) = (on(e, "basic_on"), on(e, "creative_on"), on(e, "vignette_on"));
    let bf = |id: &str| if basic_on { f(id) } else { 0.0 };
    let gs = GradeSpace::new(cx.working, f("hdr_white"));
    let hdr = gs.is_hdr();
    let specular = if hdr { bf("hdr_specular") / 100.0 } else { 0.0 };
    let temp = bf("temperature") / 100.0;
    let tint = bf("tint") / 100.0;
    let exposure = 2f32.powf(bf("exposure"));
    let contrast = bf("contrast") / 100.0;
    let hl = bf("highlights") / 100.0;
    let sh = bf("shadows") / 100.0;
    let wh = bf("whites") / 100.0;
    let bl = bf("blacks") / 100.0;
    let sat = if basic_on { f("saturation") / 100.0 } else { 1.0 } * if creative_on { f("creative_sat") / 100.0 } else { 1.0 };
    let vib = if creative_on { f("vibrance") / 100.0 } else { 0.0 };
    let faded = if creative_on { f("faded_film") / 100.0 } else { 0.0 };
    let st = color("shadow_tint");
    let ht = color("highlight_tint");
    let va = if vignette_on { f("vignette_amount") } else { 0.0 };
    let vmid = f("vignette_midpoint") / 100.0;
    let vround = f("vignette_roundness") / 100.0;
    let vfeather = f("vignette_feather") / 100.0;
    let gains = [1.0 + 0.35 * temp, 1.0 - 0.3 * tint, 1.0 - 0.35 * temp];
    let (w, h) = (img.w as f32, img.h as f32);
    let aspect = w / h;
    img.map_rgb(|c, x, y| {
        let lin = [c[0] * gains[0] * exposure, c[1] * gains[1] * exposure, c[2] * gains[2] * exposure];
        let mut v = gs.encode(lin);
        let b0 = -bl * 0.15;
        let w0 = 1.0 - wh * 0.15;
        v = v.map(|q| (q - b0) / (w0 - b0).max(1e-3));
        let l = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        let ws = (1.0 - l).clamp(0.0, 1.0).powi(3);
        let whl = l.clamp(0.0, 1.0).powi(3);
        let nl = (l + sh * 0.35 * ws + hl * 0.35 * whl).max(0.0);
        if l > 1e-5 {
            let k = nl / l;
            v = v.map(|q| q * k);
        }
        if contrast.abs() > 1e-4 {
            let k = 1.0 + contrast;
            v = v.map(|q| {
                if hdr && q > 1.0 {
                    return q;
                }
                let q = q.clamp(0.0, 1.0);
                let s = q * q * (3.0 - 2.0 * q);
                if k >= 1.0 { q + (s - q) * (k - 1.0) } else { 0.5 + (q - 0.5) * k }
            });
        }
        if specular.abs() > 1e-4 {
            v = v.map(|q| if q > 1.0 { 1.0 + (q - 1.0) * (1.0 + specular).max(0.0) } else { q });
        }
        if faded > 0.0 {
            v = v.map(|q| q * (1.0 - 0.25 * faded) + 0.12 * faded);
        }
        if creative_on {
            let l2 = (0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]).clamp(0.0, 1.0);
            for k in 0..3 {
                v[k] += (st[k] - 0.5) * 0.3 * (1.0 - l2) + (ht[k] - 0.5) * 0.3 * l2;
            }
        }
        let l3 = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        let cur_sat = v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2]);
        let s = sat * (1.0 + vib * (1.0 - cur_sat.clamp(0.0, 1.0)));
        v = v.map(|q| l3 + (q - l3) * s);
        if va.abs() > 1e-4 {
            let nx = (x as f32 / w - 0.5) * 2.0 * if vround < 0.0 { aspect.powf(-vround) } else { 1.0 };
            let ny = (y as f32 / h - 0.5) * 2.0;
            let d = (nx * nx + ny * ny).sqrt() / std::f32::consts::SQRT_2;
            let edge = ((d - vmid * 0.9) / (vfeather.max(0.01) * 0.9)).clamp(0.0, 1.0);
            let e2 = edge * edge * (3.0 - 2.0 * edge);
            let k = 1.0 + va * 0.2 * e2;
            v = v.map(|q| if va < 0.0 { q * k.max(0.0) } else { q + (1.0 - q) * (k - 1.0) });
        }
        gs.decode(v)
    });
}

/// A picture with darks, mid tones, saturated colours, HDR highlights (up to ~20× white) and
/// a few negative (out-of-gamut) components.
fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
            let k = (x * 7 + y * 13) % 11;
            let scale = [0.02, 0.1, 0.3, 1.0, 1.0, 1.0, 2.5, 6.0, 20.0, 0.6, 1.0][k];
            let c = [u * scale, (1.0 - u) * v * scale, (0.5 * v + 0.2 * u) * scale - if k == 9 { 0.05 } else { 0.0 }];
            let a = if k == 3 { 0.5 } else { 1.0 };
            img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
        }
    }
    img
}

fn settings() -> Vec<(&'static str, EffectInstance)> {
    let tint = |r: f32, g: f32, b: f32| ParamValue::Color([r, g, b, 1.0]);
    vec![
        ("identity", lumetri(&[])),
        ("white balance + exposure", lumetri(&[("temperature", fl(40.0)), ("tint", fl(-25.0)), ("exposure", fl(0.7))])),
        ("whites / blacks", lumetri(&[("whites", fl(-30.0)), ("blacks", fl(45.0))])),
        ("shadows / highlights", lumetri(&[("shadows", fl(60.0)), ("highlights", fl(-70.0))])),
        ("contrast +", lumetri(&[("contrast", fl(55.0))])),
        ("contrast −", lumetri(&[("contrast", fl(-40.0))])),
        ("specular", lumetri(&[("hdr_specular", fl(-50.0)), ("contrast", fl(20.0))])),
        ("faded film", lumetri(&[("faded_film", fl(60.0))])),
        ("split tone", lumetri(&[("shadow_tint", tint(0.3, 0.6, 0.7)), ("highlight_tint", tint(0.8, 0.55, 0.3))])),
        ("saturation + vibrance", lumetri(&[("saturation", fl(140.0)), ("creative_sat", fl(80.0)), ("vibrance", fl(50.0))])),
        ("vignette dark", lumetri(&[("vignette_amount", fl(-3.0)), ("vignette_roundness", fl(-60.0)), ("vignette_feather", fl(30.0))])),
        ("vignette light", lumetri(&[("vignette_amount", fl(2.0)), ("vignette_midpoint", fl(30.0))])),
        (
            "everything",
            lumetri(&[
                ("temperature", fl(-20.0)),
                ("exposure", fl(-0.4)),
                ("contrast", fl(30.0)),
                ("highlights", fl(25.0)),
                ("shadows", fl(-15.0)),
                ("whites", fl(10.0)),
                ("blacks", fl(-10.0)),
                ("saturation", fl(120.0)),
                ("faded_film", fl(20.0)),
                ("vibrance", fl(-30.0)),
                ("shadow_tint", tint(0.4, 0.5, 0.6)),
                ("vignette_amount", fl(-1.5)),
            ]),
        ),
        (
            "sections off",
            lumetri(&[("exposure", fl(1.0)), ("faded_film", fl(50.0)), ("basic_on", ParamValue::Bool(false)), ("creative_on", ParamValue::Bool(false))]),
        ),
    ]
}

/// The whole per-pixel Lumetri before the chain: Input LUT, Basic + Creative, Look / Curves /
/// Wheels.
fn reference_all(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if on(e, "basic_on")
        && let Some(ParamValue::Text(id)) = e.param("input_lut").map(|p| &p.value)
        && let Some(l) = crate::luts::resolve(cx.project, id)
    {
        let gs = GradeSpace::new(cx.working, e.f64_at("hdr_white", cx.t) as f32);
        let hdr = gs.is_hdr();
        img.map_rgb(|c, _, _| gs.decode(filmcraft_color::ops::within_unit(gs.encode(c), hdr, |v| l.apply(v))));
    }
    reference(img, e, cx);
    crate::effects::advanced_reference(img, e, cx);
}

fn builtin(prefix: &str) -> ParamValue {
    let b = crate::luts::builtins().iter().find(|b| b.id.starts_with(prefix)).expect("builtin LUT");
    ParamValue::Text(format!("builtin:{}", b.id))
}

/// Curves, Colour Wheels, Looks and LUTs (4.3b).
fn advanced_settings() -> Vec<(String, EffectInstance)> {
    let curve = |p: &[[f32; 2]]| ParamValue::Curve(p.to_vec());
    let wheel = |x: f64, y: f64| ParamValue::Vec2(filmcraft_geom::Vec2::new(x, y));
    let mut v: Vec<(String, EffectInstance)> = vec![
        ("luma curve".into(), lumetri(&[("curve_luma", curve(&[[0.0, 0.05], [0.3, 0.22], [0.7, 0.8], [1.0, 0.95]]))])),
        (
            "RGB curves".into(),
            lumetri(&[("curve_red", curve(&[[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]])), ("curve_blue", curve(&[[0.0, 0.1], [0.6, 0.5], [1.0, 0.9]]))]),
        ),
        (
            "hue curves".into(),
            lumetri(&[
                ("hue_vs_sat", curve(&[[0.0, 0.7], [0.33, 0.3], [0.66, 0.5]])),
                ("hue_vs_hue", curve(&[[0.1, 0.55], [0.6, 0.45]])),
                ("hue_vs_luma", curve(&[[0.0, 0.6], [0.5, 0.4]])),
                ("luma_vs_sat", curve(&[[0.2, 0.3], [0.8, 0.6]])),
                ("sat_vs_sat", curve(&[[0.1, 0.5], [0.9, 0.35]])),
            ]),
        ),
        (
            "wheels".into(),
            lumetri(&[
                ("wheel_shadows", wheel(0.3, -0.2)),
                ("wheel_midtones", wheel(-0.1, 0.4)),
                ("wheel_highlights", wheel(0.2, 0.2)),
                ("wheel_shadows_l", fl(-20.0)),
                ("wheel_highlights_l", fl(15.0)),
            ]),
        ),
        ("input LUT".into(), lumetri(&[("input_lut", builtin("flog2"))])),
        ("look LUT".into(), lumetri(&[("look_lut", builtin("look-")), ("look_intensity", fl(70.0))])),
        (
            "all of it".into(),
            lumetri(&[
                ("input_lut", builtin("slog3")),
                ("exposure", fl(0.3)),
                ("contrast", fl(20.0)),
                ("curve_luma", curve(&[[0.0, 0.0], [0.4, 0.35], [1.0, 1.0]])),
                ("hue_vs_sat", curve(&[[0.0, 0.6], [0.5, 0.45]])),
                ("wheel_midtones", wheel(0.1, 0.1)),
                ("look", ParamValue::Choice(1)),
                ("vignette_amount", fl(-1.0)),
            ]),
        ),
    ];
    for look in 1..=8 {
        v.push((format!("look {look}"), lumetri(&[("look", ParamValue::Choice(look)), ("look_intensity", fl(80.0))])));
    }
    v
}

#[test]
fn the_chain_matches_the_whole_per_pixel_lumetri() {
    let spaces = [WorkingSpace::Rec709, WorkingSpace::DaVinciWide, WorkingSpace::Rec2100Pq, WorkingSpace::Rec2100Hlg];
    let mut failures = Vec::new();
    for working in spaces {
        for (name, e) in advanced_settings() {
            let mut want = picture(41, 27);
            let mut got = want.clone();
            reference_all(&mut want, &e, &cx(working));
            apply(&mut got, &e, &cx(working));
            let worst = want.px.iter().zip(&got.px).map(|(a, b)| (a - b).abs() / a.abs().max(0.05)).fold(0.0f32, f32::max);
            // PQ's steep decode magnifies the last-bit differences of the signal scaling
            // (`GradeSpace::encode_ops` multiplies where `encode` divides) up to ~4e-4
            let tol = if working == WorkingSpace::Rec2100Pq { 1e-3 } else { 2e-4 };
            if worst > tol || got.px.iter().any(|v| !v.is_finite()) {
                failures.push(format!("{working:?} {name}: {worst:.2e}"));
            }
        }
    }
    assert!(failures.is_empty(), "chain differs from the reference:\n{}", failures.join("\n"));
}

#[test]
fn the_chain_matches_the_per_pixel_lumetri() {
    let spaces = [WorkingSpace::Rec709, WorkingSpace::DaVinciWide, WorkingSpace::Rec2100Pq, WorkingSpace::Rec2100Hlg];
    let mut failures = Vec::new();
    for working in spaces {
        for (name, e) in settings() {
            let mut want = picture(41, 27);
            let mut got = want.clone();
            reference(&mut want, &e, &cx(working));
            apply(&mut got, &e, &cx(working));
            let worst = want.px.iter().zip(&got.px).map(|(a, b)| (a - b).abs() / a.abs().max(0.05)).fold(0.0f32, f32::max);
            // PQ's steep decode magnifies the last-bit differences of the signal scaling
            // (`GradeSpace::encode_ops` multiplies where `encode` divides) up to ~4e-4
            let tol = if working == WorkingSpace::Rec2100Pq { 1e-3 } else { 2e-4 };
            if worst > tol || got.px.iter().any(|v| !v.is_finite()) {
                failures.push(format!("{working:?} {name}: {worst:.2e}"));
            }
        }
    }
    assert!(failures.is_empty(), "chain differs from the reference:\n{}", failures.join("\n"));
}

/// HSL Secondary settings (4.3c): corrections, mask views, Refine.
fn hsl_settings() -> Vec<(String, EffectInstance)> {
    let base = |more: &[(&str, ParamValue)]| {
        let mut p = vec![
            ("hsl_on", ParamValue::Bool(true)),
            ("hsl_hue", fl(30.0)),
            ("hsl_hue_range", fl(90.0)),
            ("hsl_sat_min", fl(10.0)),
            ("hsl_luma_min", fl(5.0)),
            ("hsl_luma_max", fl(90.0)),
            ("hsl_soft", fl(40.0)),
            ("hsl_sat", fl(150.0)),
            ("hsl_hue_shift", fl(-20.0)),
            ("hsl_temp", fl(30.0)),
            ("hsl_tint", fl(-20.0)),
        ];
        p.extend(more.iter().cloned());
        lumetri(&p)
    };
    let mut v = vec![
        ("correction".to_string(), base(&[])),
        ("denoise".to_string(), base(&[("hsl_denoise", fl(70.0))])),
        ("blur".to_string(), base(&[("hsl_blur", fl(30.0))])),
        (
            "denoise + blur + grade".to_string(),
            base(&[("hsl_denoise", fl(100.0)), ("hsl_blur", fl(10.0)), ("exposure", fl(0.4)), ("look", ParamValue::Choice(2))]),
        ),
    ];
    for m in 1..=3 {
        v.push((format!("show mask {m}"), base(&[("hsl_show_mask", ParamValue::Choice(m)), ("hsl_denoise", fl(40.0))])));
    }
    v
}

#[test]
fn hsl_secondary_matches_the_per_pixel_key() {
    let spaces = [WorkingSpace::Rec709, WorkingSpace::DaVinciWide, WorkingSpace::Rec2100Pq, WorkingSpace::Rec2100Hlg];
    let mut failures = Vec::new();
    for working in spaces {
        for (name, e) in hsl_settings() {
            let mut want = picture(41, 27);
            let mut got = want.clone();
            reference_all(&mut want, &e, &cx(working));
            crate::effects::hsl_secondary(&mut want, &e, &cx(working));
            apply(&mut got, &e, &cx(working));
            let worst = want.px.iter().zip(&got.px).map(|(a, b)| (a - b).abs() / a.abs().max(0.05)).fold(0.0f32, f32::max);
            // (the key of a PQ signal inherits its last-bit differences, see above)
            let tol = if working == WorkingSpace::Rec2100Pq { 2e-3 } else { 2e-4 };
            if worst > tol || got.px.iter().any(|v| !v.is_finite()) {
                failures.push(format!("{working:?} {name}: {worst:.2e}"));
            }
        }
    }
    assert!(failures.is_empty(), "HSL Secondary differs from the reference:\n{}", failures.join("\n"));
}

#[test]
fn the_gpu_draws_every_part_of_lumetri() {
    let c = cx(WorkingSpace::Rec709);
    let ops = lumetri_gpu(&lumetri(&[("exposure", fl(0.5)), ("sharpen", fl(40.0))]), &c, 64, 48).expect("GPU form");
    assert!(matches!(ops.as_slice(), [FxOp::ColorChain(_), FxOp::Unsharp { .. }]), "{ops:?}");
    assert!(ops.iter().all(FxOp::gpu_ok));
    for (_, e) in advanced_settings() {
        let ops = lumetri_gpu(&e, &c, 64, 48).expect("GPU form");
        assert!(ops.iter().all(FxOp::gpu_ok));
    }
    for (_, e) in hsl_settings() {
        let ops = lumetri_gpu(&e, &c, 64, 48).expect("GPU form");
        assert!(matches!(ops.as_slice(), [FxOp::ColorChain(_), FxOp::HslSecondary { .. }]), "{ops:?}");
        assert!(ops.iter().all(FxOp::gpu_ok));
    }
    // hostile numbers never reach the shader
    let bad = lumetri_gpu(&lumetri(&[("exposure", fl(1e30))]), &c, 64, 48).expect("GPU form");
    assert!(!bad.iter().all(FxOp::gpu_ok));
}
