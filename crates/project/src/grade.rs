//! Colour grades: a per-clip list of serial correction nodes (fork ADR 0004).
//!
//! A grade runs on the clip's grading signal after its input transform and before its effect
//! stack. Each node holds keyframable [`Param`]s keyed by name ([`NODE_PARAMS`]); a missing key is
//! the identity, so an empty node changes nothing. Nodes are addressed by a stable id, never by
//! index, so a later node graph (parallel / layer nodes) can add input lists without a format
//! break.

use std::collections::BTreeMap;

use crate::keyframe::{Param, ParamValue};
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};

/// The most nodes a grade compiles; further nodes (only in a damaged or hostile file) are kept
/// but ignored.
pub const MAX_NODES: usize = 64;

/// A clip's colour grade.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Grade {
    /// Serial nodes, applied first to last.
    #[serde(default)]
    pub nodes: Vec<GradeNode>,
    /// Bypass the whole grade (the clip renders as if it had none).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
}

/// One correction node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradeNode {
    /// Stable id, unique within its grade.
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    /// Parameters by key ([`NODE_PARAMS`]); missing keys are the identity.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Param>,
}

fn enabled_default() -> bool {
    true
}

/// What a node parameter holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeParamKind {
    /// A number with its identity value and range.
    Float { identity: f32, min: f32, max: f32 },
    /// A colour wheel: `Color([r, g, b, master])` offsets around identity (all 0), each in
    /// `-range..=range`.
    Wheel { range: f32 },
    /// A tone curve: `Curve` points in 0…1 (identity: the straight line).
    Curve,
}

/// One entry of the node schema.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeParam {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: NodeParamKind,
}

/// Mid grey (18 % scene reflectance) on the DaVinci Intermediate signal: the default contrast
/// pivot.
pub const DI_MID_GREY: f32 = 0.336;

/// Every node parameter (G0 / G1 primaries and curves; ADR 0004 §2).
pub const NODE_PARAMS: &[NodeParam] = &[
    NodeParam { key: "lift", label: "Lift", kind: NodeParamKind::Wheel { range: 1.0 } },
    NodeParam { key: "gamma", label: "Gamma", kind: NodeParamKind::Wheel { range: 1.0 } },
    NodeParam { key: "gain", label: "Gain", kind: NodeParamKind::Wheel { range: 4.0 } },
    NodeParam { key: "offset", label: "Offset", kind: NodeParamKind::Wheel { range: 1.0 } },
    NodeParam { key: "contrast", label: "Contrast", kind: NodeParamKind::Float { identity: 1.0, min: 0.0, max: 4.0 } },
    NodeParam { key: "pivot", label: "Pivot", kind: NodeParamKind::Float { identity: DI_MID_GREY, min: 0.0, max: 1.0 } },
    NodeParam { key: "saturation", label: "Saturation", kind: NodeParamKind::Float { identity: 1.0, min: 0.0, max: 4.0 } },
    NodeParam { key: "temperature", label: "Temperature", kind: NodeParamKind::Float { identity: 0.0, min: -100.0, max: 100.0 } },
    NodeParam { key: "tint", label: "Tint", kind: NodeParamKind::Float { identity: 0.0, min: -100.0, max: 100.0 } },
    NodeParam { key: "curve.master", label: "Master curve", kind: NodeParamKind::Curve },
    NodeParam { key: "curve.r", label: "Red curve", kind: NodeParamKind::Curve },
    NodeParam { key: "curve.g", label: "Green curve", kind: NodeParamKind::Curve },
    NodeParam { key: "curve.b", label: "Blue curve", kind: NodeParamKind::Curve },
    // film grain (G1.6): monochrome, luma weighted, a new pattern every frame
    NodeParam { key: "grain.strength", label: "Grain strength", kind: NodeParamKind::Float { identity: 0.0, min: 0.0, max: 1.0 } },
    // (grain cell size in full-resolution pixels: the clip's source pixels; see `filmcraft_render::grade::GradeCtx`)
    NodeParam { key: "grain.size", label: "Grain size", kind: NodeParamKind::Float { identity: 1.0, min: 1.0, max: 4.0 } },
    NodeParam { key: "grain.softness", label: "Grain softness", kind: NodeParamKind::Float { identity: 0.5, min: 0.0, max: 1.0 } },
];

/// The schema entry for `key`.
pub fn node_param(key: &str) -> Option<&'static NodeParam> {
    NODE_PARAMS.iter().find(|p| p.key == key)
}

/// The most curve points a node curve keeps when compiled.
pub const MAX_CURVE_POINTS: usize = 64;

impl GradeNode {
    /// A node with every parameter at identity.
    pub fn new(id: u64) -> Self {
        Self { id, label: String::new(), enabled: true, params: BTreeMap::new() }
    }

    pub fn param(&self, key: &str) -> Option<&Param> {
        self.params.get(key)
    }

    /// A number parameter at clip time `t`, clamped to its schema range; non-finite, missing or
    /// mistyped values give the identity.
    pub fn float_at(&self, key: &str, t: Tick) -> f32 {
        let Some(NodeParamKind::Float { identity, min, max }) = node_param(key).map(|p| p.kind) else {
            return 0.0;
        };
        match self.params.get(key).map(|p| p.value_at(t)) {
            Some(ParamValue::Float(v)) if v.is_finite() => (v as f32).clamp(min, max),
            _ => identity,
        }
    }

    /// A wheel parameter at clip time `t` as `[r, g, b, master]` offsets, each clamped to the
    /// wheel's range; non-finite components are 0.
    pub fn wheel_at(&self, key: &str, t: Tick) -> [f32; 4] {
        let Some(NodeParamKind::Wheel { range }) = node_param(key).map(|p| p.kind) else {
            return [0.0; 4];
        };
        match self.params.get(key).map(|p| p.value_at(t)) {
            Some(ParamValue::Color(c)) => c.map(|v| if v.is_finite() { v.clamp(-range, range) } else { 0.0 }),
            _ => [0.0; 4],
        }
    }

    /// A curve parameter's points (static value), finite, clamped to 0…1 and at most
    /// [`MAX_CURVE_POINTS`]; `None` when missing, mistyped or the straight line.
    pub fn curve(&self, key: &str) -> Option<Vec<[f32; 2]>> {
        let Some(ParamValue::Curve(c)) = self.params.get(key).map(|p| &p.value) else {
            return None;
        };
        let pts: Vec<[f32; 2]> =
            c.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).take(MAX_CURVE_POINTS).map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]).collect();
        if pts.len() < 2 || pts.iter().all(|p| (p[0] - p[1]).abs() < 1e-4) {
            return None;
        }
        Some(pts)
    }
}

impl Grade {
    /// A grade with one empty node (id 1).
    pub fn with_node() -> Self {
        Self { nodes: vec![GradeNode::new(1)], bypass: false }
    }

    /// The id a new node takes: one more than the largest in use, or (only after a hostile file
    /// used `u64::MAX`) the smallest unused id, so ids stay unique.
    pub fn next_id(&self) -> u64 {
        match self.nodes.iter().map(|n| n.id).max() {
            None => 1,
            Some(max) if max < u64::MAX => max + 1,
            Some(_) => {
                let used: std::collections::BTreeSet<u64> = self.nodes.iter().map(|n| n.id).collect();
                (1..=u64::MAX).find(|i| !used.contains(i)).unwrap_or(0)
            }
        }
    }

    pub fn node(&self, id: u64) -> Option<&GradeNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u64) -> Option<&mut GradeNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// The nodes that render: none when bypassed, otherwise the enabled ones among the first
    /// [`MAX_NODES`].
    pub fn active_nodes(&self) -> impl Iterator<Item = &GradeNode> {
        self.nodes.iter().take(if self.bypass { 0 } else { MAX_NODES }).filter(|n| n.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_keeps_nodes_params_and_keyframes() {
        let mut g = Grade::with_node();
        let id = g.next_id();
        let mut n = GradeNode::new(id);
        n.label = "Curves".into();
        n.enabled = false;
        n.params.insert("curve.master".into(), Param::new(ParamValue::Curve(vec![[0.0, 0.1], [1.0, 0.9]])));
        let mut lift = Param::new(ParamValue::Color([0.0; 4]));
        lift.put_keyframe(Tick(0), ParamValue::Color([0.1, 0.0, 0.0, 0.0]));
        lift.put_keyframe(Tick(1000), ParamValue::Color([0.0, 0.0, 0.2, 0.05]));
        g.nodes[0].params.insert("lift".into(), lift);
        g.nodes.push(n);
        g.bypass = true;
        let s = serde_json::to_string(&g).unwrap();
        let back: Grade = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.next_id(), 3);
    }

    #[test]
    fn minimal_json_loads_with_defaults_and_unknown_fields_are_ignored() {
        let g: Grade = serde_json::from_str(r#"{"nodes":[{"id":7,"future":{"x":1}}],"alsoFuture":3}"#).unwrap();
        assert_eq!(g.nodes.len(), 1);
        assert!(g.nodes[0].enabled && !g.bypass && g.nodes[0].params.is_empty());
        let empty: Grade = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Grade::default());
    }

    #[test]
    fn hostile_values_are_clamped_or_identity() {
        let mut n = GradeNode::new(1);
        n.params.insert("contrast".into(), Param::new(ParamValue::Float(1e30)));
        n.params.insert("saturation".into(), Param::new(ParamValue::Float(f64::NAN)));
        n.params.insert("pivot".into(), Param::new(ParamValue::Text("x".into())));
        n.params.insert("gain".into(), Param::new(ParamValue::Color([f32::INFINITY, -1e9, f32::NAN, 0.5])));
        n.params.insert("curve.r".into(), Param::new(ParamValue::Curve(vec![[f32::NAN, 0.0], [2.0, -3.0], [0.0, 0.5]])));
        let t = Tick(0);
        assert_eq!(n.float_at("contrast", t), 4.0);
        assert_eq!(n.float_at("saturation", t), 1.0);
        assert_eq!(n.float_at("pivot", t), DI_MID_GREY);
        assert_eq!(n.float_at("no.such.key", t), 0.0);
        assert_eq!(n.wheel_at("gain", t), [0.0, -4.0, 0.0, 0.5]);
        assert_eq!(n.wheel_at("lift", t), [0.0; 4]);
        assert_eq!(n.curve("curve.r"), Some(vec![[1.0, 0.0], [0.0, 0.5]]));
        n.params.insert("curve.g".into(), Param::new(ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]])));
        assert_eq!(n.curve("curve.g"), None);
        let many = (0..10_000).map(|i| [i as f32 / 10_000.0, 0.5]).collect();
        n.params.insert("curve.b".into(), Param::new(ParamValue::Curve(many)));
        assert_eq!(n.curve("curve.b").map(|c| c.len()), Some(MAX_CURVE_POINTS));
    }

    #[test]
    fn grain_keys_default_to_identity_and_clamp() {
        let mut n = GradeNode::new(1);
        let t = Tick(0);
        assert_eq!([n.float_at("grain.strength", t), n.float_at("grain.size", t), n.float_at("grain.softness", t)], [0.0, 1.0, 0.5]);
        n.params.insert("grain.strength".into(), Param::new(ParamValue::Float(1e9)));
        n.params.insert("grain.size".into(), Param::new(ParamValue::Float(-3.0)));
        n.params.insert("grain.softness".into(), Param::new(ParamValue::Float(f64::NAN)));
        assert_eq!([n.float_at("grain.strength", t), n.float_at("grain.size", t), n.float_at("grain.softness", t)], [1.0, 1.0, 0.5]);
    }

    #[test]
    fn active_nodes_honour_bypass_enabled_and_the_cap() {
        let mut g = Grade { nodes: (1..=10_000).map(GradeNode::new).collect(), bypass: false };
        g.nodes[0].enabled = false;
        assert_eq!(g.active_nodes().count(), MAX_NODES - 1);
        g.bypass = true;
        assert_eq!(g.active_nodes().count(), 0);
        let g = Grade { nodes: vec![GradeNode::new(u64::MAX)], bypass: false };
        assert_eq!(g.next_id(), 1);
        let g = Grade { nodes: vec![GradeNode::new(1), GradeNode::new(u64::MAX), GradeNode::new(2)], bypass: false };
        assert_eq!(g.next_id(), 3);
    }
}
