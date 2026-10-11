//! A clip's colour grade compiled to colour operations (fork ADR 0004).
//!
//! The grade runs between the input transform and the effect stack, on the same grading signal
//! as Lumetri ([`GradeSpace`]: DaVinci Intermediate in a DWG / DI sequence). Each node's
//! parameters become existing [`ColorOp`]s (film grain included: [`ColorOp::Grain`], a
//! positional operation like the vignette); nothing here is new grading maths. Identity
//! parameters emit nothing, and a grade that changes nothing compiles to an empty chain, so it
//! renders bit-exactly like no grade at all.

use filmcraft_color::ops::{ColorOp, GRAIN_MAX_SIZE, Table};
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

/// Where a grade renders, for its positional operations (film grain).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeCtx {
    /// Image pixels per full-resolution pixel. "Full resolution" is the clip's source pixels
    /// (`px_scale` of the effect stage: the working image's width over the source width, also
    /// when a large source is graded at the size drawn, `plan.rs`; `FILMCRAFT_FULL_SIZE_GRADE`
    /// keeps the source size, scale 1). For an adjustment layer: sequence pixels × render scale.
    pub px_scale: f32,
    /// Sequence time of the frame (freeze frames and holds still get a new grain every frame).
    pub seq_time: Tick,
    /// The track item (clip id): two stacked clips of one source don't share a grain pattern.
    pub clip: u64,
}

impl Default for GradeCtx {
    fn default() -> Self {
        Self { px_scale: 1.0, seq_time: Tick(0), clip: 0 }
    }
}

/// `item`'s grade at clip time `t` (sequence time `seq_t`) as a chain on working linear light,
/// for an image of `px_scale` pixels per source pixel (empty: no grade, bypassed, or nothing that
/// changes the picture).
pub fn item_ops(item: &TrackItem, t: Tick, seq_t: Tick, working: WorkingSpace, px_scale: f32) -> Vec<ColorOp> {
    let ctx = GradeCtx { px_scale, seq_time: seq_t, clip: item.id.0 };
    item.grade.as_ref().map(|g| compile_at(g, t, working, ctx)).unwrap_or_default()
}

/// True when `item`'s grade changes the picture at clip time `t` (an empty, identity, bypassed or
/// disabled grade doesn't, so the clip keeps every shortcut it has without one: a title stays
/// vector-sharp, a nested sequence keeps drawing its own layers). Cheap: reads the parameters,
/// builds no operations or curve tables. (A hostile grade whose chain [`compile_at`] drops as
/// non-finite still counts: the clip then takes the effect path with an empty grade.)
pub fn changes_picture(item: &TrackItem, t: Tick) -> bool {
    item.grade.as_ref().is_some_and(|g| g.active_nodes().any(|n| node_changes(n, t)))
}

/// Whether [`node_ops`] emits anything for `n` at `t` (the same conditions).
fn node_changes(n: &GradeNode, t: Tick) -> bool {
    n.float_at("temperature", t) != 0.0
        || n.float_at("tint", t) != 0.0
        || ["lift", "gamma", "gain", "offset"].iter().any(|k| {
            let w = n.wheel_at(k, t);
            [w[0] + w[3], w[1] + w[3], w[2] + w[3]].iter().any(|&v| v != 0.0)
        })
        || n.float_at("contrast", t) != 1.0
        || n.float_at("saturation", t) != 1.0
        || CURVES.iter().any(|(k, _)| n.curve(k).is_some())
        || n.float_at("grain.strength", t) > 0.0
}

const CURVES: [(&str, [bool; 3]); 4] =
    [("curve.master", [true; 3]), ("curve.r", [true, false, false]), ("curve.g", [false, true, false]), ("curve.b", [false, false, true])];

/// [`compile_at`] for a full-resolution image.
pub fn compile(grade: &Grade, t: Tick, working: WorkingSpace) -> Vec<ColorOp> {
    compile_at(grade, t, working, GradeCtx::default())
}

/// `grade` at clip time `t` as a chain on working linear light: the grading signal's encode, every
/// enabled node's operations in order, the decode. Empty when no node changes the picture.
/// `ctx.px_scale` scales positional operations (grain size), so a reduced-resolution preview
/// shows the grain the full-size render has.
///
/// Film grain is a positional colour operation ([`ColorOp::Grain`]), so a node's grain sits at the
/// node's place in the chain, on the grading signal (later nodes grade the grain too, as in a
/// node grader), and the whole grade stays one fused per-pixel pass.
pub fn compile_at(grade: &Grade, t: Tick, working: WorkingSpace, ctx: GradeCtx) -> Vec<ColorOp> {
    let gs = GradeSpace::new(working, HDR_WHITE_NITS);
    let px_scale = if ctx.px_scale.is_finite() && ctx.px_scale > 0.0 { ctx.px_scale } else { 1.0 };
    let ctx = GradeCtx { px_scale, ..ctx };
    let body: Vec<ColorOp> = grade.active_nodes().flat_map(|n| node_ops(n, t, &gs, &ctx)).collect();
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
/// lift / gamma / gain / offset, contrast around the pivot, saturation, curves, film grain.
fn node_ops(n: &GradeNode, t: Tick, gs: &GradeSpace, ctx: &GradeCtx) -> Vec<ColorOp> {
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
    for (key, channels) in CURVES {
        if let Some(points) = n.curve(key) {
            let table = Table { lo: 0.0, hi: 1.0, values: crate::effects::curve_lut(&points, CURVE_SAMPLES).into() };
            ops.push(ColorOp::Curve { table, channels, hdr });
        }
    }
    let strength = n.float_at("grain.strength", t);
    if strength > 0.0 {
        let softness = n.float_at("grain.softness", t);
        let (size, strength) = grain_cell(n.float_at("grain.size", t), ctx.px_scale, strength, softness);
        let seed = grain_seed(&[t.0 as u64, ctx.seq_time.0 as u64, ctx.clip, n.id]);
        ops.push(ColorOp::Grain { strength, inv_size: 1.0 / size, softness, seed, weights: LUMA });
    }
    ops
}

/// The grain cell size in image pixels and the strength for a `size` (source pixels) grain drawn
/// at `px_scale`. Cells smaller than a pixel are drawn 1 px and attenuated by the cell size: a
/// pixel of a downscaled render averages (1 / cell)² independent cells, whose mean has
/// 1 / √((1 / cell)²) = cell times their standard deviation. So a ¼-resolution preview shows the
/// grain strength of the full-size render box-downscaled by 4.
///
/// Soft grain correlates neighbouring pixels, so its average keeps more of its strength: over
/// many cells, by [`soft_gain`]`(softness)` (≈ 1.45 at softness ½). The gain is blended in by
/// 1 − cell² (none at 1 px cells, all of it for many cells per pixel).
fn grain_cell(size: f32, px_scale: f32, strength: f32, softness: f32) -> (f32, f32) {
    let cell = (size * px_scale).min(GRAIN_MAX_SIZE);
    if cell >= 1.0 {
        return (cell, strength);
    }
    // (finite, ≥ 0.5 × the smallest scale: never 0 or NaN here)
    let cell = cell.clamp(0.0, 1.0);
    let gain = 1.0 + (soft_gain(softness) - 1.0) * (1.0 - cell * cell);
    (1.0, strength * cell * gain)
}

/// How much more of its standard deviation soft grain keeps than hard grain when averaged over
/// many cells: per axis, every cell value contributes ∫₀¹ df / N(f) with the normalised
/// interpolation's per-axis norm N(f) = √((1 − a)² + a²), a = softness · smoothstep(f)
/// (`filmcraft_color::ops::grain_noise`); the two axes multiply.
fn soft_gain(softness: f32) -> f32 {
    let s = if softness.is_finite() { softness.clamp(0.0, 1.0) } else { 0.0 };
    const STEPS: usize = 64;
    let per_axis = (0..STEPS)
        .map(|i| {
            let f = (i as f32 + 0.5) / STEPS as f32;
            let a = s * f * f * (3.0 - 2.0 * f);
            1.0 / ((1.0 - a) * (1.0 - a) + a * a).sqrt().max(0.5)
        })
        .sum::<f32>()
        / STEPS as f32;
    per_axis * per_axis
}

/// The grain pattern's seed: a hash of the clip time and the sequence time (one pattern per
/// frame, the same every time that frame renders, new every frame also in a hold), the clip and
/// the node (stacked clips and two grain nodes don't repeat each other).
fn grain_seed(parts: &[u64]) -> u32 {
    let mut x = 0u64;
    for p in parts {
        // splitmix64 finaliser over each part
        x = (x ^ p).wrapping_add(0x9e37_79b9_7f4a_7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^= x >> 31;
    }
    (x ^ (x >> 32)) as u32
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

    fn with_id(mut n: GradeNode, id: u64) -> GradeNode {
        n.id = id;
        n
    }

    fn grain_node(id: u64, strength: f64) -> GradeNode {
        let n = node(&[("grain.strength", ParamValue::Float(strength)), ("grain.size", ParamValue::Float(2.0)), ("grain.softness", ParamValue::Float(0.25))]);
        with_id(n, id)
    }

    /// A 48×48 grey patch through `ops` (positions matter for grain).
    fn patch(ops: &[ColorOp], level: f32) -> Vec<[f32; 3]> {
        (0..48 * 48).map(|i| filmcraft_color::ops::apply_chain_at(ops, [level; 3], [(i % 48) as f32, (i / 48) as f32])).collect()
    }

    #[test]
    fn zero_grain_strength_compiles_to_nothing() {
        let n = node(&[("grain.strength", ParamValue::Float(0.0)), ("grain.size", ParamValue::Float(3.0)), ("grain.softness", ParamValue::Float(1.0))]);
        for ws in SPACES {
            assert!(compile(&Grade { nodes: vec![n.clone()], bypass: false }, Tick(5), ws).is_empty());
        }
    }

    #[test]
    fn grain_sits_at_its_nodes_place_in_the_chain() {
        let lift = node(&[("lift", ParamValue::Color([0.1, 0.0, 0.0, 0.0]))]);
        let sat = with_id(node(&[("saturation", ParamValue::Float(0.5))]), 3);
        let g = Grade { nodes: vec![lift, grain_node(2, 0.5), sat], bypass: false };
        let gs = GradeSpace::new(WorkingSpace::DaVinciWide, HDR_WHITE_NITS);
        let ops = compile_at(&g, Tick(77), WorkingSpace::DaVinciWide, GradeCtx { px_scale: 0.5, ..GradeCtx::default() });
        let enc = gs.encode_ops().len();
        let body = &ops[enc..ops.len() - gs.decode_ops().len()];
        assert!(matches!(body[0], ColorOp::Lggo { .. }));
        // grain size 2 full-resolution pixels at half resolution: 1 pixel cells
        assert!(matches!(body[1], ColorOp::Grain { strength: 0.5, inv_size: 1.0, softness: 0.25, .. }), "{:?}", body[1]);
        assert!(matches!(body[2], ColorOp::Saturation { .. }));
        assert_eq!(body.len(), 3);
    }

    #[test]
    fn grain_is_the_same_for_a_frame_and_new_every_frame() {
        let g = Grade { nodes: vec![grain_node(1, 0.6)], bypass: false };
        for ws in SPACES {
            let (a, b, c) = (compile(&g, Tick(1000), ws), compile(&g, Tick(1000), ws), compile(&g, Tick(2000), ws));
            assert_eq!(patch(&a, 0.18), patch(&b, 0.18));
            let (pa, pc) = (patch(&a, 0.18), patch(&c, 0.18));
            let differ = pa.iter().zip(&pc).filter(|(x, y)| x != y).count();
            assert!(differ > pa.len() / 2, "{differ}");
        }
        // two grain nodes on the same frame draw different patterns
        let two = |id| compile(&Grade { nodes: vec![grain_node(id, 0.6)], bypass: false }, Tick(1000), WorkingSpace::Rec709);
        assert_ne!(patch(&two(1), 0.18), patch(&two(2), 0.18));
    }

    #[test]
    fn grain_is_weaker_in_shadows_and_highlights() {
        let g = Grade { nodes: vec![grain_node(1, 1.0)], bypass: false };
        // Rec. 709: the grading signal is sRGB; 0, mid grey (sRGB 0.5) and 1 in linear light
        let ops = compile(&g, Tick(0), WorkingSpace::Rec709);
        // (measured on the signal the grain was added to)
        let enc = |x: f32| if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        let mad = |level: f32| patch(&ops, level).iter().map(|p| (enc(p[1]) - enc(level)).abs()).sum::<f32>() / 2304.0;
        let (black, mid, white) = (mad(0.0), mad(0.214), mad(1.0));
        assert!(mid > 0.01, "{mid}");
        assert!(black < mid * 0.2 && white < mid * 0.2, "{black} {mid} {white}");
    }

    #[test]
    fn hostile_grain_params_and_scales_stay_finite() {
        let mut n = GradeNode::new(u64::MAX);
        n.params.insert("grain.strength".into(), Param::new(ParamValue::Float(1e300)));
        n.params.insert("grain.size".into(), Param::new(ParamValue::Float(-1e300)));
        n.params.insert("grain.softness".into(), Param::new(ParamValue::Float(f64::NAN)));
        let g = Grade { nodes: vec![n], bypass: false };
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e-30, 1e30] {
            for t in [Tick(i64::MIN), Tick(-1), Tick(i64::MAX)] {
                let ops = compile_at(&g, t, WorkingSpace::Rec2100Pq, GradeCtx { px_scale: scale, seq_time: t, clip: u64::MAX });
                assert!(!ops.is_empty() && ops.iter().all(ColorOp::is_finite));
                let out = filmcraft_color::ops::apply_chain_at(&ops, [0.5, 0.2, 0.1], [1e6, -3.0]);
                assert!(out.iter().all(|v| v.is_finite()), "{out:?}");
            }
        }
    }

    fn std_dev(v: &[f32]) -> f32 {
        let n = v.len().max(1) as f32;
        let mean = v.iter().sum::<f32>() / n;
        (v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / n).sqrt()
    }

    /// Green channel of a flat `level` patch of `w`² pixels through `ops`.
    fn flat(ops: &[ColorOp], w: usize, level: f32) -> Vec<f32> {
        (0..w * w).map(|i| filmcraft_color::ops::apply_chain_at(ops, [level; 3], [(i % w) as f32, (i / w) as f32])[1]).collect()
    }

    /// A ¼-resolution preview shows about the grain strength of the full-size render
    /// box-downscaled by 4 (cells below a pixel are attenuated, not drawn at full amplitude).
    #[test]
    fn preview_grain_matches_a_downscaled_full_render() {
        for (size, softness) in [(1.0, 0.0), (1.0, 0.5), (1.0, 1.0), (2.0, 0.0), (2.0, 0.5), (0.5, 0.0), (4.0, 0.5)] {
            let n =
                node(&[("grain.strength", ParamValue::Float(1.0)), ("grain.size", ParamValue::Float(size)), ("grain.softness", ParamValue::Float(softness))]);
            let g = Grade { nodes: vec![n], bypass: false };
            let at = |px_scale| compile_at(&g, Tick(9), WorkingSpace::Rec709, GradeCtx { px_scale, seq_time: Tick(9), clip: 4 });
            let (w, k) = (256usize, 4usize);
            let full = flat(&at(1.0), w, 0.214);
            let small = w / k;
            let down: Vec<f32> = (0..small * small)
                .map(|i| {
                    let (x, y) = (i % small, i / small);
                    (0..k * k).map(|j| full.get((y * k + j / k) * w + x * k + j % k).copied().unwrap_or(0.0)).sum::<f32>() / (k * k) as f32
                })
                .collect();
            let preview = flat(&at(0.25), small, 0.214);
            let (a, b) = (std_dev(&down), std_dev(&preview));
            eprintln!("size {size} softness {softness}: downscaled {a:.5}, preview {b:.5}");
            assert!(b > 0.0 && (b / a - 1.0).abs() < 0.25, "size {size} softness {softness}: downscaled {a}, preview {b}");
        }
    }

    #[test]
    fn grain_seed_mixes_sequence_time_and_clip() {
        let g = Grade { nodes: vec![grain_node(1, 0.6)], bypass: false };
        let ops = |seq: i64, clip: u64| compile_at(&g, Tick(500), WorkingSpace::Rec709, GradeCtx { px_scale: 1.0, seq_time: Tick(seq), clip });
        assert_eq!(patch(&ops(10, 1), 0.18), patch(&ops(10, 1), 0.18));
        // a hold (same clip time, later sequence time) and a stacked clip of the same source
        assert_ne!(patch(&ops(10, 1), 0.18), patch(&ops(11, 1), 0.18));
        assert_ne!(patch(&ops(10, 1), 0.18), patch(&ops(10, 2), 0.18));
    }

    /// `changes_picture`'s table-free test agrees with the compiled chain.
    #[test]
    fn node_changes_agrees_with_node_ops() {
        let cases: Vec<(&str, ParamValue)> = vec![
            ("lift", ParamValue::Color([0.0; 4])),
            ("lift", ParamValue::Color([0.1, 0.0, 0.0, -0.1])),
            ("gamma", ParamValue::Color([0.0, 0.0, 0.0, 0.2])),
            ("contrast", ParamValue::Float(1.0)),
            ("contrast", ParamValue::Float(1.1)),
            ("pivot", ParamValue::Float(0.1)),
            ("saturation", ParamValue::Float(0.9)),
            ("temperature", ParamValue::Float(0.0)),
            ("tint", ParamValue::Float(-3.0)),
            ("curve.b", ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]])),
            ("curve.b", ParamValue::Curve(vec![[0.0, 0.1], [1.0, 1.0]])),
            ("grain.strength", ParamValue::Float(0.0)),
            ("grain.strength", ParamValue::Float(0.01)),
            ("grain.size", ParamValue::Float(3.0)),
        ];
        for (k, v) in cases {
            let n = node(&[(k, v.clone())]);
            let compiled = !compile(&Grade { nodes: vec![n.clone()], bypass: false }, Tick(0), WorkingSpace::DaVinciWide).is_empty();
            assert_eq!(node_changes(&n, Tick(0)), compiled, "{k} {v:?}");
        }
    }

    #[test]
    fn half_resolution_pixels_land_in_the_full_resolution_cells() {
        use filmcraft_color::ops::grain_noise;
        // size 2 at full resolution, 1 at half: half-res pixel x covers full-res 2x and 2x + 1
        for y in 0..20u32 {
            for x in 0..20u32 {
                let half = grain_noise([x as f32, y as f32], 1.0, 0.0, 77);
                for (fx, fy) in [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)] {
                    assert_eq!(grain_noise([fx as f32, fy as f32], 0.5, 0.0, 77), half, "{x} {y}");
                }
            }
        }
    }
}
