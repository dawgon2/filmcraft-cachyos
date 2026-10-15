//! `tnr.*` commands (fork 6.7b step 5): clip-level temporal NR edits, undo / redo, copy / paste,
//! Paste Attributes and hostile parameters.

use serde_json::{Value, json};

use crate::{EngineError, Session};
use filmcraft_project::{ClipId, ParamValue, TemporalNr};
use filmcraft_time::Tick;

/// A demo session with two video clips: (session, first, second); the first is selected.
fn demo() -> (Session, ClipId, ClipId) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let a = q.video_tracks[0].item_at(s.playhead()).unwrap().id;
    let b = q.video_tracks[0].items.iter().map(|i| i.id).find(|i| *i != a).unwrap();
    s.state.selection = vec![a];
    (s, a, b)
}

fn tnr(s: &Session, c: ClipId) -> Option<TemporalNr> {
    s.active_sequence().unwrap().find_item(c).unwrap().1.temporal_nr.clone()
}

fn value(s: &Session, c: ClipId, k: &str) -> Option<ParamValue> {
    tnr(s, c).and_then(|t| t.params.get(k).map(|p| p.value.clone()))
}

fn is_bad(r: crate::Result<Value>) -> bool {
    matches!(r, Err(EngineError::BadParams { .. }))
}

#[test]
fn tnr_commands_edit_undo_and_refuse_hostile_params() {
    let (mut s, a, b) = demo();
    assert!(tnr(&s, a).is_none());
    let list = s.execute("tnr.params", json!({})).unwrap();
    assert_eq!(list["params"].as_array().unwrap().len(), 4);

    // get on a clip without temporal NR: identity values
    let g = s.execute("tnr.get", json!({})).unwrap();
    assert_eq!((g["present"].clone(), g["enabled"].clone(), g["identity"].clone()), (json!(false), json!(true), json!(true)));
    assert_eq!(g["values"]["frames"], json!(1.0));

    // set creates the field (selected clip), clamps, one undo step each
    let undo0 = s.history.undo.len();
    s.execute("tnr.set", json!({"param": "luma", "value": 0.4})).unwrap();
    s.execute("tnr.set", json!({"clip": a.0, "param": "frames", "value": 9.0})).unwrap();
    assert_eq!(s.history.undo.len(), undo0 + 2);
    assert_eq!(value(&s, a, "luma"), Some(ParamValue::Float(0.4)));
    assert_eq!(value(&s, a, "frames"), Some(ParamValue::Float(2.0)), "clamped to the schema");
    let g = s.execute("tnr.get", json!({"clip": a.0})).unwrap();
    assert_eq!((g["present"].clone(), g["identity"].clone()), (json!(true), json!(false)));
    assert!((g["values"]["luma"].as_f64().unwrap() - 0.4).abs() < 1e-6);

    // a slider drag (merge) is one undo step
    let undo1 = s.history.undo.len();
    for v in [0.1, 0.2, 0.3] {
        s.execute("tnr.set", json!({"param": "chroma", "value": v, "merge": true})).unwrap();
    }
    assert_eq!(s.history.undo.len(), undo1 + 1);
    assert_eq!(value(&s, a, "chroma"), Some(ParamValue::Float(0.3)));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(value(&s, a, "chroma"), None);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(value(&s, a, "chroma"), Some(ParamValue::Float(0.3)));

    // toggle: explicit and flip; undo
    assert_eq!(s.execute("tnr.toggle", json!({})).unwrap()["enabled"], json!(false));
    assert!(!tnr(&s, a).unwrap().enabled);
    assert_eq!(s.execute("tnr.toggle", json!({"on": true})).unwrap()["enabled"], json!(true));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!tnr(&s, a).unwrap().enabled);
    s.execute("edit.redo", json!({})).unwrap();
    assert!(tnr(&s, a).unwrap().enabled);
    assert!(is_bad(s.execute("tnr.toggle", json!({"clip": b.0}))), "no temporal NR to flip");
    assert!(is_bad(s.execute("tnr.toggle", json!({"on": "yes"}))));

    // toggleAnimation: keyframes at `time`, then a keyed set
    let t1 = Tick::from_seconds_f64(0.5);
    let r = s.execute("tnr.toggleAnimation", json!({"param": "motion", "time": t1.0})).unwrap();
    assert_eq!(r["animated"], json!(true));
    assert!(tnr(&s, a).unwrap().params["motion"].is_animated());
    s.execute("tnr.set", json!({"param": "motion", "value": 0.9, "time": t1.0})).unwrap();
    assert!(tnr(&s, a).unwrap().params["motion"].is_animated());
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!tnr(&s, a).unwrap().params.contains_key("motion"));
    s.execute("edit.redo", json!({})).unwrap();
    assert!(tnr(&s, a).unwrap().params["motion"].is_animated());

    // moveKeyframe: the lane's drag, one undo step; a missing keyframe / bad times refused
    let from = tnr(&s, a).unwrap().params["motion"].keyframes[0].time;
    let to = from + Tick::from_seconds_f64(1.0);
    s.execute("tnr.moveKeyframe", json!({"param": "motion", "mediaTime": from.0, "to": to.0})).unwrap();
    assert_eq!(tnr(&s, a).unwrap().params["motion"].keyframes[0].time, to);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(tnr(&s, a).unwrap().params["motion"].keyframes[0].time, from);
    assert!(is_bad(s.execute("tnr.moveKeyframe", json!({"param": "motion", "mediaTime": to.0, "to": from.0}))), "no keyframe there");
    assert!(is_bad(s.execute("tnr.moveKeyframe", json!({"param": "motion", "mediaTime": from.0}))), "no `to`");
    assert!(is_bad(s.execute("tnr.moveKeyframe", json!({"param": "luma", "mediaTime": from.0, "to": "x"}))));
    assert!(is_bad(s.execute("tnr.moveKeyframe", json!({"param": "frames", "mediaTime": from.0, "to": to.0}))), "not animated");

    // copy / paste
    s.execute("tnr.copy", json!({})).unwrap();
    let r = s.execute("tnr.paste", json!({"clips": [b.0]})).unwrap();
    assert_eq!(r["clips"], json!(1));
    assert_eq!(tnr(&s, b), tnr(&s, a));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(tnr(&s, b).is_none());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(tnr(&s, b), tnr(&s, a));

    // reset one key, then everything (removes the field); clear
    s.execute("tnr.reset", json!({"param": "luma"})).unwrap();
    assert_eq!(value(&s, a, "luma"), None);
    assert!(tnr(&s, a).is_some());
    s.execute("tnr.reset", json!({})).unwrap();
    assert!(tnr(&s, a).is_none(), "reset without a param removes the field");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(tnr(&s, a).is_some());
    s.execute("tnr.clear", json!({"clip": b.0})).unwrap();
    assert!(tnr(&s, b).is_none());
    assert!(is_bad(s.execute("tnr.clear", json!({"clip": b.0}))), "nothing to remove");
    assert!(is_bad(s.execute("tnr.copy", json!({"clip": b.0}))), "nothing to copy");

    // hostile parameters never change the project
    let before = s.project.clone();
    for p in [
        json!({"param": "x", "value": 0.5}),
        json!({"param": "tnr.luma", "value": 0.5}),
        json!({"param": "luma", "value": f64::NAN}),
        json!({"param": "luma", "value": "0.5"}),
        json!({"param": "luma"}),
        json!({"value": 0.5}),
        json!({"param": 7, "value": 0.5}),
        json!({"clip": "a", "param": "luma", "value": 0.5}),
        json!({"clip": 999_999_999u64, "param": "luma", "value": 0.5}),
        json!({"clip": -1, "param": "luma", "value": 0.5}),
    ] {
        assert!(is_bad(s.execute("tnr.set", p.clone())), "{p}");
    }
    assert!(is_bad(s.execute("tnr.toggleAnimation", json!({"param": "nope"}))));
    assert!(is_bad(s.execute("tnr.reset", json!({"param": "nope"}))));
    assert!(is_bad(s.execute("tnr.reset", json!({"param": 3}))));
    assert!(is_bad(s.execute("tnr.paste", json!({"clips": "all"}))));
    assert!(is_bad(s.execute("tnr.paste", json!({"clips": [999_999_999u64]}))));
    assert!(is_bad(s.execute("tnr.get", json!({"clip": 999_999_999u64}))));
    assert!(std::sync::Arc::ptr_eq(&before, &s.project), "refusals leave the project alone");

    // no selection
    s.state.selection.clear();
    for id in ["tnr.get", "tnr.toggle", "tnr.reset", "tnr.clear", "tnr.copy", "tnr.paste"] {
        assert!(is_bad(s.execute(id, json!({}))), "{id}");
    }
    assert!(is_bad(s.execute("tnr.set", json!({"param": "luma", "value": 0.5}))));

    // a graphic clip is refused
    let g = s.execute("graphics.newText", json!({"text": "Title"})).unwrap();
    let gc = ClipId(g["clip"].as_u64().unwrap());
    let before_g = s.project.clone();
    for (id, p) in [
        ("tnr.set", json!({"clip": gc.0, "param": "luma", "value": 0.5})),
        ("tnr.get", json!({"clip": gc.0})),
        ("tnr.toggle", json!({"clip": gc.0, "on": true})),
        ("tnr.toggleAnimation", json!({"clip": gc.0, "param": "luma"})),
        ("tnr.paste", json!({"clips": [gc.0]})),
    ] {
        match s.execute(id, p) {
            Err(EngineError::BadParams { msg, .. }) => assert!(msg.contains("media clips and subclips"), "{id}: {msg}"),
            other => panic!("{id}: {other:?}"),
        }
    }
    assert!(std::sync::Arc::ptr_eq(&before_g, &s.project));
    assert!(tnr(&s, gc).is_none());
}

#[test]
fn paste_attributes_copies_temporal_nr_with_the_grade_flag_off_and_on() {
    let (mut s, a, b) = demo();
    s.execute("tnr.set", json!({"clip": a.0, "param": "luma", "value": 0.6})).unwrap();
    s.execute("tnr.toggle", json!({"clip": a.0, "on": false})).unwrap();
    s.state.selection = vec![a];
    s.execute("edit.copy", json!({})).unwrap();

    s.execute("edit.pasteAttributes", json!({"clips": [b.0], "temporalNr": false})).unwrap();
    assert!(tnr(&s, b).is_none(), "temporalNr:false leaves it out");

    s.execute("edit.pasteAttributes", json!({"clips": [b.0], "grade": false})).unwrap();
    assert_eq!(tnr(&s, b), tnr(&s, a), "copied with the grade flag off");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(tnr(&s, b).is_none());

    s.execute("edit.pasteAttributes", json!({"clips": [b.0], "grade": true})).unwrap();
    assert_eq!(tnr(&s, b), tnr(&s, a), "copied with the grade flag on");
    assert!(!tnr(&s, b).unwrap().enabled, "the switch travels with the values");

    // a graphic clip never receives one
    let g = s.execute("graphics.newText", json!({"text": "Title"})).unwrap();
    let gc = ClipId(g["clip"].as_u64().unwrap());
    s.execute("edit.pasteAttributes", json!({"clips": [gc.0]})).unwrap();
    assert!(tnr(&s, gc).is_none());
}

/// Since schema v17 temporal NR isn't a node parameter: `grade.setParam tnr.*` points to `tnr.set`.
#[test]
fn grade_set_param_refuses_tnr_keys_with_a_pointer() {
    let (mut s, a, _) = demo();
    let before = s.project.clone();
    let r = s.execute("grade.setParam", json!({"clip": a.0, "param": "tnr.luma", "value": 0.5}));
    let Err(EngineError::BadParams { msg, .. }) = r else { panic!("refused: {r:?}") };
    assert!(msg.contains("tnr.set"), "{msg}");
    assert!(std::sync::Arc::ptr_eq(&before, &s.project));
}

/// A migrated project can carry a temporal NR on a clip that doesn't take one (a Color Matte):
/// it can still be removed; editing it is refused. A huge `clips` list is refused up front.
#[test]
fn a_temporal_nr_on_a_clip_that_doesnt_take_it_can_be_removed() {
    let (mut s, a, _) = demo();
    let item = s.active_sequence().unwrap().find_item(a).unwrap().1.item;
    let p = std::sync::Arc::make_mut(&mut s.project);
    p.item_mut(item).unwrap().as_media_mut().unwrap().media =
        filmcraft_project::MediaRef::Generator(filmcraft_media::Generator::ColorMatte { color: [0.1, 0.2, 0.3, 1.0] });
    let seq = s.state.active_sequence.unwrap();
    std::sync::Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap().find_item_mut(a).unwrap().1.temporal_nr = Some(TemporalNr::default());
    assert!(is_bad(s.execute("tnr.set", json!({"clip": a.0, "param": "luma", "value": 0.5}))));
    s.execute("tnr.clear", json!({"clip": a.0})).unwrap();
    assert!(tnr(&s, a).is_none());
    let many: Vec<u64> = (0..100_000).collect();
    assert!(is_bad(s.execute("tnr.paste", json!({"clips": many}))));
}
