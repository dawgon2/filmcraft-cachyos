//! The Grading Coach's context hints (fork GRADE-PLAN G2.12, part 3).
//!
//! The first time someone does something on the Color page that usually needs a word of
//! explanation (adds a node, opens the qualifier, tracks a window, puts a qualifier after a heavy
//! look…), a small card says why it matters, what it is, and links the lesson about it. Each hint
//! shows once; Settings ▸ Color ▸ Grading Coach ▸ Show Grading Coach hints (`color.coach.hints`)
//! turns them all off.
//!
//! The hints are data (`assets/coach/hints.toml`, embedded at build time): `id`, `title`, `why`,
//! `text`, an optional `lesson`, and `when`, a list of triggers (any one shows the hint):
//!
//! - `{ command = "id" | [ids], params = { … } }`: a command that succeeded with parameters
//!   matching the pattern (the lesson matcher, [`crate::coach::matches`]);
//! - `{ element = "automation.id" }`: a control on screen (the UI reports the ones hints wait
//!   for each frame, [`Session::coach_hint_elements`]; agents can call `coach.seen`);
//! - `{ rule = "qualifierAfterHeavyLook" }`: a state rule the engine checks on the addressed
//!   clip's grade after every successful `grade.*` command (see [`heavy_look`]).
//!
//! The engine decides which hint shows: never while a lesson runs, one at a time (a trigger
//! while a hint is on screen is not used up), and only hints not yet seen. A shown hint is
//! remembered in `color.coach.hintsSeen` (at most [`MAX_SEEN`] ids) and the preferences are
//! saved. Commands: `coach.hint` (the hint on screen), `coach.dismissHint` (`off: true` also
//! turns hints off), `coach.resetHints` (show every hint again).
//!
//! The file is checked like a lesson file: bounded size, counts and lengths, valid ids and
//! patterns, unknown fields refused.

use std::sync::{Arc, OnceLock};

use filmcraft_project::grade::{Grade, GradeNode};
use filmcraft_time::Tick;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::coach::{MAX_ALTERNATIVES, MAX_TEXT, MAX_TITLE, OneOrMany, check_pattern, check_text, matches, valid_id};
use crate::commands::{CommandSpec, always, bool_p};
use crate::{Result, Session};

/// Most hint ids remembered as seen.
pub const MAX_SEEN: usize = 256;
/// Most hints in the file.
pub const MAX_HINTS: usize = 64;
/// Largest hints file accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// A state rule the engine evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rule {
    /// An enabled node with its qualifier on comes after an enabled node with a heavy look
    /// ([`heavy_look`]) on the addressed clip.
    QualifierAfterHeavyLook,
}

/// What shows a hint.
#[derive(Clone, Debug, PartialEq)]
pub enum Trigger {
    /// One of these commands succeeded, with parameters matching `params` (None: any).
    Command { ids: Vec<String>, params: Option<Value> },
    /// A control on screen.
    Element(String),
    /// A state rule.
    Rule(Rule),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub id: String,
    pub title: String,
    /// Why it matters (shown first).
    pub why: String,
    /// What it is and when to use it.
    pub text: String,
    /// The lesson the card links to.
    pub lesson: Option<String>,
    pub when: Vec<Trigger>,
}

impl Hint {
    pub fn to_json(&self) -> Value {
        let lesson = self.lesson.as_deref().map(|id| json!({"id": id, "title": crate::coach::lesson(id).map(|l| l.title.clone())}));
        json!({"id": self.id, "title": self.title, "why": self.why, "text": self.text, "lesson": lesson})
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(rename = "hint")]
    hints: Vec<RawHint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHint {
    id: String,
    title: String,
    why: String,
    text: String,
    lesson: Option<String>,
    when: Vec<RawTrigger>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrigger {
    command: Option<OneOrMany>,
    params: Option<Value>,
    element: Option<String>,
    rule: Option<Rule>,
}

/// Parse and validate a hints file. Errors name the hint (1-based) and what is wrong.
pub fn parse(src: &str) -> std::result::Result<Vec<Hint>, String> {
    if src.len() > MAX_FILE_BYTES {
        return Err(format!("the hints file is larger than {MAX_FILE_BYTES} bytes"));
    }
    let raw: RawFile = toml::from_str(src).map_err(|e| e.to_string())?;
    if raw.hints.is_empty() || raw.hints.len() > MAX_HINTS {
        return Err(format!("the file needs 1 to {MAX_HINTS} hints"));
    }
    let mut out: Vec<Hint> = Vec::with_capacity(raw.hints.len());
    for (i, h) in raw.hints.into_iter().enumerate() {
        let n = i + 1;
        if !valid_id(&h.id) || h.id.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(format!("hint {n}: the id must be lower-case letters, digits, `.`, `_` or `-`"));
        }
        if out.iter().any(|x| x.id == h.id) {
            return Err(format!("hint {n}: duplicate id `{}`", h.id));
        }
        check_text(&format!("hint {n}'s title"), &h.title, MAX_TITLE, true)?;
        check_text(&format!("hint {n}'s why"), &h.why, MAX_TEXT, true)?;
        check_text(&format!("hint {n}'s text"), &h.text, MAX_TEXT, true)?;
        if let Some(l) = &h.lesson
            && !valid_id(l)
        {
            return Err(format!("hint {n}: `lesson` is not a lesson id"));
        }
        if h.when.is_empty() || h.when.len() > MAX_ALTERNATIVES {
            return Err(format!("hint {n}: `when` needs 1 to {MAX_ALTERNATIVES} triggers"));
        }
        let mut when = Vec::with_capacity(h.when.len());
        for t in h.when {
            let kinds = usize::from(t.command.is_some()) + usize::from(t.element.is_some()) + usize::from(t.rule.is_some());
            if kinds != 1 {
                return Err(format!("hint {n}: a trigger needs exactly one of `command`, `element` or `rule`"));
            }
            if t.params.is_some() && t.command.is_none() {
                return Err(format!("hint {n}: `params` only goes with `command`"));
            }
            when.push(if let Some(c) = t.command {
                let ids = c.ids("`command`").map_err(|m| format!("hint {n}: {m} (a command id)"))?;
                if let Some(p) = &t.params {
                    if !p.is_object() {
                        return Err(format!("hint {n}: `params` must be a table"));
                    }
                    check_pattern(p, 0, &mut 0).map_err(|m| format!("hint {n}: {m}"))?;
                }
                Trigger::Command { ids, params: t.params }
            } else if let Some(e) = t.element {
                if !valid_id(&e) {
                    return Err(format!("hint {n}: `element` is not an automation id"));
                }
                Trigger::Element(e)
            } else if let Some(r) = t.rule {
                Trigger::Rule(r)
            } else {
                return Err(format!("hint {n}: an empty trigger"));
            });
        }
        out.push(Hint { id: h.id, title: h.title, why: h.why, text: h.text, lesson: h.lesson, when });
    }
    Ok(out)
}

/// The hints file shipped with the app.
pub const BUNDLED: &str = include_str!("../../../assets/coach/hints.toml");

/// The bundled hints (none when the file is broken, which is logged; a test keeps it valid).
pub fn bundled() -> &'static [Arc<Hint>] {
    static HINTS: OnceLock<Vec<Arc<Hint>>> = OnceLock::new();
    HINTS.get_or_init(|| match parse(BUNDLED) {
        Ok(v) => v.into_iter().map(Arc::new).collect(),
        Err(e) => {
            log::warn!("coach hints: {e}");
            Vec::new()
        }
    })
}

pub fn hint(id: &str) -> Option<Arc<Hint>> {
    bundled().iter().find(|h| h.id == id).cloned()
}

// ---------- the state rule ----------

/// Whether a node changes the picture a lot: a LUT mixed in at 30 % or more, contrast or
/// saturation 0.35 or more away from 1, a curve bent 0.05 or more off the straight line, temperature or tint of 40 or more, a lift /
/// gamma / offset wheel moved 0.25 or more, or gain 0.5 or more (values at the clip's start).
pub fn heavy_look(n: &GradeNode) -> bool {
    let t = Tick::ZERO;
    let far = |k: &str, d: f32| (n.float_at(k, t) - 1.0).abs() >= d;
    let wheel = |k: &str, d: f32| n.wheel_at(k, t).iter().any(|v| v.abs() >= d);
    (!n.lut.is_empty() && n.float_at("lut.mix", t) >= 0.3)
        || far("contrast", 0.35)
        || far("saturation", 0.35)
        || ["curve.master", "curve.r", "curve.g", "curve.b"].iter().any(|k| n.curve(k).is_some_and(|c| curve_bend(&c) >= 0.05))
        || n.float_at("temperature", t).abs() >= 40.0
        || n.float_at("tint", t).abs() >= 40.0
        || ["lift", "gamma", "offset"].iter().any(|k| wheel(k, 0.25))
        || wheel("gain", 0.5)
}

/// How far a curve's points are from the straight line (the largest |out - in|).
fn curve_bend(points: &[[f32; 2]]) -> f32 {
    points.iter().map(|p| (p[1] - p[0]).abs()).fold(0.0, f32::max)
}

/// An enabled node qualifying after an enabled node with a heavy look.
pub fn qualifier_after_heavy_look(g: &Grade) -> bool {
    let mut heavy_before = false;
    for n in g.active_nodes() {
        if heavy_before && n.qualifier_at(Tick::ZERO).is_some() {
            return true;
        }
        heavy_before |= heavy_look(n);
    }
    false
}

fn rule_holds(s: &Session, r: Rule, params: &Value) -> bool {
    match r {
        Rule::QualifierAfterHeavyLook => {
            let Ok(c) = crate::grade::target(s, params, "coach") else { return false };
            let Some(q) = s.active_sequence() else { return false };
            q.find_item(c).and_then(|(_, it)| it.grade.as_ref()).is_some_and(qualifier_after_heavy_look)
        }
    }
}

// ---------- choosing a hint ----------

/// Whether a new hint may show now.
fn may_show(s: &Session) -> bool {
    s.prefs.color.coach.hints && s.coach.active.is_none() && s.coach.hint.is_none()
}

fn unseen(s: &Session) -> impl Iterator<Item = &'static Arc<Hint>> + '_ {
    let seen = &s.prefs.color.coach.hints_seen;
    bundled().iter().filter(move |h| !seen.contains(&h.id))
}

/// Show `h` and remember it as seen (saving the preferences).
fn show(s: &mut Session, h: Arc<Hint>) {
    let seen = &mut s.prefs.color.coach.hints_seen;
    if !seen.contains(&h.id) {
        seen.push(h.id.clone());
        let extra = seen.len().saturating_sub(MAX_SEEN);
        seen.drain(..extra);
    }
    s.coach.hint = Some(h);
    s.coach.revision = s.coach.revision.wrapping_add(1);
    save(s);
}

fn save(s: &mut Session) {
    if let Some(path) = s.prefs_path.clone()
        && let Err(e) = s.prefs.save(&path)
    {
        s.log.push(crate::panels::Level::Warning, "coach", format!("couldn't save the coach's hints: {e}"));
    }
}

/// A command succeeded outside a lesson: show the first unseen hint it triggers.
pub(crate) fn observe_command(s: &mut Session, id: &str, params: &Value) {
    if !may_show(s) {
        return;
    }
    let grade_cmd = id.starts_with("grade.");
    let hit = unseen(s).find(|h| {
        h.when.iter().any(|t| match t {
            Trigger::Command { ids, params: pattern } => ids.iter().any(|w| w == id) && pattern.as_ref().is_none_or(|p| matches(p, params)),
            Trigger::Rule(r) => grade_cmd && rule_holds(s, *r, params),
            Trigger::Element(_) => false,
        })
    });
    if let Some(h) = hit.cloned() {
        show(s, h);
    }
}

/// A control is on screen outside a lesson: show the first unseen hint waiting for it.
pub(crate) fn observe_element(s: &mut Session, element: &str) {
    if !may_show(s) {
        return;
    }
    let hit = unseen(s).find(|h| h.when.iter().any(|t| matches!(t, Trigger::Element(e) if e == element)));
    if let Some(h) = hit.cloned() {
        show(s, h);
    }
}

impl Session {
    /// The controls unseen hints wait for (the UI reports those on screen with
    /// [`Session::coach_element_seen`]); empty while no hint may show.
    pub fn coach_hint_elements(&self) -> Vec<&'static str> {
        if !may_show(self) {
            return Vec::new();
        }
        unseen(self)
            .flat_map(|h| {
                h.when.iter().filter_map(|t| match t {
                    Trigger::Element(e) => Some(e.as_str()),
                    _ => None,
                })
            })
            .collect()
    }
}

// ---------- commands ----------

fn state(s: &Session) -> Value {
    json!({
        "hint": s.coach.hint.as_ref().map(|h| h.to_json()),
        "enabled": s.prefs.color.coach.hints,
        "seen": s.prefs.color.coach.hints_seen,
        "revision": s.coach.revision,
    })
}

type RunFn = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: RunFn) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("coach.hint", "Current Coach Hint", r#"{"all":bool?}"#, |s, p| {
            let mut v = state(s);
            if bool_p(p, "all") == Some(true) {
                v["all"] = Value::Array(bundled().iter().map(|h| h.to_json()).collect());
            }
            Ok(v)
        }),
        spec("coach.dismissHint", "Dismiss Coach Hint", r#"{"off":bool?}"#, |s, p| {
            s.coach.hint = None;
            s.coach.revision = s.coach.revision.wrapping_add(1);
            if bool_p(p, "off") == Some(true) && s.prefs.color.coach.hints {
                s.prefs.color.coach.hints = false;
                save(s);
            }
            Ok(state(s))
        }),
        spec("coach.resetHints", "Show Coach Hints Again", "{}", |s, _| {
            s.coach.hint = None;
            s.coach.revision = s.coach.revision.wrapping_add(1);
            if !s.prefs.color.coach.hints_seen.is_empty() {
                s.prefs.color.coach.hints_seen.clear();
                save(s);
            }
            Ok(state(s))
        }),
    ]
}
