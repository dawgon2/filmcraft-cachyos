//! "Ask the coach" without a model (fork GRADE-PLAN G2.12, part 2): a question is answered from
//! the fork's user guide (`docs/guide/`, embedded at build time), with checked steps.
//!
//! 1. The guide is split into sections, one per `##` heading of a page (its `###` subsections
//!    included), each with its provenance tags ([Resolve-style], [FilmCraft], [Fork],
//!    [Standard]; a section without its own tag line inherits the page's).
//! 2. Text is tokenised: lower case, colour → color and grey → gray, a stop-word list, a small
//!    English stemmer (plurals, -ing, -ed, -er, -est, a final e) and a synonym table that maps
//!    beginner words to the guide's terms ("sky" → qualifier, "darker" → gamma / gain / lift /
//!    exposure…; expansions count half).
//! 3. Sections are ranked with BM25 (k1 1.2, b 0.5); a word of the section heading counts five
//!    times (the page title once). The confidence is the best score over the most a perfect section could
//!    score for this query (0…1).
//! 4. The answer's steps are the best section's first numbered list, markdown stripped, keeping
//!    the on-screen label text. `{ui:id}` gives a step its control (automation id) and a
//!    `` `command.id` `` its command.
//! 5. **Every id an answer names is checked**: commands against the session's registered command
//!    ids (engine commands plus the ones the frontend registered with the shortcuts registry),
//!    automation ids with the frontend's [`UiIdCheck`] when one is installed
//!    ([`Session::coach_ids`]). A step naming an unknown id is dropped and counted in `removed`.
//! 6. With a low confidence, or no numbered steps left, the answer is the closest lesson instead
//!    ("Try the lesson …") plus the section's summary.
//!
//! `coach.ask {question}` is a query (not journaled). Questions are hostile input: only the first
//! [`MAX_QUESTION_CHARS`] characters are read, words longer than [`MAX_WORD`] are ignored and a
//! query keeps at most [`MAX_QUERY_TERMS`] terms.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};

use crate::Session;
use crate::commands::{CommandSpec, always, bad};

/// Characters of a question that are read (the rest is ignored).
pub const MAX_QUESTION_CHARS: usize = 1000;
/// Longest word that is indexed or searched for, in characters.
pub const MAX_WORD: usize = 40;
/// Most distinct terms in one query (after synonyms).
pub const MAX_QUERY_TERMS: usize = 64;
/// Most steps in one answer.
pub const MAX_STEPS: usize = 20;
/// Longest step text, in characters.
pub const MAX_STEP_TEXT: usize = 600;
/// Longest summary, in characters.
pub const MAX_SUMMARY: usize = 400;
/// Most sources listed with an answer.
pub const MAX_SOURCES: usize = 3;
/// Below this confidence the guide doesn't cover the question (no steps, no sources).
pub const MIN_CONFIDENCE: f64 = 0.15;
/// The share of the question's words (or their synonyms) the best section must contain.
pub const MIN_COVERAGE: f64 = 0.75;
/// A lesson is suggested only when it matches the question at least this well (0…1).
pub const MIN_LESSON: f64 = 0.25;

const K1: f64 = 1.2;
const B: f64 = 0.5;
/// How much a word of a section's heading or page title counts compared with one in the body.
const TITLE_WEIGHT: f64 = 5.0;
/// A "how do I …" question asks for steps: sections that have them count this much more.
const HOW_BOOST: f64 = 1.15;
/// A section whose heading words all appear in the question scores this much more (in part when
/// some do). One-word headings (glossary entries) only count for a "what is …" question.
const HEADING_MATCH: f64 = 0.3;
/// How much a synonym's expansion counts compared with a word of the question.
const SYNONYM_WEIGHT: f64 = 0.5;

/// The guide pages, relative to `docs/guide/` (a test keeps this list equal to the directory).
pub const GUIDE_PAGES: &[(&str, &str)] = &[
    ("README.md", include_str!("../../../docs/guide/README.md")),
    ("getting-started.md", include_str!("../../../docs/guide/getting-started.md")),
    ("export.md", include_str!("../../../docs/guide/export.md")),
    ("glossary.md", include_str!("../../../docs/guide/glossary.md")),
    ("hardware.md", include_str!("../../../docs/guide/hardware.md")),
    ("proxies.md", include_str!("../../../docs/guide/proxies.md")),
    ("render-cache.md", include_str!("../../../docs/guide/render-cache.md")),
    ("color/README.md", include_str!("../../../docs/guide/color/README.md")),
    ("color/coach.md", include_str!("../../../docs/guide/color/coach.md")),
    ("color/color-management.md", include_str!("../../../docs/guide/color/color-management.md")),
    ("color/curves.md", include_str!("../../../docs/guide/color/curves.md")),
    ("color/finishing.md", include_str!("../../../docs/guide/color/finishing.md")),
    ("color/lumetri.md", include_str!("../../../docs/guide/color/lumetri.md")),
    ("color/matching.md", include_str!("../../../docs/guide/color/matching.md")),
    ("color/nodes.md", include_str!("../../../docs/guide/color/nodes.md")),
    ("color/noise.md", include_str!("../../../docs/guide/color/noise.md")),
    ("color/primaries.md", include_str!("../../../docs/guide/color/primaries.md")),
    ("color/recipes.md", include_str!("../../../docs/guide/color/recipes.md")),
    ("color/secondaries.md", include_str!("../../../docs/guide/color/secondaries.md")),
];

/// Pages that are about the guide itself (how it is written), not about the app: not searched.
const NOT_INDEXED: &[&str] = &["README.md"];

/// The provenance tags the guide uses.
pub const TAGS: &[&str] = &["[Resolve-style]", "[FilmCraft]", "[Fork]", "[Standard]"];

/// Checks automation ids against what the frontend has on screen (or has shown this session).
/// The egui UI installs one in [`Session::coach_ids`]; without it, automation ids are passed
/// through unchecked (the answer says `uiChecked: false`).
pub trait UiIdCheck: Send + Sync {
    fn known_ui_id(&self, id: &str) -> bool;
}

/// Words that carry no meaning for the search.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be", "been", "being", "but", "by", "can", "could", "did", "didn", "do",
    "does", "doesn", "doing", "don", "e", "eg", "etc", "for", "from", "get", "gets", "getting", "go", "goes", "going", "gone", "got", "had", "has", "have",
    "he", "help", "her", "here", "him", "his", "how", "i", "if", "in", "into", "is", "isn", "it", "its", "just", "let", "lets", "like", "make", "makes",
    "making", "may", "me", "might", "more", "most", "much", "must", "my", "need", "now", "of", "on", "one", "or", "our", "out", "please", "really", "she",
    "should", "so", "some", "than", "that", "the", "their", "them", "then", "there", "these", "they", "thing", "this", "those", "to", "too", "ui", "up", "use",
    "using", "very", "want", "was", "way", "we", "went", "were", "what", "when", "where", "which", "who", "why", "will", "with", "without", "would", "you",
    "your",
];

/// Beginner words → the guide's terms (each side tokenised like everything else).
const SYNONYMS: &[(&str, &str)] = &[
    ("sky", "qualifier sky"),
    ("gray", "qualifier matte hue"),
    ("blue", "qualifier"),
    ("bluer", "qualifier"),
    ("skin", "qualifier skin color"),
    ("face", "window face"),
    ("faces", "window face"),
    ("person", "window track face"),
    ("only", "qualifier"),
    ("match", "shot match still"),
    ("cameras", "shot match"),
    ("camera", "shot match"),
    ("noise", "noise reduction"),
    ("noisy", "noise reduction"),
    ("grainy", "noise reduction"),
    ("speckle", "noise reduction"),
    ("darker", "gamma gain lift exposure"),
    ("darken", "gamma gain lift exposure"),
    ("brighter", "gamma gain lift exposure"),
    ("brighten", "gamma gain lift exposure brighten"),
    ("lighter", "gamma gain lift exposure"),
    ("exposure", "gain gamma"),
    ("shadows", "lift shadows"),
    ("highlights", "gain highlight"),
    ("midtones", "gamma"),
    ("mids", "gamma"),
    ("flat", "log2 interpret"),
    ("log", "log2 interpret"),
    ("dull", "saturation contrast"),
    ("washed", "saturation contrast"),
    ("vivid", "saturation"),
    ("colorful", "saturation"),
    ("punchy", "saturation contrast"),
    ("warm", "temperature warm"),
    ("warmer", "temperature warm"),
    ("cold", "temperature"),
    ("cool", "temperature"),
    ("cast", "temperature tint balance"),
    ("neutral", "balance temperature tint"),
    ("white", "balance"),
    ("before", "bypass compare"),
    ("copy", "paste append"),
    ("reuse", "copy paste append"),
    ("track", "track window"),
    ("follow", "track window"),
    ("moving", "track window"),
    ("circle", "window"),
    ("shape", "window"),
    ("mask", "window qualifier matte"),
    ("vignette", "window"),
    ("isolate", "qualifier window"),
    ("select", "qualifier"),
    ("selection", "qualifier"),
    ("clipping", "soft clip highlight"),
    ("clipped", "soft clip highlight"),
    ("blown", "soft clip highlight"),
    ("lut", "lut"),
    ("lesson", "lesson coach"),
    ("tutorial", "lesson coach"),
    ("learn", "lesson coach"),
    ("slow", "hardware decod"),
    ("stutter", "hardware decod playback"),
    ("lag", "hardware playback"),
    ("gpu", "hardware"),
    ("shortcut", "shortcut keymap"),
    ("keyboard", "shortcut keymap"),
    ("explain", "explainer"),
    ("cinematic", "grain look"),
    ("film", "grain"),
    ("off", "disable bypass"),
    ("remove", "delete"),
    ("delete", "remove"),
    ("undo", "undo"),
];

// ---------- tokenising ----------

fn stopwords() -> &'static std::collections::HashSet<&'static str> {
    static S: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| STOPWORDS.iter().copied().collect())
}

/// British / American spellings to one form.
fn normalise(w: &str) -> String {
    w.replace("colour", "color").replace("grey", "gray").replace("centre", "center").replace("normalis", "normaliz")
}

/// `cutt` → `cut` (after a suffix was removed); not for vowels, l, s or z.
fn undouble(s: &mut String) {
    let mut last = s.bytes().rev();
    if let (Some(a), Some(b)) = (last.next(), last.next())
        && a == b
        && !matches!(a, b'a' | b'e' | b'i' | b'o' | b'u' | b'l' | b's' | b'z')
    {
        s.pop();
    }
}

/// A small English stemmer: plurals, -ing, -ed, -est, -er and a final e. Consistent rather than
/// linguistically right (grade, graded and grading all become "grad").
pub fn stem(w: &str) -> String {
    let mut s = w.to_string();
    if !s.is_ascii() || s.len() <= 3 {
        return s;
    }
    if s.ends_with("sses") {
        s.truncate(s.len() - 2);
    } else if s.ends_with("ies") && s.len() > 4 {
        s.truncate(s.len() - 3);
        s.push('y');
    } else if ["ches", "shes", "xes", "zes"].iter().any(|x| s.ends_with(x)) {
        s.truncate(s.len() - 2);
    } else if s.ends_with('s') && !s.ends_with("ss") && !s.ends_with("us") && !s.ends_with("is") {
        s.pop();
    }
    for suf in ["ing", "ed", "est", "er"] {
        if s.ends_with(suf) && s.len() >= suf.len() + 3 {
            s.truncate(s.len() - suf.len());
            undouble(&mut s);
            break;
        }
    }
    if s.ends_with('e') && s.len() > 3 {
        s.pop();
    }
    s
}

/// The words of a text: lower case, normalised, without stop words, stemmed. Reads at most
/// `max_chars` characters.
pub fn tokenize(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut Vec<String>| {
        if !word.is_empty() {
            let w = std::mem::take(word);
            if w.chars().count() <= MAX_WORD && !stopwords().contains(w.as_str()) && (w.chars().count() >= 2) {
                let w = normalise(&w);
                if !stopwords().contains(w.as_str()) {
                    out.push(stem(&w));
                }
            }
        }
    };
    for c in text.chars().take(max_chars) {
        if c.is_alphanumeric() {
            if word.chars().count() <= MAX_WORD {
                word.extend(c.to_lowercase());
            }
        } else {
            flush(&mut word, &mut out);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn synonyms() -> &'static HashMap<String, Vec<String>> {
    static S: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    S.get_or_init(|| {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in SYNONYMS {
            if let Some(key) = tokenize(k, 64).into_iter().next() {
                m.entry(key).or_default().extend(tokenize(v, 256));
            }
        }
        m
    })
}

/// The query's terms with their weights: the question's words (1) and their synonyms (½).
pub fn query_terms(question: &str) -> Vec<(String, f64)> {
    let mut terms: Vec<(String, f64)> = Vec::new();
    let add = |t: &str, w: f64, terms: &mut Vec<(String, f64)>| {
        if let Some(e) = terms.iter_mut().find(|(x, _)| x == t) {
            e.1 = e.1.max(w);
        } else if terms.len() < MAX_QUERY_TERMS {
            terms.push((t.to_string(), w));
        }
    };
    let words = tokenize(question, MAX_QUESTION_CHARS);
    for w in &words {
        add(w, 1.0, &mut terms);
    }
    for w in &words {
        if let Some(exp) = synonyms().get(w) {
            for e in exp {
                add(e, SYNONYM_WEIGHT, &mut terms);
            }
        }
    }
    terms
}

// ---------- the guide ----------

/// One `##` section of a guide page.
#[derive(Clone, Debug)]
pub struct Section {
    /// The page, relative to `docs/guide/`.
    pub page: &'static str,
    pub page_title: String,
    /// The `##` heading (the page title for the text before the first one).
    pub heading: String,
    /// Provenance tags, as the guide writes them.
    pub provenance: Vec<&'static str>,
    /// The section's markdown, its `###` subsections included.
    pub body: String,
}

fn heading_level(line: &str) -> Option<(usize, &str)> {
    let n = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&n) && line.as_bytes().get(n) == Some(&b' ') { line.get(n + 1..).map(|t| (n, t.trim())) } else { None }
}

fn tags_in(line: &str) -> Vec<&'static str> {
    TAGS.iter().copied().filter(|t| line.contains(t)).collect()
}

/// A line that only holds provenance tags (and maybe a short description after "·").
fn is_tag_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("**[") && !tags_in(t).is_empty()
}

/// Split a page into its sections.
pub fn split_page(page: &'static str, src: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut page_title = String::new();
    let mut page_tags: Vec<&'static str> = Vec::new();
    let mut cur: Option<Section> = None;
    let mut fence = false;
    let mut want_tags = false;
    for line in src.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let head = if fence { None } else { heading_level(line) };
        match head {
            Some((1, t)) => {
                page_title = t.to_string();
                want_tags = true;
                cur = Some(Section { page, page_title: page_title.clone(), heading: page_title.clone(), provenance: Vec::new(), body: String::new() });
                continue;
            }
            Some((2, t)) => {
                out.extend(cur.take());
                cur = Some(Section { page, page_title: page_title.clone(), heading: t.to_string(), provenance: Vec::new(), body: String::new() });
                want_tags = true;
                continue;
            }
            _ => {}
        }
        let Some(sec) = cur.as_mut() else { continue };
        if want_tags && !line.trim().is_empty() && head.is_none() {
            want_tags = false;
            if is_tag_line(line) {
                let tags = tags_in(line);
                if sec.heading == page_title && out.is_empty() {
                    page_tags = tags.clone();
                }
                sec.provenance = tags;
                continue;
            }
        }
        sec.body.push_str(line);
        sec.body.push('\n');
    }
    out.extend(cur);
    for s in &mut out {
        if s.provenance.is_empty() {
            // the first tag line of a subsection, else the page's
            s.provenance = s.body.lines().find(|l| is_tag_line(l)).map(tags_in).unwrap_or_else(|| page_tags.clone());
        }
    }
    // a page's untagged introduction (a status line) and empty sections aren't answers
    out.retain(|s| !s.body.trim().is_empty() && !s.provenance.is_empty());
    out
}

struct Doc {
    tf: HashMap<String, f64>,
    len: f64,
}

struct Index {
    docs: Vec<Doc>,
    df: HashMap<String, usize>,
    avg: f64,
}

impl Index {
    /// Documents of (title, body): a word of the title counts [`TITLE_WEIGHT`] times.
    fn build(texts: impl Iterator<Item = (String, String)>) -> Index {
        let mut docs = Vec::new();
        let mut df: HashMap<String, usize> = HashMap::new();
        for (title, body) in texts {
            let mut tf: HashMap<String, f64> = HashMap::new();
            for t in tokenize(&title, 4096) {
                *tf.entry(t).or_default() += TITLE_WEIGHT;
            }
            for t in tokenize(&body, 1 << 20) {
                *tf.entry(t).or_default() += 1.0;
            }
            for t in tf.keys() {
                *df.entry(t.clone()).or_default() += 1;
            }
            let len = tf.values().sum::<f64>();
            docs.push(Doc { tf, len });
        }
        let avg = if docs.is_empty() { 1.0 } else { (docs.iter().map(|d| d.len).sum::<f64>() / docs.len() as f64).max(1.0) };
        Index { docs, df, avg }
    }

    fn idf(&self, t: &str) -> f64 {
        let n = self.docs.len() as f64;
        let df = self.df.get(t).copied().unwrap_or(0) as f64;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    /// (doc index, score), best first, only scores above 0.
    fn rank(&self, terms: &[(String, f64)]) -> Vec<(usize, f64)> {
        let mut v: Vec<(usize, f64)> = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let s = terms
                    .iter()
                    .map(|(t, w)| {
                        let f = d.tf.get(t).copied().unwrap_or(0.0);
                        if f <= 0.0 {
                            return 0.0;
                        }
                        w * self.idf(t) * f * (K1 + 1.0) / (f + K1 * (1.0 - B + B * d.len / self.avg))
                    })
                    .sum::<f64>();
                (i, s)
            })
            .filter(|(_, s)| *s > 0.0 && s.is_finite())
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// The most a section could score for these terms. A word of the question the index lacks
    /// counts with the highest idf (no section has it), so a question about something the guide
    /// doesn't cover scores low; a synonym it lacks doesn't count.
    fn ceiling(&self, terms: &[(String, f64)]) -> f64 {
        terms.iter().map(|(t, w)| if self.df.contains_key(t) || *w >= 1.0 { w * self.idf(t) * (K1 + 1.0) } else { 0.0 }).sum()
    }

    /// Whether document `i` contains the term.
    fn has(&self, i: usize, t: &str) -> bool {
        self.docs.get(i).is_some_and(|d| d.tf.contains_key(t))
    }
}

struct Guide {
    sections: Vec<Section>,
    /// Whether each section has numbered steps.
    has_steps: Vec<bool>,
    /// Each section's heading words.
    heading_terms: Vec<Vec<String>>,
    index: Index,
    lessons: Index,
}

fn guide() -> &'static Guide {
    static G: OnceLock<Guide> = OnceLock::new();
    G.get_or_init(|| {
        let sections: Vec<Section> = GUIDE_PAGES.iter().filter(|(p, _)| !NOT_INDEXED.contains(p)).flat_map(|(p, src)| split_page(p, src)).collect();
        let index = Index::build(sections.iter().map(|s| (s.heading.clone(), format!("{}\n{}", s.page_title, s.body))));
        let lessons = Index::build(crate::coach::bundled().iter().map(|l| {
            let steps: Vec<&str> = l.steps.iter().flat_map(|s| [s.text.as_str(), s.why.as_str()]).collect();
            (l.title.clone(), format!("{} {}", l.summary, steps.join(" ")))
        }));
        let has_steps = (0..sections.len()).map(|i| steps_section(&sections, i).is_some()).collect();
        let heading_terms = sections.iter().map(|s| tokenize(&s.heading, 512)).collect();
        Guide { sections, has_steps, heading_terms, index, lessons }
    })
}

/// The indexed sections (for tests and tools).
pub fn sections() -> &'static [Section] {
    &guide().sections
}

/// The sections best matching a question: (section index into [`sections`], score), best first.
pub fn rank(question: &str) -> Vec<(usize, f64)> {
    rank_terms(question, &query_terms(question))
}

/// The section whose steps answer for section `i`: itself when it has a numbered list, else the
/// page's "Steps …" section right after it (a "Why" section and its "Steps").
fn steps_section(sections: &[Section], i: usize) -> Option<&Section> {
    let sec = sections.get(i)?;
    if !numbered_list(&sec.body).is_empty() {
        return Some(sec);
    }
    sections.get(i.saturating_add(1)).filter(|n| n.page == sec.page && n.heading.starts_with("Steps") && !numbered_list(&n.body).is_empty())
}

/// BM25; for a "how …" question a section with numbered steps counts [`HOW_BOOST`] times, and a
/// section whose heading words are in the question up to [`HEADING_MATCH`] more (one-word
/// headings only for a "what …" question).
fn rank_terms(question: &str, terms: &[(String, f64)]) -> Vec<(usize, f64)> {
    let g = guide();
    let mut ranked = g.index.rank(terms);
    let lower = question.chars().take(MAX_QUESTION_CHARS).collect::<String>().to_lowercase();
    let has = |word: &str| lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == word);
    let (how, what) = (has("how"), has("what") || has("explain") || has("mean"));
    for (i, s) in &mut ranked {
        if how && g.has_steps.get(*i).copied().unwrap_or(false) {
            *s *= HOW_BOOST;
        }
        if let Some(h) = g.heading_terms.get(*i).filter(|h| if what { !h.is_empty() } else { h.len() >= 2 }) {
            let hit = h.iter().filter(|t| terms.iter().any(|(q, w)| *w >= 1.0 && q == *t)).count();
            *s *= 1.0 + HEADING_MATCH * hit as f64 / h.len() as f64;
        }
    }
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
}

// ---------- steps ----------

/// One step of an answer.
#[derive(Clone, Debug, PartialEq)]
pub struct AskStep {
    /// What to do, markdown removed (the on-screen labels stay).
    pub text: String,
    /// The automation ids the step names (`{ui:…}`), in order.
    pub ui: Vec<String>,
    /// The command ids the step names (`` `…` ``), in order.
    pub commands: Vec<String>,
}

/// An id-shaped word with a namespace (`grade.addNode`): ASCII letters, digits and `._-`.
fn id_shaped(s: &str) -> bool {
    s.len() <= crate::coach::MAX_ID
        && s.contains('.')
        && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !s.ends_with('.')
        && ![".md", ".toml", ".json", ".rs", ".tsv"].iter().any(|x| s.ends_with(x))
}

/// Plain text made from markdown, and the ids it named.
#[derive(Default)]
struct Plain {
    text: String,
    /// `{ui:…}` ids (taken out of the text).
    ui: Vec<String>,
    /// Backticked id-shaped words kept in the text.
    code: Vec<String>,
    /// Backticked id-shaped words taken out of the text (`keep` refused them).
    dropped: Vec<String>,
}

/// Markdown to plain text: `**`, `*`, backticks and `[text](link)` removed, `{ui:…}` taken out
/// (into `ui`). A backticked id-shaped word stays in the text only when `keep` accepts it.
fn plain(src: &str, keep: &dyn Fn(&str) -> bool) -> Plain {
    let mut p = Plain::default();
    let mut out = String::new();
    let mut rest = src;
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("{ui:")
            && let Some(end) = r.find('}')
        {
            let id = r.get(..end).unwrap_or("").trim();
            if !id.is_empty() && p.ui.len() < 16 {
                p.ui.push(id.to_string());
            }
            rest = r.get(end + 1..).unwrap_or("");
            continue;
        }
        if c == '`'
            && let Some(end) = rest.get(1..).and_then(|r| r.find('`'))
        {
            let inner = rest.get(1..end + 1).unwrap_or("");
            if !id_shaped(inner) {
                out.push_str(inner);
            } else if keep(inner) {
                if p.code.len() < 16 {
                    p.code.push(inner.to_string());
                }
                out.push_str(inner);
            } else if p.dropped.len() < 16 {
                p.dropped.push(inner.to_string());
            }
            rest = rest.get(end + 2..).unwrap_or("");
            continue;
        }
        if c == '['
            && let Some(close) = rest.find("](")
            && let Some(paren) = rest.get(close + 2..).and_then(|r| r.find(')'))
            && !rest.get(1..close).unwrap_or("").contains(['[', ']'])
        {
            out.push_str(rest.get(1..close).unwrap_or(""));
            rest = rest.get(close + 2 + paren + 1..).unwrap_or("");
            continue;
        }
        if c != '*' {
            out.push(if c.is_control() { ' ' } else { c });
        }
        rest = rest.get(c.len_utf8()..).unwrap_or("");
    }
    p.text = tidy(&out);
    p
}

/// Spacing and punctuation left behind when ids are taken out: `(a, , b)` → `(a, b)`, `( )` → ``.
fn tidy(s: &str) -> String {
    let mut s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    for _ in 0..8 {
        let before = s.len();
        for (a, b) in [
            (" ,", ","),
            (" .", "."),
            (" :", ":"),
            (" ;", ";"),
            (" )", ")"),
            ("( ", "("),
            (",,", ","),
            ("(,", "("),
            (",)", ")"),
            ("()", ""),
            ("  ", " "),
            (", .", "."),
            (", :", ":"),
            (": ,", ":"),
        ] {
            s = s.replace(a, b);
        }
        if s.len() == before {
            break;
        }
    }
    s.trim().trim_end_matches(',').trim().to_string()
}

fn numbered(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    t.get(digits..)?.strip_prefix(". ")
}

/// The first numbered list of a section, one string per item (continuation lines joined).
pub fn numbered_list(body: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut blank = false;
    let mut fence = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            if !items.is_empty() {
                break;
            }
            continue;
        }
        if fence {
            continue;
        }
        if let Some(rest) = numbered(line) {
            if items.len() >= MAX_STEPS {
                break;
            }
            items.push(rest.to_string());
            blank = false;
            continue;
        }
        let Some(last) = items.last_mut() else { continue };
        if line.trim().is_empty() {
            blank = true;
            continue;
        }
        let indented = line.starts_with("  ") || line.starts_with('\t');
        if heading_level(line).is_some() || (blank && !indented) {
            break;
        }
        last.push(' ');
        last.push_str(line.trim());
    }
    items
}

/// A section's first paragraph of prose, as plain text (lists, their continuation lines, tables,
/// code and headings skipped). A backticked id stays only when `keep` accepts it.
pub fn summary(body: &str, keep: &dyn Fn(&str) -> bool) -> String {
    let mut para = String::new();
    let mut in_list = false;
    let mut fence = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if t.is_empty() {
            in_list = false;
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if numbered(t).is_some() || t.starts_with("- ") || t.starts_with("* ") {
            in_list = true;
        }
        let skip = in_list || heading_level(t).is_some() || is_tag_line(t) || t.starts_with('|') || t.starts_with('>');
        if skip {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        para.push_str(t);
        para.push(' ');
    }
    let para = para.replace("**Why:**", "");
    truncate(&plain(&para, keep).text, MAX_SUMMARY)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// A section's steps, markdown stripped and ids extracted (unchecked: every backticked id
/// counts as a command).
pub fn section_steps(body: &str) -> Vec<AskStep> {
    numbered_list(body)
        .iter()
        .map(|item| {
            let p = plain(item, &|_| true);
            AskStep { text: truncate(&p.text, MAX_STEP_TEXT), ui: p.ui, commands: p.code }
        })
        .filter(|s| !s.text.is_empty())
        .collect()
}

// ---------- the answer ----------

/// What the check needs to know about the session.
pub struct Checks<'a> {
    /// A registered command id (engine or frontend).
    pub command: &'a dyn Fn(&str) -> bool,
    /// The first part of a registered command id (`grade` of `grade.addNode`): an id-shaped
    /// word in another namespace (a preference key, a parameter name) is not a command.
    pub namespace: &'a dyn Fn(&str) -> bool,
    /// A known automation id (None: not checked, no UI).
    pub ui: Option<&'a dyn Fn(&str) -> bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub page: &'static str,
    pub heading: String,
    pub provenance: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub question: String,
    pub steps: Vec<AskStep>,
    pub sources: Vec<Source>,
    /// The closest lesson (id, title).
    pub lesson: Option<(String, String)>,
    /// 0…1: how well the best section covers the question.
    pub confidence: f64,
    /// Steps dropped because they named an unknown command or control.
    pub removed: usize,
    /// Of those, how many named an unknown control / an unknown command (a step can do both).
    pub removed_controls: usize,
    pub removed_commands: usize,
    /// Whether the guide covers the question (confidence and word coverage high enough). When
    /// it doesn't, there are no steps, sources or summary.
    pub covered: bool,
    /// The share of the question's words the best section contains (0…1).
    pub coverage: f64,
    /// The index in `sources` of the section the steps come from.
    pub steps_source: usize,
    /// The unknown ids (at most 16).
    pub removed_ids: Vec<String>,
    /// No steps: the answer is the closest lesson and the summary.
    pub fallback: bool,
    /// The best section's first paragraph.
    pub summary: String,
    /// Whether automation ids were checked (a UI is attached).
    pub ui_checked: bool,
}

impl Answer {
    pub fn to_json(&self) -> Value {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|s| {
                let mut v = json!({"text": s.text});
                if let Some(u) = s.ui.first() {
                    v["ui"] = json!(u);
                }
                if let Some(c) = s.commands.first() {
                    v["command"] = json!(c);
                }
                if s.ui.len() > 1 {
                    v["uis"] = json!(s.ui);
                }
                if s.commands.len() > 1 {
                    v["commands"] = json!(s.commands);
                }
                v
            })
            .collect();
        let lesson = self.lesson.as_ref().map(|(_, t)| t.as_str());
        let message = match (self.fallback, self.covered, lesson) {
            (false, _, _) => Value::Null,
            (true, true, Some(title)) => json!(format!("Try the lesson \u{201c}{title}\u{201d}.")),
            (true, true, None) => json!("The guide has no step-by-step answer for this yet; see the source below."),
            (true, false, Some(title)) => json!(format!("The guide doesn't cover this yet. The closest lesson is \u{201c}{title}\u{201d}.")),
            (true, false, None) if query_terms(&self.question).is_empty() => {
                json!("Ask about a grading task, for example \u{201c}how do I make the sky bluer?\u{201d}")
            }
            (true, false, None) => json!("The guide doesn't cover this yet."),
        };
        json!({
            "question": self.question,
            "steps": steps,
            "sources": self.sources.iter().map(|s| json!({"page": s.page, "heading": s.heading, "provenance": s.provenance})).collect::<Vec<_>>(),
            "lesson": self.lesson.as_ref().map(|(id, title)| json!({"id": id, "title": title})),
            "confidence": (self.confidence * 1000.0).round() / 1000.0,
            "removed": self.removed,
            "removedControls": self.removed_controls,
            "removedCommands": self.removed_commands,
            "removedIds": self.removed_ids,
            "covered": self.covered,
            "coverage": (self.coverage * 1000.0).round() / 1000.0,
            "stepsSource": self.steps_source,
            "fallback": self.fallback,
            "summary": self.summary,
            "message": message,
            "uiChecked": self.ui_checked,
        })
    }
}

/// The share of the question's own words found (themselves or a synonym) in the sections
/// `docs` of the guide index.
fn coverage(terms: &[(String, f64)], docs: &[usize]) -> f64 {
    coverage_in(&guide().index, terms, docs)
}

/// The same in any index (the lessons').
fn coverage_in(index: &Index, terms: &[(String, f64)], docs: &[usize]) -> f64 {
    let own: Vec<&String> = terms.iter().filter(|(_, w)| *w >= 1.0).map(|(t, _)| t).collect();
    if own.is_empty() {
        return 0.0;
    }
    let found = |t: &str| docs.iter().any(|d| index.has(*d, t));
    let hit = own.iter().filter(|t| found(t) || synonyms().get(t.as_str()).is_some_and(|e| e.iter().any(|x| found(x)))).count();
    hit as f64 / own.len() as f64
}

/// Answer a question from the guide, checking every id it names.
pub fn ask(question: &str, checks: &Checks) -> Answer {
    let g = guide();
    let terms = query_terms(question);
    let ranked = rank_terms(question, &terms);
    let ceiling = g.index.ceiling(&terms) * HOW_BOOST * (1.0 + HEADING_MATCH);
    let shown: String = question.chars().take(200).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let mut a = Answer {
        question: shown.trim().to_string(),
        steps: Vec::new(),
        sources: Vec::new(),
        lesson: None,
        confidence: 0.0,
        removed: 0,
        removed_controls: 0,
        removed_commands: 0,
        covered: false,
        coverage: 0.0,
        steps_source: 0,
        removed_ids: Vec::new(),
        fallback: true,
        summary: String::new(),
        ui_checked: checks.ui.is_some(),
    };
    // the closest lesson, when it is relevant enough
    let lesson_ceiling = g.lessons.ceiling(&terms);
    let lesson = g
        .lessons
        .rank(&terms)
        .first()
        .filter(|(i, _)| coverage_in(&g.lessons, &terms, &[*i]) >= MIN_COVERAGE)
        .map(|(i, score)| (*i, if lesson_ceiling > 0.0 { score / lesson_ceiling } else { 0.0 }));
    let pick = |min: f64| lesson.filter(|(_, c)| *c >= min).and_then(|(i, _)| crate::coach::bundled().get(i)).map(|l| (l.id.clone(), l.title.clone()));
    // a question the guide doesn't cover gets a lesson only when that lesson is clearly about it
    a.lesson = pick(2.0 * MIN_LESSON);
    let Some(&(best, top)) = ranked.first() else { return a };
    a.confidence = if ceiling > 0.0 { (top / ceiling).clamp(0.0, 1.0) } else { 0.0 };
    let Some(sec) = g.sections.get(best) else { return a };
    let from = steps_section(&g.sections, best);
    let from_index = from.filter(|n| !std::ptr::eq(*n, sec)).map(|_| best.saturating_add(1));
    let docs: Vec<usize> = [Some(best), from_index].into_iter().flatten().collect();
    a.coverage = coverage(&terms, &docs);
    a.covered = a.confidence >= MIN_CONFIDENCE && a.coverage >= MIN_COVERAGE;
    if !a.covered {
        return a;
    }
    a.lesson = pick(MIN_LESSON);
    for (i, score) in ranked.iter().take(MAX_SOURCES) {
        if *score < top * 0.4 {
            break;
        }
        if let Some(s) = g.sections.get(*i) {
            a.sources.push(Source { page: s.page, heading: s.heading.clone(), provenance: s.provenance.clone() });
        }
    }
    if let Some(next) = from.filter(|n| !std::ptr::eq(*n, sec)) {
        let src = Source { page: next.page, heading: next.heading.clone(), provenance: next.provenance.clone() };
        match a.sources.iter().position(|x| *x == src) {
            Some(k) => a.steps_source = k,
            None => {
                a.sources.insert(1.min(a.sources.len()), src);
                a.sources.truncate(MAX_SOURCES);
                a.steps_source = 1.min(a.sources.len().saturating_sub(1));
            }
        }
    }
    // an id-shaped word is kept in the text only when it is a registered command
    let keep = |id: &str| (checks.command)(id);
    a.summary = summary(&sec.body, &keep);
    let items = from.map(|f| numbered_list(&f.body)).unwrap_or_default();
    for item in items {
        let raw = plain(&item, &|_| true);
        // a word in a command namespace that isn't a registered command: the step is wrong
        let bad_cmd: Vec<&String> = raw.code.iter().filter(|c| c.split('.').next().is_some_and(|ns| (checks.namespace)(ns)) && !(checks.command)(c)).collect();
        let bad_ui: Vec<&String> = raw.ui.iter().filter(|u| checks.ui.is_some_and(|f| !f(u))).collect();
        if !bad_cmd.is_empty() || !bad_ui.is_empty() {
            a.removed += 1;
            a.removed_commands += usize::from(!bad_cmd.is_empty());
            a.removed_controls += usize::from(!bad_ui.is_empty());
            for id in bad_cmd.into_iter().chain(bad_ui) {
                if a.removed_ids.len() < 16 && !a.removed_ids.contains(id) {
                    a.removed_ids.push(id.clone());
                }
            }
            continue;
        }
        // other id-shaped words (preference keys, parameter names) can't be checked: left out
        let p = plain(&item, &keep);
        if p.text.is_empty() {
            continue;
        }
        // without a UI the controls can't be checked: none are named (the labels stay in the text)
        let ui = if checks.ui.is_some() { p.ui } else { Vec::new() };
        a.steps.push(AskStep { text: truncate(&p.text, MAX_STEP_TEXT), ui, commands: p.code });
    }
    a.fallback = a.steps.is_empty();
    a
}

// ---------- the command ----------

impl Session {
    /// Answer a question with this session's checks (registered commands; automation ids when a
    /// UI installed [`Session::coach_ids`]).
    pub fn coach_ask(&self, question: &str) -> Answer {
        let shortcuts = &self.shortcuts;
        let command = |id: &str| shortcuts.is_known_command(id);
        let namespace = |ns: &str| shortcuts.is_command_namespace(ns);
        let ui_check: Option<Arc<dyn UiIdCheck>> = self.coach_ids.clone();
        let ui_fn = |id: &str| ui_check.as_ref().is_some_and(|c| c.known_ui_id(id));
        let checks = Checks { command: &command, namespace: &namespace, ui: ui_check.is_some().then_some(&ui_fn as &dyn Fn(&str) -> bool) };
        ask(question, &checks)
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "coach.ask",
        label: "Ask the Coach",
        menu: &[],
        shortcut: None,
        params: r#"{"question":str}"#,
        enabled: always,
        run: |s, p| {
            let q = p.get("question").and_then(Value::as_str).ok_or_else(|| bad("coach.ask", "need `question` (text)"))?;
            Ok(s.coach_ask(q).to_json())
        },
        journal: false,
    }]
}
