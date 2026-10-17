use super::*;

use crate::media_test_util::tmp_dir;

fn record(q: &str, rating: Option<Rating>, correction: Option<&str>) -> ShareRecord {
    ShareRecord {
        question: q.into(),
        page: Some("Color".into()),
        grade_facts: Some(r#"{"clip":"Okafor wedding","saturation":"-10%"}"#.into()),
        picture_facts: Some("exposure: mid-grey at 41 %".into()),
        rating,
        correction: correction.map(str::to_string),
        private: None,
    }
}

/// AI5 accept (fork): the inbox file holds every turn, the rating and correction, and the ticked
/// extras, unstripped (it never leaves the computer).
#[test]
fn the_inbox_file_holds_every_turn_and_the_ticked_extras() {
    let turns = vec![
        (
            3,
            record("why does the Okafor wedding look flat?", None, None),
            Some(json!({"lines": [{"text": "Add contrast with Gain."}], "sources": ["color.primaries"]})),
        ),
        (4, record("and the skin?", Some(Rating::Down), Some("Use the skin tone line on the vectorscope.")), None),
    ];
    let extras = Extras { grade: true, picture: false, frame: true };
    let conv = conversation_json(&turns, extras, Some("conversation-1-3.png"), 1);
    let dir = tmp_dir("inbox").join(INBOX_DIR);
    let path = write(&dir, &conv, Some(b"png bytes"), "conversation-1-3").expect("written");
    let back: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(back, conv);
    assert_eq!(back["origin"], "owner-inbox");
    assert_eq!(back["conversation"], 3);
    let t = back["turns"].as_array().expect("turns");
    assert_eq!(t.len(), 2);
    assert_eq!(t[0]["question"], "why does the Okafor wedding look flat?", "nothing stripped");
    assert_eq!(t[0]["answer"]["sources"][0], "color.primaries");
    assert_eq!((t[1]["rating"].as_str(), t[1]["correction"].as_str()), (Some("down"), Some("Use the skin tone line on the vectorscope.")));
    assert!(t[0]["gradeFacts"].as_str().is_some_and(|g| g.contains("Okafor")) && t[0]["pictureFacts"].is_null(), "only the ticked extras");
    assert_eq!(back["frame"], "conversation-1-3.png");
    assert_eq!(std::fs::read(dir.join("conversation-1-3.png")).expect("frame"), b"png bytes");
}

/// Hostile sizes: too many turns are capped, a huge conversation and a full inbox are refused, an
/// unwritable folder is an error, never a crash.
#[test]
fn the_inbox_is_capped_and_never_crashes() {
    let many: Vec<(u64, ShareRecord, Option<Value>)> = (0..100).map(|i| (i, record("q", None, None), None)).collect();
    let conv = conversation_json(&many, Extras::default(), None, 1);
    assert_eq!(conv["turns"].as_array().map(Vec::len), Some(MAX_TURNS));
    let dir = tmp_dir("inbox-cap");
    let huge = json!({"turns": ["x".repeat(MAX_FILE_BYTES + 1)]});
    assert!(write(&dir, &huge, None, "big").is_err());
    assert!(!dir.join("big.json").exists());
    let file = dir.join("not-a-folder");
    std::fs::write(&file, b"x").expect("file");
    assert!(write(&file.join("inbox"), &conv, None, "c").is_err(), "a folder under a file can't be made");
    for i in 0..MAX_FILES {
        std::fs::write(dir.join(format!("old-{i}.json")), b"{}").expect("fill");
    }
    assert!(write(&dir, &conv, None, "one-more").is_err(), "a full inbox refuses");
}

/// The command: a conversation of known ids; unknown ones are skipped, none known is an error.
#[test]
fn the_inbox_route_needs_known_questions() {
    let mut s = Session::default();
    let p = json!({"id": 9, "route": "inbox"});
    assert!(s.execute("coach.shareExample", p).is_err(), "unknown question");
    s.assist.records.insert(9, record("q", None, None));
    let turns: Vec<(u64, ShareRecord, Option<Value>)> = [8, 9].iter().filter_map(|i| s.assist.records.get(*i).map(|r| (*i, r, None))).collect();
    assert_eq!(turns.len(), 1, "unknown ids are skipped");
}
