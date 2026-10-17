use super::*;
use std::path::PathBuf;
use std::sync::Arc;

use filmcraft_media::DemoScene;
use serde_json::json;

use crate::ai_explain::{ExplainBackend, Fallback};
use crate::media_test_util::{make_movie, session_with, tmp_dir};

/// A model that always gives the same answer.
struct Fixed(&'static str);

impl ExplainBackend for Fixed {
    fn ready(&self, _: &filmcraft_llm::ModelInfo) -> std::result::Result<(), (Fallback, String)> {
        Ok(())
    }
    fn generate(&self, _: &filmcraft_llm::ModelInfo, _: &str, _: &str, _: filmcraft_llm::GenOptions) -> std::result::Result<String, String> {
        Ok(self.0.into())
    }
}

/// Names a real project could hold, none of which may leave the computer.
const PROJECT: &str = "Okafor Wedding Secret Cut";
const BIN: &str = "Client Rushes Okafor";
const CLIP: &str = "jane-doe-private-take";

/// A session whose project, bin, clip and file names are all private, with one answered question
/// whose wording and correction mention them, a path, a LAN address and an e-mail.
fn private_session() -> (Session, u64, PathBuf) {
    let dir = tmp_dir("share").join("Okafor family");
    std::fs::create_dir_all(&dir).expect("dir");
    let movie = dir.join(format!("{CLIP}.mov"));
    make_movie(&movie, DemoScene::Aurora, 64, 36, 4);
    let (mut s, items, _) = session_with(&[&movie]);
    let mut p = (*s.project).clone();
    p.name = PROJECT.into();
    p.root.children.push(filmcraft_project::BinEntry::Bin(filmcraft_project::Bin { id: Default::default(), name: BIN.into(), children: vec![] }));
    if let Some(it) = p.items.get_mut(&items[0]) {
        it.metadata.insert("Scene".into(), "Grandma Okafor toast".into());
    }
    s.project = Arc::new(p);
    s.path = Some(dir.join("okafor.fcproj").display().to_string());
    s.assist.backend = Some(Arc::new(Fixed("Give each node one job, then adjust each on its own.")));
    let q = format!("why does {CLIP} from {} look flat? my NAS is <lan-address>, mail me at jane@example.org", movie.display());
    let r = s
        .execute(
            "coach.assist",
            json!({"question": q, "page": "Color", "style": "free", "gradeFacts": format!("{{\"clip\":\"{CLIP}\",\"saturation\":\"-10%\"}}")}),
        )
        .expect("asks");
    let id = r["id"].as_u64().expect("id");
    s.execute("coach.assistResult", json!({"id": id, "wait": true})).expect("answered");
    let correction = format!("It should say the {PROJECT} footage in {BIN} needs the F-Log2 input transform; files at \\\\nas.local\\share and fe80::1");
    s.execute("coach.feedback", json!({"id": id, "rating": "down", "correction": correction})).expect("rated");
    (s, id, dir)
}

fn assert_private_gone(text: &str) {
    for leak in [PROJECT, BIN, CLIP, "Okafor", "okafor.fcproj", "<lan-address>", "jane@example.org", "nas.local", "fe80::1", "/tmp", "Grandma"] {
        assert!(!text.to_lowercase().contains(&leak.to_lowercase()), "`{leak}` leaked:\n{text}");
    }
    if let Ok(home) = std::env::var("HOME") {
        assert!(!text.contains(&home), "home leaked:\n{text}");
    }
}

/// Undoes [`url_encode`] for the test (ASCII and UTF-8 bytes).
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).expect("hex");
            out.push(u8::from_str_radix(h, 16).expect("hex"));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("utf-8")
}

fn query_value(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}="))).map(url_decode)
}

/// AI5 accept: the preview is what the GitHub link and the saved file carry, byte for byte.
#[test]
fn the_preview_is_exactly_what_is_shared_or_saved() {
    let (mut s, id, dir) = private_session();
    let ticks = json!({"id": id, "grade": true, "picture": true});
    let mut preview_params = ticks.clone();
    preview_params["route"] = json!("preview");
    let preview = s.execute("coach.shareExample", preview_params).expect("preview")["text"].as_str().expect("text").to_string();
    assert!(preview.contains("Give each node one job") && preview.contains("👎") && preview.contains("F-Log2 input transform"), "{preview}");
    assert!(preview.contains("Page: Color") && preview.contains("saturation"), "{preview}");

    let mut gh = ticks.clone();
    gh["route"] = json!("github");
    gh["consent"] = json!(true);
    let r = s.execute("coach.shareExample", gh).expect("github");
    let url = r["url"].as_str().expect("url");
    assert_eq!(r["tooLong"], false);
    assert!(url.starts_with("https://github.com/dawgon2/revolution/issues/new?"), "{url}");
    assert_eq!(query_value(url, "labels").as_deref(), Some("assistant-example"));
    assert_eq!(query_value(url, "body").as_deref(), Some(preview.as_str()), "the link's body differs from the preview");
    assert_private_gone(&query_value(url, "title").expect("title"));

    let mut file = ticks;
    file["route"] = json!("file");
    file["folder"] = json!(dir.display().to_string());
    let r = s.execute("coach.shareExample", file).expect("file");
    let saved = std::fs::read_to_string(r["path"].as_str().expect("path")).expect("read");
    assert_eq!(saved, preview, "the saved file differs from the preview");
}

/// AI5 accept: names, paths, addresses and e-mails never appear, whatever is ticked.
#[test]
fn private_names_never_appear() {
    let (mut s, id, _) = private_session();
    for (grade, picture) in [(false, false), (true, true)] {
        let r = s.execute("coach.shareExample", json!({"id": id, "route": "preview", "grade": grade, "picture": picture})).expect("preview");
        let text = r["text"].as_str().expect("text");
        assert_private_gone(text);
        let named = text.contains("<clip>") || text.contains("<file>");
        assert!(named && text.contains("<path>") && text.contains("<address>") && text.contains("<email>"), "{text}");
    }
    // unticked extras stay out
    let r = s.execute("coach.shareExample", json!({"id": id})).expect("preview");
    let text = r["text"].as_str().expect("text");
    assert!(!text.contains("saturation") && !text.contains("## Frame"), "{text}");
}

/// AI5 accept: nothing goes to GitHub without the consent tick; saving works without it.
#[test]
fn github_needs_consent_and_saving_does_not() {
    let (mut s, id, dir) = private_session();
    for consent in [json!(null), json!(false)] {
        let e = s.execute("coach.shareExample", json!({"id": id, "route": "github", "consent": consent}));
        assert!(e.is_err(), "shared without consent");
    }
    let r = s.execute("coach.shareExample", json!({"id": id, "route": "file", "folder": dir.display().to_string()})).expect("saves without consent");
    assert!(std::path::Path::new(r["path"].as_str().expect("path")).is_file());
}

/// The frame is saved next to the text under the name the text gives; the link can't carry it.
#[test]
fn a_ticked_frame_is_saved_next_to_the_text() {
    let (mut s, id, dir) = private_session();
    let out = dir.join("out");
    std::fs::create_dir_all(&out).expect("dir");
    let r = s.execute("coach.shareExample", json!({"id": id, "route": "file", "frame": true, "folder": out.display().to_string()})).expect("file");
    let frame = r["framePath"].as_str().expect("frame saved");
    let text = std::fs::read_to_string(r["path"].as_str().expect("path")).expect("read");
    assert!(text.contains("A small frame goes with this example"), "{text}");
    assert_eq!(std::path::Path::new(frame).file_stem(), std::path::Path::new(r["path"].as_str().expect("path")).file_stem(), "text and frame share a name");
    let img = image::open(frame).expect("png");
    assert!(img.width() > 0 && img.width() <= 480, "{}", img.width());
    // a second save doesn't overwrite the first
    let r2 = s.execute("coach.shareExample", json!({"id": id, "route": "file", "folder": out.display().to_string()})).expect("again");
    assert_ne!(r2["path"], r["path"]);
}

/// A long example goes as a short note that says to paste the saved file.
#[test]
fn a_long_example_says_to_paste_the_file() {
    let (mut s, id, _) = private_session();
    s.execute("coach.feedback", json!({"id": id, "rating": "down", "correction": "üüü ".repeat(500)})).expect("rated");
    let r = s.execute("coach.shareExample", json!({"id": id, "route": "github", "consent": true})).expect("github");
    let url = r["url"].as_str().expect("url");
    assert_eq!(r["tooLong"], true);
    assert!(url.len() <= MAX_URL_CHARS && query_value(url, "body").is_some_and(|b| b.contains("Save example")), "{url}");
}

/// Hostile and wrong input: errors, never a crash.
#[test]
fn bad_requests_are_refused() {
    let (mut s, id, dir) = private_session();
    let bad = [
        ("coach.feedback", json!({})),
        ("coach.feedback", json!({"id": 999_999})),
        ("coach.feedback", json!({"id": id, "rating": "sideways"})),
        ("coach.shareExample", json!({"id": 999_999})),
        ("coach.shareExample", json!({"id": id, "route": "carrier-pigeon"})),
        ("coach.shareExample", json!({"id": id, "route": "file"})),
        ("coach.shareExample", json!({"id": id, "route": "file", "folder": dir.join("missing").display().to_string()})),
        ("coach.shareExample", json!({"id": id, "route": "file", "folder": "\u{0}"})),
    ];
    for (cmd, p) in bad {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
    // a huge correction is capped; clearing the rating works
    s.execute("coach.feedback", json!({"id": id, "rating": "up", "correction": "x".repeat(1_000_000)})).expect("capped");
    assert_eq!(s.assist.records.get(id).and_then(|r| r.correction).map(|c| c.chars().count()), Some(MAX_CORRECTION_CHARS));
    s.execute("coach.feedback", json!({"id": id, "rating": "none"})).expect("cleared");
    assert_eq!(s.assist.records.get(id).map(|r| (r.rating, r.correction)), Some((None, None)));
}

#[test]
fn patterns_are_stripped_word_by_word() {
    let cases = [
        ("see /home/someone/Videos/a.mov.", "see <path>."),
        ("run --out=/home/someone/x.mov now", "run --out=<path> now"),
        ("**/home/someone/x**", "**<path>**"),
        ("model at http://<lan-address>:11434/api", "model at <address>"),
        ("share smb://nas.local/share and nas.local:445", "share <address> and <computer>"),
        ("link fe80::1%eth0 up", "link <address> up"),
        ("on C:\\Users\\x\\clip.mp4 now", "on <path> now"),
        ("at ~/clips and file:///x", "at <path> and <address>"),
        ("host 10.0.0.7:8080 ok", "host <address> ok"),
        ("mail a.b@c.de", "mail <email>"),
        ("ping studio.local!", "ping <computer>!"),
        ("Lift 0.25, Gamma 1.0 at 50% and 1/2 speed", "Lift 0.25, Gamma 1.0 at 50% and 1/2 speed"),
        ("timecode 01:02:03:04 stays", "timecode 01:02:03:04 stays"),
    ];
    for (input, want) in cases {
        assert_eq!(strip_patterns(input), want, "{input}");
    }
}

fn names(list: &[(&str, &'static str)]) -> Private {
    let mut p = Private::default();
    for (n, s) in list {
        p.add(n, s);
    }
    p
}

/// Review finding: overlapping names are taken out together, case is ignored (also outside
/// ASCII), and short names only as whole words.
#[test]
fn names_overlap_case_and_whole_words() {
    let p = names(&[
        ("Beach Party", "<clip>"),
        ("Sarah Beach", "<clip>"),
        ("Beach day 2", "<clip>"),
        ("2 Trip", "<clip>"),
        ("dan", "<user>"),
        ("Über Dreh", "<bin>"),
    ]);
    assert_eq!(p.strip("at Sarah Beach Party."), "at <clip>.");
    assert_eq!(p.strip("BEACH DAY 2 TRIP!"), "<clip>!");
    assert_eq!(p.strip("dan can dance; ask Dan."), "<user> can dance; ask <user>.");
    assert_eq!(p.strip("über dreh and ÜBER DREH"), "<bin> and <bin>");
    assert_eq!(p.strip("Café"), "Café");
    // a later name never matches inside a stand-in
    assert_eq!(names(&[("clip", "<clip>"), ("Clips A", "<clip>")]).strip("the clip"), "the <clip>");
}

/// Review finding: a path with spaces leaks no folder: the media's folders are names too.
#[test]
fn folders_of_media_paths_are_private() {
    let mut p = Private::default();
    p.add_path("/home/someone/My Videos/Beach Trip/x.mov");
    let t = p.strip("I keep it in /home/someone/My Videos/Beach Trip/ and Beach Trip copies");
    assert!(!t.contains("Beach") && !t.contains("My Videos") && !t.contains("someone"), "{t}");
    assert!(t.contains("Videos") || t.contains("<"), "{t}");
}

/// Review finding: names cap fails closed; duplicates don't use it up.
#[test]
fn too_many_names_refuse_to_share() {
    let mut p = Private::default();
    for _ in 0..10 {
        p.add("Same Name", "<clip>");
    }
    assert!(!p.overflow);
    for i in 0..=MAX_NAMES {
        p.add(&format!("name {i}"), "<clip>");
    }
    assert!(p.overflow);
}

/// Review finding: a question asked in one project can't leak its names after another project opens.
#[test]
fn names_from_when_the_question_was_asked_stay_private() {
    let (mut s, id, _) = private_session();
    s.execute("file.openDemoProject", json!({})).expect("another project");
    let text = s.execute("coach.shareExample", json!({"id": id, "grade": true})).expect("preview")["text"].as_str().expect("text").to_string();
    assert_private_gone(&text);
}

/// Review finding: a share is refused when the example changed since its preview.
#[test]
fn a_changed_example_is_not_shared() {
    let (mut s, id, dir) = private_session();
    let hash = s.execute("coach.shareExample", json!({"id": id})).expect("preview")["hash"].as_str().expect("hash").to_string();
    let ok = json!({"id": id, "route": "file", "folder": dir.display().to_string(), "expect": hash});
    s.execute("coach.shareExample", ok).expect("unchanged: saved");
    let changed = json!({"id": id, "route": "file", "grade": true, "folder": dir.display().to_string(), "expect": hash});
    assert!(s.execute("coach.shareExample", changed).is_err());
}
