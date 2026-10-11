//! A clip's colour grade compiled to colour operations (fork ADR 0004).
//!
//! The grade runs between the input transform and the effect stack, on the same grading signal
//! as Lumetri ([`GradeSpace`]: DaVinci Intermediate in a DWG / DI sequence). Each node's
//! parameters become existing [`ColorOp`]s; nothing here is new grading maths. Identity
//! parameters emit nothing, and a grade that changes nothing compiles to an empty chain, so it
//! renders bit-exactly like no grade at all.

use filmcraft_color::ops::{ColorOp, Table};
use filmcraft_color::{GradeSpace, WorkingSpace};
use filmcraft_project::TrackItem;
use filmcraft_project::grade::{Grade, GradeNode};
use filmcraft_time::Tick;

/// Luma weights of the saturation control (Lumetri's).
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// HDR White of the grading signal in HDR working spaces (Lumetri's default).
const HDR_WHITE_NITS: f32 = 1000.0;

/// Samples of a compiled tone curve (Lumetri's).
const CURVE_SAMPLES: usize = 1024;

/// `item`'s grade at clip time `t` as a chain on working linear light (empty: no grade, bypassed,
/// or nothing that changes the picture).
pub fn item_ops(item: &TrackItem, t: Tick, working: WorkingSpace) -> Vec<ColorOp> {
    item.grade.as_ref().map(|g| compile(g, t, working)).unwrap_or_default()
}

/// True when `item`'s grade changes the picture at clip time `t` (an empty, identity, bypassed or
/// disabled grade doesn't, so the clip keeps every shortcut it has without one: a title stays
/// vector-sharp, a nested sequence keeps drawing its own layers).
pub fn changes_picture(item: &TrackItem, t: Tick, working: WorkingSpace) -> bool {
    !item_ops(item, t, working).is_empty()
}

/// `grade` at clip time `t` as a chain on working linear light: the grading signal's encode, every
/// enabled node's operations in order, the decode. Empty when no node changes the picture.
pub fn compile(grade: &Grade, t: Tick, working: WorkingSpace) -> Vec<ColorOp> {
    let gs = GradeSpace::new(working, HDR_WHITE_NITS);
    let body: Vec<ColorOp> = grade.active_nodes().flat_map(|n| node_ops(n, t, &gs)).collect();
    if body.is_empty() {
        return body;
    }
    let mut ops = gs.encode_ops();
    ops.extend(body);
    ops.extend(gs.decode_ops());
    // (every value is clamped to its schema range, but a chain that still isn't finite is dropped:
    // a grade never poisons the picture)
    if ops.iter().all(ColorOp::is_finite) { ops } else { Vec::new() }
}

/// One node's operations on the grading signal, in order: temperature / tint (in linear light),
/// lift / gamma / gain / offset, contrast around the pivot, saturation, curves.
fn node_ops(n: &GradeNode, t: Tick, gs: &GradeSpace) -> Vec<ColorOp> {
    let mut ops = Vec::new();
    // white balance in linear light (Lumetri's gains)
    let (temp, tint) = (n.float_at("temperature", t) / 100.0, n.float_at("tint", t) / 100.0);
    if temp != 0.0 || tint != 0.0 {
        let g = [1.0 + 0.35 * temp, 1.0 - 0.3 * tint, 1.0 - 0.35 * temp];
        ops.extend(gs.decode_ops());
        ops.push(ColorOp::Matrix { m: [[g[0], 0.0, 0.0], [0.0, g[1], 0.0], [0.0, 0.0, g[2]]], offset: [0.0; 3] });
        ops.extend(gs.encode_ops());
    }
    // wheels: offsets around identity, the master ring added to every channel
    let wheel = |key: &str| {
        let w = n.wheel_at(key, t);
        [w[0] + w[3], w[1] + w[3], w[2] + w[3]]
    };
    let (lift, gamma, gain, offset) = (wheel("lift"), wheel("gamma"), wheel("gain"), wheel("offset"));
    if [lift, gamma, gain, offset].iter().flatten().any(|&v| v != 0.0) {
        ops.push(ColorOp::Lggo {
            lift,
            // up brightens the mids: v^(1/γ) with γ = 2^wheel (½ … 2 for one wheel's range)
            gamma: gamma.map(|g| g.exp2()),
            gain: gain.map(|g| (1.0 + g).max(0.0)),
            offset,
        });
    }
    let (contrast, pivot) = (n.float_at("contrast", t), n.float_at("pivot", t));
    if contrast != 1.0 {
        let k = contrast;
        ops.push(ColorOp::Matrix { m: [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]], offset: [pivot * (1.0 - k); 3] });
    }
    let sat = n.float_at("saturation", t);
    if sat != 1.0 {
        ops.push(ColorOp::Saturation { sat, vibrance: 0.0, weights: LUMA });
    }
    let hdr = gs.is_hdr();
    for (key, channels) in
        [("curve.master", [true; 3]), ("curve.r", [true, false, false]), ("curve.g", [false, true, false]), ("curve.b", [false, false, true])]
    {
        if let Some(points) = n.curve(key) {
            let table = Table { lo: 0.0, hi: 1.0, values: crate::effects::curve_lut(&points, CURVE_SAMPLES).into() };
            ops.push(ColorOp::Curve { table, channels, hdr });
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::grade::MAX_NODES;
    use filmcraft_project::{Param, ParamValue};

    fn node(params: &[(&str, ParamValue)]) -> GradeNode {
        let mut n = GradeNode::new(1);
        for (k, v) in params {
            n.params.insert((*k).into(), Param::new(v.clone()));
        }
        n
    }

    const SPACES: [WorkingSpace; 2] = [WorkingSpace::Rec709, WorkingSpace::DaVinciWide];

    #[test]
    fn identity_grades_compile_to_nothing() {
        let identity = node(&[
            ("lift", ParamValue::Color([0.0; 4])),
            ("contrast", ParamValue::Float(1.0)),
            ("pivot", ParamValue::Float(0.2)),
            ("saturation", ParamValue::Float(1.0)),
            ("curve.master", ParamValue::Curve(vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]])),
            ("unknown.key", ParamValue::Float(9.0)),
        ]);
        for ws in SPACES {
            assert!(compile(&Grade::default(), Tick(0), ws).is_empty());
            assert!(compile(&Grade::with_node(), Tick(0), ws).is_empty());
            assert!(compile(&Grade { nodes: vec![identity.clone(); 3], bypass: false }, Tick(0), ws).is_empty());
        }
    }

    #[test]
    fn bypassed_and_disabled_nodes_compile_to_nothing() {
        let mut lifted = node(&[("lift", ParamValue::Color([0.1, 0.0, 0.0, 0.0]))]);
        let mut g = Grade { nodes: vec![lifted.clone()], bypass: true };
        assert!(compile(&g, Tick(0), WorkingSpace::DaVinciWide).is_empty());
        lifted.enabled = false;
        g = Grade { nodes: vec![lifted], bypass: false };
        assert!(compile(&g, Tick(0), WorkingSpace::DaVinciWide).is_empty());
    }

    #[test]
    fn a_node_compiles_to_the_signal_ops_between_encode_and_decode() {
        let n = node(&[
            ("lift", ParamValue::Color([0.1, 0.0, 0.0, 0.0])),
            ("gamma", ParamValue::Color([0.0, 1.0, 0.0, 0.0])),
            ("gain", ParamValue::Color([0.0, 0.0, 0.5, 0.0])),
            ("offset", ParamValue::Color([0.0, 0.0, 0.0, 0.02])),
            ("contrast", ParamValue::Float(1.2)),
            ("pivot", ParamValue::Float(0.4)),
            ("saturation", ParamValue::Float(0.5)),
            ("curve.g", ParamValue::Curve(vec![[0.0, 0.1], [1.0, 1.0]])),
        ]);
        let gs = GradeSpace::new(WorkingSpace::DaVinciWide, HDR_WHITE_NITS);
        let ops = compile(&Grade { nodes: vec![n], bypass: false }, Tick(0), WorkingSpace::DaVinciWide);
        let enc = gs.encode_ops();
        let dec = gs.decode_ops();
        assert_eq!(ops[..enc.len()], enc[..]);
        assert_eq!(ops[ops.len() - dec.len()..], dec[..]);
        let body = &ops[enc.len()..ops.len() - dec.len()];
        assert_eq!(body[0], ColorOp::Lggo { lift: [0.1, 0.0, 0.0], gamma: [1.0, 2.0, 1.0], gain: [1.0, 1.0, 1.5], offset: [0.02; 3] });
        let k = 1.2f32;
        assert_eq!(body[1], ColorOp::Matrix { m: [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]], offset: [0.4 * (1.0 - k); 3] });
        assert_eq!(body[2], ColorOp::Saturation { sat: 0.5, vibrance: 0.0, weights: LUMA });
        assert!(matches!(&body[3], ColorOp::Curve { channels: [false, true, false], hdr: false, .. }));
        assert_eq!(body.len(), 4);
    }

    /// The compiled chain on a pixel against the node's controls computed by hand (the oracle).
    #[test]
    fn compiled_chain_matches_a_hand_computed_pixel() {
        let n = node(&[
            ("lift", ParamValue::Color([0.05, 0.0, 0.0, 0.0])),
            ("gain", ParamValue::Color([0.0, 0.2, 0.0, 0.0])),
            ("contrast", ParamValue::Float(1.5)),
            ("pivot", ParamValue::Float(0.5)),
        ]);
        // Rec. 709 working space: the grading signal is sRGB-encoded display values
        let ops = compile(&Grade { nodes: vec![n], bypass: false }, Tick(0), WorkingSpace::Rec709);
        let lin = [0.18f32, 0.18, 0.18];
        let out = filmcraft_color::ops::apply_chain(&ops, lin);
        let srgb_enc = |x: f32| if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        let srgb_dec = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let v = srgb_enc(0.18);
        let (lift, gain) = ([0.05f32, 0.0, 0.0], [1.0f32, 1.2, 1.0]);
        for c in 0..3 {
            let a = lift[c] + (gain[c] - lift[c]) * v;
            let s = (a - 0.5) * 1.5 + 0.5;
            let want = srgb_dec(s.clamp(0.0, 1.0));
            assert!((out[c] - want).abs() < 1e-4, "channel {c}: {} vs {want}", out[c]);
        }
    }

    #[test]
    fn keyframed_wheels_interpolate_over_clip_time() {
        let mut n = GradeNode::new(1);
        let mut p = Param::new(ParamValue::Color([0.0; 4]));
        p.put_keyframe(Tick(0), ParamValue::Color([0.0; 4]));
        p.put_keyframe(Tick(100), ParamValue::Color([0.0, 0.0, 0.0, 0.2]));
        n.params.insert("offset".into(), p);
        let g = Grade { nodes: vec![n], bypass: false };
        assert!(compile(&g, Tick(0), WorkingSpace::Rec709).is_empty());
        let mid = compile(&g, Tick(50), WorkingSpace::Rec709);
        assert!(mid.iter().any(|o| matches!(o, ColorOp::Lggo { offset, .. } if (offset[0] - 0.1).abs() < 1e-6)));
    }

    #[test]
    fn temperature_runs_in_linear_light() {
        let n = node(&[("temperature", ParamValue::Float(50.0))]);
        let gs = GradeSpace::new(WorkingSpace::DaVinciWide, HDR_WHITE_NITS);
        let ops = compile(&Grade { nodes: vec![n], bypass: false }, Tick(0), WorkingSpace::DaVinciWide);
        let mut want = gs.encode_ops();
        want.extend(gs.decode_ops());
        want.push(ColorOp::Matrix { m: [[1.175, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.825]], offset: [0.0; 3] });
        want.extend(gs.encode_ops());
        want.extend(gs.decode_ops());
        assert_eq!(ops, want);
    }

    #[test]
    fn hostile_grades_never_produce_non_finite_ops() {
        let mut nodes = Vec::new();
        for i in 0..10_000u64 {
            let mut n = GradeNode::new(i);
            n.params.insert("gain".into(), Param::new(ParamValue::Color([f32::MAX, f32::NAN, -f32::MAX, f32::INFINITY])));
            n.params.insert("gamma".into(), Param::new(ParamValue::Color([-1e30, 1e30, 0.0, 0.0])));
            n.params.insert("contrast".into(), Param::new(ParamValue::Float(f64::INFINITY)));
            n.params.insert("pivot".into(), Param::new(ParamValue::Float(-1e300)));
            n.params.insert("curve.master".into(), Param::new(ParamValue::Curve(vec![[0.5, f32::NAN]; 10_000])));
            nodes.push(n);
        }
        let g = Grade { nodes, bypass: false };
        for ws in [WorkingSpace::Rec709, WorkingSpace::DaVinciWide, WorkingSpace::Rec2100Pq, WorkingSpace::Rec2100Hlg] {
            let ops = compile(&g, Tick(i64::MAX), ws);
            assert!(!ops.is_empty() && ops.iter().all(ColorOp::is_finite));
            let lggo = ops.iter().filter(|o| matches!(o, ColorOp::Lggo { .. })).count();
            assert_eq!(lggo, MAX_NODES);
            let out = filmcraft_color::ops::apply_chain(&ops, [0.5, 0.2, 0.1]);
            assert!(out.iter().all(|v| !v.is_nan()), "{out:?}");
        }
    }
}
