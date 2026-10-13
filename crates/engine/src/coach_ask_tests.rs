//! "Ask the coach" (fork GRADE-PLAN G2.12, part 2): the guide index, the 30-question set, the
//! command check and hostile questions.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::Session;
use crate::coach_ask::{self, Checks, GUIDE_PAGES, stem, tokenize};

#[derive(Deserialize)]
struct QuestionFile {
    question: Vec<Question>,
    offtopic: Vec<OffTopic>,
}

#[derive(Deserialize)]
struct Question {
    q: String,
    page: String,
    heading: String,
}

#[derive(Deserialize)]
struct OffTopic {
    ask: String,
}

fn question_file() -> QuestionFile {
    toml::from_str(include_str!("../../../assets/coach/questions.toml")).expect("questions.toml parses")
}

fn questions() -> Vec<Question> {
    question_file().question
}

/// The session's checks without a UI, plus an automation-id check that accepts everything.
fn with_ui(q: &str) -> coach_ask::Answer {
    let s = Session::default();
    let command = |id: &str| s.shortcuts.is_known_command(id);
    let namespace = |ns: &str| s.shortcuts.is_command_namespace(ns);
    let ui = |_: &str| true;
    coach_ask::ask(q, &Checks { command: &command, namespace: &namespace, ui: Some(&ui) })
}

fn ask(s: &mut Session, q: &str) -> Value {
    s.execute("coach.ask", json!({ "question": q })).expect("coach.ask answers")
}

#[test]
fn the_page_list_matches_the_guide_directory() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide");
    let mut on_disk = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).expect("docs/guide is readable") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                on_disk.push(p.strip_prefix(&root).expect("under root").to_string_lossy().replace('\\', "/"));
            }
        }
    }
    on_disk.sort();
    let mut listed: Vec<String> = GUIDE_PAGES.iter().map(|(p, _)| p.to_string()).collect();
    listed.sort();
    assert_eq!(listed, on_disk, "add new guide pages to coach_ask::GUIDE_PAGES");
}

#[test]
fn every_indexed_section_has_provenance() {
    assert!(coach_ask::sections().len() > 50);
    for s in coach_ask::sections() {
        assert!(!s.provenance.is_empty(), "{} › {} has no provenance tag", s.page, s.heading);
    }
    assert!(!coach_ask::sections().iter().any(|s| s.page == "README.md"), "the guide's own README is not searched");
}

#[test]
fn tokenizing_normalises_stems_and_drops_stop_words() {
    assert_eq!(tokenize("How do I make the Colour grey?", 100), ["color", "gray"]);
    assert_eq!(stem("grading"), stem("graded"));
    assert_eq!(stem("grade"), stem("grades"));
    assert_eq!(stem("darker"), "dark");
    assert_eq!(stem("matches"), "match");
    assert_eq!(stem("cutting"), "cut");
    assert_eq!(stem("qualifiers"), stem("qualifier"));
    // non-ASCII words are kept whole
    assert_eq!(tokenize("Ñandú 天空", 100), ["ñandú", "天空"]);
}

#[test]
fn steps_come_from_the_numbered_list_with_their_ids() {
    let sec = coach_ask::sections().iter().find(|s| s.heading.starts_with("3. Make the sky bluer")).expect("the sky recipe");
    let steps = coach_ask::section_steps(&sec.body);
    assert_eq!(steps.len(), 7);
    assert_eq!(steps[1].ui, ["grade.tab.qualifier", "grade.qual.pick"]);
    assert_eq!(steps[1].commands, ["grade.pickQualifier"]);
    assert!(steps[1].text.starts_with("Open the Qualifier tab and click Pick, then click the sky"), "{}", steps[1].text);
    assert!(!steps.iter().any(|s| s.text.contains("**") || s.text.contains("{ui:")));
    assert_eq!(sec.provenance, ["[Resolve-style]"]);
}

#[test]
fn thirty_beginner_questions_find_their_section_and_pass_the_command_check() {
    let mut s = Session::default();
    let qs = questions();
    assert_eq!(qs.len(), 30);
    let mut hits = 0;
    let mut misses = Vec::new();
    for q in &qs {
        let best = coach_ask::rank(&q.q).first().and_then(|(i, _)| coach_ask::sections().get(*i)).map(|s| (s.page, s.heading.as_str()));
        if best == Some((q.page.as_str(), q.heading.as_str())) {
            hits += 1;
        } else {
            misses.push(format!("{:?} → {best:?} (want {} › {})", q.q, q.page, q.heading));
        }
        let a = ask(&mut s, &q.q);
        assert_eq!(a["covered"], json!(true), "{:?} is covered by the guide: {a}", q.q);
        for st in a["steps"].as_array().expect("steps") {
            for c in st["command"].as_str().into_iter().chain(st["commands"].as_array().into_iter().flatten().filter_map(Value::as_str)) {
                assert!(s.shortcuts.is_known_command(c), "{:?}: unknown command {c}", q.q);
            }
            assert!(st["ui"].is_null() && st["uis"].is_null(), "{:?}: no UI, so no automation ids: {st}", q.q);
            let text = st["text"].as_str().unwrap_or("");
            assert!(!text.is_empty() && !text.contains("{ui:") && !text.contains(",,") && !text.contains("()"), "{:?}: {text:?}", q.q);
        }
        assert!(!a["summary"].as_str().unwrap_or("").contains("{ui:"));
    }
    eprintln!("ask the coach: {hits}/30 best sections; misses: {misses:#?}");
    assert!(hits >= 28, "only {hits}/30 questions found their section first: {misses:#?}");
}

#[test]
fn questions_the_guide_does_not_cover_fall_back() {
    let mut s = Session::default();
    let off = question_file().offtopic;
    assert!(off.len() >= 8);
    for q in &off {
        let a = ask(&mut s, &q.ask);
        assert_eq!((a["covered"].clone(), a["fallback"].clone()), (json!(false), json!(true)), "{:?}: {a}", q.ask);
        assert_eq!(a["steps"], json!([]), "{:?}", q.ask);
        assert_eq!(a["sources"], json!([]), "{:?}", q.ask);
        assert!(a["message"].as_str().is_some_and(|m| m.starts_with("The guide doesn't cover this yet")), "{:?}: {a}", q.ask);
        assert!(a["lesson"].is_null(), "{:?}: no lesson is about this: {a}", q.ask);
    }
}

#[test]
fn unknown_ids_drop_their_step_and_are_counted() {
    let ok = |_: &str| true;
    let none = |_: &str| false;
    let not_pick = |id: &str| id != "grade.qual.pick";
    let q = "how do I make the sky bluer without changing skin?";
    let all = coach_ask::ask(q, &Checks { command: &ok, namespace: &ok, ui: Some(&ok) });
    assert_eq!((all.steps.len(), all.removed), (7, 0));
    assert_eq!(all.steps[1].ui, ["grade.tab.qualifier", "grade.qual.pick"]);
    let a = coach_ask::ask(q, &Checks { command: &ok, namespace: &ok, ui: Some(&not_pick) });
    assert_eq!((a.steps.len(), a.removed, a.removed_controls, a.removed_commands), (6, 1, 1, 0));
    assert_eq!(a.removed_ids, ["grade.qual.pick"]);
    // no command known: the steps naming one go; with every step gone, the lesson is offered
    let b = coach_ask::ask(q, &Checks { command: &none, namespace: &ok, ui: Some(&none) });
    assert!(b.steps.is_empty() && b.fallback && b.removed == 7);
    assert!(b.removed_controls > 0 && b.removed_commands > 0);
    assert_eq!(b.lesson.as_ref().map(|l| l.0.as_str()), Some("fix-skin"));
    // a word in an unknown namespace isn't a command: it can't be checked, so it is left out of
    // the text; without a UI no automation id is named
    let c = coach_ask::ask(q, &Checks { command: &none, namespace: &none, ui: None });
    assert_eq!((c.steps.len(), c.removed), (7, 0));
    assert!(c.steps.iter().all(|s| s.commands.is_empty() && s.ui.is_empty()));
    assert!(!c.steps[1].text.contains("grade.pickQualifier"), "{}", c.steps[1].text);
    assert!(c.steps[1].text.starts_with("Open the Qualifier tab and click Pick, then click the sky"), "{}", c.steps[1].text);
    assert!(!c.ui_checked);
}

#[test]
fn a_question_without_steps_offers_the_closest_lesson() {
    let mut s = Session::default();
    // "Undo" in nodes.md is prose only
    let a = ask(&mut s, "how do I undo a node");
    assert_eq!(a["sources"][0]["page"], json!("color/nodes.md"));
    assert_eq!(a["fallback"], json!(true));
    assert!(a["lesson"]["id"].is_string(), "{a}");
    assert!(a["message"].as_str().is_some_and(|m| m.starts_with("Try the lesson")), "{a}");
    assert!(!a["summary"].as_str().unwrap_or("").is_empty());
}

#[test]
fn provenance_is_passed_through() {
    let mut s = Session::default();
    let a = ask(&mut s, "how do I start a lesson?");
    assert_eq!(a["sources"][0]["page"], json!("color/coach.md"));
    assert_eq!(a["sources"][0]["provenance"], json!(["[Fork]"]));
}

#[test]
fn hostile_questions_answer_without_crashing() {
    let mut s = Session::default();
    assert!(s.execute("coach.ask", json!({})).is_err(), "no question");
    assert!(s.execute("coach.ask", json!({"question": 3})).is_err());
    for q in ["", "   ", "the and of to how what why", "\u{0}\u{1}\u{7f}\n\t\r", "🎨🎨🎨", "?!.,;"] {
        let a = ask(&mut s, q);
        assert_eq!(a["steps"], json!([]), "{q:?}");
        assert_eq!(a["fallback"], json!(true));
        assert_eq!(a["confidence"], json!(0.0));
    }
    let huge = "sky ".repeat(256 * 1024);
    let t = std::time::Instant::now();
    let a = ask(&mut s, &huge);
    assert!(t.elapsed().as_secs() < 5, "a 1 MB question is cut short");
    assert!(a["question"].as_str().is_some_and(|q| q.chars().count() <= 200));
    let long_word = "x".repeat(1 << 20);
    let a = ask(&mut s, &long_word);
    assert_eq!(a["steps"], json!([]));
    let ctl: String = (0u32..0x2000).filter_map(char::from_u32).collect();
    let a = ask(&mut s, &ctl);
    assert!(!a["question"].as_str().unwrap_or("").chars().any(char::is_control));
    // a query never keeps more than MAX_QUERY_TERMS terms
    let many: String = (0..5000).map(|i| format!("word{i} ")).collect();
    assert!(coach_ask::query_terms(&many).len() <= coach_ask::MAX_QUERY_TERMS);
    // not journaled
    assert!(s.journal.iter().all(|(id, _)| id != "coach.ask"));
}

#[test]
fn steps_from_a_following_steps_section_name_its_provenance() {
    let a = with_ui("clean up noisy shadows");
    assert_eq!(a.sources.first().map(|s| s.heading.as_str()), Some("Why"));
    let src = a.sources.get(a.steps_source).expect("the steps' source");
    assert_eq!((src.page, src.heading.as_str()), ("color/noise.md", "Steps"));
    assert_eq!(a.to_json()["stepsSource"], json!(a.steps_source));
}

#[test]
fn summaries_skip_lists_and_keep_only_checked_ids() {
    let body = "1. First step that goes on\n   onto a second line.\n\nThe real prose with `grade.addNode` and `color.coach.hints` {ui:x.y}.\n";
    let known = |id: &str| id == "grade.addNode";
    assert_eq!(coach_ask::summary(body, &known), "The real prose with grade.addNode and.");
    // an untrusted id never reaches the summary of a real answer
    let a = with_ui("how do I start a lesson?");
    assert!(!a.summary.contains("coach.start") || Session::default().shortcuts.is_known_command("coach.start"));
}

#[test]
fn removed_ids_leave_tidy_text() {
    let none = |_: &str| false;
    let a = coach_ask::ask("how do I make the sky bluer without changing skin?", &Checks { command: &none, namespace: &none, ui: None });
    for s in &a.steps {
        assert!(!s.text.contains("( ") && !s.text.contains(" )") && !s.text.contains("()") && !s.text.contains(",,") && !s.text.contains(" ,"), "{:?}", s.text);
    }
}

#[test]
fn a_lesson_is_offered_only_when_relevant() {
    let mut s = Session::default();
    let a = ask(&mut s, "how do I make the sky bluer without changing skin?");
    assert_eq!(a["lesson"]["id"], json!("fix-skin"));
    let a = ask(&mut s, "how do I add music");
    assert!(a["lesson"].is_null(), "{a}");
}

#[test]
fn intros_fold_into_the_first_section_and_stubs_are_not_answers() {
    // stop words are checked after stemming too, and question framing words are stop words
    assert_eq!(tokenize("whats the qualifier vs no window, ok?", 100), ["qualifi", "window"]);
    let page = "# Title words\n\n**[Fork]**\n\nIntro about zebras.\n\n## First\n\nBody one.\n\n## Stub\n\n**[Fork]**\n\n*To write.*\n";
    let secs = coach_ask::split_page("x.md", page);
    assert_eq!(secs.iter().map(|s| s.heading.as_str()).collect::<Vec<_>>(), ["First"]);
    assert!(secs[0].intro.contains("zebras") && !secs[0].body.contains("zebras"));
    assert_eq!(secs[0].provenance, ["[Fork]"]);
    // a page without `##` keeps its introduction as a section
    let only = coach_ask::split_page("y.md", "# Only\n\n**[Fork]**\n\nJust prose.\n");
    assert_eq!(only.iter().map(|s| s.heading.as_str()).collect::<Vec<_>>(), ["Only"]);
    let all = coach_ask::sections();
    assert!(!all.iter().any(|s| s.page == "glossary.md" && s.heading == "At a glance"));
    assert!(!all.iter().any(|s| s.body.trim() == "*To write.*"));
    assert!(!all.iter().any(|s| s.page == "color/secondaries.md" && s.heading == s.page_title), "the intro is not its own section");
}

// ---------- the eval set (docs/fork/coach-eval/questions.toml, FORK-PLAN §1a.47) ----------

/// Questions of the eval set that must pass (DIAGNOSIS.md §1 rule). This may only go up; raise it
/// when the guide or the coach improves (FORK-PLAN §1a.47).
const EVAL_FLOOR: usize = 82;

#[derive(Deserialize)]
struct EvalFile {
    question: Vec<EvalQuestion>,
}

#[derive(Deserialize)]
struct EvalSection {
    page: String,
    heading: String,
}

#[derive(Deserialize)]
struct EvalQuestion {
    id: String,
    q: String,
    #[serde(default)]
    page: Option<String>,
    #[serde(default)]
    heading: Option<String>,
    #[serde(default)]
    alt: Vec<EvalSection>,
    status: String,
}

/// Whether the coach answers an eval question right: for `covered` / `partial`, covered and the
/// best section is `page › heading` or an `alt`; for `uncovered`, not covered, no steps, no
/// sources. Err: why not.
fn eval_one(s: &Session, q: &EvalQuestion) -> Result<(), String> {
    let a = s.coach_ask(&q.q);
    let best = a.sources.first().map(|x| format!("{} › {}", x.page, x.heading));
    let why = format!("conf {:.3} cov {:.3} best {}", a.confidence, a.coverage, best.as_deref().unwrap_or("—"));
    if q.status == "uncovered" {
        return if !a.covered && a.steps.is_empty() && a.sources.is_empty() { Ok(()) } else { Err(format!("answered: {why}")) };
    }
    let want: Vec<String> =
        q.page.iter().zip(q.heading.iter()).map(|(p, h)| format!("{p} › {h}")).chain(q.alt.iter().map(|x| format!("{} › {}", x.page, x.heading))).collect();
    if !a.covered {
        return Err(format!("not covered: {why}"));
    }
    if best.as_ref().is_some_and(|b| want.contains(b)) {
        Ok(())
    } else {
        Err(format!("wrong section: {why}; want {}", want.first().map_or("—", String::as_str)))
    }
}

#[test]
fn the_eval_set_passes_at_least_the_floor() {
    let file: EvalFile = toml::from_str(include_str!("../../../docs/fork/coach-eval/questions.toml")).expect("the eval set parses");
    assert!(file.question.len() >= 100);
    let s = Session::default();
    let mut misses = Vec::new();
    for q in &file.question {
        assert!(["covered", "partial", "uncovered"].contains(&q.status.as_str()), "{}: status {}", q.id, q.status);
        if let Err(e) = eval_one(&s, q) {
            misses.push(format!("{} [{}] {:?}: {e}", q.id, q.status, q.q));
        }
    }
    let pass = file.question.len() - misses.len();
    eprintln!("coach eval: {pass}/{} pass (floor {EVAL_FLOOR}); misses:\n{}", file.question.len(), misses.join("\n"));
    assert!(pass >= EVAL_FLOOR, "coach eval: {pass}/{} below the floor {EVAL_FLOOR}; misses:\n{}", file.question.len(), misses.join("\n"));
}
