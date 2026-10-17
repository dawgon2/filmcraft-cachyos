//! Grade comparison, Shot Match and the node explainer (fork GRADE-PLAN G2.8–G2.10): engine
//! commands over `filmcraft_render::grade_match` and `filmcraft_render::grade_explain`.
//!
//! - `grade.compareData`: which clips carry the same grade (by their data: ids and labels
//!   ignored), so a pasted grade and an accidental tweak are told apart.
//! - `grade.compareShots`: do two shots *look* matched? Both graded frames are measured and
//!   compared; the answer is a score and plain-language differences. A guide, not a verdict.
//! - `grade.checkScene`: the shot that differs most from the rest of a scene.
//! - `grade.shotMatch`: append a node whose primaries match the clip to a reference (one undo
//!   step).
//! - `grade.explainNode` / `grade.explainGrade`: measured facts and a template sentence per node;
//!   `grade.explainNode` also routes through the optional local AI (`crate::ai_explain`, G2.11).
//!
//! Frames are rendered at ¼ size in working linear light; the qualifier's highlight / matte view
//! is switched off for every measurement.

use filmcraft_project::grade::{Grade, GradeNode, MAX_NODES};
use filmcraft_project::{ClipId, ItemId};
use filmcraft_render::Image;
use filmcraft_render::grade_match::{self, ShotComparison, ShotStats};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, clip_p, has_seq, time_p};
use crate::grade::{current_for, edit_grade, is_video, media_time, node_arg, pick, target};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn query(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}

/// The most clips a scene check measures (each is a render: the command runs synchronously).
const MAX_SCENE: usize = 64;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        query("grade.compareData", "Compare Grade Data", r#"{"clips":[id]?}"#, compare_data),
        query("grade.compareShots", "Compare Shots", r#"{"clip":id?,"time":ticks?,"with":id?,"withTime":ticks?,"referenceTime":ticks?}"#, compare_shots),
        query("grade.checkScene", "Check Scene", r#"{"clips":[id]?}"#, check_scene),
        CommandSpec {
            id: "grade.shotMatch",
            label: "Shot Match",
            menu: &["Clip", "Grade"],
            shortcut: None,
            params: r#"{"clip":id?,"with":id?,"withTime":ticks?,"referenceTime":ticks?,"faceDetection":bool?}"#,
            enabled: has_seq,
            run: shot_match,
            journal: true,
        },
        query(
            "grade.explainNode",
            "Explain Node",
            r#"{"clip":id?,"node":id?,"time":ticks?,"overlay":path?,"wait":bool? (AI text: blocks at most 20 s),"prefetch":bool? (start the AI text in the background only)}"#,
            explain_node,
        ),
        query("grade.explainGrade", "Explain Grade", r#"{"clip":id?,"time":ticks?}"#, explain_grade),
    ]
}

fn seq_id(s: &Session) -> Result<ItemId> {
    s.state.active_sequence.ok_or(EngineError::NoSequence)
}

/// The sequence time a clip is measured at: the time parameter with prefix `key` (`time` for "",
/// `withTime` for "with") clamped into the clip,
/// else the playhead when it is over the clip, else the clip's middle frame.
fn clip_time(s: &Session, p: &Value, key: &str, clip: ClipId, cmd: &str) -> Result<Tick> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(cmd, "no such clip"))?;
    let last = (it.end() - Tick(1)).max(it.start);
    let want = time_p(s, p, key).or_else(|| Some(s.playhead()).filter(|t| it.range().contains(*t)));
    Ok(want.unwrap_or(Tick(it.start.0.saturating_add((last.0.saturating_sub(it.start.0)) / 2))).clamp(it.start, last))
}

/// What of a clip a measurement renders.
#[derive(Clone, Copy)]
struct Probe {
    /// Keep the first this many grade nodes (None: all).
    nodes: Option<usize>,
    /// Keep the effect stack after the grade.
    effects: bool,
}

/// The clip's layer at sequence time `t` as `probe` says, at ¼ size in working linear light.
fn clip_frame(s: &Session, clip: ClipId, t: Tick, probe: Probe, cmd: &str) -> Result<Image> {
    let seq = seq_id(s)?;
    let mut pr = (*s.project).clone();
    if let Some((_, pi)) = pr.sequence_mut(seq).and_then(|q| q.find_item_mut(clip)) {
        if let Some(g) = pi.grade.as_mut() {
            if let Some(n) = probe.nodes {
                g.nodes.truncate(n);
            }
            // (the qualifier's view replaces the picture: never measure it)
            for n in &mut g.nodes {
                n.params.remove("qual.show");
            }
        }
        if !probe.effects {
            // (a graphic's layers are its picture, not effects after the grade)
            for e in pi.effects.iter_mut().filter(|e| !e.def().is_some_and(|d| d.intrinsic) && !filmcraft_project::graphic::is_layer(e)) {
                e.enabled = false;
            }
        }
    }
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let opts = filmcraft_render::RenderOptions { scale: 0.25, working_output: true, ..Default::default() };
    filmcraft_render::render_clip(&pr, seq, clip, t, opts, &provider).ok_or_else(|| bad(cmd, "the clip has no picture"))
}

fn working(s: &Session) -> Result<filmcraft_color::WorkingSpace> {
    Ok(s.active_sequence().ok_or(EngineError::NoSequence)?.settings.color.working)
}

fn measure(img: &Image, s: &Session, cmd: &str) -> Result<ShotStats> {
    ShotStats::measure_in(img, working(s)?).ok_or_else(|| bad(cmd, "nothing to measure in the frame"))
}

fn comparison_json(c: &ShotComparison) -> Value {
    let ranges: Vec<Value> = ["shadows", "midtones", "highlights"]
        .iter()
        .zip(&c.per_range)
        .map(|(name, r)| {
            json!({
                "range": name,
                "populated": r.populated,
                "shared": r.shared,
                "exposureStops": r.exposure_stops,
                "warmth": r.warmth,
                "tint": r.tint,
            })
        })
        .collect();
    json!({
        "score": c.score,
        "exposureStops": c.exposure_stops,
        "warmth": c.warmth,
        "tint": c.tint,
        "saturationPct": c.saturation_pct,
        "overlap": c.overlap,
        "ranges": ranges,
        "notes": c.notes,
    })
}

/// Video clips of the active sequence to look at: `clips`, else (`all`) every video clip, else
/// the selected video clips.
fn clips_for(s: &Session, p: &Value, all: bool, cmd: &str) -> Result<Vec<ClipId>> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if let Some(a) = p.get("clips").and_then(Value::as_array) {
        let mut v = Vec::new();
        for c in a {
            let c = c.as_u64().map(ClipId).ok_or_else(|| bad(cmd, "`clips` must be clip ids"))?;
            if !is_video(q, c) {
                return Err(bad(cmd, format!("clip {} is not on a video track", c.0)));
            }
            v.push(c);
        }
        return Ok(v);
    }
    if all {
        return Ok(q.video_tracks.iter().flat_map(|t| t.items.iter().map(|i| i.id)).collect());
    }
    Ok(s.state.selection.iter().copied().filter(|c| is_video(q, *c)).collect())
}

fn compare_data(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "grade.compareData";
    let clips = clips_for(s, p, true, CMD)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let none = grade_match::grade_fingerprint(&Grade::default());
    let mut by: std::collections::BTreeMap<u64, Vec<u64>> = std::collections::BTreeMap::new();
    let mut each = serde_json::Map::new();
    for c in clips {
        let Some((_, it)) = q.find_item(c) else { continue };
        let f = it.grade.as_ref().map_or(none, grade_match::grade_fingerprint);
        each.insert(c.0.to_string(), json!(format!("{f:016x}")));
        if f != none {
            by.entry(f).or_default().push(c.0);
        }
    }
    let groups: Vec<Value> = by.into_iter().filter(|(_, v)| v.len() > 1).map(|(f, v)| json!({"fingerprint": format!("{f:016x}"), "clips": v})).collect();
    Ok(json!({"groups": groups, "clips": each}))
}

/// The reference frame of a comparison or a match: `with` (a clip, as it finally looks) or
/// `referenceTime` (the sequence frame there).
fn reference_frame(s: &Session, p: &Value, clip: ClipId, cmd: &str) -> Result<Image> {
    if p.get("with").is_some() {
        let w = clip_p(p, "with").ok_or_else(|| bad(cmd, "`with` must be a clip id"))?;
        if w == clip {
            return Err(bad(cmd, "compare with another clip (or pass `referenceTime`)"));
        }
        let t = clip_time(s, p, "with", w, cmd)?;
        return clip_frame(s, w, t, Probe { nodes: None, effects: true }, cmd);
    }
    let rt = time_p(s, p, "reference").ok_or_else(|| bad(cmd, "need `with` (a clip) or `referenceTime`"))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if q.find_item(clip).is_some_and(|(_, it)| it.range().contains(rt)) {
        return Err(bad(cmd, "the reference frame is inside the clip; pick a frame from another shot"));
    }
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let opts = filmcraft_render::RenderOptions { scale: 0.25, working_output: true, ..Default::default() };
    Ok(filmcraft_render::render_sequence(&s.project, seq_id(s)?, rt, opts, &provider))
}

fn compare_shots(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "grade.compareShots";
    let clip = target(s, p, CMD)?;
    let t = clip_time(s, p, "", clip, CMD)?;
    let a = measure(&reference_frame(s, p, clip, CMD)?, s, CMD)?;
    let b = measure(&clip_frame(s, clip, t, Probe { nodes: None, effects: true }, CMD)?, s, CMD)?;
    let mut v = comparison_json(&grade_match::compare(&a, &b));
    v["clip"] = json!(clip.0);
    Ok(v)
}

fn check_scene(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "grade.checkScene";
    let clips = clips_for(s, p, false, CMD)?;
    if clips.len() < 2 {
        return Err(bad(CMD, "select at least two video clips (or pass `clips`)"));
    }
    let truncated = clips.len() > MAX_SCENE;
    let mut stats = Vec::new();
    let mut measured = Vec::new();
    for c in clips.into_iter().take(MAX_SCENE) {
        let t = clip_time(s, &json!({}), "", c, CMD)?;
        let Ok(img) = clip_frame(s, c, t, Probe { nodes: None, effects: true }, CMD) else { continue };
        if let Some(st) = ShotStats::measure_in(&img, working(s)?) {
            stats.push(st);
            measured.push(c);
        }
    }
    let consensus = grade_match::consensus(&stats).ok_or_else(|| bad(CMD, "no clip could be measured"))?;
    let shots: Vec<Value> = grade_match::outliers(&stats)
        .into_iter()
        .filter_map(|(i, d)| {
            let (c, st) = (measured.get(i)?, stats.get(i)?);
            let cmp = grade_match::compare(&consensus, st);
            Some(json!({"clip": c.0, "difference": d, "notes": cmp.notes, "comparison": comparison_json(&cmp)}))
        })
        .collect();
    Ok(json!({"shots": shots, "truncated": truncated}))
}

fn shot_match(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "grade.shotMatch";
    let clip = target(s, p, CMD)?;
    let t = clip_time(s, p, "", clip, CMD)?;
    let reference = reference_frame(s, p, clip, CMD)?;
    // the new node goes at the end of the grade, before the effects
    let current = clip_frame(s, clip, t, Probe { nodes: None, effects: false }, CMD)?;
    let skin = bool_p(p, "faceDetection").unwrap_or(true);
    let m = grade_match::match_node(&current, &reference, working(s)?, skin);
    let (before, after) = (m.before, m.after);
    let id = edit_grade(s, &json!({"clip": clip.0}), CMD, "Shot Match", true, move |g| {
        if g.nodes.len() >= MAX_NODES {
            return Err(bad(CMD, format!("a grade holds at most {MAX_NODES} nodes")));
        }
        let mut n = m.node(g.next_id());
        n.label = "Shot Match".into();
        let id = n.id;
        g.nodes.push(n);
        Ok(id)
    })?;
    s.state.grade_node = Some(crate::GradeSel { clip, node: id });
    // (before / after: the solver's residual, lower is closer to the reference)
    Ok(json!({"clip": clip.0, "node": id, "before": before, "after": after}))
}

/// One node's facts: its input (the nodes before it) and output measured at `t`.
fn node_facts(
    s: &Session,
    clip: ClipId,
    t: Tick,
    mt: Tick,
    pos: usize,
    node: &GradeNode,
    cmd: &str,
) -> Result<(filmcraft_render::grade_explain::NodeFacts, Image, Image)> {
    let input = clip_frame(s, clip, t, Probe { nodes: Some(pos), effects: false }, cmd)?;
    let output = clip_frame(s, clip, t, Probe { nodes: Some(pos + 1), effects: false }, cmd)?;
    let facts = filmcraft_render::grade_explain::explain_node(node, mt, &input, &output, working(s)?);
    Ok((facts, input, output))
}

fn node_label(n: &GradeNode, pos: usize) -> String {
    if n.label.is_empty() { format!("Node {}", pos + 1) } else { n.label.clone() }
}

fn explain_node(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "grade.explainNode";
    let want = node_arg(p, CMD)?;
    let (clip, current) = current_for(s, p, CMD)?;
    let t = clip_time(s, p, "", clip, CMD)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(CMD, "no such clip"))?;
    let g = it.grade.clone().ok_or_else(|| bad(CMD, "the clip has no grade"))?;
    let id = pick(&g, want, current, CMD)?;
    let pos = g.nodes.iter().position(|n| n.id == id).ok_or_else(|| bad(CMD, format!("no node {id}")))?;
    let node = g.nodes.get(pos).cloned().ok_or_else(|| bad(CMD, format!("no node {id}")))?;
    let mt = media_time(s, &json!({"time": t.0}), clip);
    let (facts, input, output) = node_facts(s, clip, t, mt, pos, &node, CMD)?;
    let label = node_label(&node, pos);
    let sentence = facts.sentence(&label);
    // (G2.11: the text to show: a checked local-AI rewrite when enabled and ready, else the
    // template; `aiKey` while the model is still writing, poll `ai.explanation`)
    let ai = crate::ai_explain::explain_fields(
        s,
        &facts,
        &node.label,
        pos + 1,
        &label,
        &sentence,
        bool_p(p, "wait").unwrap_or(false),
        bool_p(p, "prefetch").unwrap_or(false),
    );
    let mut v = json!({
        "clip": clip.0,
        "node": id,
        "sentence": sentence,
        "summary": facts.summary(),
        "facts": serde_json::to_value(&facts).unwrap_or(Value::Null),
    });
    if let (Some(o), Value::Object(extra)) = (v.as_object_mut(), ai) {
        o.extend(extra);
    }
    if let Some(path) = p.get("overlay").and_then(Value::as_str) {
        let ov = filmcraft_render::grade_explain::change_overlay(&input, &output, working(s)?);
        write_png(&ov, path).map_err(|e| bad(CMD, format!("{path}: {e}")))?;
        v["overlay"] = json!(path);
    }
    Ok(v)
}

fn explain_grade(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "grade.explainGrade";
    let clip = target(s, p, CMD)?;
    let t = clip_time(s, p, "", clip, CMD)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(CMD, "no such clip"))?;
    let g = it.grade.clone().ok_or_else(|| bad(CMD, "the clip has no grade"))?;
    let mt = media_time(s, &json!({"time": t.0}), clip);
    let ws = working(s)?;
    let mut nodes = Vec::new();
    let mut out = Vec::new();
    // (each node against the frame before it: n + 1 renders, node i's output is node i + 1's
    // input; a disabled node explains itself as disabled)
    let mut input = clip_frame(s, clip, t, Probe { nodes: Some(0), effects: false }, CMD)?;
    for (pos, n) in g.nodes.iter().enumerate().take(MAX_NODES) {
        let output = clip_frame(s, clip, t, Probe { nodes: Some(pos + 1), effects: false }, CMD)?;
        let facts = filmcraft_render::grade_explain::explain_node(n, mt, &input, &output, ws);
        input = output;
        let label = node_label(n, pos);
        out.push(json!({"node": n.id, "label": label, "sentence": facts.sentence(&label), "summary": facts.summary()}));
        nodes.push((label, facts));
    }
    Ok(json!({"clip": clip.0, "nodes": out, "text": filmcraft_render::grade_explain::explain_grade(&nodes)}))
}

/// A premultiplied 0…1 image as an 8-bit PNG.
fn write_png(img: &Image, path: &str) -> std::result::Result<(), String> {
    let (w, h) = (u32::try_from(img.w).map_err(|e| e.to_string())?, u32::try_from(img.h).map_err(|e| e.to_string())?);
    let rgba: Vec<u8> = img
        .px
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let un = |v: f32| if a > 1e-6 { v / a } else { 0.0 };
            let q = |v: f32| (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * 255.0).round() as u8;
            [q(un(r)), q(un(g)), q(un(b)), q(a)]
        })
        .collect();
    let im = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad image size")?;
    im.save(path).map_err(|e| e.to_string())
}

/// AI4.1a: the current grade's facts as text for the assistant (`coach.assist` with
/// `context: "auto"`): the current node of the selected video clip (or its only node), measured
/// at the playhead. None when nothing is selected, the clip has no grade, or the frame can't be
/// rendered (the question is still answered, just without them).
pub(crate) fn current_grade_facts(s: &Session) -> Option<String> {
    const CMD: &str = "coach.assist";
    let (clip, current) = current_for(s, &json!({}), CMD).ok()?;
    let q = s.active_sequence()?;
    let (_, it) = q.find_item(clip)?;
    let g = it.grade.as_ref()?;
    let id = pick(g, None, current, CMD).ok()?;
    let pos = g.nodes.iter().position(|n| n.id == id)?;
    let node = g.nodes.get(pos)?.clone();
    let t = clip_time(s, &json!({}), "", clip, CMD).ok()?;
    let mt = media_time(s, &json!({"time": t.0}), clip);
    let (facts, _, _) = node_facts(s, clip, t, mt, pos, &node, CMD).ok()?;
    let label = node_label(&node, pos);
    Some(format!("{} ({} of {} nodes): {}", label, pos + 1, g.nodes.len(), facts.sentence(&label)))
}

/// AI4.1a: what is selected, as text for the assistant: the selected clips' names (at most
/// three, then a count) and the current grade node's label. None when nothing is selected.
pub(crate) fn selection_text(s: &Session) -> Option<String> {
    const SHOWN: usize = 3;
    let q = s.active_sequence()?;
    let names: Vec<&str> = s.state.selection.iter().filter_map(|c| q.find_item(*c)).map(|(_, it)| it.name.as_str()).collect();
    let first = names.first()?;
    let mut out = if names.len() == 1 { format!("clip \"{first}\"") } else { format!("{} clips: ", names.len()) };
    if names.len() > 1 {
        let shown: Vec<String> = names.iter().take(SHOWN).map(|n| format!("\"{n}\"")).collect();
        out.push_str(&shown.join(", "));
        if names.len() > SHOWN {
            out.push_str(&format!(" and {} more", names.len() - SHOWN));
        }
    }
    if let Some(sel) = s.state.grade_node
        && let Some((_, it)) = q.find_item(sel.clip)
        && let Some(g) = &it.grade
        && let Some(pos) = g.nodes.iter().position(|n| n.id == sel.node)
        && let Some(n) = g.nodes.get(pos)
    {
        out.push_str(&format!("; grade node \"{}\"", node_label(n, pos)));
    }
    Some(out)
}

/// AI4.1b: the frame under the playhead, measured for the assistant
/// ([`filmcraft_render::picture_facts`]): the selected video clip (else the topmost video clip at
/// the playhead) before its grade (after the input transform) and, when it has one, after it.
/// None when there is no clip or nothing to measure.
pub(crate) fn current_picture_facts(s: &Session) -> Option<String> {
    const CMD: &str = "coach.assist";
    let q = s.active_sequence()?;
    let clip = target(s, &json!({}), CMD).ok().or_else(|| q.video_tracks.iter().rev().find_map(|t| t.item_at(s.playhead())).map(|i| i.id))?;
    let (_, it) = q.find_item(clip)?;
    let graded = it.grade.as_ref().is_some_and(|g| !g.nodes.is_empty());
    let t = clip_time(s, &json!({}), "", clip, CMD).ok()?;
    // (cached per frame: the key covers what the measured frames depend on)
    let key = shot_key(s, q, clip, it, Some(t.0));
    let cache = &s.assist.picture_cache;
    if let Some((k, v)) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref()
        && *k == key
    {
        return Some(v.clone());
    }
    let out = picture_sheet(s, clip, it, t, graded)?;
    *cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((key, out.clone()));
    Some(out)
}

/// The cache key of a clip's picture: the clip, its state (grade, effects, source), the
/// sequence's colour settings and the media, plus the frame time when given.
fn shot_key(s: &Session, q: &filmcraft_project::Sequence, clip: ClipId, it: &filmcraft_project::TrackItem, t: Option<i64>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (clip.0, t, format!("{it:?}"), format!("{:?}", q.settings.color)).hash(&mut h);
    if let Some((_, m, _)) = s.project.resolve_media(it.item) {
        format!("{m:?}").hash(&mut h);
    }
    h.finish()
}

/// AI4.2b: the shot under the playhead (the selected video clip, else the topmost one) as a
/// key: the same clip in the same state is the same shot, wherever the playhead is in it.
pub(crate) fn current_shot_key(s: &Session) -> Option<u64> {
    let q = s.active_sequence()?;
    let clip = target(s, &json!({}), "coach.assist").ok().or_else(|| q.video_tracks.iter().rev().find_map(|t| t.item_at(s.playhead())).map(|i| i.id))?;
    let (_, it) = q.find_item(clip)?;
    Some(shot_key(s, q, clip, it, None))
}

/// AI4.2: the Program frame under the playhead as the vision model takes it: display RGB in
/// 0..=1, row by row, rendered small (at most about `max_pixels`, aspect kept; proxies allowed,
/// captions left out). None without a sequence, or while a frame is still loading.
pub(crate) fn program_frame_rgb(s: &Session, max_pixels: u64) -> Option<(Vec<f32>, usize, usize)> {
    let seq = s.state.active_sequence?;
    let q = s.project.sequence(seq)?;
    let (w, h) = (f64::from(q.settings.width), f64::from(q.settings.height));
    if w < 1.0 || h < 1.0 || max_pixels == 0 {
        return None;
    }
    let scale = (max_pixels as f64 / (w * h)).sqrt().min(1.0) as f32;
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let _ = filmcraft_media::pending::take();
    let opts = filmcraft_render::RenderOptions { scale, captions: false, ..Default::default() };
    let img = filmcraft_render::render_sequence(&s.project, seq, s.playhead(), opts, &provider);
    if filmcraft_media::pending::take() || img.w == 0 || img.h == 0 {
        return None;
    }
    let rgba = img.to_rgba8();
    let rgb = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0)).collect();
    Some((rgb, img.w, img.h))
}

/// The fact sheet text of [`current_picture_facts`] (rendered and measured now).
fn picture_sheet(s: &Session, clip: ClipId, it: &filmcraft_project::TrackItem, t: Tick, graded: bool) -> Option<String> {
    use filmcraft_render::picture_facts::PictureFacts;
    const CMD: &str = "coach.assist";
    let w = working(s).ok()?;
    let sheet = |nodes: Option<usize>| {
        let img = clip_frame(s, clip, t, Probe { nodes, effects: false }, CMD).ok()?;
        Some(PictureFacts::measure(&img, w)?.sheet())
    };
    let source = source_line(s, it.item);
    let body = if graded {
        match (sheet(Some(0)), sheet(None)) {
            (Some(b), Some(a)) => format!("before the grade:\n{b}\nafter the grade:\n{a}"),
            (b, a) => a.or(b)?,
        }
    } else {
        format!("ungraded:\n{}", sheet(None)?)
    };
    Some(format!("clip \"{}\"{source}, {body}", it.name))
}

/// AI4.1c: the clip's source colour space for the fact sheet: log footage is converted by the
/// input transform before it is measured (so "dark" means after that conversion), and a
/// display-referred source is measured as it is. Empty when the media isn't known.
fn source_line(s: &Session, item: ItemId) -> String {
    let Some((_, media, _)) = s.project.resolve_media(item) else { return String::new() };
    let Some(v) = media.info.video.as_ref() else { return String::new() };
    let cs = media.interpret.color_space.unwrap_or_else(|| filmcraft_color::ColorSpace::from_info(&v.color));
    let kind = if cs.is_log() {
        "log footage, measured after the input transform"
    } else if cs.is_hdr() {
        "HDR, measured after the input transform"
    } else {
        "display-referred (not log)"
    };
    format!(" (source {}: {kind})", cs.label())
}
