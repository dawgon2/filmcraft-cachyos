//! The comp-graph explainer (fork COMPOSITOR-PLAN C1 item 7): what a comp does, node by node, in
//! plain words, in the order the pictures flow. Template sentences built only from the graph
//! itself (node names, real parameter labels and values that differ from their defaults), so they
//! can't name a control or a node that isn't there. `comp.explain` returns them; a local model
//! may later reword them under the same check as the grade explainer (G2.11).

use filmcraft_project::comp::{self, Comp, CompKind, CompNode, MediaInSource};
use filmcraft_project::dag::NodeId;
use filmcraft_project::effect::{ParamKind, find_effect};
use filmcraft_project::{ParamValue, Project};
use filmcraft_time::{TICKS_PER_SECOND, Tick};

/// Longest node label quoted in a sentence, in characters.
const MAX_LABEL: usize = 60;
/// Most changed parameters listed per node.
const MAX_PARAMS_LISTED: usize = 4;

/// One node's sentence.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub node: NodeId,
    /// The node's name as the canvas shows it (its label, else kind and id).
    pub name: String,
    pub text: String,
}

/// The explanation: a one-line summary and the nodes in evaluation order, then the nodes that
/// don't reach Media Out.
#[derive(Clone, Debug, PartialEq)]
pub struct Explanation {
    pub summary: String,
    pub lines: Vec<Line>,
    pub unused: Vec<Line>,
}

/// The on-screen name of a node kind (as the Compositor's canvas and Add Node menu show it).
pub fn kind_label(key: &str) -> &'static str {
    match key {
        "mediaIn" => "Media In",
        "mediaOut" => "Media Out",
        "merge" => "Merge",
        "transform" => "Transform",
        "key" => "Key",
        "mask" => "Mask",
        "blur" => "Blur",
        "color" => "Color",
        "text" => "Text",
        "solid" => "Solid",
        "gradient" => "Gradient",
        _ => "Node",
    }
}

/// A node's name: its label (cut to [`MAX_LABEL`] characters), else "Kind id".
pub fn node_name(n: &CompNode) -> String {
    let label: String = n.label.chars().filter(|c| !c.is_control()).take(MAX_LABEL).collect();
    if label.trim().is_empty() { format!("{} {}", kind_label(n.kind.key()), n.id) } else { label.trim().to_string() }
}

/// Explains `c` at comp time `t`.
pub fn explain(c: &Comp, project: &Project, t: Tick) -> Explanation {
    let name_of = |id: NodeId| c.node(id).map_or_else(|| format!("node {id}"), node_name);
    let input = |id: NodeId, port: u8| c.edges.iter().find(|e| e.to.node == id && e.to.index == port).map(|e| name_of(e.from.node));
    let order = match c.validate() {
        Ok(o) => o.nodes().collect::<Vec<_>>(),
        Err(e) => {
            return Explanation {
                summary: format!("This comp can't be shown: {e}. It renders transparent until that is fixed."),
                lines: Vec::new(),
                unused: Vec::new(),
            };
        }
    };
    let line = |n: &CompNode| Line { node: n.id, name: node_name(n), text: sentence(n, project, t, &input) };
    let lines: Vec<Line> = order.iter().filter_map(|id| c.node(*id)).map(line).collect();
    let unused: Vec<Line> = c
        .nodes
        .iter()
        .filter(|n| !order.contains(&n.id))
        .map(|n| Line { node: n.id, name: node_name(n), text: "isn't connected to Media Out, so it changes nothing.".to_string() })
        .collect();
    let summary = match c.output.and_then(|o| c.node(o)) {
        None => "This comp has no Media Out node, so it is transparent.".to_string(),
        Some(out) => match input(out.id, 0) {
            None => format!("{} has no input, so the comp is transparent.", node_name(out)),
            Some(_) => {
                let sources = lines.iter().filter(|l| c.node(l.node).is_some_and(|n| n.kind.inputs() == 0)).count();
                format!("{} node(s) build the picture from {} source(s); the result is what {} shows.", lines.len(), sources, node_name(out))
            }
        },
    };
    Explanation { summary, lines, unused }
}

fn sentence(n: &CompNode, project: &Project, t: Tick, input: &dyn Fn(NodeId, u8) -> Option<String>) -> String {
    let a = || input(n.id, 0).unwrap_or_else(|| "nothing (no input connected)".to_string());
    if n.bypass && n.kind.inputs() > 0 {
        return format!("is bypassed: {} passes through unchanged.", a());
    }
    let changed = changed_params(n, t);
    let with = if changed.is_empty() { String::new() } else { format!(" ({})", changed.join(", ")) };
    match &n.kind {
        CompKind::MediaIn { source, offset } => {
            let from = match source {
                MediaInSource::Item { item } => match project.item(*item) {
                    Some(i) => format!("the clip \u{201c}{}\u{201d}", i.name.chars().filter(|c| !c.is_control()).take(MAX_LABEL).collect::<String>()),
                    None => "a clip that is no longer in the project (it shows nothing)".to_string(),
                },
                MediaInSource::Layer { track } => format!("track V{} under the comp clip", track.saturating_add(1)),
            };
            let later = if offset.0 != 0 { format!(", {:.2} s later than the comp", offset.0 as f64 / TICKS_PER_SECOND as f64) } else { String::new() };
            format!("reads {from}{later}.")
        }
        CompKind::MediaOut => match input(n.id, 0) {
            Some(i) => format!("is the comp's result: it shows {i}."),
            None => "has no input, so the comp is transparent.".to_string(),
        },
        CompKind::Merge => {
            let fg = input(n.id, 1).unwrap_or_else(|| "nothing".to_string());
            let bg = input(n.id, 0).unwrap_or_else(|| "transparency".to_string());
            format!("puts {fg} (foreground) over {bg} (background){with}.")
        }
        CompKind::Transform => {
            if changed.is_empty() {
                format!("passes {} through unchanged (every setting at its default).", a())
            } else {
                format!("moves, scales or turns {}{with}.", a())
            }
        }
        CompKind::Key { effect } => {
            let name = find_effect(effect).map_or("a key", |d| d.name);
            format!("makes part of {} transparent with {name}{with}.", a())
        }
        CompKind::Mask { masks } => format!("keeps only the parts of {} inside its {} mask(s).", a(), masks.len()),
        CompKind::Blur => format!("blurs {}{with}.", a()),
        CompKind::Color { .. } => format!("grades {} with one colour node (open it to see its corrections).", a()),
        CompKind::Text { .. } => "draws a text layer.".to_string(),
        CompKind::Solid => format!("is a flat colour{with}."),
        CompKind::Gradient => format!("is a gradient between two colours{with}."),
    }
}

/// "Label value" for the parameters that differ from their defaults at `t` (at most
/// [`MAX_PARAMS_LISTED`]; colours aren't listed).
fn changed_params(n: &CompNode, t: Tick) -> Vec<String> {
    let mut out = Vec::new();
    for d in comp::params(&n.kind) {
        if out.len() >= MAX_PARAMS_LISTED {
            break;
        }
        let Some(v) = n.value_at(d.id, t) else { continue };
        if v == d.default {
            continue;
        }
        let text = match (&v, &d.kind) {
            (ParamValue::Float(f), ParamKind::Float { unit, decimals, .. }) if f.is_finite() => {
                format!("{} {:.*}{}", d.label, usize::from((*decimals).min(3)), f, unit)
            }
            (ParamValue::Float(f), ParamKind::Angle) if f.is_finite() => format!("{} {f:.1}°", d.label),
            (ParamValue::Choice(i), ParamKind::Choice(names)) => match names.get(*i as usize) {
                Some(name) => format!("{} {name}", d.label),
                None => continue,
            },
            (ParamValue::Bool(b), _) => format!("{} {}", d.label, if *b { "on" } else { "off" }),
            (ParamValue::Vec2(p), _) if p.x.is_finite() && p.y.is_finite() => format!("{} {:.0}, {:.0}", d.label, p.x, p.y),
            _ => continue,
        };
        out.push(text);
    }
    out
}
