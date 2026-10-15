//! The assistant's prompt (fork AI-PLAN AI3 item 2, step AI3.2b): the guide sections
//! [`coach_ask::retrieve`] finds for a question, the current grade's facts and what is on screen,
//! packed under a size budget for a local chat model. No model is run here.
//!
//! Each section goes in as its compact header and numbered steps first; the full text only for
//! the best one or two while the budget allows (AI-PLAN AI3 0d: ≈ 3–5× fewer prompt tokens). Every
//! section is labelled with its source id so the answer can cite it and the grounding check
//! (AI3.4) can tell guide steps from general advice.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};

use crate::ai_explain::ExplainBackend;
use crate::coach_ask::{self, MAX_QUESTION_CHARS, MAX_RETRIEVE, Retrieved};
use crate::commands::{CommandSpec, always, bad};
use crate::{Result, Session};

/// Default prompt budget in characters (≈ 1 100 tokens at ~4 characters a token): prefill on the CPU
/// runs at ~40 tokens/s for SmolLM3-3B on the reference PC (AI3.3a), so the prompt stays short.
pub const DEFAULT_BUDGET_CHARS: usize = 4_500;
/// Largest budget accepted, in characters.
pub const MAX_BUDGET_CHARS: usize = 64_000;
/// Sections retrieved for one question.
pub const SECTIONS: usize = 6;
/// How many of the best sections may go in with their full text.
pub const FULL_TEXT: usize = 1;
/// Longest grade facts / on-screen text taken, in characters.
pub const MAX_CONTEXT_CHARS: usize = 3_000;
/// Longest full section text taken, in characters.
const MAX_BODY_CHARS: usize = 4_000;

const SYSTEM: &str = "You are FilmCraft's editing and grading assistant. \
Answer the user's question using the guide sections given. \
FilmCraft steps, buttons, panels, menus and commands may come only from those sections; never invent one. \
Cite the sections you used by their id in square brackets, like [color.nodes.why]. \
Advice the sections don't state is allowed only on a line starting with \"General advice:\". \
If the sections don't cover the question, say you don't know and name the closest section. \
If a section says something is not in FilmCraft yet, say so and describe no steps for it. \
Name buttons, panels and shortcuts only as the sections write them. \
Give options with their trade-offs, not one right way. Keep it short and plain: at most 6 steps, no headings.";

/// What the user is looking at, when the question is asked (all optional).
#[derive(Clone, Debug, Default)]
pub struct AssistContext {
    /// The page or workspace, e.g. "Color".
    pub page: Option<String>,
    /// The selected node, clip or tool, in words.
    pub selection: Option<String>,
    /// The current grade's facts (the node explainer's JSON, G2.10).
    pub grade_facts_json: Option<String>,
    /// A follow-up: the question asked just before (AI3.5); retrieval reads both questions.
    pub previous_question: Option<String>,
    /// The assistant's checked answer to [`AssistContext::previous_question`].
    pub previous_answer: Option<String>,
    /// Free answers, or only the guide's own steps picked by the model (owner question Q35 (b)).
    pub style: AssistStyle,
}

/// How the model answers (AI3.6, OWNER-QUESTIONS Q35).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssistStyle {
    /// The model writes the answer; [`coach_ask::ground_with_text`] takes out what it invents.
    #[default]
    Free,
    /// The model only picks and orders the guide's numbered steps by key; FilmCraft writes the
    /// answer from the guide's own text, so nothing can be invented. Provisional (Q35 (b)): not
    /// the library default, but [`DEFAULT_STYLE`] for the command.
    Pick,
}

/// The style `coach.assist` (and so the Ask the coach box) uses when none is given: pick, since
/// the 85-case eval gave pick 69 / 85 auto-pass with nothing invented against free's 7 / 85
/// (2026-10-14, synthetic). Provisional (owner question Q35); `AssistStyle::default()` stays free
/// for library callers.
pub const DEFAULT_STYLE: AssistStyle = AssistStyle::Pick;

impl AssistStyle {
    /// From the `style` command parameter (`free` / `pick`).
    pub fn parse(s: &str) -> Option<AssistStyle> {
        match s {
            "free" => Some(AssistStyle::Free),
            "pick" => Some(AssistStyle::Pick),
            _ => None,
        }
    }
}

/// Most characters of the earlier question / answer a follow-up carries.
pub const MAX_PREVIOUS_CHARS: usize = 800;

/// A prompt ready for [`filmcraft_llm::chat_prompt`].
#[derive(Clone, Debug, PartialEq)]
pub struct AssistPrompt {
    pub system: String,
    pub user: String,
    /// The source ids sent, best first (a section without a header is `page#heading`).
    pub sources: Vec<String>,
    /// The guide text sent (what a named control must come from, [`coach_ask::ground_with_text`]).
    pub sent_text: String,
    /// When the best sections say FilmCraft doesn't have this yet: (section id, what they say).
    pub gap: Option<(String, String)>,
}

/// The source id of a retrieved section.
pub fn source_id(r: &Retrieved) -> String {
    match r.header {
        Some(h) => h.id.clone(),
        None => format!("{}#{}", r.section.page, r.section.heading),
    }
}

/// `s` cut to at most `max` characters.
fn chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// One section as the prompt carries it: id, heading, header line, numbered steps, and the full
/// text when `full`.
fn section_block(r: &Retrieved, full: bool) -> String {
    let mut out = format!("[{}] {} ({})\n", source_id(r), r.section.heading, r.section.page);
    if let Some(h) = &r.section.header {
        out.push_str(h.trim());
        out.push('\n');
    }
    let steps = coach_ask::numbered_list(&r.section.body);
    for (n, st) in steps.iter().take(coach_ask::MAX_STEPS).enumerate() {
        out.push_str(&format!("{}. {}\n", n.saturating_add(1), chars(st, coach_ask::MAX_STEP_TEXT)));
    }
    if full || steps.is_empty() {
        let body = if full { r.section.body.as_str() } else { r.section.body.split("\n\n").next().unwrap_or("") };
        out.push_str(&chars(body.trim(), MAX_BODY_CHARS));
        out.push('\n');
    }
    out
}

/// Most new tokens when the answer is "not yet, but here is the workaround".
pub const GAP_ANSWER_TOKENS: usize = 150;

/// What section `r` says FilmCraft lacks: its "**FilmCraft:** not yet …" line (practice notes)
/// or its "Planned: …" text (the guide's "later" sections), as plain text.
fn gap_line(r: &Retrieved) -> Option<String> {
    let body = r.section.body.as_str();
    let line = body.lines().map(str::trim).find(|l| {
        let l = l.trim_start_matches(['-', '*', ' ']).to_lowercase().replace("**", "");
        if l.starts_with("planned:") {
            return true;
        }
        // "FilmCraft: not yet …", "FilmCraft: no outside node yet; …", "… are not yet built"
        let Some(rest) = l.strip_prefix("filmcraft:") else { return false };
        let clause = rest.split([';', '.']).next().unwrap_or("");
        let words: Vec<&str> = clause.split(|c: char| !c.is_alphanumeric()).collect();
        words.contains(&"yet") && (words.contains(&"no") || words.contains(&"not"))
    })?;
    // the rest of that bullet (wrapped lines)
    let mut text = line.to_string();
    let rest = body.split(line).nth(1).unwrap_or("");
    for l in rest.lines().skip(1) {
        if l.trim().is_empty() || l.trim_start().starts_with('-') {
            break;
        }
        text.push(' ');
        text.push_str(l.trim());
    }
    Some(chars(text.replace("**", "").trim_start_matches(['-', ' ']), 400))
}

/// A section further down counts for the gap when it scores at least this share of the best.
const GAP_SCORE: f64 = 0.7;
/// How many of the best sections may state the gap.
const GAP_SECTIONS: usize = 1;

/// The gap the best sections state (the best of the first three that score nearly as well).
fn gap_of(found: &[Retrieved]) -> Option<(String, String)> {
    let top = found.first()?.score;
    found.iter().take(GAP_SECTIONS).filter(|r| r.score >= GAP_SCORE * top).find_map(|r| gap_line(r).map(|g| (source_id(r), g)))
}

/// The prompt for `question`, at most `budget_chars` characters in all (clamped to
/// [`MAX_BUDGET_CHARS`]; the system text and question always go in). Sections that don't fit
/// are left out (a full text falls back to header + steps first).
pub fn assistant_prompt(question: &str, ctx: &AssistContext, budget_chars: usize) -> AssistPrompt {
    let budget = budget_chars.min(MAX_BUDGET_CHARS);
    let question = filmcraft_llm::sanitize(&chars(question.trim(), MAX_QUESTION_CHARS));
    let mut context = String::new();
    for (label, v) in [("Page", &ctx.page), ("Selected", &ctx.selection), ("Current grade (facts)", &ctx.grade_facts_json)] {
        if let Some(v) = v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            context.push_str(&format!("{label}: {}\n", chars(v, MAX_CONTEXT_CHARS)));
        }
    }
    let context = filmcraft_llm::sanitize(&context);
    let mut head = if context.is_empty() { String::new() } else { format!("On screen:\n{context}\n") };
    let prev = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(|v| filmcraft_llm::sanitize(&chars(v, MAX_PREVIOUS_CHARS)));
    let previous_question = prev(&ctx.previous_question);
    if let Some(pq) = &previous_question {
        head.push_str(&format!("Earlier in this conversation:\nQuestion: {pq}\n"));
        if let Some(pa) = prev(&ctx.previous_answer) {
            head.push_str(&format!("Your answer: {pa}\n"));
        }
        head.push_str("The new question may follow up on it.\n\n");
    }
    let tail = format!("Question: {question}");
    let fixed = SYSTEM.chars().count().saturating_add(head.chars().count()).saturating_add(tail.chars().count()).saturating_add("Guide sections:\n\n".len());
    let mut left = budget.saturating_sub(fixed);
    let mut blocks = String::new();
    let mut sources = Vec::new();
    // a follow-up ("and how do I undo that?") is about the earlier question's topic too
    let query = previous_question.as_ref().map_or_else(|| question.clone(), |pq| format!("{pq} {question}"));
    let found = coach_ask::retrieve(&query, SECTIONS.min(MAX_RETRIEVE));
    let gap = gap_of(&found);
    let gap_note = gap.as_ref().map(|(id, g)| {
        filmcraft_llm::sanitize(&format!(
            "Important, what FilmCraft doesn't have yet [{id}]: {g}\nSay this first, then give only the workaround the sections describe. Describe no steps for what is missing.\n\n"
        ))
    });
    left = left.saturating_sub(gap_note.as_ref().map_or(0, |n| n.chars().count()));
    for (i, r) in found.iter().enumerate() {
        let tries = if i < FULL_TEXT { [true, false].as_slice() } else { [false].as_slice() };
        for &full in tries {
            let b = filmcraft_llm::sanitize(&section_block(r, full));
            let n = b.chars().count().saturating_add(1);
            if n <= left {
                left = left.saturating_sub(n);
                blocks.push_str(&b);
                blocks.push('\n');
                sources.push(source_id(r));
                break;
            }
        }
    }
    let note = gap_note.unwrap_or_default();
    let user =
        if sources.is_empty() { format!("{head}Guide sections: none found.\n\n{tail}") } else { format!("{head}Guide sections:\n{blocks}\n{note}{tail}") };
    AssistPrompt { system: SYSTEM.to_string(), user, sources, sent_text: blocks, gap }
}

// ---------- pick-the-steps answers (AI3.6, Q35 (b)) ----------

const PICK_SYSTEM: &str = "You are FilmCraft's editing and grading assistant. \
You answer only by choosing steps from the guide sections given. Every step has a key such as A1 or B3. \
Reply with the keys of the steps that answer the question, in the order to do them, at most 6, separated by spaces. \
A step that answers a yes / no question by saying what FilmCraft does counts as an answer. \
Write nothing else. If no step answers the question, reply NONE.";

/// Most new tokens for a list of step keys.
pub const PICK_ANSWER_TOKENS: usize = 40;
/// Most steps offered per section, and most steps picked.
pub const PICK_STEPS: usize = 8;
/// Most steps an answer keeps.
pub const PICK_MAX: usize = 6;
/// Picked steps from a section scoring below this share of the best section's retrieval score are
/// dropped (unless that leaves nothing): SmolLM3-3B pads answers with runs of keys from unrelated
/// sections. 0.5 dropped 39 padded lines of the 85-case eval and no expected source
/// (docs/fork/coach-eval/AI3-MUST-READ.md); 0.6 already lost useful steps.
pub const PICK_MIN_RELEVANCE: f64 = 0.5;

/// One step the model may pick.
#[derive(Clone, Debug, PartialEq)]
pub struct PickStep {
    /// "A1", "B3": section letter, step number.
    pub key: String,
    /// The section it comes from.
    pub source: String,
    /// The guide's text of the step.
    pub text: String,
    /// Its section's retrieval score as a share of the best section's (0..=1).
    pub relevance: f64,
}

/// A pick-the-steps prompt: like [`AssistPrompt`], plus the keyed steps.
#[derive(Clone, Debug, PartialEq)]
pub struct PickPrompt {
    pub prompt: AssistPrompt,
    pub steps: Vec<PickStep>,
}

/// The steps a section offers: its numbered steps, or its first paragraph as one step.
fn section_steps(r: &Retrieved) -> Vec<String> {
    let mut steps = coach_ask::numbered_list(&r.section.body);
    if steps.is_empty() {
        let para = r.section.body.split("\n\n").map(str::trim).find(|p| !p.is_empty() && !p.starts_with('<') && !p.starts_with('#')).unwrap_or("");
        if para.starts_with("- ") || para.starts_with("* ") {
            // a bullet list: one step per bullet (wrapped lines joined)
            for l in para.lines().map(str::trim) {
                match l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
                    Some(item) => steps.push(item.to_string()),
                    None => {
                        if let Some(last) = steps.last_mut() {
                            last.push(' ');
                            last.push_str(l);
                        }
                    }
                }
            }
        } else if !para.is_empty() {
            steps.push(para.to_string());
        }
    }
    steps.into_iter().take(PICK_STEPS).map(|s| chars(s.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), coach_ask::MAX_STEP_TEXT)).collect()
}

/// The pick-the-steps prompt for `question` (same retrieval, context and gap note as
/// [`assistant_prompt`]), at most `budget_chars` characters; a section that doesn't fit, or whose picks
/// [`parse_picks`] would drop (relevance below [`PICK_MIN_RELEVANCE`]), is left out.
pub fn pick_prompt(question: &str, ctx: &AssistContext, budget_chars: usize) -> PickPrompt {
    let free = assistant_prompt(question, ctx, budget_chars);
    let budget = budget_chars.min(MAX_BUDGET_CHARS);
    let query = ctx.previous_question.as_deref().map(str::trim).filter(|v| !v.is_empty()).map_or_else(
        || filmcraft_llm::sanitize(&chars(question.trim(), MAX_QUESTION_CHARS)),
        |pq| filmcraft_llm::sanitize(&format!("{} {}", chars(pq, MAX_PREVIOUS_CHARS), chars(question.trim(), MAX_QUESTION_CHARS))),
    );
    let found = coach_ask::retrieve(&query, SECTIONS.min(MAX_RETRIEVE));
    // the free prompt's head (context, earlier exchange) and tail (gap note, question) are reused
    let (head, rest) = free.user.split_once("Guide sections:").unwrap_or(("", free.user.as_str()));
    let asked = rest.rfind("Question: ").and_then(|i| rest.get(i..)).unwrap_or("");
    let tail = match &free.gap {
        Some((id, g)) => filmcraft_llm::sanitize(&format!(
            "Note, FilmCraft doesn't have this yet [{id}]: {g}\nPick only steps of the workaround the sections describe, or NONE.\n\n{asked}"
        )),
        None => asked.to_string(),
    };
    let fixed =
        PICK_SYSTEM.chars().count().saturating_add(head.chars().count()).saturating_add(tail.chars().count()).saturating_add("Guide sections:\n\n".len());
    let mut left = budget.saturating_sub(fixed);
    let (mut blocks, mut sources, mut steps) = (String::new(), Vec::new(), Vec::new());
    let letters = ('A'..='Z').take(SECTIONS);
    let best = found.iter().map(|r| r.score).fold(0.0_f64, f64::max);
    for (r, letter) in found.iter().zip(letters) {
        let id = source_id(r);
        let texts = section_steps(r);
        let relevance = if best > 0.0 && r.score.is_finite() { (r.score / best).clamp(0.0, 1.0) } else { 1.0 };
        // a section whose picks `parse_picks` would drop only costs prefill time
        if texts.is_empty() || relevance < PICK_MIN_RELEVANCE {
            continue;
        }
        let mut b = format!("[{id}] {} ({})\n", r.section.heading, r.section.page);
        let mut keyed = Vec::new();
        for (n, t) in texts.iter().enumerate() {
            let key = format!("{letter}{}", n.saturating_add(1));
            b.push_str(&format!("{key}: {t}\n"));
            keyed.push(PickStep { key, source: id.clone(), text: t.clone(), relevance });
        }
        let b = filmcraft_llm::sanitize(&b);
        let n = b.chars().count().saturating_add(1);
        if n > left {
            continue;
        }
        left = left.saturating_sub(n);
        blocks.push_str(&b);
        blocks.push('\n');
        sources.push(id);
        steps.extend(keyed);
    }
    let user = if sources.is_empty() { format!("{head}Guide sections: none found.\n\n{tail}") } else { format!("{head}Guide sections:\n{blocks}\n{tail}") };
    PickPrompt { prompt: AssistPrompt { system: PICK_SYSTEM.to_string(), user, sources, sent_text: blocks, gap: free.gap }, steps }
}

/// The steps a model reply picks, in its order (unknown keys ignored, repeats once, steps below
/// [`PICK_MIN_RELEVANCE`] dropped while others remain, at most [`PICK_MAX`]); empty for "NONE" or
/// nothing usable.
pub fn parse_picks<'a>(reply: &str, steps: &'a [PickStep]) -> Vec<&'a PickStep> {
    let mut out = picked_keys(reply, steps);
    if out.iter().any(|s| s.relevance >= PICK_MIN_RELEVANCE) {
        out.retain(|s| s.relevance >= PICK_MIN_RELEVANCE);
    }
    out.truncate(PICK_MAX);
    out
}

/// Every known step a reply names, in its order, each once.
fn picked_keys<'a>(reply: &str, steps: &'a [PickStep]) -> Vec<&'a PickStep> {
    let mut out: Vec<&PickStep> = Vec::new();
    for word in reply.split(|c: char| !c.is_ascii_alphanumeric()).take(200) {
        let w = word.to_ascii_uppercase();
        if let Some(st) = steps.iter().find(|s| s.key == w)
            && !out.iter().any(|o| o.key == st.key)
        {
            out.push(st);
        }
    }
    out
}

/// The answer text the picked steps make: one guide step per line with its section's citation.
fn picked_text(picks: &[&PickStep]) -> String {
    picks.iter().enumerate().map(|(n, s)| format!("{}. {} [{}]", n.saturating_add(1), s.text, s.source)).collect::<Vec<_>>().join("\n")
}

// ---------- answering (AI3.3c) ----------

/// Most new tokens of an assistant answer (~6 tokens/s on the reference PC: about half a minute).
pub const MAX_ANSWER_TOKENS: usize = 200;
/// Time for one assistant answer, prompt included.
pub const ANSWER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(90);

/// Why the keyword coach answered instead of the model.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum AssistFallback {
    /// The model can't run now (not installed, no memory, this build, …).
    NotReady(String),
    /// Loading or generating failed.
    Error(String),
    /// Nothing of the model's text survived the grounding check.
    Ungrounded,
    /// No guide section matched the question, so the model wasn't asked: "I don't know" beats a
    /// guess (AI-PLAN AI3 item 4).
    NothingFound,
}

/// The assistant's answer: the model's grounded text, or the keyword coach's answer and why.
#[derive(Clone, Debug, Default)]
pub struct AssistAnswer {
    /// The sections the model was given.
    pub sources: Vec<String>,
    /// The model's answer after [`coach_ask::ground`] (None when the coach answered).
    pub grounded: Option<coach_ask::Grounded>,
    /// Set when the keyword coach answered instead.
    pub fallback: Option<AssistFallback>,
    /// The model said the sections don't answer the question (its lines then name the closest
    /// section; the keyword coach's answer comes along in `coach`).
    pub dont_know: bool,
    /// The lesson that shows the model's answer ("Show me"), as (id, title).
    pub lesson: Option<(String, String)>,
    pub coach: Option<coach_ask::Answer>,
}

/// Whether a grounded answer opens by saying the guide doesn't cover the question.
fn says_dont_know(g: &coach_ask::Grounded) -> bool {
    const SAYS: [&str; 8] = ["don't know", "do not know", "not sure", "don't cover", "do not cover", "doesn't cover", "does not cover", "not covered"];
    g.lines.iter().take(2).any(|l| {
        let t = l.text.to_lowercase().replace('\u{2019}', "'");
        SAYS.iter().any(|w| t.contains(w))
    })
}

/// Answer `question` with `model` on `backend` (blocking: run it off the UI thread), grounded
/// against `checks`; the keyword coach answers when the model can't run, fails, or none of its
/// text passes the check.
pub fn answer(backend: &dyn ExplainBackend, model: &filmcraft_llm::ModelInfo, question: &str, ctx: &AssistContext, checks: &coach_ask::Checks) -> AssistAnswer {
    answer_streaming(backend, model, question, ctx, checks, &mut |_| true)
}

/// [`answer`], calling `on_text` with the model's unchecked text so far as it grows (for a live
/// preview; only the returned answer is grounded). `on_text` returning false stops the model,
/// and the keyword coach answers.
pub fn answer_streaming(
    backend: &dyn ExplainBackend,
    model: &filmcraft_llm::ModelInfo,
    question: &str,
    ctx: &AssistContext,
    checks: &coach_ask::Checks,
    on_text: &mut dyn FnMut(&str) -> bool,
) -> AssistAnswer {
    let (prompt, steps) = match ctx.style {
        AssistStyle::Free => (assistant_prompt(question, ctx, DEFAULT_BUDGET_CHARS), Vec::new()),
        AssistStyle::Pick => {
            let p = pick_prompt(question, ctx, DEFAULT_BUDGET_CHARS);
            (p.prompt, p.steps)
        }
    };
    let pick = ctx.style == AssistStyle::Pick;
    let coach = |sources: Vec<String>, why: AssistFallback| AssistAnswer {
        sources,
        grounded: None,
        fallback: Some(why),
        dont_know: false,
        lesson: None,
        coach: Some(coach_ask::ask(question, checks)),
    };
    if prompt.sources.is_empty() {
        return coach(prompt.sources, AssistFallback::NothingFound);
    }
    if let Err((_, why)) = backend.ready(model) {
        return coach(prompt.sources, AssistFallback::NotReady(why));
    }
    let max_tokens = if pick {
        PICK_ANSWER_TOKENS
    } else if prompt.gap.is_some() {
        GAP_ANSWER_TOKENS
    } else {
        MAX_ANSWER_TOKENS
    };
    let opts = filmcraft_llm::GenOptions { max_tokens, deadline: ANSWER_DEADLINE, ..model.recommended_options() };
    let mut stopped = false;
    let mut watch = |t: &str| {
        // a pick reply is keys: the preview shows the steps they name
        let go = if pick { on_text(&picked_text(&parse_picks(t, &steps))) } else { on_text(t) };
        stopped |= !go;
        go
    };
    let text = match backend.generate_streaming(model, &prompt.system, &prompt.user, opts, &mut watch) {
        Ok(_) if stopped => return coach(prompt.sources, AssistFallback::Error("stopped before the answer was finished".into())),
        Ok(t) if pick => {
            let picks = parse_picks(&t, &steps);
            if picks.is_empty() && prompt.gap.is_none() {
                // "NONE": the guide's closest section, and the keyword coach's answer with it
                let closest = prompt.sources.first().cloned().unwrap_or_default();
                format!("The guide sections found don't cover this; the closest is [{closest}].")
            } else {
                picked_text(&picks)
            }
        }
        Ok(t) => t,
        Err(e) => return coach(prompt.sources, AssistFallback::Error(e)),
    };
    // picked steps are the guide's own text, whose ids the guide's tests check against the app
    // (the `coach_ui` walk); a session without the UI's id list would wrongly drop them
    let all = |_: &str| true;
    let guide_checks = coach_ask::Checks { command: &all, namespace: &all, ui: Some(&all) };
    let mut g = coach_ask::ground_with_text(&text, &prompt.sources, &prompt.sent_text, if pick { &guide_checks } else { checks });
    // a missing feature is said first, whatever the model wrote
    if let Some((id, gap)) = &prompt.gap {
        let said = g.lines.first().is_some_and(|l| {
            let t = l.text.to_lowercase();
            t.contains("not yet") || t.contains("doesn't have") || t.contains("does not have") || t.contains("not available")
        });
        if !said {
            g.lines.insert(0, coach_ask::GroundedLine { text: format!("{gap} [{id}]"), general: false, ui: Vec::new(), commands: Vec::new() });
            if !g.cited.contains(id) {
                g.cited.insert(0, id.clone());
            }
        }
    }
    if g.lines.is_empty() {
        return coach(prompt.sources, AssistFallback::Ungrounded);
    }
    // "I don't know": keep the model's pointer to the closest section, and give the keyword
    // coach's answer with it
    let dont_know = prompt.gap.is_none() && says_dont_know(&g);
    let coach = dont_know.then(|| coach_ask::ask(question, checks));
    let lesson = if dont_know { None } else { coach_ask::lesson_for(question, if g.cited.is_empty() { &prompt.sources } else { &g.cited }) };
    AssistAnswer { sources: prompt.sources, grounded: Some(g), fallback: None, dont_know, lesson, coach }
}

// ---------- jobs and commands (AI3.3d) ----------

/// The assistant's model until a preference picks another (FORK-PLAN §1a.74).
pub const ASSIST_MODEL: &str = "smollm3-3b-q4km";
/// Answers kept for `coach.assistResult` (the oldest go first).
const MAX_KEPT: usize = 32;
/// Questions waiting or running at once.
pub const MAX_PENDING: usize = 4;
/// The longest live preview `coach.assistResult` returns (chars).
pub const MAX_PARTIAL_CHARS: usize = 16_000;

impl AssistAnswer {
    pub fn to_json(&self) -> Value {
        let lines: Vec<Value> =
            self.grounded.iter().flat_map(|g| &g.lines).map(|l| json!({"text": l.text, "general": l.general, "ui": l.ui, "commands": l.commands})).collect();
        let g = self.grounded.as_ref();
        json!({
            "source": if self.grounded.is_some() { "model" } else { "coach" },
            "sources": self.sources,
            "lines": lines,
            "cited": g.map(|g| g.cited.clone()).unwrap_or_default(),
            "removedLines": g.map_or(0, |g| g.removed_lines),
            "removedIds": g.map(|g| g.removed_ids.clone()).unwrap_or_default(),
            "inventedCitations": g.map(|g| g.invented_citations.clone()).unwrap_or_default(),
            "fallback": self.fallback,
            "dontKnow": self.dont_know,
            "lesson": self.lesson.as_ref().map(|(id, title)| json!({"id": id, "title": title})),
            "coach": self.coach.as_ref().map(coach_ask::Answer::to_json),
        })
    }
}

/// Questions being answered in the background, by id.
#[derive(Default)]
pub struct AssistJobs {
    /// `None` while the answer is being written.
    entries: Arc<Mutex<BTreeMap<u64, Option<AssistAnswer>>>>,
    /// The model's unchecked text so far, while an answer is being written.
    partial: Arc<Mutex<BTreeMap<u64, String>>>,
    next: u64,
    /// One model answer at a time.
    gate: Arc<Mutex<()>>,
    /// A backend for tests; `None`: the process's real model backend.
    pub backend: Option<Arc<dyn ExplainBackend>>,
}

impl std::fmt::Debug for AssistJobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistJobs").field("next", &self.next).finish_non_exhaustive()
    }
}

/// The registered command ids at the time of the question (the worker can't read the session).
struct Snapshot {
    ids: HashSet<String>,
    ui: Option<Arc<dyn coach_ask::UiIdCheck>>,
}

impl Snapshot {
    fn of(s: &Session) -> Snapshot {
        Snapshot { ids: s.shortcuts.commands().into_iter().map(|c| c.id).collect(), ui: s.coach_ids.clone() }
    }

    fn with<R>(&self, f: impl FnOnce(&coach_ask::Checks) -> R) -> R {
        let command = |id: &str| self.ids.contains(id);
        let namespace = |ns: &str| !ns.is_empty() && self.ids.iter().any(|id| id.strip_prefix(ns).is_some_and(|r| r.starts_with('.')));
        let ui_fn = |id: &str| self.ui.as_ref().is_some_and(|c| c.known_ui_id(id));
        f(&coach_ask::Checks { command: &command, namespace: &namespace, ui: self.ui.is_some().then_some(&ui_fn as &dyn Fn(&str) -> bool) })
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AssistJobs {
    /// Start answering; the id to ask [`AssistJobs::result`] with. Errors when too many wait.
    fn start(&mut self, snap: Snapshot, model: &'static filmcraft_llm::ModelInfo, question: String, ctx: AssistContext) -> std::result::Result<u64, String> {
        let mut entries = lock(&self.entries);
        if entries.values().filter(|e| e.is_none()).count() >= MAX_PENDING {
            return Err(format!("{MAX_PENDING} questions are already being answered; wait for one"));
        }
        self.next = self.next.saturating_add(1);
        let id = self.next;
        while entries.len() >= MAX_KEPT {
            let Some(oldest) = entries.keys().next().copied() else { break };
            entries.remove(&oldest);
            lock(&self.partial).remove(&oldest);
        }
        let backend = self.backend.clone().or_else(|| crate::ai_explain::real_backend().map(|b| b as Arc<dyn ExplainBackend>));
        let Some(backend) = backend else {
            let a = snap.with(|c| AssistAnswer {
                sources: assistant_prompt(&question, &ctx, DEFAULT_BUDGET_CHARS).sources,
                grounded: None,
                fallback: Some(AssistFallback::NotReady("this build can't run a model (built without the `llm` feature)".into())),
                dont_know: false,
                lesson: None,
                coach: Some(coach_ask::ask(&question, c)),
            });
            entries.insert(id, Some(a));
            return Ok(id);
        };
        entries.insert(id, None);
        drop(entries);
        let (store, partial, gate) = (self.entries.clone(), self.partial.clone(), self.gate.clone());
        let spawned = std::thread::Builder::new().name("filmcraft-assist".into()).spawn(move || {
            let _turn = lock(&gate);
            if !lock(&store).contains_key(&id) {
                return; // forgotten while it waited
            }
            // a question forgotten while it waited or ran stops the model
            let mut preview = |t: &str| {
                if !lock(&store).contains_key(&id) {
                    return false;
                }
                lock(&partial).insert(id, chars(t, MAX_PARTIAL_CHARS));
                true
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                snap.with(|c| answer_streaming(backend.as_ref(), model, &question, &ctx, c, &mut preview))
            }));
            lock(&partial).remove(&id);
            let a = r.unwrap_or_else(|_| AssistAnswer {
                sources: Vec::new(),
                grounded: None,
                fallback: Some(AssistFallback::Error("the assistant stopped unexpectedly".into())),
                dont_know: false,
                lesson: None,
                coach: snap.with(|c| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| coach_ask::ask(&question, c))).ok()),
            });
            // (a forgotten question stays forgotten)
            if let Some(slot) = lock(&store).get_mut(&id) {
                *slot = Some(a);
            }
        });
        if let Err(e) = spawned {
            lock(&self.entries).remove(&id);
            return Err(format!("could not start the assistant: {e}"));
        }
        Ok(id)
    }

    /// Forget a question: a running answer stops at its next words, a waiting one never starts
    /// the model. False: unknown id.
    pub fn forget(&self, id: u64) -> bool {
        let known = lock(&self.entries).remove(&id).is_some();
        lock(&self.partial).remove(&id);
        known
    }

    /// The model's unchecked text so far for a question still being answered.
    pub fn partial(&self, id: u64) -> Option<String> {
        lock(&self.partial).get(&id).cloned()
    }

    /// `None`: unknown id; `Some(None)`: still being written.
    pub fn result(&self, id: u64) -> Option<Option<AssistAnswer>> {
        lock(&self.entries).get(&id).cloned()
    }

    /// [`AssistJobs::result`], waiting up to `cap` for it.
    pub fn wait(&self, id: u64, cap: std::time::Duration) -> Option<Option<AssistAnswer>> {
        let t = web_time::Instant::now();
        loop {
            match self.result(id) {
                // (the web build has no model, so its answers are ready at once: it never sleeps)
                Some(None) if t.elapsed() < cap && !cfg!(target_arch = "wasm32") => std::thread::sleep(std::time::Duration::from_millis(20)),
                other => return other,
            }
        }
    }
}

fn opt_text(p: &Value, k: &str) -> Option<String> {
    p.get(k).and_then(Value::as_str).map(|v| chars(v, MAX_CONTEXT_CHARS))
}

fn assist_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let q = p.get("question").and_then(Value::as_str).ok_or_else(|| bad("coach.assist", "need `question` (text)"))?;
    let model_id = p.get("model").and_then(Value::as_str).unwrap_or(ASSIST_MODEL);
    let model = filmcraft_llm::models::find(model_id).ok_or_else(|| bad("coach.assist", format!("unknown model `{}`", chars(model_id, 80))))?;
    let ctx = AssistContext {
        page: opt_text(p, "page"),
        selection: opt_text(p, "selection"),
        grade_facts_json: opt_text(p, "gradeFacts"),
        previous_question: opt_text(p, "previousQuestion"),
        previous_answer: opt_text(p, "previousAnswer"),
        style: match p.get("style").and_then(Value::as_str) {
            None => DEFAULT_STYLE,
            Some(v) => AssistStyle::parse(v).ok_or_else(|| bad("coach.assist", "`style` is `free` or `pick`"))?,
        },
    };
    let snap = Snapshot::of(s);
    let id = s.assist.start(snap, model, chars(q, MAX_QUESTION_CHARS), ctx).map_err(|e| bad("coach.assist", e))?;
    Ok(json!({"id": id, "model": model.id}))
}

fn result_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad("coach.assistResult", "need `id` (from coach.assist)"))?;
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(false);
    let r = if wait { s.assist.wait(id, ANSWER_DEADLINE.saturating_add(std::time::Duration::from_secs(30))) } else { s.assist.result(id) };
    Ok(match r {
        None => json!({"state": "unknown"}),
        // `partial`: the model's text so far, not yet checked against the guide
        Some(None) => json!({"state": "pending", "partial": s.assist.partial(id)}),
        Some(Some(a)) => {
            let mut v = a.to_json();
            v["state"] = json!("done");
            v
        }
    })
}

fn forget_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad("coach.assistForget", "need `id` (from coach.assist)"))?;
    Ok(json!({"forgotten": s.assist.forget(id)}))
}

// ---------- AI3.6: scoring the assistant on the advanced set ----------

/// One question of `docs/fork/coach-eval/advanced.toml`.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub kind: String,
    pub q: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub must: Vec<String>,
    #[serde(default)]
    pub must_not: Vec<String>,
}

#[derive(serde::Deserialize)]
struct EvalFile {
    #[serde(default)]
    question: Vec<EvalCase>,
}

/// Most cases read from one eval file (a hostile file can't make the run endless).
pub const MAX_EVAL_CASES: usize = 1000;

/// Parses an advanced-set file (at most [`MAX_EVAL_CASES`] cases).
pub fn parse_eval_set(text: &str) -> std::result::Result<Vec<EvalCase>, String> {
    let mut f: EvalFile = toml::from_str(text).map_err(|e| e.to_string())?;
    f.question.truncate(MAX_EVAL_CASES);
    Ok(f.question)
}

/// The machine-checkable half of a case's score. `must` facts need a reader (person or judge):
/// they come back in `review`, so `auto_pass` is an upper bound on "answered correctly".
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalScore {
    pub id: String,
    /// The model answered (not the keyword coach, not "I don't know").
    pub model_answered: bool,
    /// At least one cited section is one of the case's `sources`.
    pub cites_source: bool,
    /// Lines with unknown controls / commands / labels plus citations of sections not sent:
    /// any of them fails the case (AI-PLAN AI3 accept: "any invented one fails the case").
    pub invented: usize,
    /// `gap` cases: the answer says the feature isn't there yet (None for other kinds).
    pub states_gap: Option<bool>,
    pub auto_pass: bool,
    /// Why it failed, in a few words (empty when it passed).
    pub why: Vec<&'static str>,
    /// The case's `must` facts, for the reader.
    pub review: Vec<String>,
}

/// Whether a line says a feature isn't in FilmCraft yet.
fn says_gap(text: &str) -> bool {
    const SAYS: [&str; 7] = ["not yet", "not available", "doesn't have", "does not have", "isn't available", "planned", "not in filmcraft"];
    let t = text.to_lowercase().replace('\u{2019}', "'");
    // "FilmCraft: no outside node yet; …": a clause with both "no" and "yet"
    let no_yet = t.split([';', '.', ',']).any(|c| {
        let w: Vec<&str> = c.split(|ch: char| !ch.is_alphanumeric()).collect();
        w.contains(&"no") && w.contains(&"yet")
    });
    no_yet || SAYS.iter().any(|w| t.contains(w))
}

/// Scores one answer against its case (AI3.6).
pub fn score_case(case: &EvalCase, a: &AssistAnswer) -> EvalScore {
    let mut s = EvalScore { id: case.id.clone(), review: case.must.clone(), ..EvalScore::default() };
    let Some(g) = a.grounded.as_ref() else {
        s.why.push("the keyword coach answered");
        return s;
    };
    s.model_answered = !a.dont_know;
    if a.dont_know {
        s.why.push("said it doesn't know");
    }
    s.cites_source = g.cited.iter().any(|c| case.sources.contains(c));
    if !s.cites_source {
        s.why.push("no expected source cited");
    }
    s.invented = g.removed_lines.saturating_add(g.invented_citations.len());
    if s.invented > 0 {
        s.why.push("invented a control, command or citation");
    }
    if case.kind == "gap" {
        let said = g.lines.iter().take(3).any(|l| says_gap(&l.text));
        s.states_gap = Some(said);
        if !said {
            s.why.push("didn't say the feature isn't there yet");
        }
    }
    s.auto_pass = s.why.is_empty();
    s
}

/// Totals over a run of [`score_case`].
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalSummary {
    pub cases: usize,
    pub auto_pass: usize,
    pub model_answered: usize,
    pub cites_source: usize,
    /// Cases with at least one invented control / command / citation.
    pub invented_cases: usize,
    /// Auto-pass rate in percent (0 when there are no cases).
    pub percent: f64,
}

pub fn summarize(scores: &[EvalScore]) -> EvalSummary {
    let count = |f: &dyn Fn(&EvalScore) -> bool| scores.iter().filter(|s| f(s)).count();
    let cases = scores.len();
    let auto_pass = count(&|s| s.auto_pass);
    EvalSummary {
        cases,
        auto_pass,
        model_answered: count(&|s| s.model_answered),
        cites_source: count(&|s| s.cites_source),
        invented_cases: count(&|s| s.invented > 0),
        percent: if cases == 0 { 0.0 } else { auto_pass as f64 * 100.0 / cases as f64 },
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "coach.assist",
            label: "Ask the Assistant",
            menu: &[],
            shortcut: None,
            params: r#"{"question":str,"page":str?,"selection":str?,"gradeFacts":str?,"model":str?,"previousQuestion":str?,"previousAnswer":str?,"style":str?}"#,
            enabled: always,
            run: assist_cmd,
            journal: false,
        },
        CommandSpec {
            id: "coach.assistResult",
            label: "Assistant Answer",
            menu: &[],
            shortcut: None,
            params: r#"{"id":int,"wait":bool?}"#,
            enabled: always,
            run: result_cmd,
            journal: false,
        },
        CommandSpec {
            id: "coach.assistForget",
            label: "Stop the Assistant's Answer",
            menu: &[],
            shortcut: None,
            params: r#"{"id":int}"#,
            enabled: always,
            run: forget_cmd,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "coach_assist_tests.rs"]
mod tests;
