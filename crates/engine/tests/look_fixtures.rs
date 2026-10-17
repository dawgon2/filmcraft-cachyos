//! AI4 item 2b acceptance harness (CLOUD-QUEUE 24): synthetic frames for the `needs_clip`
//! questions (`docs/fork/coach-eval/looks.toml`), a `PictureLooker` run over each, and the score:
//! does the look mention what the question is about, and does it invent nothing the frame doesn't
//! show. The CPU tests check the fixtures and the scoring with a pixel-statistics fake looker; the
//! real Qwen3-VL run is `real_vision_model_looks_at_the_fixtures` (ignored, `--features llm`,
//! `FILMCRAFT_DATA_DIR=~/.local/share/filmcraft-fork`, `--release`, ~5–30 s per look on the CPU).

use filmcraft_engine::coach_assist::{PictureInput, PictureLooker};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Size the fixtures are drawn at (16:9, under `coach_assist::LOOK_PIXELS`).
const W: usize = 512;
const H: usize = 288;
/// Looks the pixel-statistics fake gets right (measured 2026-10-16: it can't see bands or screens).
const FAKE_FLOOR: usize = 7;
/// Looks the real Qwen3-VL 4B gets right (measured 2026-10-16 02:39 EDT on the CPU: 14 / 15 with
/// nothing invented; the miss sees "a darker blue area" below the shore but doesn't call it water,
/// fair for a flat band, left as a miss).
#[cfg(feature = "llm")]
const REAL_FLOOR: usize = 14;

#[derive(Deserialize)]
struct Looks {
    look: Vec<Look>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Look {
    id: String,
    question: String,
    background: Option<[f32; 3]>,
    top: Option<[f32; 3]>,
    bottom: Option<[f32; 3]>,
    steps: Option<u32>,
    #[serde(default)]
    rects: Vec<Rect>,
    #[serde(default)]
    noise: f32,
    must: Vec<Vec<String>>,
    #[serde(default)]
    must_not: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rect {
    at: [f32; 4],
    rgb: [f32; 3],
}

#[derive(Deserialize)]
struct Questions {
    #[serde(default)]
    question: Vec<Question>,
}

#[derive(Deserialize)]
struct Question {
    id: String,
    q: String,
    needs_clip: Option<bool>,
}

fn eval_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/fork/coach-eval")
}

fn looks() -> Vec<Look> {
    let p = eval_dir().join("looks.toml");
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    toml::from_str::<Looks>(&s).unwrap_or_else(|e| panic!("{}: {e}", p.display())).look
}

/// Every `needs_clip` question (pool batches and the coach set): id → text.
fn clip_questions() -> Vec<(String, String)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(eval_dir().join("pool"))
        .expect("the pool folder exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.push(eval_dir().join("questions.toml"));
    let mut out = Vec::new();
    for p in files {
        let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let qs: Questions = toml::from_str(&s).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        out.extend(qs.question.into_iter().filter(|q| q.needs_clip == Some(true)).map(|q| (q.id, q.q)));
    }
    out
}

/// The fixture's frame, display RGB 0..=1, row-major RGB triples.
fn draw(l: &Look) -> PictureInput {
    let mut rgb = vec![0.0_f32; W * H * 3];
    for y in 0..H {
        let mut t = y as f32 / (H - 1) as f32;
        if let Some(n) = l.steps.filter(|&n| n > 1) {
            t = (t * n as f32).floor().min(n as f32 - 1.0) / (n - 1) as f32;
        }
        let c = match (l.background, l.top, l.bottom) {
            (Some(b), _, _) => b,
            (None, Some(a), Some(b)) => [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t),
            _ => [0.0; 3],
        };
        for x in 0..W {
            rgb[(y * W + x) * 3..][..3].copy_from_slice(&c);
        }
    }
    for r in &l.rects {
        let x0 = (r.at[0] * W as f32) as usize;
        let y0 = (r.at[1] * H as f32) as usize;
        let x1 = (((r.at[0] + r.at[2]) * W as f32) as usize).min(W);
        let y1 = (((r.at[1] + r.at[3]) * H as f32) as usize).min(H);
        for y in y0..y1 {
            for x in x0..x1 {
                rgb[(y * W + x) * 3..][..3].copy_from_slice(&r.rgb);
            }
        }
    }
    if l.noise > 0.0 {
        let mut s: u32 = 0x9E37_79B9;
        for v in &mut rgb {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let u = (s >> 8) as f32 / (1u32 << 24) as f32; // 0..1
            *v = (*v + (u * 2.0 - 1.0) * l.noise).clamp(0.0, 1.0);
        }
    }
    PictureInput { rgb, w: W, h: H }
}

/// The reply as lower-case words separated by single spaces, padded, for whole-word lookups.
fn words(s: &str) -> String {
    let w: Vec<String> = s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect();
    format!(" {} ", w.join(" "))
}

fn has(text: &str, word: &str) -> bool {
    text.contains(&words(word))
}

/// (every `must` group named, an invented `must_not` word if any).
fn score(l: &Look, reply: &str) -> (bool, Option<String>) {
    let t = words(reply);
    let named = l.must.iter().all(|g| g.iter().any(|w| has(&t, w)));
    let invented = l.must_not.iter().find(|w| has(&t, w)).cloned();
    (named, invented)
}

#[test]
fn look_fixtures_are_well_formed_and_point_at_clip_questions() {
    let qs = clip_questions();
    let mut ids = HashSet::new();
    let all = looks();
    assert!(!all.is_empty());
    for l in &all {
        assert!(ids.insert(l.id.clone()), "{}: duplicate id", l.id);
        let q = qs.iter().find(|(id, _)| *id == l.question).unwrap_or_else(|| panic!("{}: {} is not a needs_clip question", l.id, l.question));
        assert!(l.background.is_some() != (l.top.is_some() && l.bottom.is_some()), "{}: either `background` or `top` + `bottom`", l.id);
        let colours = l.background.iter().chain(&l.top).chain(&l.bottom).chain(l.rects.iter().map(|r| &r.rgb));
        for c in colours {
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{}: colour {c:?} outside 0..=1", l.id);
        }
        for r in &l.rects {
            let [x, y, w, h] = r.at;
            assert!(x >= 0.0 && y >= 0.0 && w > 0.0 && h > 0.0 && x + w <= 1.0001 && y + h <= 1.0001, "{}: rect {:?} outside the frame", l.id, r.at);
        }
        assert!((0.0..=0.5).contains(&l.noise), "{}: noise", l.id);
        assert!(!l.must.is_empty() && l.must.iter().all(|g| !g.is_empty()), "{}: `must` needs non-empty groups", l.id);
        // a `must_not` word the question itself uses would punish a look for echoing it
        let qw = words(&q.1);
        for w in &l.must_not {
            assert!(!has(&qw, w), "{}: must_not {w:?} is in the question", l.id);
        }
    }
}

#[test]
fn scoring_wants_every_group_and_catches_inventions() {
    let l = looks().into_iter().find(|l| l.id == "look-wb-shade").expect("look-wb-shade");
    assert_eq!(score(&l, "The whole frame has a cool, blue cast; the car looks pale blue."), (true, None));
    assert_eq!(score(&l, "A car on a road."), (false, None));
    assert_eq!(score(&l, "Warm orange light, a blue car."), (true, Some("orange".into())));
    // whole words only: "blueprint" is not "blue"
    assert!(!score(&l, "A blueprint of a car.").0);
}

/// A looker that only measures pixels: mean exposure, cast, saturation, white sky, noise.
struct StatsLooker;

impl PictureLooker for StatsLooker {
    fn look(&self, p: &PictureInput, _question: &str) -> Result<String, String> {
        let n = (p.w * p.h).max(1) as f32;
        let mut m = [0.0_f32; 3];
        let (mut sat, mut grain) = (0.0_f32, 0.0_f32);
        for (i, px) in p.rgb.as_chunks::<3>().0.iter().enumerate() {
            for c in 0..3 {
                m[c] += px[c] / n;
            }
            sat += (px[0].max(px[1]).max(px[2]) - px[0].min(px[1]).min(px[2])) / n;
            if i % p.w != 0 {
                grain += (px[1] - p.rgb[i * 3 - 2]).abs() / n;
            }
        }
        let top = p.rgb.as_chunks::<3>().0.iter().take(p.w * p.h / 3);
        let white = top.filter(|px| px.iter().all(|&v| v > 0.95)).count() as f32 / (n / 3.0);
        let luma = 0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2];
        let mut said = Vec::new();
        if white > 0.5 {
            said.push("The sky is white and blown out.");
        } else if luma < 0.12 {
            said.push("The frame is very dark.");
        }
        if sat < 0.02 {
            said.push("It is grey and flat, like black and white.");
        } else if m[2] - m[0] > 0.12 {
            said.push("Everything has a blue cast.");
        } else if m[0] - m[2] > 0.2 {
            said.push("The light is warm orange.");
        } else if m[1] - m[0].max(m[2]) > 0.08 {
            said.push("The picture looks green.");
        }
        if grain > 0.04 {
            said.push("The shadows are noisy.");
        }
        Ok(said.join(" "))
    }
}

fn run(looker: &dyn PictureLooker) -> (usize, Vec<String>) {
    let qs = clip_questions();
    let (mut ok, mut report) = (0, Vec::new());
    for l in looks() {
        let q = qs.iter().find(|(id, _)| *id == l.question).map(|(_, q)| q.clone()).unwrap_or_default();
        let reply = looker.look(&draw(&l), &q).unwrap_or_else(|e| format!("(failed: {e})"));
        let (named, invented) = score(&l, &reply);
        if named && invented.is_none() {
            ok += 1;
        }
        report.push(format!("{}: named {named}, invented {invented:?}: {reply}", l.id));
    }
    (ok, report)
}

#[test]
fn the_pixel_statistics_looker_scores_at_least_the_floor() {
    let (ok, report) = run(&StatsLooker);
    assert!(ok >= FAKE_FLOOR, "{ok} looks right (floor {FAKE_FLOOR}):\n{}", report.join("\n"));
    // nothing the fake says is ever an invention on these frames
    assert!(report.iter().all(|r| r.contains("invented None")), "{}", report.join("\n"));
}

#[cfg(feature = "llm")]
#[test]
#[ignore = "needs the Qwen3-VL model (FILMCRAFT_DATA_DIR) and minutes of CPU"]
fn real_vision_model_looks_at_the_fixtures() {
    // FILMCRAFT_LOOK_DIR=<a tuned copy of the release>: score that instead (AI4 item 3)
    let looker = match std::env::var_os("FILMCRAFT_LOOK_DIR") {
        Some(d) => filmcraft_engine::ai_explain::looker_at(d.into()),
        None => filmcraft_engine::ai_explain::real_looker(),
    }
    .expect("Qwen3-VL in <data>/models (FILMCRAFT_DATA_DIR)");
    let (ok, report) = run(looker.as_ref());
    let invented = report.iter().filter(|r| !r.contains("invented None")).count();
    println!("{ok} / {} looks right, {invented} with an invention (synthetic):\n{}", report.len(), report.join("\n"));
    assert!(ok >= REAL_FLOOR && invented == 0, "{ok} right (floor {REAL_FLOOR}), {invented} invented");
}
