//! Color Grading panels (fork GRADE-PLAN G1): **Nodes**, the selected clip's serial node strip,
//! and **Primaries**, the Lift / Gamma / Gain / Offset wheels, contrast / pivot / saturation /
//! temperature / tint and the per-node curves of the selected node.
//!
//! Every edit goes through the `grade.*` engine commands; drags use one `merge` key per press so a
//! drag is one undo step. Automation ids: `grade.node.<id>`, `grade.addNode`, `grade.insertNode`,
//! `grade.removeNode`, `grade.toggleNode`, `grade.resetNode`, `grade.bypass`, `grade.tab.<tab>`,
//! `grade.wheel.<key>`, `grade.param.<key>`, `grade.curve.<channel>`.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::grade::{Grade, GradeNode, MAX_NODES, NodeParamKind, node_param};
use filmcraft_project::{ClipId, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::widgets;

/// The first selected clip on a video track.
fn selected(app: &FilmcraftApp) -> Option<(ClipId, TrackItem)> {
    let seq = app.session.active_sequence()?;
    app.session.state.selection.iter().find_map(|c| {
        let (tid, it) = seq.find_item(*c)?;
        (seq.track(tid)?.kind == filmcraft_project::TrackKind::Video).then(|| (*c, it.clone()))
    })
}

/// The node the panels edit: the engine's current node (`grade.selectNode`) when it belongs to
/// this clip and still exists, else the first.
fn current_node(app: &FilmcraftApp, clip: ClipId, g: &Grade) -> Option<u64> {
    app.session.state.grade_node.filter(|sel| sel.clip == clip && g.node(sel.node).is_some()).map(|sel| sel.node).or_else(|| g.nodes.first().map(|n| n.id))
}

fn select_node(app: &mut FilmcraftApp, clip: ClipId, node: u64) {
    run(app, "grade.selectNode", json!({"clip": clip.0, "node": node}));
}

/// A label cut to `n` characters (with an ellipsis), for labels from hostile files.
fn short(text: &str, n: usize) -> String {
    if text.chars().count() <= n { text.to_string() } else { format!("{}…", text.chars().take(n.saturating_sub(1)).collect::<String>()) }
}

/// Run a command, showing a failure in the status line.
fn run(app: &mut FilmcraftApp, cmd: &str, p: Value) -> Option<Value> {
    match app.session.execute(cmd, p) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e.to_string();
            None
        }
    }
}

/// The Primaries panel's gesture number: bumped on every pointer press and key press, so a drag
/// is one undo step while separate clicks and key nudges are separate steps.
fn gesture(ui: &egui::Ui) -> u64 {
    let id = egui::Id::new("grade-gesture");
    let mut g: u64 = ui.data(|d| d.get_temp(id)).unwrap_or(0);
    if ui.input(|i| i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)))) {
        g = g.wrapping_add(1);
        ui.data_mut(|d| d.insert_temp(id, g));
    }
    g
}

// ---------- Nodes ----------

const NODE_W: f32 = 92.0;
const NODE_H: f32 = 46.0;
const NODE_GAP: f32 = 22.0;

pub fn show_nodes(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it)) = selected(app) else {
        crate::dock::placeholder(ui, rect, &t, "(select a video clip to grade)");
        return;
    };
    let grade = it.grade.clone().unwrap_or_default();
    let cur = current_node(app, clip, &grade);
    let mut cmds: Vec<(&'static str, Value)> = Vec::new();
    let mut select: Option<u64> = None;
    let mut autos: Vec<(String, Rect, String)> = Vec::new();

    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 4.0))).id_salt("grade-nodes"));
    bui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Nodes · {}", short(&it.name, 40))).color(t.text_dim));
        if grade.bypass {
            ui.label(egui::RichText::new("bypassed").color(t.text_faint).italics());
        }
    });
    // the buttons wrap in a narrow panel (the Color page puts the nodes in a side column)
    bui.horizontal_wrapped(|ui| {
        let mut button = |ui: &mut egui::Ui, id: &str, text: &str, tip: &str, enabled: bool| -> bool {
            let r = ui.add_enabled(enabled, egui::Button::new(text).small()).on_hover_text(tip);
            autos.push((id.to_string(), r.rect, tip.to_string()));
            r.clicked()
        };
        let has = cur.is_some();
        if button(ui, "grade.addNode", "+ Serial", "Add a serial node after the selected one", true) {
            cmds.push(("grade.addNode", json!({"clip": clip.0, "after": cur})));
        }
        if button(ui, "grade.insertNode", "+ Before", "Add a serial node before the selected one", true) {
            cmds.push(("grade.insertNode", json!({"clip": clip.0, "before": cur})));
        }
        if button(ui, "grade.removeNode", "Delete", "Delete the selected node", has) {
            cmds.push(("grade.removeNode", json!({"clip": clip.0, "node": cur})));
        }
        let enabled = cur.and_then(|id| grade.node(id)).is_none_or(|n| n.enabled);
        if button(ui, "grade.toggleNode", if enabled { "Disable" } else { "Enable" }, "Enable / disable the selected node", has) {
            cmds.push(("grade.toggleNode", json!({"clip": clip.0, "node": cur})));
        }
        if button(ui, "grade.resetNode", "Reset", "Reset the selected node", has) {
            cmds.push(("grade.resetNode", json!({"clip": clip.0, "node": cur})));
        }
        if button(ui, "grade.bypass", "Bypass", "Bypass the clip's grade", it.grade.is_some()) {
            cmds.push(("grade.bypass", json!({"clip": clip.0})));
        }
    });

    // the strip: input → nodes → output, scrolling sideways (a file may hold more nodes than fit;
    // only the first MAX_NODES render, the rest are listed as a count)
    let shown = grade.nodes.len().min(MAX_NODES);
    let strip_w = 40.0 + shown as f32 * (NODE_W + NODE_GAP) + NODE_GAP + 120.0;
    let strip_rect = Rect::from_min_max(pos2(rect.min.x, bui.cursor().min.y), rect.max);
    let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(strip_rect).id_salt(("grade-strip", clip.0)));
    egui::ScrollArea::horizontal().id_salt(("grade-strip-scroll", clip.0)).show(&mut sui, |ui| {
        let (area, _) = ui.allocate_exact_size(vec2(strip_w, NODE_H + 12.0), Sense::hover());
        let top = area.min.y + 4.0;
        let mut x = area.min.x + 12.0;
        let p = ui.painter_at(ui.clip_rect());
        let y_mid = top + NODE_H / 2.0;
        let dot = |x: f32| p.circle_filled(pos2(x, y_mid), 4.0, t.text_faint);
        dot(x);
        x += 8.0;
        if grade.nodes.is_empty() {
            p.text(pos2(x + NODE_GAP, y_mid), Align2::LEFT_CENTER, "No nodes: + Serial adds one", egui::FontId::proportional(12.0), t.text_faint);
        }
        for (i, n) in grade.nodes.iter().take(MAX_NODES).enumerate() {
            p.line_segment([pos2(x, y_mid), pos2(x + NODE_GAP, y_mid)], Stroke::new(1.5, t.text_faint));
            x += NODE_GAP;
            let r = Rect::from_min_size(pos2(x, top), vec2(NODE_W, NODE_H));
            let resp = ui.interact(r, egui::Id::new(("grade-node", clip.0, n.id)), Sense::click());
            let sel = cur == Some(n.id);
            let fill = if sel { t.accent.gamma_multiply(0.35) } else { Color32::from_gray(0x2a) };
            p.rect_filled(r, 4.0, fill);
            p.rect_stroke(r, 4.0, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { t.accent } else { t.text_faint }), egui::StrokeKind::Inside);
            let col = if n.enabled && !grade.bypass { t.text } else { t.text_faint };
            p.text(r.min + vec2(6.0, 4.0), Align2::LEFT_TOP, format!("{:02}", i + 1), egui::FontId::monospace(11.0), col);
            let name = if n.label.is_empty() { summary(n) } else { short(&n.label, 13) };
            p.text(r.left_bottom() + vec2(6.0, -5.0), Align2::LEFT_BOTTOM, name, egui::FontId::proportional(11.0), col);
            if !n.enabled {
                p.line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.0, t.text_faint));
            }
            if resp.clicked() {
                select = Some(n.id);
            }
            if resp.double_clicked() {
                cmds.push(("grade.toggleNode", json!({"clip": clip.0, "node": n.id})));
            }
            if ui.clip_rect().intersects(r) {
                autos.push((format!("grade.node.{}", n.id), r.intersect(ui.clip_rect()), format!("Node {:02}", i + 1)));
            }
            x += NODE_W;
        }
        p.line_segment([pos2(x, y_mid), pos2(x + NODE_GAP, y_mid)], Stroke::new(1.5, t.text_faint));
        dot(x + NODE_GAP + 4.0);
        if grade.nodes.len() > MAX_NODES {
            let more = grade.nodes.len() - MAX_NODES;
            p.text(
                pos2(x + NODE_GAP + 14.0, y_mid),
                Align2::LEFT_CENTER,
                format!("+{more} more (not rendered)"),
                egui::FontId::proportional(11.0),
                t.text_faint,
            );
        }
    });

    for (id, r, label) in autos {
        if r.is_positive() {
            app.auto.add(&id, r, &label);
        }
    }
    if let Some(id) = select {
        select_node(app, clip, id);
    }
    for (cmd, p) in cmds {
        if let Some(v) = run(app, cmd, p)
            && let Some(id) = v.get("node").and_then(Value::as_u64)
        {
            select_node(app, clip, id);
        }
    }
}

/// A short description of what a node changes (shown when it has no label).
fn summary(n: &GradeNode) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ["lift", "gamma", "gain", "offset", "contrast", "saturation", "temperature", "tint"].iter().any(|k| n.params.contains_key(*k)) {
        parts.push("Primaries");
    }
    if n.params.keys().any(|k| k.starts_with("curve.")) {
        parts.push("Curves");
    }
    if n.float_at("grain.strength", Tick::ZERO) > 0.0 || n.params.get("grain.strength").is_some_and(|p| p.is_animated()) {
        parts.push("Grain");
    }
    if parts.is_empty() && n.params.contains_key("pivot") {
        parts.push("Primaries");
    }
    if parts.is_empty() { "Empty".into() } else { parts.join(" + ") }
}

// ---------- Primaries / Curves ----------

/// One wheel: node key, title, colour scale (at the rim the spread between the hue and its
/// opposite equals it), full-scale master offset.
const WHEELS: [(&str, &str, f32, f32); 4] =
    [("lift", "Lift", 0.25, 0.5), ("gamma", "Gamma", 0.25, 0.5), ("gain", "Gain", 0.5, 1.0), ("offset", "Offset", 0.25, 0.5)];

/// Number sliders below the wheels; `""` draws a separator (the film grain group follows).
const SLIDERS: [&str; 9] = ["contrast", "pivot", "saturation", "temperature", "tint", "", "grain.strength", "grain.size", "grain.softness"];

const CURVES: [(&str, &str); 4] = [("curve.master", "Y"), ("curve.r", "R"), ("curve.g", "G"), ("curve.b", "B")];

/// Hue-ring angles of red, green and blue on [`widgets::color::color_wheel`] (hue = 1 − angle / τ,
/// counter-clockwise from +x): red 0°, green 240°, blue 120°.
const RGB_ANGLES: [f32; 3] = [0.0, 240.0, 120.0];

/// Colour offsets `[r, g, b]` (mean kept) → puck position (−1…1) for a wheel of full scale `scale`.
pub(crate) fn puck_of(rgb: [f32; 3], scale: f32) -> [f32; 2] {
    if scale <= 0.0 {
        return [0.0, 0.0];
    }
    let (mut x, mut y) = (0.0, 0.0);
    for (c, a) in rgb.iter().zip(RGB_ANGLES) {
        let (s, co) = a.to_radians().sin_cos();
        x += c * co;
        y += c * s;
    }
    let (x, y) = (x / scale, y / scale);
    let len = (x * x + y * y).sqrt();
    if len > 1.0 { [x / len, y / len] } else { [x, y] }
}

/// Puck position → colour offsets `[r, g, b]` (summing to 0) for a wheel of full scale `scale`.
pub(crate) fn rgb_of(puck: [f32; 2], scale: f32) -> [f32; 3] {
    RGB_ANGLES.map(|a| {
        let (s, co) = a.to_radians().sin_cos();
        2.0 / 3.0 * scale * (puck[0] * co + puck[1] * s)
    })
}

pub fn show_primaries(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it)) = selected(app) else {
        crate::dock::placeholder(ui, rect, &t, "(select a video clip to grade)");
        return;
    };
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 6.0))).id_salt("grade-primaries"));
    let Some(grade) = it.grade.clone().filter(|g| !g.nodes.is_empty()) else {
        bui.add_space(8.0);
        bui.label(egui::RichText::new(format!("{} has no grade", short(&it.name, 40))).color(t.text_dim));
        bui.add_space(6.0);
        let r = bui.button("Add Node");
        app.auto.add("grade.addFirst", r.rect, "Add Node");
        if r.clicked()
            && let Some(id) = run(app, "grade.addNode", json!({"clip": clip.0})).and_then(|v| v.get("node").and_then(Value::as_u64))
        {
            select_node(app, clip, id);
        }
        return;
    };
    let Some(node_id) = current_node(app, clip, &grade) else { return };
    let Some(node) = grade.node(node_id).cloned() else { return };
    let index = grade.nodes.iter().position(|n| n.id == node_id).unwrap_or(0);
    let ph = app.session.playhead();
    let mt = it.effect_time_at(ph.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
    let mut sets: Vec<Value> = Vec::new();
    let mut autos: Vec<(String, Rect, String)> = Vec::new();
    let gest = gesture(&bui);
    let key_of = |what: &str| format!("{node_id}:{what}#{gest}");
    let mut tab = if app.ui.grade_tab == "curves" { "curves" } else { "primaries" };

    bui.horizontal(|ui| {
        for (id, label) in [("primaries", "Primaries"), ("curves", "Curves")] {
            let r = ui.selectable_label(tab == id, label);
            autos.push((format!("grade.tab.{id}"), r.rect, label.to_string()));
            if r.clicked() {
                tab = id;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let name = if node.label.is_empty() { String::new() } else { format!(" · {}", short(&node.label, 24)) };
            ui.label(egui::RichText::new(format!("Node {:02}{name}", index + 1)).color(t.text_dim));
        });
    });
    bui.add_space(4.0);
    let base = json!({"clip": clip.0, "node": node_id});
    let set = |key: &str, value: Value, merge: Option<String>| {
        let mut p = base.clone();
        p["param"] = json!(key);
        p["value"] = value;
        if let Some(m) = merge {
            p["merge"] = json!(m);
        }
        p
    };

    // the controls scroll when the panel is short (the Color page puts it below the viewer)
    let scroll_rect = Rect::from_min_max(bui.cursor().min, bui.max_rect().max);
    let mut sui = bui.new_child(egui::UiBuilder::new().max_rect(scroll_rect).id_salt("grade-primaries-scroll"));
    egui::ScrollArea::vertical().id_salt(("grade-primaries", tab)).auto_shrink([false, false]).show(&mut sui, |bui| {
        if tab == "primaries" {
            // four wheels in a row when the panel is wide (Resolve's Color page), else two by two
            let cols = if bui.available_width() >= 4.0 * 130.0 { 4 } else { 2 };
            let size = ((bui.available_width() - 12.0 * (cols as f32 - 1.0)) / cols as f32).clamp(90.0, 170.0);
            for row in WHEELS.chunks(cols) {
                bui.horizontal(|ui| {
                    for &(key, title, scale, master_scale) in row {
                        ui.vertical(|ui| {
                            ui.set_width(size);
                            let w = node.wheel_at(key, mt);
                            ui.label(egui::RichText::new(title).color(t.text_dim));
                            let puck = puck_of([w[0], w[1], w[2]], scale);
                            let light = (w[3] / master_scale * 100.0).clamp(-100.0, 100.0);
                            let wheel_rect = Rect::from_min_size(ui.cursor().min, vec2(size, size));
                            let ch = widgets::color::color_wheel(ui, puck, light, size, &t, egui::Id::new(("grade-wl", clip.0, node_id, key)));
                            autos.push((format!("grade.wheel.{key}"), wheel_rect, title.to_string()));
                            let mean = (w[0] + w[1] + w[2]) / 3.0;
                            if let Some(p) = ch.puck {
                                let c = rgb_of(p, scale);
                                let v = [mean + c[0], mean + c[1], mean + c[2], w[3]];
                                sets.push(set(key, json!(v), Some(key_of(key))));
                            }
                            if let Some(l) = ch.lightness {
                                sets.push(set(key, json!([w[0], w[1], w[2], l / 100.0 * master_scale]), Some(key_of(key))));
                            }
                            ui.label(
                                egui::RichText::new(format!("{:+.2} {:+.2} {:+.2}  {:+.2}", w[0], w[1], w[2], w[3])).monospace().small().color(t.text_faint),
                            );
                        });
                    }
                });
                bui.add_space(4.0);
            }
            bui.separator();
            for key in SLIDERS {
                if key.is_empty() {
                    bui.add_space(2.0);
                    bui.label(egui::RichText::new("Film grain").color(t.text_dim));
                    continue;
                }
                let Some(def) = node_param(key) else { continue };
                let NodeParamKind::Float { min, max, .. } = def.kind else { continue };
                let mut v = node.float_at(key, mt);
                bui.horizontal(|ui| {
                    ui.add_sized(vec2(84.0, 18.0), egui::Label::new(egui::RichText::new(def.label).color(t.text_dim)));
                    let r = ui.add(egui::Slider::new(&mut v, min..=max).max_decimals(3));
                    autos.push((format!("grade.param.{key}"), r.rect, def.label.to_string()));
                    if r.changed() {
                        sets.push(set(key, json!(v), Some(key_of(key))));
                    }
                    let reset = ui.small_button("↺").on_hover_text(format!("Reset {}", def.label));
                    autos.push((format!("grade.param.{key}.reset"), reset.rect, format!("Reset {}", def.label)));
                    if reset.clicked() {
                        sets.push(json!({"reset": key}));
                    }
                });
            }
        } else {
            let key = egui::Id::new("grade-curve-channel");
            let mut ch: usize = bui.data(|d| d.get_temp(key)).unwrap_or(0);
            bui.horizontal(|ui| {
                for (i, (k, label)) in CURVES.iter().enumerate() {
                    let r = ui.selectable_label(ch == i, *label);
                    autos.push((format!("grade.curve.{}", k.trim_start_matches("curve.")), r.rect, format!("{label} curve")));
                    if r.clicked() {
                        ch = i;
                    }
                }
            });
            bui.data_mut(|d| d.insert_temp(key, ch));
            let (k, _) = CURVES.get(ch).copied().unwrap_or(CURVES[0]);
            let col = match ch {
                1 => Color32::from_rgb(0xe0, 0x50, 0x50),
                2 => Color32::from_rgb(0x50, 0xc0, 0x50),
                3 => Color32::from_rgb(0x50, 0x80, 0xf0),
                _ => Color32::from_gray(0xd0),
            };
            let pts = node.curve(k).unwrap_or_else(|| vec![[0.0, 0.0], [1.0, 1.0]]);
            if let Some(new_pts) = widgets::color::curve_editor(bui, &pts, false, col, &t, egui::Id::new(("grade-curve", clip.0, node_id, k))) {
                let v: Vec<Value> = new_pts.iter().map(|q| json!([q[0], q[1]])).collect();
                sets.push(set(k, json!(v), Some(key_of(k))));
            }
            let r = bui.small_button("Reset curve");
            autos.push(("grade.curve.reset".into(), r.rect, "Reset curve".into()));
            if r.clicked() {
                sets.push(json!({"reset": k}));
            }
        }
    });

    for (id, r, label) in autos {
        if r.is_positive() {
            app.auto.add(&id, r, &label);
        }
    }
    app.ui.grade_tab = tab.to_string();
    for p in sets {
        if let Some(k) = p.get("reset").and_then(Value::as_str) {
            run(app, "grade.resetNode", json!({"clip": clip.0, "node": node_id, "param": k}));
        } else {
            run(app, "grade.setParam", p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puck_and_rgb_round_trip() {
        for p in [[0.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.3, 0.4], [-0.6, 0.2]] {
            let rgb = rgb_of(p, 0.25);
            assert!((rgb.iter().sum::<f32>()).abs() < 1e-6, "zero-sum");
            let back = puck_of(rgb, 0.25);
            assert!((back[0] - p[0]).abs() < 1e-5 && (back[1] - p[1]).abs() < 1e-5, "{p:?} → {back:?}");
        }
        // red is at the wheel's 0° (hue 0)
        let r = rgb_of([1.0, 0.0], 0.3);
        assert!(r[0] > 0.0 && r[1] < 0.0 && r[2] < 0.0);
        // a neutral offset has no puck; scale 0 and huge values stay inside the wheel
        let n = puck_of([0.2, 0.2, 0.2], 0.25);
        assert!(n[0].abs() < 1e-6 && n[1].abs() < 1e-6, "{n:?}");
        assert_eq!(puck_of([1.0, 0.0, 0.0], 0.0), [0.0, 0.0]);
        let a = puck_of([0.3, 0.1, 0.1], 0.25);
        let b = puck_of([0.2, 0.0, 0.0], 0.25);
        assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "the neutral part has no puck");
        assert_eq!(short("abcdef", 4), "abc…");
        assert_eq!(short("abc", 4), "abc");
        let far = puck_of([100.0, -50.0, 0.0], 0.25);
        assert!((far[0] * far[0] + far[1] * far[1]).sqrt() <= 1.0 + 1e-5);
    }
}
