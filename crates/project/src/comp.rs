//! Compositions (C1): a node graph rendered as one picture, used on the timeline like a nested
//! sequence. The graph is a [`crate::dag`] graph; every node maps to maths that already exists
//! (blend modes, the Motion matrix, key / mask / blur effects, one grade node). See ADR 0007
//! (`docs/adr/0007-node-graph.md`) and `docs/fork/DESIGN-C1.md`.

use std::collections::BTreeMap;

use filmcraft_geom::Vec2;
use filmcraft_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};

use crate::ItemId;
use crate::dag::{self, DagError, DagNode, Edge, NodeId, Order};
use crate::effect::{EffectInstance, ParamDef, ch, col, find_effect, pt};
use crate::grade::{GradeNode, MAX_WINDOWS};
use crate::keyframe::{Param, ParamValue};
use crate::mask::Mask;

/// Largest comp side in pixels.
pub const MAX_SIDE: u32 = 16_384;
/// Largest comp area in pixels.
pub const MAX_PIXELS: u64 = 256 * 1024 * 1024;
/// Most parameters a node keeps.
pub const MAX_PARAMS: usize = 64;
/// Longest node label in characters.
pub const MAX_LABEL: usize = 64;
/// Longest text value in characters (text nodes).
pub const MAX_TEXT: usize = 4_096;
/// The key effects a Key node can use.
pub const KEY_EFFECTS: &[&str] = &["ultra_key", "luma_key"];

/// A composition: a node graph rendered as one picture of `width × height` at `rate`.
/// A project item (`ItemKind::Comp`), so one comp can be on the timeline many times.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comp {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub rate: FrameRate,
    #[serde(default)]
    pub duration: Tick,
    #[serde(default)]
    pub nodes: Vec<CompNode>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// The Media Out node (None: the comp is transparent).
    #[serde(default)]
    pub output: Option<NodeId>,
    /// Working / grading space of the Color nodes: the sequence's when the comp was made.
    #[serde(default = "default_color")]
    pub color: filmcraft_color::ColorPipeline,
}

fn default_color() -> filmcraft_color::ColorPipeline {
    filmcraft_color::ColorPipeline::REC709
}

/// One node of a comp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompNode {
    pub id: NodeId,
    pub kind: CompKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
    /// Keyframable parameters by key ([`params`] is the schema; missing = its default).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Param>,
    /// Canvas position (UI only, saved so the layout survives).
    #[serde(default)]
    pub pos: [f32; 2],
}

/// Where a Media In node's picture comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MediaInSource {
    /// A project item (media, subclip, nested sequence or another comp).
    Item { item: ItemId },
    /// Video track `track` of the sequence the comp clip sits in, rendered alone at the comp
    /// clip's time.
    Layer { track: u32 },
}

/// What a node does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CompKind {
    /// A picture from the project or the timeline, `offset` later than comp time.
    MediaIn {
        source: MediaInSource,
        #[serde(default)]
        offset: Tick,
    },
    /// The comp's result (pass-through).
    MediaOut,
    /// Input 0 = background, input 1 = foreground; params `opacity`, `blend` (as Opacity).
    Merge,
    /// Motion's parameters: position, scale, scale width, uniform scale, rotation, anchor.
    Transform,
    /// A key effect (one of [`KEY_EFFECTS`]); params are that effect's.
    Key { effect: String },
    /// Alpha × the coverage of these masks (each with its own mode, feather and invert).
    Mask {
        #[serde(default)]
        masks: Vec<Mask>,
    },
    /// Gaussian Blur's parameters.
    Blur,
    /// One grade node (its parameters live in `node.params`, not the comp node's).
    Color { node: GradeNode },
    /// A graphic text layer (the text effect instance a graphic clip holds).
    Text { layer: Box<EffectInstance> },
    /// A flat colour; param `color`.
    Solid,
    /// Linear or radial between two colours; params `start`, `end` (0..1 of the frame),
    /// `color_a`, `color_b`, `shape`.
    Gradient,
}

/// Where a new comp's Media Out node sits on the canvas (x; canvas units).
pub const MEDIA_OUT_X: f32 = 640.0;

/// Every kind's key, as `comp.addNode` / `comp.params` name it.
pub const KINDS: &[&str] = &["mediaIn", "mediaOut", "merge", "transform", "key", "mask", "blur", "color", "text", "solid", "gradient"];

impl CompKind {
    /// The kind's key (one of [`KINDS`]).
    pub fn key(&self) -> &'static str {
        match self {
            CompKind::MediaIn { .. } => "mediaIn",
            CompKind::MediaOut => "mediaOut",
            CompKind::Merge => "merge",
            CompKind::Transform => "transform",
            CompKind::Key { .. } => "key",
            CompKind::Mask { .. } => "mask",
            CompKind::Blur => "blur",
            CompKind::Color { .. } => "color",
            CompKind::Text { .. } => "text",
            CompKind::Solid => "solid",
            CompKind::Gradient => "gradient",
        }
    }

    /// A new node of kind `key` with default contents (Media In: track 0; Key: Ultra Key; Text: an
    /// default text layer). None for an unknown key.
    pub fn from_key(key: &str) -> Option<CompKind> {
        Some(match key {
            "mediaIn" => CompKind::MediaIn { source: MediaInSource::Layer { track: 0 }, offset: Tick::ZERO },
            "mediaOut" => CompKind::MediaOut,
            "merge" => CompKind::Merge,
            "transform" => CompKind::Transform,
            "key" => CompKind::Key { effect: "ultra_key".into() },
            "mask" => CompKind::Mask { masks: Vec::new() },
            "blur" => CompKind::Blur,
            "color" => CompKind::Color { node: GradeNode::new(1) },
            "text" => CompKind::Text { layer: Box::new(find_effect(crate::graphic::TEXT_LAYER)?.instance()) },
            "solid" => CompKind::Solid,
            "gradient" => CompKind::Gradient,
            _ => return None,
        })
    }

    /// Number of input ports.
    pub fn inputs(&self) -> u8 {
        match self {
            CompKind::MediaIn { .. } | CompKind::Text { .. } | CompKind::Solid | CompKind::Gradient => 0,
            CompKind::Merge => 2,
            CompKind::MediaOut | CompKind::Transform | CompKind::Key { .. } | CompKind::Mask { .. } | CompKind::Blur | CompKind::Color { .. } => 1,
        }
    }
}

impl DagNode for CompNode {
    fn id(&self) -> NodeId {
        self.id
    }
    fn inputs(&self) -> u8 {
        self.kind.inputs()
    }
    fn outputs(&self) -> u8 {
        1
    }
    fn bypassed(&self) -> bool {
        self.bypass
    }
}

/// The parameter schema of a node kind (Color nodes use `grade::NODE_PARAMS`, Text nodes their
/// layer's own parameters: both return an empty list here).
pub fn params(kind: &CompKind) -> Vec<ParamDef> {
    let from = |id: &str, skip: &[&str]| -> Vec<ParamDef> {
        find_effect(id).map(|d| d.params.iter().filter(|p| !skip.contains(&p.id)).cloned().collect()).unwrap_or_default()
    };
    match kind {
        CompKind::Merge => from("opacity", &[]),
        CompKind::Transform => from("motion", &["anti_flicker"]),
        CompKind::Key { effect } if KEY_EFFECTS.contains(&effect.as_str()) => from(effect, &[]),
        CompKind::Blur => from("gaussian_blur", &[]),
        CompKind::Solid => vec![col("color", "Color", [1.0, 1.0, 1.0, 1.0])],
        CompKind::Gradient => vec![
            pt("start", "Start", 0.0, 0.5),
            pt("end", "End", 1.0, 0.5),
            col("color_a", "Start Color", [0.0, 0.0, 0.0, 1.0]),
            col("color_b", "End Color", [1.0, 1.0, 1.0, 1.0]),
            ch("shape", "Shape", &["Linear", "Radial"], 0),
        ],
        _ => Vec::new(),
    }
}

impl CompNode {
    pub fn new(id: NodeId, kind: CompKind) -> Self {
        Self { id, kind, label: String::new(), bypass: false, params: BTreeMap::new(), pos: [0.0, 0.0] }
    }

    /// A parameter's value at comp time `t`: the stored one, else the schema default.
    pub fn value_at(&self, key: &str, t: Tick) -> Option<ParamValue> {
        match self.params.get(key) {
            Some(p) => Some(p.value_at(t)),
            None => params(&self.kind).into_iter().find(|d| d.id == key).map(|d| d.default),
        }
    }

    fn sanitize(&mut self) {
        if self.params.len() > MAX_PARAMS {
            self.params = std::mem::take(&mut self.params).into_iter().take(MAX_PARAMS).collect();
        }
        for p in self.params.values_mut() {
            cap_text(&mut p.value);
            for k in &mut p.keyframes {
                cap_text(&mut k.value);
            }
        }
        if self.label.chars().count() > MAX_LABEL {
            self.label = self.label.chars().take(MAX_LABEL).collect();
        }
        for v in &mut self.pos {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
        match &mut self.kind {
            CompKind::Mask { masks } => masks.truncate(MAX_WINDOWS),
            CompKind::Color { node } => {
                node.windows.truncate(MAX_WINDOWS);
                if node.params.len() > MAX_PARAMS * 4 {
                    node.params = std::mem::take(&mut node.params).into_iter().take(MAX_PARAMS * 4).collect();
                }
            }
            CompKind::Text { layer } => {
                if layer.params.len() > MAX_PARAMS {
                    layer.params = std::mem::take(&mut layer.params).into_iter().take(MAX_PARAMS).collect();
                }
                for p in layer.params.values_mut() {
                    cap_text(&mut p.value);
                    for k in &mut p.keyframes {
                        cap_text(&mut k.value);
                    }
                }
            }
            CompKind::MediaIn { offset, .. } => {
                // Keep offsets within ± a day so adding them to a time can't overflow.
                let day = filmcraft_time::TICKS_PER_SECOND.saturating_mul(86_400);
                offset.0 = offset.0.clamp(-day, day);
            }
            _ => {}
        }
    }
}

fn cap_text(v: &mut ParamValue) {
    if let ParamValue::Text(s) = v
        && s.chars().count() > MAX_TEXT
    {
        *s = s.chars().take(MAX_TEXT).collect();
    }
}

impl Comp {
    /// An empty comp with one Media Out node (id 1) as its output.
    pub fn new(width: u32, height: u32, rate: FrameRate, duration: Tick) -> Self {
        let mut c = Self {
            width,
            height,
            rate,
            duration,
            // Media Out on the right of the canvas, where the flow ends (new nodes start at 0, 0)
            nodes: vec![CompNode { pos: [MEDIA_OUT_X, 0.0], ..CompNode::new(1, CompKind::MediaOut) }],
            edges: Vec::new(),
            output: Some(1),
            color: default_color(),
        };
        c.sanitize();
        c
    }

    /// An id no node has yet (max + 1; 1 for an empty comp).
    pub fn next_id(&self) -> NodeId {
        self.nodes.iter().map(|n| n.id).max().unwrap_or(0).saturating_add(1)
    }

    pub fn node(&self, id: NodeId) -> Option<&CompNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut CompNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// The evaluation order of the nodes the output depends on (see [`dag::validate`]).
    pub fn validate(&self) -> Result<Order, DagError> {
        let out = self.output.ok_or(DagError::NoOutput)?;
        dag::validate(&self.nodes, &self.edges, out)
    }

    /// Brings a comp from a file within the caps: sizes clamped (1 ..= [`MAX_SIDE`], area ≤
    /// [`MAX_PIXELS`]), counts truncated, labels / texts shortened, non-finite canvas positions
    /// zeroed, a bad frame rate or negative duration reset. A cycle or a bad edge is left in place
    /// for [`Comp::validate`] to report (the comp then renders transparent).
    pub fn sanitize(&mut self) {
        self.width = self.width.clamp(1, MAX_SIDE);
        self.height = self.height.clamp(1, MAX_SIDE);
        let max_h = MAX_PIXELS / u64::from(self.width);
        if u64::from(self.height) > max_h {
            self.height = u32::try_from(max_h).unwrap_or(MAX_SIDE).clamp(1, MAX_SIDE);
        }
        if self.rate.num <= 0 || self.rate.den <= 0 {
            self.rate = FrameRate::default();
        }
        if self.duration.0 < 0 {
            self.duration = Tick::ZERO;
        }
        self.nodes.truncate(dag::MAX_NODES);
        self.edges.truncate(dag::MAX_EDGES);
        for n in &mut self.nodes {
            n.sanitize();
        }
    }

    /// The project items Media In nodes read (for self-nesting checks and media management).
    pub fn sources(&self) -> impl Iterator<Item = ItemId> + '_ {
        self.nodes.iter().filter_map(|n| match &n.kind {
            CompKind::MediaIn { source: MediaInSource::Item { item }, .. } => Some(*item),
            _ => None,
        })
    }

    /// The comp's frame size as a vector.
    pub fn size(&self) -> Vec2 {
        Vec2::new(f64::from(self.width), f64::from(self.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::Port;

    fn edge(from: NodeId, to: NodeId, port: u8) -> Edge {
        Edge::new(Port::new(from, 0), Port::new(to, port))
    }

    /// Media In (item 7) → Key → Merge over a Solid → Color → Out.
    fn sample() -> Comp {
        let mut c = Comp::new(1920, 1080, FrameRate { num: 24, den: 1 }, Tick(1_000));
        c.nodes.push(CompNode::new(2, CompKind::MediaIn { source: MediaInSource::Item { item: ItemId(7) }, offset: Tick::ZERO }));
        c.nodes.push(CompNode::new(3, CompKind::from_key("key").unwrap()));
        c.nodes.push(CompNode::new(4, CompKind::Solid));
        c.nodes.push(CompNode::new(5, CompKind::Merge));
        c.nodes.push(CompNode::new(6, CompKind::from_key("color").unwrap()));
        c.edges = vec![edge(2, 3, 0), edge(4, 5, 0), edge(3, 5, 1), edge(5, 6, 0), edge(6, 1, 0)];
        c
    }

    #[test]
    fn sample_validates_in_dependency_order() {
        let c = sample();
        let order: Vec<NodeId> = c.validate().unwrap().nodes().collect();
        assert_eq!(order, vec![2, 3, 4, 5, 6, 1]);
        assert_eq!(c.next_id(), 7);
        assert_eq!(c.sources().collect::<Vec<_>>(), vec![ItemId(7)]);
    }

    #[test]
    fn serde_round_trip_and_tags() {
        let c = sample();
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains(r#""type":"mediaIn""#), "{json}");
        assert!(json.contains(r#""kind":"item""#), "{json}");
        let back: Comp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn minimal_json_loads_with_defaults() {
        let c: Comp = serde_json::from_str(r#"{"width":640,"height":360}"#).unwrap();
        assert!(c.nodes.is_empty());
        assert_eq!(c.output, None);
        assert_eq!(c.validate(), Err(DagError::NoOutput));
        let n: CompNode = serde_json::from_str(r#"{"id":3,"kind":{"type":"merge"}}"#).unwrap();
        assert_eq!(n.kind, CompKind::Merge);
        assert_eq!(n.pos, [0.0, 0.0]);
    }

    #[test]
    fn every_kind_key_round_trips() {
        for &k in KINDS {
            let kind = CompKind::from_key(k).unwrap();
            assert_eq!(kind.key(), k);
            let json = serde_json::to_string(&CompNode::new(1, kind)).unwrap();
            let back: CompNode = serde_json::from_str(&json).unwrap();
            // Compared as JSON: text layers hold NaN "auto" points, which never compare equal.
            assert_eq!(serde_json::to_string(&back).unwrap(), json, "{k}");
        }
        assert_eq!(CompKind::from_key("nope"), None);
    }

    #[test]
    fn port_counts() {
        assert_eq!(CompKind::Merge.inputs(), 2);
        assert_eq!(CompKind::Solid.inputs(), 0);
        assert_eq!(CompKind::MediaOut.inputs(), 1);
        assert_eq!(CompNode::new(1, CompKind::Blur).outputs(), 1);
    }

    #[test]
    fn param_schemas_come_from_the_effects_they_reuse() {
        let ids = |k: &CompKind| params(k).into_iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids(&CompKind::Merge), vec!["opacity", "blend"]);
        let t = ids(&CompKind::Transform);
        assert!(t.contains(&"position") && t.contains(&"rotation") && !t.contains(&"anti_flicker"));
        assert!(ids(&CompKind::Blur).contains(&"blurriness"));
        assert!(!ids(&CompKind::from_key("key").unwrap()).is_empty());
        assert!(ids(&CompKind::Key { effect: "not_a_key".into() }).is_empty());
        assert_eq!(ids(&CompKind::Gradient), vec!["start", "end", "color_a", "color_b", "shape"]);
        let n = CompNode::new(1, CompKind::Merge);
        assert_eq!(n.value_at("opacity", Tick::ZERO), Some(ParamValue::Float(100.0)));
        assert_eq!(n.value_at("nope", Tick::ZERO), None);
    }

    #[test]
    fn sanitize_holds_the_caps() {
        let mut c = sample();
        c.width = 0;
        c.height = u32::MAX;
        c.rate = FrameRate { num: 0, den: -1 };
        c.duration = Tick(-5);
        let mut n = CompNode::new(50, CompKind::Mask { masks: vec![Mask::linear("m", Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)); MAX_WINDOWS + 5] });
        n.label = "x".repeat(1000);
        n.pos = [f32::NAN, f32::INFINITY];
        for i in 0..(MAX_PARAMS + 10) {
            n.params.insert(format!("p{i}"), Param::new(ParamValue::Text("y".repeat(MAX_TEXT + 10))));
        }
        c.nodes.push(n);
        c.nodes.push(CompNode::new(51, CompKind::MediaIn { source: MediaInSource::Layer { track: 1 }, offset: Tick(i64::MAX) }));
        for i in 0..(dag::MAX_NODES as u64) {
            c.nodes.push(CompNode::new(100 + i, CompKind::Solid));
        }
        c.edges.extend(std::iter::repeat_n(edge(4, 5, 0), dag::MAX_EDGES));
        c.sanitize();
        assert_eq!((c.width, c.height), (1, MAX_SIDE));
        assert!(u64::from(c.width) * u64::from(c.height) <= MAX_PIXELS);
        assert_eq!(c.rate, FrameRate::default());
        assert_eq!(c.duration, Tick::ZERO);
        assert_eq!(c.nodes.len(), dag::MAX_NODES);
        assert_eq!(c.edges.len(), dag::MAX_EDGES);
        let n = c.node(50).unwrap();
        assert_eq!(n.label.chars().count(), MAX_LABEL);
        assert_eq!(n.pos, [0.0, 0.0]);
        assert_eq!(n.params.len(), MAX_PARAMS);
        assert!(n.params.values().all(|p| matches!(&p.value, ParamValue::Text(s) if s.chars().count() == MAX_TEXT)));
        assert!(matches!(&n.kind, CompKind::Mask { masks } if masks.len() == MAX_WINDOWS));
        assert!(matches!(c.node(51).unwrap().kind, CompKind::MediaIn { offset, .. } if offset.0 < i64::MAX / 2));
        // The duplicate edges are left for validate to report, not dropped.
        assert!(matches!(c.validate(), Err(DagError::DuplicateInput(_))));

        let mut wide = Comp::new(MAX_SIDE, MAX_SIDE, FrameRate::default(), Tick::ZERO);
        wide.sanitize();
        assert!(u64::from(wide.width) * u64::from(wide.height) <= MAX_PIXELS);
    }

    #[test]
    fn hostile_json_never_panics() {
        for json in [
            r#"{"width":-1,"height":2}"#,
            r#"{"width":1,"height":1,"nodes":[{"id":1,"kind":{"type":"merge"},"pos":[1e40,0]}]}"#,
            r#"{"width":1,"height":1,"nodes":[{"id":1,"kind":{"type":"warp"}}]}"#,
            r#"{"width":1,"height":1,"edges":[{"from":{"node":1,"index":300},"to":{"node":1}}]}"#,
            r#"{"width":4,"height":4,"output":1,"nodes":[{"id":1,"kind":{"type":"mediaOut"}}],"edges":[{"from":{"node":1},"to":{"node":1}}]}"#,
        ] {
            if let Ok(mut c) = serde_json::from_str::<Comp>(json) {
                c.sanitize();
                let _ = c.validate();
            }
        }
    }
}
