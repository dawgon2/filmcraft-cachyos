//! GPU-vs-CPU parity of colour-operation chains (`filmcraft_color::ops::ColorOp::apply` is the
//! reference; fork ADR 0003). Each operation alone, then chains, on the working image read back
//! as f32: values must match to [`TOL`] relative (f32 arithmetic and the GPU's `pow` / `log` /
//! `exp` differ in the last bits).

use std::sync::Arc;

use super::blend_tests::device;
use super::fx_tests::{compare, picture};
use super::*;
use filmcraft_color::Range;
use filmcraft_color::log::LogCurve;
use filmcraft_color::ops::{ColorOp, Table, Tf, gamut_op};
use filmcraft_color::spaces::Gamut;
use filmcraft_render::gpufx::FxOp;
use filmcraft_render::plan::LayerFx;

const TOL: f32 = 2e-4;
/// PQ decoding of the test picture's super-whites (signal up to 1.6, beyond PQ's 0…1): `c2 − c3·p`
/// nearly cancels there and magnifies the last bits of `pow` (Intel UHD 750: 2.4e-4).
const TOL_PQ_DECODE: f32 = 1e-3;

fn lggo() -> ColorOp {
    ColorOp::Lggo { lift: [0.05, -0.02, 0.0], gamma: [1.2, 0.9, 1.0], gain: [1.1, 0.95, 1.3], offset: [0.01, 0.0, -0.02] }
}

fn cases() -> Vec<(&'static str, Vec<ColorOp>)> {
    let mut v: Vec<(&'static str, Vec<ColorOp>)> = vec![
        ("709 → DWG", vec![gamut_op(Gamut::Bt709, Gamut::DaVinciWide)]),
        ("DWG → 709", vec![gamut_op(Gamut::DaVinciWide, Gamut::Bt709)]),
        ("matrix + offset", vec![ColorOp::Matrix { m: [[1.2, 0.1, -0.1], [0.0, 0.9, 0.05], [0.02, -0.3, 1.4]], offset: [0.01, -0.02, 0.03] }]),
        ("F-Log2 decode (full)", vec![ColorOp::decode_log(LogCurve::FLog2, Range::Full)]),
        ("S-Log3 decode (limited)", vec![ColorOp::decode_log(LogCurve::SLog3, Range::Limited)]),
        ("curve table", vec![ColorOp::Table(Table::sample(0.0, 1.0, 33, |x| x * x * (3.0 - 2.0 * x)))]),
        ("lggo", vec![lggo()]),
        ("clamp", vec![ColorOp::Clamp { lo: 0.1, hi: 0.8 }]),
        ("saturation", vec![ColorOp::Saturation { sat: 1.6, vibrance: 0.0, weights: [0.2126, 0.7152, 0.0722] }]),
        ("desaturate", vec![ColorOp::Saturation { sat: 0.0, vibrance: 0.0, weights: [0.2627, 0.678, 0.0593] }]),
    ];
    for (t, enc, dec) in [
        (Tf::Srgb, "sRGB encode", "sRGB decode"),
        (Tf::Gamma24, "γ 2.4 encode", "γ 2.4 decode"),
        (Tf::Pq, "PQ encode", "PQ decode"),
        (Tf::Hlg, "HLG encode", "HLG decode"),
        (Tf::DaVinciIntermediate, "DI encode", "DI decode"),
    ] {
        v.push((enc, vec![ColorOp::Encode(t)]));
        v.push((dec, vec![ColorOp::Decode(t)]));
    }
    v.push(("HLG OOTF", vec![ColorOp::HlgOotf { weights: [0.2627, 0.678, 0.0593], k: 4.926, exponent: 0.2 }]));
    v.push(("tone map 4000 → 203", vec![filmcraft_color::transform::ToneMap::new(4000.0, 203.0).op()]));
    v.push(("gamut compress", vec![ColorOp::Matrix { m: [[1.4, -0.3, -0.1], [-0.2, 1.3, -0.1], [0.0, -0.4, 1.4]], offset: [0.0; 3] }, ColorOp::GamutCompress]));
    v.push((
        "desaturate into gamut",
        vec![
            ColorOp::Matrix { m: [[1.4, -0.3, -0.1], [-0.2, 1.3, -0.1], [0.0, -0.4, 1.4]], offset: [0.0; 3] },
            ColorOp::DesaturateIntoGamut { weights: [0.2126, 0.7152, 0.0722] },
        ],
    ));
    // Lumetri's own operations (4.3a)
    let luma = [0.2126, 0.7152, 0.0722];
    v.push(("tone regions", vec![ColorOp::ToneRegions { shadows: 0.2, highlights: -0.25, weights: luma }]));
    v.push(("contrast +", vec![ColorOp::Contrast { amount: 0.5, keep_above_one: false }]));
    v.push(("contrast − (HDR)", vec![ColorOp::Contrast { amount: -0.4, keep_above_one: true }]));
    v.push(("specular", vec![ColorOp::Specular { gain: 0.6 }]));
    v.push(("split tone", vec![ColorOp::SplitTone { shadows: [-0.06, 0.03, 0.06], highlights: [0.09, 0.015, -0.06], weights: luma }]));
    v.push(("vibrance", vec![ColorOp::Saturation { sat: 1.2, vibrance: 0.6, weights: luma }]));
    v.push(("vignette dark", vec![ColorOp::Vignette { amount: -3.0, midpoint: 0.45, feather: 0.27, sx: 1.3, size: [67.0, 41.0] }]));
    v.push(("vignette light", vec![ColorOp::Vignette { amount: 2.0, midpoint: 0.27, feather: 0.45, sx: 1.0, size: [67.0, 41.0] }]));
    // film grain (G1.6): hard per-pixel, soft 2.5 px, smooth 4 px cells; seeds using all 32 bits
    v.push(("grain hard", vec![ColorOp::Grain { strength: 1.0, inv_size: 1.0, softness: 0.0, seed: 0xdead_beef, weights: luma }]));
    v.push(("grain soft", vec![ColorOp::Grain { strength: 0.7, inv_size: 1.0 / 2.5, softness: 0.6, seed: 7, weights: luma }]));
    v.push(("grain smooth", vec![ColorOp::Grain { strength: 0.4, inv_size: 0.25, softness: 1.0, seed: u32::MAX, weights: luma }]));
    // Curves, Wheels, Looks, LUTs (4.3b)
    let curve = Table::sample(0.0, 1.0, 1024, |x| x + 0.15 * (std::f64::consts::TAU * x).sin() * x * (1.0 - x));
    v.push(("curve (luma)", vec![ColorOp::Curve { table: curve.clone(), channels: [true; 3], hdr: false }]));
    v.push(("curve (red, HDR)", vec![ColorOp::Curve { table: curve.clone(), channels: [true, false, false], hdr: true }]));
    v.push(("wheels", vec![ColorOp::Wheels { shadows: [0.06, -0.03, 0.02], midtones: [-0.02, 0.05, 0.0], highlights: [0.1, -0.05, 0.08], weights: luma }]));
    let hue = |k: f64| Some(Table::sample(0.0, 1.0, 1024, move |x| 0.5 + 0.2 * (std::f64::consts::TAU * (x + k)).sin()));
    for hdr in [false, true] {
        let h = filmcraft_color::ops::HslCurves {
            hue_vs_hue: hue(0.1),
            hue_vs_sat: hue(0.3),
            hue_vs_luma: hue(0.5),
            luma_vs_sat: hue(0.7),
            sat_vs_sat: hue(0.9),
            hdr,
        };
        v.push(("HSL curves", vec![ColorOp::HslCurves(Arc::new(h))]));
    }
    let h = filmcraft_color::ops::HslCurves { hue_vs_hue: None, hue_vs_sat: hue(0.2), hue_vs_luma: None, luma_vs_sat: None, sat_vs_sat: hue(0.4), hdr: false };
    v.push(("HSL curves (two)", vec![ColorOp::HslCurves(Arc::new(h))]));
    for look in 1..=8 {
        v.push(("look", vec![ColorOp::Look { look, intensity: 0.8, hdr: false }]));
        v.push(("look (HDR)", vec![ColorOp::Look { look, intensity: 1.0, hdr: true }]));
    }
    let mut cube = filmcraft_color::Lut3d::from_fn(17, |c| [c[0] * c[0] * 0.9 + 0.05 * c[2], c[1].sqrt() * 0.95, (c[2] + 0.2 * c[0]).min(1.0)]);
    cube.domain_min = [-0.05, 0.0, 0.0];
    cube.domain_max = [1.1, 1.0, 1.2];
    let shaper =
        filmcraft_color::Lut1d { domain_min: [0.0; 3], domain_max: [1.5, 1.5, 1.5], data: (0..64).map(|i| [(i as f32 / 63.0).powf(0.8); 3]).collect() };
    for (name, l, hdr) in [
        ("LUT (cube)", filmcraft_color::Lut::from_cube(cube.clone()), false),
        ("LUT (shaper + cube, HDR)", filmcraft_color::Lut { title: String::new(), shaper: Some(shaper.clone()), cube: Some(cube.clone()) }, true),
        ("LUT (shaper)", filmcraft_color::Lut { title: String::new(), shaper: Some(shaper), cube: None }, false),
    ] {
        v.push((name, vec![ColorOp::Lut { lut: Arc::new(l), intensity: 0.85, hdr }]));
    }
    // whole Lumetri grades in every working space
    for working in [
        filmcraft_color::WorkingSpace::Rec709,
        filmcraft_color::WorkingSpace::DaVinciWide,
        filmcraft_color::WorkingSpace::Rec2100Pq,
        filmcraft_color::WorkingSpace::Rec2100Hlg,
    ] {
        let mut e = filmcraft_project::find_effect("lumetri").expect("lumetri").instance();
        for (k, val) in [
            ("temperature", -20.0),
            ("exposure", -0.4),
            ("contrast", 30.0),
            ("highlights", 25.0),
            ("shadows", -15.0),
            ("whites", 10.0),
            ("blacks", -10.0),
            ("saturation", 120.0),
            ("faded_film", 20.0),
            ("vibrance", -30.0),
            ("hdr_specular", 30.0),
            ("vignette_amount", -1.5),
        ] {
            if let Some(p) = e.params.get_mut(k) {
                p.value = filmcraft_project::ParamValue::Float(val);
            }
        }
        let cx = filmcraft_render::effects::FxCtx {
            t: filmcraft_time::Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            working,
        };
        for (k, val) in [
            ("curve_luma", filmcraft_project::ParamValue::Curve(vec![[0.0, 0.03], [0.4, 0.35], [1.0, 0.97]])),
            ("hue_vs_sat", filmcraft_project::ParamValue::Curve(vec![[0.0, 0.6], [0.5, 0.4]])),
            ("wheel_midtones", filmcraft_project::ParamValue::Vec2(filmcraft_geom::Vec2::new(0.1, -0.2))),
            ("look", filmcraft_project::ParamValue::Choice(3)),
        ] {
            if let Some(p) = e.params.get_mut(k) {
                p.value = val;
            }
        }
        let ops = filmcraft_render::effects::lumetri_gpu(&e, &cx, 67, 41).expect("GPU form");
        let Some(FxOp::ColorChain(chain)) = ops.first() else { panic!("{ops:?}") };
        v.push(("Lumetri grade", chain.to_vec()));
    }
    // every input transform into a Rec. 709 sequence (log tone mapped, camera gamuts compressed)
    for cs in filmcraft_color::ColorSpace::ALL {
        v.push(("input transform", filmcraft_color::transform::input_ops(cs, Range::Limited, &filmcraft_color::ColorPipeline::REC709, None)));
    }
    // an F-Log2 clip graded in DaVinci Wide Gamut / Intermediate and shown in Rec. 709
    v.push((
        "F-Log2 → DWG/DI grade → 709",
        vec![
            ColorOp::decode_log(LogCurve::FLog2, Range::Limited),
            gamut_op(Gamut::Bt2020, Gamut::DaVinciWide),
            ColorOp::Encode(Tf::DaVinciIntermediate),
            lggo(),
            ColorOp::Saturation { sat: 1.3, vibrance: 0.0, weights: [0.2627, 0.678, 0.0593] },
            ColorOp::Decode(Tf::DaVinciIntermediate),
            gamut_op(Gamut::DaVinciWide, Gamut::Bt709),
            ColorOp::Encode(Tf::Gamma24),
        ],
    ));
    v
}

/// Every operation and chain on the GPU against the CPU.
#[test]
fn gpu_color_ops_match_cpu() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let (w, h) = (67u32, 41u32);
    let px = picture(w, h);
    let frame = VideoFrame::rgba_f32(w, h, px.clone());
    let mut failures = Vec::new();
    for (name, ops) in cases() {
        let op = FxOp::ColorChain(Arc::from(ops));
        assert!(op.gpu_ok(), "{name}");
        let mut cpu = filmcraft_render::Image { w: w as usize, h: h as usize, px: px.clone() };
        op.apply(&mut cpu);
        let fx = LayerFx { size: (w, h), decimation: 1, ops: vec![op] };
        let (_, _, gpu) = c.effect_image(&frame, &fx).expect("effect image");
        let (worst, flips) = compare(&cpu.px, &gpu);
        eprintln!("{name}: max rel diff {worst:.2e}, {:.3}% off by > 2 %", flips * 100.0);
        let tol = if name == "PQ decode" { TOL_PQ_DECODE } else { TOL };
        if worst > tol || flips > 0.0 {
            failures.push(format!("{name}: {worst:.2e}, flips {flips:.4}"));
        }
    }
    assert!(failures.is_empty(), "GPU colour ops differ from the CPU:\n{}", failures.join("\n"));
}

/// Lumetri HSL Secondary (key, Denoise, Blur, correction, mask views) on the GPU against the CPU.
#[test]
fn gpu_hsl_secondary_matches_cpu() {
    use filmcraft_project::ParamValue;
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let (w, h) = (67u32, 41u32);
    let px = picture(w, h);
    let frame = VideoFrame::rgba_f32(w, h, px.clone());
    let mut failures = Vec::new();
    for working in [
        filmcraft_color::WorkingSpace::Rec709,
        filmcraft_color::WorkingSpace::DaVinciWide,
        filmcraft_color::WorkingSpace::Rec2100Pq,
        filmcraft_color::WorkingSpace::Rec2100Hlg,
    ] {
        for (denoise, blur, show) in [(0.0, 0.0, 0), (70.0, 0.0, 0), (0.0, 30.0, 0), (100.0, 10.0, 0), (40.0, 5.0, 1), (40.0, 0.0, 2), (40.0, 20.0, 3)] {
            let mut e = filmcraft_project::find_effect("lumetri").expect("lumetri").instance();
            for (k, val) in [
                ("hsl_on", ParamValue::Bool(true)),
                ("hsl_hue", ParamValue::Float(200.0)),
                ("hsl_hue_range", ParamValue::Float(120.0)),
                ("hsl_soft", ParamValue::Float(40.0)),
                ("hsl_sat", ParamValue::Float(150.0)),
                ("hsl_hue_shift", ParamValue::Float(-20.0)),
                ("hsl_temp", ParamValue::Float(30.0)),
                ("hsl_denoise", ParamValue::Float(denoise)),
                ("hsl_blur", ParamValue::Float(blur)),
                ("hsl_show_mask", ParamValue::Choice(show)),
            ] {
                if let Some(p) = e.params.get_mut(k) {
                    p.value = val;
                }
            }
            let cx = filmcraft_render::effects::FxCtx {
                t: filmcraft_time::Tick::ZERO,
                px_scale: 1.0,
                seconds: 0.0,
                timecode: "",
                clip_name: "",
                project: None,
                env: None,
                working,
            };
            let ops = filmcraft_render::effects::lumetri_gpu(&e, &cx, w as usize, h as usize).expect("GPU form");
            assert!(ops.iter().all(FxOp::gpu_ok));
            let mut cpu = filmcraft_render::Image { w: w as usize, h: h as usize, px: px.clone() };
            for op in &ops {
                op.apply(&mut cpu);
            }
            let fx = LayerFx { size: (w, h), decimation: 1, ops };
            let (_, _, gpu) = c.effect_image(&frame, &fx).expect("effect image");
            let (worst, flips) = compare(&cpu.px, &gpu);
            eprintln!("HSL {working:?} denoise {denoise} blur {blur} show {show}: {worst:.2e}, flips {flips:.4}");
            let tol = if working == filmcraft_color::WorkingSpace::Rec2100Pq { 2e-3 } else { TOL };
            if worst > tol || flips > 0.0 {
                failures.push(format!("{working:?} {denoise}/{blur}/{show}: {worst:.2e}, flips {flips:.4}"));
            }
        }
    }
    assert!(failures.is_empty(), "GPU HSL Secondary differs from the CPU:\n{}", failures.join("\n"));
}

/// Masked effects (a whole Lumetri with HSL Secondary and Sharpen inside two masks) and opacity
/// masks on the GPU against the CPU.
#[test]
fn gpu_masked_effects_match_cpu() {
    use filmcraft_project::{MaskMode, ParamValue};
    use filmcraft_render::mask::FlatMask;
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let (w, h) = (67u32, 41u32);
    let px = picture(w, h);
    let frame = VideoFrame::rgba_f32(w, h, px.clone());
    let ellipse: Vec<[f32; 2]> = (0..48)
        .map(|i| {
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            [33.0 + 22.0 * a.cos(), 20.0 + 13.0 * a.sin()]
        })
        .collect();
    let square = vec![[5.0, 5.0], [25.0, 5.0], [25.0, 30.0], [5.0, 30.0]];
    let mask = |pts: &Vec<[f32; 2]>, mode, feather, inverted| FlatMask { pts: pts.clone(), feather, expansion: 1.5, opacity: 0.9, inverted, mode };
    let masks: Arc<[FlatMask]> = vec![mask(&ellipse, MaskMode::Add, 6.0, false), mask(&square, MaskMode::Subtract, 2.0, false)].into();
    let mut e = filmcraft_project::find_effect("lumetri").expect("lumetri").instance();
    for (k, val) in [
        ("exposure", ParamValue::Float(0.8)),
        ("contrast", ParamValue::Float(40.0)),
        ("sharpen", ParamValue::Float(60.0)),
        ("hsl_on", ParamValue::Bool(true)),
        ("hsl_hue", ParamValue::Float(200.0)),
        ("hsl_hue_range", ParamValue::Float(120.0)),
        ("hsl_blur", ParamValue::Float(10.0)),
        ("hsl_sat", ParamValue::Float(0.0)),
    ] {
        if let Some(p) = e.params.get_mut(k) {
            p.value = val;
        }
    }
    let cx = filmcraft_render::effects::FxCtx {
        t: filmcraft_time::Tick::ZERO,
        px_scale: 1.0,
        seconds: 0.0,
        timecode: "",
        clip_name: "",
        project: None,
        env: None,
        working: filmcraft_color::WorkingSpace::Rec709,
    };
    let inner = filmcraft_render::effects::lumetri_gpu(&e, &cx, w as usize, h as usize).expect("GPU form");
    let cases: Vec<(&str, Vec<FxOp>)> = vec![
        ("masked Lumetri", vec![FxOp::Masked { ops: inner.clone(), masks: masks.clone() }]),
        (
            "masked Lumetri + opacity mask",
            vec![FxOp::Masked { ops: inner, masks }, FxOp::OpacityMask { masks: vec![mask(&ellipse, MaskMode::Add, 10.0, true)].into() }],
        ),
        ("no active mask", vec![FxOp::Masked { ops: vec![FxOp::BlackWhite], masks: vec![mask(&square, MaskMode::None, 0.0, false)].into() }]),
    ];
    for (name, ops) in cases {
        assert!(ops.iter().all(FxOp::gpu_ok), "{name}");
        let mut cpu = filmcraft_render::Image { w: w as usize, h: h as usize, px: px.clone() };
        for op in &ops {
            op.apply(&mut cpu);
        }
        let fx = LayerFx { size: (w, h), decimation: 1, ops };
        let (_, _, gpu) = c.effect_image(&frame, &fx).expect("effect image");
        let (worst, flips) = compare(&cpu.px, &gpu);
        eprintln!("{name}: {worst:.2e}, flips {flips:.4}");
        assert!(worst <= 1e-3 && flips == 0.0, "{name}: {worst:.2e}, flips {flips}");
    }
}

/// Non-finite numbers never reach the shader.
#[test]
fn non_finite_chains_stay_on_the_cpu() {
    let bad = FxOp::ColorChain(Arc::from(vec![ColorOp::Saturation { sat: f32::NAN, vibrance: 0.0, weights: [0.3; 3] }]));
    assert!(!bad.gpu_ok());
    let bad = FxOp::ColorChain(Arc::from(vec![ColorOp::Table(Table { lo: 0.0, hi: f32::INFINITY, values: vec![0.0, 1.0].into() })]));
    assert!(!bad.gpu_ok());
}

/// A colour-managed sequence's plan: layers composited in the working space (DaVinci Wide Gamut),
/// the output transform on the accumulator, then a display-referred layer on top — like
/// `execute_cpu`.
#[test]
fn output_stage_converts_the_composite_like_the_cpu() {
    use super::blend_tests::{interior, ramp_layer, stats8, yuv_frame};
    use filmcraft_geom::Affine;
    use filmcraft_render::blend::Blend;
    use filmcraft_render::plan::{FramePlan, OutputStage, PlanLayer, execute_cpu};
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let pipe = filmcraft_color::ColorPipeline { working: filmcraft_color::WorkingSpace::DaVinciWide, ..filmcraft_color::ColorPipeline::REC709 };
    let ops = filmcraft_color::OutputTransform::display(&pipe).ops();
    assert!(!ops.is_empty());
    // bright working-space values (scene light up to ~6×) so the tone map has work to do
    let bright = {
        let f = ramp_layer(200, 120, 1);
        let filmcraft_frame::PixelData::RgbaF32(px) = &f.data else { unreachable!() };
        Arc::new(VideoFrame::rgba_f32(200, 120, px.iter().enumerate().map(|(i, v)| if i % 4 == 3 { *v } else { v * 6.0 }).collect()))
    };
    let (w, h) = (320usize, 180usize);
    for at in [2usize, 3] {
        let plan = FramePlan::Layers {
            width: w,
            height: h,
            layers: vec![
                PlanLayer { frame: yuv_frame(640, 360), matrix: Affine::scale(0.5, 0.5), opacity: 1.0, blend: Blend::Normal, fx: None },
                PlanLayer { frame: bright.clone(), matrix: Affine::translate(100.0, 40.0), opacity: 0.9, blend: Blend::Screen, fx: None },
                // a caption: drawn after the conversion when `at` = 2
                PlanLayer { frame: ramp_layer(80, 30, 4), matrix: Affine::translate(20.0, 140.0), opacity: 1.0, blend: Blend::Normal, fx: None },
            ],
            output: Some(OutputStage { ops: ops.clone().into(), at }),
        };
        let cpu = execute_cpu(&plan).over_black_rgba8();
        c.composite(&plan);
        let (_, _, gpu) = c.read_output().expect("readback");
        let (p99, mean, max) = stats8(&cpu, &gpu, &interior(&plan));
        eprintln!("output at {at}: 8-bit p99 {p99} mean {mean:.3} max {max}");
        assert!(p99 <= 3 && mean < 0.75, "at {at}: p99 {p99}, mean {mean}, max {max}");
    }
}

/// Colour grades (fork ADR 0004) compiled to colour operations run on the GPU like on the CPU:
/// three serial nodes (primaries, contrast / saturation / white balance, curves) in a Rec. 709 and
/// a DWG / DI working space; an identity grade compiles to nothing.
#[test]
fn gpu_compiled_grades_match_cpu() {
    use filmcraft_color::WorkingSpace;
    use filmcraft_project::grade::{Grade, GradeNode};
    use filmcraft_project::{Param, ParamValue};
    use filmcraft_time::Tick;
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let node = |id: u64, params: &[(&str, ParamValue)]| {
        let mut n = GradeNode::new(id);
        for (k, v) in params {
            n.params.insert((*k).into(), Param::new(v.clone()));
        }
        n
    };
    let grade = Grade {
        nodes: vec![
            node(
                1,
                &[
                    ("lift", ParamValue::Color([0.04, 0.0, -0.03, 0.01])),
                    ("gamma", ParamValue::Color([0.0, 0.2, -0.1, 0.0])),
                    ("gain", ParamValue::Color([0.1, 0.0, 0.2, -0.05])),
                    ("offset", ParamValue::Color([0.0, 0.0, 0.0, 0.02])),
                ],
            ),
            node(
                2,
                &[
                    ("contrast", ParamValue::Float(1.25)),
                    ("pivot", ParamValue::Float(0.4)),
                    ("saturation", ParamValue::Float(1.3)),
                    ("temperature", ParamValue::Float(-20.0)),
                    ("tint", ParamValue::Float(10.0)),
                    // film grain at this node's place (G1.6)
                    ("grain.strength", ParamValue::Float(0.6)),
                    ("grain.size", ParamValue::Float(1.5)),
                    ("grain.softness", ParamValue::Float(0.4)),
                ],
            ),
            node(
                3,
                &[
                    ("curve.master", ParamValue::Curve(vec![[0.0, 0.03], [0.4, 0.35], [1.0, 0.97]])),
                    ("curve.r", ParamValue::Curve(vec![[0.0, 0.0], [0.6, 0.65], [1.0, 1.0]])),
                ],
            ),
        ],
        bypass: false,
    };
    let mut c = GpuCompositor::new(&dev, &q);
    let (w, h) = (67u32, 41u32);
    let px = picture(w, h);
    let frame = VideoFrame::rgba_f32(w, h, px.clone());
    let mut failures = Vec::new();
    for ws in [WorkingSpace::Rec709, WorkingSpace::DaVinciWide] {
        assert!(filmcraft_render::grade::compile(&Grade::with_node(), Tick(0), ws).is_empty());
        let ops = filmcraft_render::grade::compile(&grade, Tick(123_456_789), ws);
        assert!(ops.iter().any(|o| matches!(o, ColorOp::Grain { .. })));
        let op = FxOp::ColorChain(Arc::from(ops));
        assert!(op.gpu_ok(), "{ws:?}");
        let mut cpu = filmcraft_render::Image { w: w as usize, h: h as usize, px: px.clone() };
        op.apply(&mut cpu);
        let fx = LayerFx { size: (w, h), decimation: 1, ops: vec![op] };
        let (_, _, gpu) = c.effect_image(&frame, &fx).expect("effect image");
        let (worst, flips) = compare(&cpu.px, &gpu);
        eprintln!("grade in {ws:?}: max rel diff {worst:.2e}, {:.3}% off by > 2 %", flips * 100.0);
        if worst > TOL || flips > 0.0 {
            failures.push(format!("{ws:?}: {worst:.2e}, flips {flips:.4}"));
        }
    }
    assert!(failures.is_empty(), "GPU grades differ from the CPU:\n{}", failures.join("\n"));
}
