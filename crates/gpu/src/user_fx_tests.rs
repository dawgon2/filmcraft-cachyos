//! User shader effects on the GPU (fork Y3.4, ADR 0006): a module composed by
//! `filmcraft_usershader` runs as its own pass; the expected images follow the prelude's
//! arithmetic (straight sRGB in, clamped, linear premultiplied out). A module that doesn't build
//! is skipped (the image passes through), never a panic.

use super::blend_tests::device;
use super::fx_tests::{full_precision, picture};
use super::*;
use filmcraft_render::gpufx::{FxOp, UserOp};
use filmcraft_render::plan::LayerFx;
use std::sync::Arc;

fn enc(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}
fn dec(v: f32) -> f32 {
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The prelude's `effect` contract on the CPU: `f` maps straight sRGB (clamped at 0 below) to
/// straight sRGB; the result is clamped and stored linear premultiplied.
fn expected(base: &[f32], f: impl Fn([f32; 3]) -> [f32; 3]) -> Vec<f32> {
    base.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            // (a transparent source pixel reads as transparent black; the effects here keep its alpha 0)
            if p[3] <= 1e-6 {
                return [0.0; 4];
            }
            let c = [enc((p[0] / p[3]).max(0.0)), enc((p[1] / p[3]).max(0.0)), enc((p[2] / p[3]).max(0.0))];
            let o = f(c);
            let a = p[3].clamp(0.0, 1.0);
            [dec(o[0].clamp(0.0, 1.0)) * a, dec(o[1].clamp(0.0, 1.0)) * a, dec(o[2].clamp(0.0, 1.0)) * a, a]
        })
        .collect()
}

fn op(name: &str, src: &str, pv: Vec<[f32; 4]>) -> FxOp {
    let fx = filmcraft_usershader::parse(src, name).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    FxOp::User(Box::new(UserOp { id: format!("user.{name}"), hash: fx.hash, module: filmcraft_usershader::compose(&fx, "rgba32float").into(), pv, time: 0.0 }))
}

fn worst(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

const PASS: &str = "fn effect(px: vec2<f32>) -> vec4<f32> {\n    return sample(px);\n}\n";
const INVERT: &str = "fn effect(px: vec2<f32>) -> vec4<f32> {\n    let c = load(vec2<i32>(px));\n    return vec4(1.0 - c.rgb, c.a);\n}\n";
const TINTED: &str = "// @param amount: float = 0.5 [0.0, 1.0] \"Amount\"\n// @param tint: color = #ff8000 \"Tint\"\n// @param on: bool = true \"On\"\nfn effect(px: vec2<f32>) -> vec4<f32> {\n    let c = sample(px);\n    if !P.on { return c; }\n    return vec4(mix(c.rgb, P.tint, P.amount), c.a);\n}\n";

/// Pass-through, invert and a parameterised mix match the prelude's arithmetic, on the
/// full-precision stage and the half-float one.
#[test]
fn user_effects_match_the_prelude_on_the_gpu() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    for (full, tol) in [(true, 2e-4f32), (false, 3e-3)] {
        let mut c = if full { full_precision(&dev, &q) } else { GpuCompositor::with_fx_format(&dev, &q, fx::FX_FORMAT_HALF) };
        let (w, h) = (67u32, 41u32);
        let frame = VideoFrame::rgba_f32(w, h, picture(w, h));
        let (_, _, base) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![] }).expect("working image");
        let cases: Vec<(&str, FxOp, Vec<f32>)> = vec![
            ("pass", op("pass", PASS, vec![]), expected(&base, |c| c)),
            ("invert", op("invert", INVERT, vec![]), expected(&base, |c| c.map(|v| 1.0 - v))),
            (
                "tinted",
                op("tinted", TINTED, vec![[0.25, 0.0, 0.0, 0.0], [1.0, 0.5, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0]]),
                expected(&base, |c| [c[0] * 0.75 + 0.25, c[1] * 0.75 + 0.125, c[2] * 0.75]),
            ),
            ("tinted off", op("tinted", TINTED, vec![[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0], [0.0; 4]]), expected(&base, |c| c)),
        ];
        let mut failures = Vec::new();
        for (name, o, want) in cases {
            assert!(o.gpu_ok(), "{name}");
            let (_, _, got) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![o] }).expect("effect image");
            let d = worst(&got, &want);
            eprintln!("full={full} {name}: max diff {d:.2e}");
            if d > tol {
                failures.push(format!("full={full} {name}: {d:.2e}"));
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
    }
}

/// A user pass between built-in passes keeps the order (Invert then a built-in Invert is the
/// identity within the clamp), and a module that fails to build is skipped, logged, no panic.
#[test]
fn user_effects_chain_and_broken_modules_pass_through() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut c = full_precision(&dev, &q);
    let (w, h) = (33u32, 17u32);
    let frame = VideoFrame::rgba_f32(w, h, picture(w, h));
    let (_, _, base) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![] }).expect("working image");
    let broken = FxOp::User(Box::new(UserOp {
        id: "user.broken".into(),
        hash: 7,
        module: Arc::from("@compute @workgroup_size(1) fn nope() { let = ; }"),
        pv: vec![],
        time: 0.0,
    }));
    let (_, _, got) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![broken.clone()] }).expect("effect image");
    assert_eq!(worst(&got, &base), 0.0, "a broken module passes the image through");
    // twice (cached as broken), and between working passes
    let pass = op("pass", PASS, vec![]);
    let (_, _, got) = c
        .effect_image(
            &frame,
            &LayerFx { size: (w, h), decimation: 1, ops: vec![broken, pass.clone(), op("invert", INVERT, vec![]), op("invert", INVERT, vec![])] },
        )
        .expect("chain");
    let (_, _, want) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![pass] }).expect("pass");
    let d = worst(&got, &want);
    assert!(d < 2e-4, "invert twice after pass-through: {d:.2e}");
}

/// The registry → `FxOp::eval` → GPU path: a registered user effect evaluates its parameters in
/// declaration order (float, colour as straight sRGB, bool, point in working pixels with Auto →
/// the centre) and renders; once unregistered it has no GPU op (the CPU passes it through).
#[test]
fn registered_user_effects_evaluate_their_parameters() {
    use filmcraft_project::effect::{UserEffectSpec, UserParamSpec, register_user_effects};
    use filmcraft_project::{ParamKind, ParamValue, find_effect};
    let src = "// @param amount: float = 0.5 [0.0, 1.0] \"Amount\"\n// @param tint: color = #ff8000 \"Tint\"\n// @param on: bool = true \"On\"\n// @param centre: point = auto \"Centre\"\nfn effect(px: vec2<f32>) -> vec4<f32> {\n    let c = sample(px);\n    let d = select(0.0, 1.0, distance(px, P.centre) < 4.0);\n    if !P.on { return c; }\n    return vec4(mix(c.rgb, P.tint, P.amount * d), c.a);\n}\n";
    let fx = filmcraft_usershader::parse(src, "spot").expect("parses");
    let params = vec![
        UserParamSpec {
            id: "amount".into(),
            label: "Amount".into(),
            kind: ParamKind::Float { min: 0.0, max: 1.0, soft_min: 0.0, soft_max: 1.0, unit: "", decimals: 2 },
            default: ParamValue::Float(0.5),
        },
        UserParamSpec { id: "tint".into(), label: "Tint".into(), kind: ParamKind::Color, default: ParamValue::Color([1.0, 0.5, 0.0, 1.0]) },
        UserParamSpec { id: "on".into(), label: "On".into(), kind: ParamKind::Bool, default: ParamValue::Bool(true) },
        UserParamSpec {
            id: "centre".into(),
            label: "Centre".into(),
            kind: ParamKind::Point,
            default: ParamValue::Vec2(filmcraft_geom::Vec2::new(f64::NAN, f64::NAN)),
        },
    ];
    let spec = UserEffectSpec {
        id: "user.gpu-spot".into(),
        name: "Spot".into(),
        category: None,
        params,
        hash: fx.hash,
        module: filmcraft_usershader::compose(&fx, "rgba32float").into(),
    };
    assert_eq!(register_user_effects(&[spec]), ["user.gpu-spot"]);
    let e = find_effect("user.gpu-spot").expect("registered").instance();
    let cx = filmcraft_render::effects::FxCtx {
        t: filmcraft_time::Tick::ZERO,
        px_scale: 0.5,
        seconds: 2.5,
        timecode: "",
        clip_name: "",
        project: None,
        env: None,
        working: filmcraft_color::WorkingSpace::Rec709,
    };
    let (w, h) = (33usize, 17usize);
    let Some(FxOp::User(u)) = FxOp::eval(&e, &cx, w, h) else { panic!("a user op") };
    assert_eq!(u.pv, vec![[0.5, 0.0, 0.0, 0.0], [1.0, 0.5, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0], [16.5, 8.5, 0.0, 0.0]]);
    assert_eq!((u.time, u.hash), (2.5, fx.hash));
    let op = FxOp::User(u);
    assert!(op.gpu_ok());
    let mut cpu = filmcraft_render::Image { w, h, px: picture(w as u32, h as u32) };
    let before = cpu.px.clone();
    op.apply(&mut cpu);
    assert_eq!(cpu.px, before, "the CPU passes a user effect through");
    if let Some((dev, q)) = device() {
        let mut c = full_precision(&dev, &q);
        let frame = VideoFrame::rgba_f32(w as u32, h as u32, picture(w as u32, h as u32));
        let (_, _, base) = c.effect_image(&frame, &LayerFx { size: (w as u32, h as u32), decimation: 1, ops: vec![] }).expect("working image");
        let (_, _, got) = c.effect_image(&frame, &LayerFx { size: (w as u32, h as u32), decimation: 1, ops: vec![op] }).expect("effect image");
        let changed = got.chunks(4).zip(base.chunks(4)).filter(|(a, b)| worst(a, b) > 1e-3).count();
        // the spot (radius 4 around the centre) and nothing else
        assert!((5..=60).contains(&changed), "{changed} pixels changed");
    }
    register_user_effects(&[]);
    assert!(FxOp::eval(&e, &cx, w, h).is_none(), "a missing user effect has no GPU op");
}

/// The slow-pass rule (ADR 0006): a user pass over the limit on 3 frames in a row is turned off
/// (one message; later frames pass the image through); a fast frame in between resets the count.
/// Needs timestamp queries (skipped without them).
#[test]
fn slow_user_effects_are_turned_off_after_three_frames() {
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        eprintln!("no timestamp queries; skipping");
        return;
    }
    let desc = wgpu::DeviceDescriptor { required_features: adapter.features() & crate::wanted_features(), ..Default::default() };
    let Ok((dev, q)) = pollster::block_on(adapter.request_device(&desc)) else {
        eprintln!("no device; skipping");
        return;
    };
    let mut c = full_precision(&dev, &q);
    let (w, h) = (33u32, 17u32);
    let frame = VideoFrame::rgba_f32(w, h, picture(w, h));
    let fx = |ops: Vec<FxOp>| LayerFx { size: (w, h), decimation: 1, ops };
    let (_, _, base) = c.effect_image(&frame, &fx(vec![])).expect("working image");
    let invert = op("invert", INVERT, vec![]);
    let run = |c: &mut GpuCompositor| c.effect_image(&frame, &fx(vec![invert.clone()])).expect("effect image").2;
    let tiny = 1e-9;
    c.set_user_effect_slow_limit(tiny);
    let inverted = run(&mut c);
    assert!(worst(&inverted, &base) > 0.1);
    run(&mut c);
    // a fast frame resets the count
    c.set_user_effect_slow_limit(1e9);
    run(&mut c);
    c.set_user_effect_slow_limit(tiny);
    run(&mut c);
    run(&mut c);
    assert!(c.take_user_effect_messages().is_empty(), "two slow frames in a row are allowed");
    assert_eq!(worst(&run(&mut c), &inverted), 0.0, "the third slow frame still renders");
    let msgs = c.take_user_effect_messages();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs.first().is_some_and(|m| m.contains("user.invert")), "{msgs:?}");
    assert_eq!(worst(&run(&mut c), &base), 0.0, "turned off: the image passes through");
    assert!(c.take_user_effect_messages().is_empty(), "one message");
}

/// Quarantine support (ADR 0006): user passes count as in flight until the GPU finishes them, and
/// a device loss hands the ones in flight to the hook.
#[test]
fn user_effects_in_flight_reach_the_device_lost_hook() {
    let got: Arc<std::sync::Mutex<Vec<(String, u64)>>> = Arc::default();
    let sink = got.clone();
    crate::set_user_effect_lost_hook(Box::new(move |keys| sink.lock().unwrap().extend_from_slice(keys)));
    let key = ("user.lost-test".to_string(), 9u64);
    crate::user_in_flight(std::slice::from_ref(&key), true);
    crate::user_in_flight(std::slice::from_ref(&key), true);
    crate::user_in_flight(std::slice::from_ref(&key), false);
    assert!(crate::user_effects_in_flight().contains(&key), "two submissions, one done");
    crate::notify_device_lost();
    assert!(got.lock().unwrap().contains(&key));
    crate::user_in_flight(std::slice::from_ref(&key), false);
    crate::user_in_flight(std::slice::from_ref(&key), false);
    assert!(!crate::user_effects_in_flight().contains(&key), "never below zero");
    // a rendered frame leaves nothing in flight once the GPU is done
    let Some((dev, q)) = device() else { return };
    let mut c = full_precision(&dev, &q);
    let frame = VideoFrame::rgba_f32(9, 7, picture(9, 7));
    let o = op("flight-check", INVERT, vec![]);
    let FxOp::User(u) = &o else { unreachable!() };
    let k = (u.id.clone(), u.hash);
    c.effect_image(&frame, &LayerFx { size: (9, 7), decimation: 1, ops: vec![o.clone()] }).expect("effect image");
    assert!(!crate::user_effects_in_flight().contains(&k));
}

/// The bundled examples (`assets/effects`, ADR 0006) render on the GPU with their defaults (and
/// over time): finite, inside 0..1 premultiplied, and they change the picture.
#[test]
fn bundled_examples_render() {
    let Some((dev, q)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let examples = [
        ("chromatic-aberration", include_str!("../../../assets/effects/chromatic-aberration.wgsl")),
        ("glitch", include_str!("../../../assets/effects/glitch.wgsl")),
        ("vhs", include_str!("../../../assets/effects/vhs.wgsl")),
        ("light-leak", include_str!("../../../assets/effects/light-leak.wgsl")),
    ];
    let (w, h) = (96u32, 64u32);
    let frame = VideoFrame::rgba_f32(w, h, picture(w, h));
    for half in [false, true] {
        let mut c = if half { GpuCompositor::with_fx_format(&dev, &q, fx::FX_FORMAT_HALF) } else { full_precision(&dev, &q) };
        let (_, _, base) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![] }).expect("working image");
        for (name, src) in examples {
            let fx = filmcraft_usershader::parse(src, name).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            // defaults: float / bool / colour (sRGB) / point (Auto: the centre)
            let pv: Vec<[f32; 4]> = fx
                .params
                .iter()
                .map(|p| match p.kind {
                    filmcraft_usershader::ParamKind::Float { default, .. } => [default, 0.0, 0.0, 0.0],
                    filmcraft_usershader::ParamKind::Bool(b) => [f32::from(u8::from(b)), 0.0, 0.0, 0.0],
                    filmcraft_usershader::ParamKind::Color(c) => [c[0], c[1], c[2], 1.0],
                    filmcraft_usershader::ParamKind::Point(pt) => {
                        let p = pt.unwrap_or([w as f32 / 2.0, h as f32 / 2.0]);
                        [p[0], p[1], 0.0, 0.0]
                    }
                })
                .collect();
            for time in [0.0f32, 1.37] {
                let o = FxOp::User(Box::new(UserOp {
                    id: format!("example.{name}"),
                    hash: fx.hash,
                    module: filmcraft_usershader::compose(&fx, "rgba32float").into(),
                    pv: pv.clone(),
                    time,
                }));
                let (_, _, got) = c.effect_image(&frame, &LayerFx { size: (w, h), decimation: 1, ops: vec![o] }).expect("effect image");
                let bad = got
                    .chunks(4)
                    .filter(|p| !p.iter().all(|v| v.is_finite() && (-1e-6..=1.0 + 1e-3).contains(v)) || p[0].max(p[1]).max(p[2]) > p[3] + 1e-3)
                    .count();
                assert_eq!(bad, 0, "{name} half={half} t={time}: {bad} pixels out of range");
                let changed = got.chunks(4).zip(base.chunks(4)).filter(|(a, b)| worst(a, b) > 2e-3).count();
                assert!(changed > (w * h / 50) as usize, "{name} half={half} t={time}: only {changed} pixels changed");
            }
        }
    }
}
