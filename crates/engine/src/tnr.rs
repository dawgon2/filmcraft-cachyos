//! Clip-level temporal noise reduction commands (`tnr.*`, fork 6.7b, design
//! `docs/fork/DESIGN-6.7b.md` §1.6): edit a media / subclip item's `temporal_nr`, applied before
//! its grade. Every edit is one undo step through [`Session::edit_sequence`].
//!
//! Commands address a clip by `clip` (default: the first selected video clip, as the `grade.*`
//! commands do). Clips that are not media or subclips (graphics, adjustment layers, nested
//! sequences, solid generators such as Color Matte) are refused.

use filmcraft_project::grade::NodeParamKind;
use filmcraft_project::{ClipId, Param, ParamValue, TNR_PARAMS, TemporalNr, tnr_param};
use filmcraft_time::Tick;
use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, has_seq, str_p};
use crate::grade::{is_video, media_time, target};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: has_seq, run, journal: true }
}
fn query(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}
/// A command in the Clip > Temporal NR menu.
fn menu_spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &["Clip", "Temporal NR"], shortcut: None, params, enabled: has_seq, run, journal: true }
}

/// The refusal for clips the temporal NR does not apply to.
const NOT_MEDIA: &str = "temporal NR applies to media clips and subclips";
/// The most clips one `tnr.paste` touches.
const MAX_TARGETS: usize = 10_000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        query("tnr.get", "Get Temporal NR", r#"{"clip":id?,"time":ticks?}"#, get),
        query("tnr.params", "List Temporal NR Parameters", "{}", params),
        spec("tnr.set", "Set Temporal NR Parameter", r#"{"clip":id?,"param":"luma|chroma|frames|motion","value":num,"time":ticks?,"merge":bool|str?}"#, set),
        menu_spec("tnr.toggle", "Enable / Disable Temporal NR", r#"{"clip":id?,"on":bool?}"#, toggle),
        spec("tnr.toggleAnimation", "Toggle Temporal NR Parameter Animation", r#"{"clip":id?,"param":str,"time":ticks?}"#, toggle_animation),
        spec("tnr.moveKeyframe", "Move Temporal NR Keyframe", r#"{"clip":id?,"param":str,"mediaTime":ticks,"to":ticks}"#, move_keyframe),
        menu_spec("tnr.reset", "Reset Temporal NR", r#"{"clip":id?,"param":str?}"#, reset),
        menu_spec("tnr.clear", "Remove Temporal NR", r#"{"clip":id?}"#, clear),
        menu_spec("tnr.copy", "Copy Temporal NR", r#"{"clip":id?}"#, copy),
        menu_spec("tnr.paste", "Paste Temporal NR", r#"{"clips":[id]?}"#, paste),
    ]
}

/// True when clip `c` of the active sequence takes a temporal NR: a media clip (not a solid
/// generator) or a subclip, on a video track.
pub(crate) fn takes_tnr(s: &Session, c: ClipId) -> bool {
    let Some(q) = s.active_sequence() else { return false };
    let Some((_, it)) = q.find_item(c) else { return false };
    is_video(q, c) && filmcraft_render::grade::takes_temporal_nr(&s.project, it.item)
}

/// The addressed clip when it has a temporal NR, whatever it shows: removing one is always
/// allowed (a migrated project can carry one on a clip that doesn't take it).
fn holder_target(s: &Session, p: &Value, cmd: &str) -> Result<ClipId> {
    let c = target(s, p, cmd)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    match q.find_item(c) {
        Some((_, it)) if it.temporal_nr.is_some() => Ok(c),
        Some(_) => media_target(s, p, cmd),
        None => Err(bad(cmd, "no such clip")),
    }
}

/// The addressed clip, checked to take a temporal NR.
fn media_target(s: &Session, p: &Value, cmd: &str) -> Result<ClipId> {
    let c = target(s, p, cmd)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if q.find_item(c).is_none() {
        return Err(bad(cmd, "no such clip"));
    }
    if !takes_tnr(s, c) {
        return Err(bad(cmd, NOT_MEDIA));
    }
    Ok(c)
}

/// The schema entry of `p`'s `param` (required).
fn param_def(p: &Value, cmd: &str) -> Result<(&'static str, f32, f32, f32)> {
    let key = str_p(p, "param").ok_or_else(|| bad(cmd, "need `param` (luma, chroma, frames or motion)"))?;
    param_key(key, cmd)
}

fn param_key(key: &str, cmd: &str) -> Result<(&'static str, f32, f32, f32)> {
    match tnr_param(key).map(|d| (d.key, d.kind)) {
        Some((k, NodeParamKind::Float { identity, min, max })) => Ok((k, identity, min, max)),
        _ => Err(bad(cmd, format!("no temporal NR parameter `{key}` (see tnr.params)"))),
    }
}

/// Run `f` on the clip's temporal NR slot as one undo step (merged with the previous edit of the
/// same `merge` key).
fn edit_tnr<R>(s: &mut Session, c: ClipId, label: &str, merge: Option<&str>, f: impl FnOnce(&mut Option<TemporalNr>) -> Result<R>) -> Result<R> {
    s.edit_sequence_as(label, merge, |q, _, _| {
        let (_, it) = q.find_item_mut(c).ok_or(filmcraft_edit::EditError::NoItem(c))?;
        f(&mut it.temporal_nr)
    })
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.get";
    let c = media_target(s, p, CMD)?;
    let mt = media_time(s, p, c);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(c).ok_or_else(|| bad(CMD, "no such clip"))?;
    let t = it.temporal_nr.clone().unwrap_or_default();
    let mut values = Map::new();
    for d in &TNR_PARAMS {
        values.insert(d.key.to_string(), json!(t.float_at(d.key, mt)));
    }
    Ok(json!({
        "clip": c.0,
        "present": it.temporal_nr.is_some(),
        "enabled": t.enabled,
        "identity": t.is_identity_at(mt),
        "time": mt.0,
        "values": values,
        "params": t.params,
    }))
}

fn params(_: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = TNR_PARAMS
        .iter()
        .filter_map(|p| match p.kind {
            NodeParamKind::Float { identity, min, max } => {
                Some(json!({"key": p.key, "label": p.label, "kind": "float", "identity": identity, "min": min, "max": max}))
            }
            _ => None,
        })
        .collect();
    Ok(json!({ "params": list }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.set";
    let (key, _, min, max) = param_def(p, CMD)?;
    let raw = p.get("value").ok_or_else(|| bad(CMD, "need `value`"))?;
    let v = raw.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(CMD, format!("`{key}` wants a finite number")))?;
    let v = ParamValue::Float(v.clamp(f64::from(min), f64::from(max)));
    let c = media_target(s, p, CMD)?;
    let mt = media_time(s, p, c);
    let merge = match p.get("merge") {
        Some(Value::Bool(true)) => Some(format!("tnr.{}.{key}", c.0)),
        Some(Value::String(m)) => Some(format!("tnr.{}.{key}.{m}", c.0)),
        _ => None,
    };
    edit_tnr(s, c, "Change Temporal NR", merge.as_deref(), |slot| {
        let t = slot.get_or_insert_with(TemporalNr::default);
        match t.params.get_mut(key) {
            Some(prm) => prm.set_at(mt, v),
            None => {
                t.params.insert(key.to_string(), Param::new(v));
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.toggle";
    let on = bool_p(p, "on");
    if p.get("on").is_some_and(|v| !v.is_null()) && on.is_none() {
        return Err(bad(CMD, "`on` must be true or false"));
    }
    let c = media_target(s, p, CMD)?;
    let now = edit_tnr(s, c, "Enable / Disable Temporal NR", None, |slot| {
        let t = match (slot.as_mut(), on) {
            (Some(t), _) => t,
            // switching an absent temporal NR explicitly creates it
            (None, Some(_)) => slot.get_or_insert_with(TemporalNr::default),
            (None, None) => return Err(bad(CMD, "the clip has no temporal NR (pass `on` or use tnr.set)")),
        };
        t.enabled = on.unwrap_or(!t.enabled);
        Ok(t.enabled)
    })?;
    Ok(json!({ "enabled": now }))
}

fn toggle_animation(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.toggleAnimation";
    let (key, identity, _, _) = param_def(p, CMD)?;
    let c = media_target(s, p, CMD)?;
    let mt = media_time(s, p, c);
    let animated = edit_tnr(s, c, "Toggle Animation", None, |slot| {
        let t = slot.get_or_insert_with(TemporalNr::default);
        let prm = t.params.entry(key.to_string()).or_insert_with(|| Param::new(ParamValue::Float(f64::from(identity))));
        prm.toggle_animation(mt);
        Ok(prm.is_animated())
    })?;
    Ok(json!({ "animated": animated }))
}

/// Moves the keyframe of `param` at media time `mediaTime` to `to` (a keyframe already at `to` is
/// replaced, as `effects.moveKeyframe` does): the Effect Controls lane's drag.
fn move_keyframe(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.moveKeyframe";
    let (key, ..) = param_def(p, CMD)?;
    let tick = |k: &str| p.get(k).and_then(Value::as_i64).map(Tick).ok_or_else(|| bad(CMD, format!("need `{k}` (ticks)")));
    let (from, to) = (tick("mediaTime")?, tick("to")?);
    let c = media_target(s, p, CMD)?;
    edit_tnr(s, c, "Move Keyframe", None, |slot| {
        let prm = slot.as_mut().and_then(|t| t.params.get_mut(key)).ok_or_else(|| bad(CMD, "the parameter is not animated"))?;
        let i = prm.keyframes.iter().position(|k| k.time == from).ok_or_else(|| bad(CMD, "no keyframe at `mediaTime`"))?;
        let mut k = prm.keyframes.remove(i);
        k.time = to;
        prm.keyframes.retain(|x| x.time != to);
        let at = prm.keyframes.partition_point(|x| x.time < to);
        prm.keyframes.insert(at, k);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.reset";
    let key = match p.get("param") {
        None | Some(Value::Null) => None,
        Some(Value::String(k)) => Some(param_key(k, CMD)?.0),
        Some(_) => return Err(bad(CMD, "`param` must be a parameter key")),
    };
    let c = if key.is_none() { holder_target(s, p, CMD)? } else { media_target(s, p, CMD)? };
    edit_tnr(s, c, "Reset Temporal NR", None, |slot| {
        if slot.is_none() {
            return Err(bad(CMD, "the clip has no temporal NR"));
        }
        match key {
            Some(k) => {
                if let Some(t) = slot.as_mut() {
                    t.params.remove(k);
                    // nothing left to keep: the clip goes back to no temporal NR
                    if t.enabled && t.params.is_empty() {
                        *slot = None;
                    }
                }
            }
            None => *slot = None,
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.clear";
    let c = holder_target(s, p, CMD)?;
    edit_tnr(s, c, "Remove Temporal NR", None, |slot| match slot.take() {
        Some(_) => Ok(()),
        None => Err(bad(CMD, "the clip has no temporal NR")),
    })?;
    Ok(Value::Null)
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.copy";
    let c = media_target(s, p, CMD)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(c).ok_or_else(|| bad(CMD, "no such clip"))?;
    let t = it.temporal_nr.clone().ok_or_else(|| bad(CMD, "the clip has no temporal NR"))?;
    s.state.tnr_clipboard = Some(t);
    Ok(json!({ "clip": c.0 }))
}

/// The clips `clips` names (every one must take a temporal NR), else the selected clips that do.
fn paste_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<ClipId>> {
    let mut out: Vec<ClipId> = Vec::new();
    match p.get("clips") {
        None | Some(Value::Null) => {
            for c in &s.state.selection {
                if out.len() < MAX_TARGETS && !out.contains(c) && takes_tnr(s, *c) {
                    out.push(*c);
                }
            }
        }
        Some(Value::Array(a)) => {
            if a.len() > MAX_TARGETS {
                return Err(bad(cmd, format!("at most {MAX_TARGETS} clips")));
            }
            for v in a {
                let c = v.as_u64().map(ClipId).ok_or_else(|| bad(cmd, "`clips` must be clip ids"))?;
                if !takes_tnr(s, c) {
                    return Err(bad(cmd, format!("clip {}: {NOT_MEDIA}", c.0)));
                }
                if out.len() < MAX_TARGETS && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        Some(_) => return Err(bad(cmd, "`clips` must be a list of clip ids")),
    }
    if out.is_empty() {
        return Err(bad(cmd, "need `clips` (or selected media clips)"));
    }
    Ok(out)
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tnr.paste";
    let t = s.state.tnr_clipboard.clone().ok_or_else(|| bad(CMD, "copy a temporal NR first (tnr.copy)"))?;
    let targets = paste_targets(s, p, CMD)?;
    let n = s.edit_sequence("Paste Temporal NR", |q, _, _| {
        let mut n = 0usize;
        for c in &targets {
            let (_, it) = q.find_item_mut(*c).ok_or(filmcraft_edit::EditError::NoItem(*c))?;
            if it.temporal_nr.as_ref() != Some(&t) {
                it.temporal_nr = Some(t.clone());
                n = n.saturating_add(1);
            }
        }
        Ok(n)
    })?;
    Ok(json!({ "clips": n }))
}
