//! White-balance picker and Auto balance (GRADE-PLAN G2c): `grade.wb.pick`, `grade.wb.auto`,
//! `lumetri.wb.pick`, `lumetri.wb.auto` on colour mattes with known casts.

use serde_json::{Value, json};

use crate::Session;
use filmcraft_project::ClipId;
use filmcraft_project::grade::Grade;

fn matte(hex: &str) -> (Session, ClipId) {
    let mut s = Session::default();
    let id = s.execute("file.newColorMatte", json!({"color": hex, "width": 160, "height": 90, "seconds": 2})).unwrap()["item"].as_u64().unwrap();
    s.execute("file.newSequenceFromClip", json!({"items": [id]})).unwrap();
    let q = s.active_sequence().unwrap();
    let c = q.video_tracks[0].item_at(s.playhead()).unwrap().id;
    s.state.selection = vec![c];
    (s, c)
}

fn grade(s: &Session, c: ClipId) -> Option<Grade> {
    s.active_sequence().unwrap().find_item(c).unwrap().1.grade.clone()
}

fn node(s: &mut Session, c: ClipId) -> u64 {
    s.execute("grade.addNode", json!({"clip": c.0})).unwrap()["node"].as_u64().unwrap()
}

fn tt(r: &Value) -> (f64, f64) {
    (r["temperature"].as_f64().unwrap(), r["tint"].as_f64().unwrap())
}

fn lumetri_tt(s: &Session, c: ClipId) -> (f64, f64) {
    let it = s.active_sequence().unwrap().find_item(c).unwrap().1.clone();
    let e = it.effects.iter().find(|e| e.effect == "lumetri").unwrap();
    let f = |k: &str| e.param(k).and_then(|p| p.value.as_f64()).unwrap();
    (f("temperature"), f("tint"))
}

#[test]
fn pick_neutralises_a_warm_cast_in_one_undo_step_and_twice_changes_nothing() {
    // a warm, slightly green grey
    let (mut s, c) = matte("#a08c78");
    let a = node(&mut s, c);
    let before = grade(&s, c);
    let r = s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap();
    let (t, n) = tt(&r);
    assert!(t < -5.0, "warm needs a cooler temperature: {r}");
    assert_eq!(r["clamped"], json!(false));
    let g = grade(&s, c).unwrap();
    let at = |k| g.nodes[0].float_at(k, filmcraft_time::Tick(0)) as f64;
    assert!((at("temperature") - t).abs() < 1e-3 && (at("tint") - n).abs() < 1e-3);
    // picking again samples the node's input: same values
    let r2 = s.execute("grade.wb.pick", json!({"node": a, "x": 0.3, "y": 0.6})).unwrap();
    let (t2, n2) = tt(&r2);
    assert!((t2 - t).abs() < 1e-3 && (n2 - n).abs() < 1e-3, "{r} vs {r2}");
    // the node's output is neutral: a node after it finds nothing to do
    let b = node(&mut s, c);
    let r3 = s.execute("grade.wb.pick", json!({"node": b, "x": 0.5, "y": 0.5})).unwrap();
    let (t3, n3) = tt(&r3);
    assert!(t3.abs() < 0.5 && n3.abs() < 0.5, "{r3}");
    // one undo step per pick
    s.execute("edit.undo", json!({})).unwrap(); // pick on b
    s.execute("edit.undo", json!({})).unwrap(); // add b
    s.execute("edit.undo", json!({})).unwrap(); // second pick on a
    s.execute("edit.undo", json!({})).unwrap(); // first pick on a
    assert_eq!(grade(&s, c), before);
}

#[test]
fn a_cast_beyond_the_sliders_clamps_and_says_so() {
    let (mut s, c) = matte("#b49678");
    let a = node(&mut s, c);
    let r = s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap();
    assert_eq!(r["clamped"], json!(true), "{r}");
    assert_eq!(tt(&r).0, -100.0);
    assert!(r["message"].as_str().unwrap().contains("limit"));
}

#[test]
fn a_box_and_auto_balance_agree_with_a_click_on_a_flat_frame() {
    let (mut s, c) = matte("#7890b4");
    let a = node(&mut s, c);
    let click = tt(&s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap());
    let rect = tt(&s.execute("grade.wb.pick", json!({"node": a, "x": 0.9, "y": 0.9, "x2": 0.1, "y2": 0.2})).unwrap());
    let auto = tt(&s.execute("grade.wb.auto", json!({"node": a})).unwrap());
    assert!(click.0 > 5.0, "cool needs a warmer temperature: {click:?}");
    for o in [rect, auto] {
        assert!((o.0 - click.0).abs() < 1e-2 && (o.1 - click.1).abs() < 1e-2, "{o:?} vs {click:?}");
    }
}

#[test]
fn bad_picks_are_refused_with_a_reason_and_change_nothing() {
    for (hex, word) in [("#ffffff", "clipped"), ("#000000", "dark"), ("#ff0000", "colourful")] {
        let (mut s, c) = matte(hex);
        let a = node(&mut s, c);
        let before = grade(&s, c);
        let effects = s.active_sequence().unwrap().find_item(c).unwrap().1.effects.clone();
        let e = s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap_err().to_string();
        assert!(e.contains(word), "{hex}: {e}");
        assert!(s.execute("grade.wb.auto", json!({"node": a})).is_err(), "{hex}");
        assert!(s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).is_err(), "{hex}");
        assert_eq!(grade(&s, c), before, "{hex}");
        // (a refused Lumetri pick doesn't leave a new Lumetri behind)
        assert_eq!(s.active_sequence().unwrap().find_item(c).unwrap().1.effects, effects, "{hex}");
    }
    let (mut s, c) = matte("#a08c78");
    let a = node(&mut s, c);
    for p in [
        json!({"node": a}),
        json!({"node": a, "x": "left", "y": 0.5}),
        json!({"node": a, "x": null, "y": 0.5}),
        json!({"node": 999_999, "x": 0.5, "y": 0.5}),
        json!({"node": -1, "x": 0.5, "y": 0.5}),
    ] {
        assert!(s.execute("grade.wb.pick", p.clone()).is_err(), "{p}");
        if !p["x"].is_f64() {
            assert!(s.execute("lumetri.wb.pick", p.clone()).is_err(), "{p}");
        }
    }
    // out-of-frame points are refused with a reason; the frame's edges are fine
    for p in [json!({"node": a, "x": -5.0, "y": 0.5}), json!({"node": a, "x": 0.5, "y": 1e300}), json!({"node": a, "x": 0.5, "y": 0.5, "x2": 7.0, "y2": 0.5})] {
        let e = s.execute("grade.wb.pick", p.clone()).unwrap_err().to_string();
        assert!(e.contains("outside the frame"), "{p}: {e}");
        assert!(s.execute("lumetri.wb.pick", p.clone()).is_err(), "{p}");
    }
    s.execute("grade.wb.pick", json!({"node": a, "x": 0.0, "y": 1.0, "x2": 1.0, "y2": 0.0})).unwrap();
    // no grade yet: refused
    let (mut s, c) = matte("#a08c78");
    assert!(s.execute("grade.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).is_err());
}

#[test]
fn lumetri_wb_selector_matches_the_node_picker_and_twice_changes_nothing() {
    let (mut s, c) = matte("#a08cb4");
    let a = node(&mut s, c);
    let want = tt(&s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap());
    s.execute("grade.clear", json!({"clip": c.0})).unwrap();
    let r = s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap();
    assert!((tt(&r).0 - want.0).abs() < 1e-2 && (tt(&r).1 - want.1).abs() < 1e-2, "{r} vs {want:?}");
    let got = lumetri_tt(&s, c);
    assert!((got.0 - want.0).abs() < 1e-2 && (got.1 - want.1).abs() < 1e-2);
    s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.2, "y": 0.2})).unwrap();
    let again = lumetri_tt(&s, c);
    assert!((again.0 - got.0).abs() < 1e-3 && (again.1 - got.1).abs() < 1e-3);
    let auto = tt(&s.execute("lumetri.wb.auto", json!({"clip": c.0})).unwrap());
    assert!((auto.0 - got.0).abs() < 1e-2 && (auto.1 - got.1).abs() < 1e-2);
    // one undo step: back to the first pick's values (the picks set the same values)
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(lumetri_tt(&s, c), again);
}

#[test]
fn lumetri_pick_on_a_clip_without_lumetri_adds_it_in_the_same_undo_step() {
    let (mut s, c) = matte("#8c8ca0");
    let effects = s.active_sequence().unwrap().find_item(c).unwrap().1.effects.clone();
    assert!(!effects.iter().any(|e| e.effect == "lumetri"));
    let r = s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap();
    assert!(tt(&r).0 > 1.0, "a blue cast needs a warmer temperature: {r}");
    let it = s.active_sequence().unwrap().find_item(c).unwrap().1.clone();
    let pos = it.effects.iter().position(|e| e.effect == "lumetri").unwrap();
    // before the intrinsic effects, as effects.apply puts it
    assert!(it.effects[pos..].iter().skip(1).all(|e| e.def().is_some_and(|d| d.intrinsic)));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_sequence().unwrap().find_item(c).unwrap().1.effects, effects, "one undo step");
}

#[test]
fn lumetri_with_basic_off_and_an_input_lut_samples_after_the_lut() {
    let (mut s, c) = matte("#a08c78");
    s.execute("lumetri.setInputLut", json!({"clip": c.0, "lut": "builtin:look-monochrome"})).unwrap();
    s.execute("lumetri.setSection", json!({"clip": c.0, "section": "basic", "on": false})).unwrap();
    let r1 = s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap();
    // (a black-and-white Input LUT: the white balance sees grey)
    assert!(tt(&r1).0.abs() < 0.5 && tt(&r1).1.abs() < 0.5, "{r1}");
    let it = s.active_sequence().unwrap().find_item(c).unwrap().1.clone();
    let e = it.effects.iter().find(|e| e.effect == "lumetri").unwrap();
    assert_eq!(e.param("basic_on").and_then(|p| p.value.as_bool()), Some(true));
    // picking again (now with Basic on) gives the same values: the LUT was in the first sample too
    let r2 = s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap();
    assert!((tt(&r1).0 - tt(&r2).0).abs() < 1e-3 && (tt(&r1).1 - tt(&r2).1).abs() < 1e-3, "{r1} vs {r2}");
    // and it differs from a pick without the LUT
    let (mut s2, c2) = matte("#a08c78");
    let plain = tt(&s2.execute("lumetri.wb.pick", json!({"clip": c2.0, "x": 0.5, "y": 0.5})).unwrap());
    assert!((plain.0 - tt(&r1).0).abs() > 0.5 || (plain.1 - tt(&r1).1).abs() > 0.5, "{plain:?} vs {r1}");
}

#[test]
fn lumetri_undo_returns_to_the_previous_values() {
    let (mut s, c) = matte("#a08c78");
    s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap();
    let idx = s.active_sequence().unwrap().find_item(c).unwrap().1.effects.iter().position(|e| e.effect == "lumetri").unwrap();
    s.execute("effects.setParam", json!({"clip": c.0, "effect": idx, "param": "temperature", "value": 33.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": c.0, "effect": idx, "param": "tint", "value": -12.0})).unwrap();
    s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(lumetri_tt(&s, c), (33.0, -12.0));
}

#[test]
fn picks_in_a_davinci_wide_sequence_and_on_an_animated_node() {
    let (mut s, c) = matte("#a08c78");
    s.execute("sequence.colorSettings", json!({"workingSpace": "dwg-di"})).unwrap();
    let a = node(&mut s, c);
    let r = s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap();
    assert!(tt(&r).0 < -5.0, "{r}");
    // a node after it finds the picture neutral
    let b = node(&mut s, c);
    let r2 = tt(&s.execute("grade.wb.pick", json!({"node": b, "x": 0.5, "y": 0.5})).unwrap());
    assert!(r2.0.abs() < 0.5 && r2.1.abs() < 0.5, "{r2:?}");
    // animated Temperature: the pick lands as a keyframe at the playhead
    s.execute("grade.toggleAnimation", json!({"node": a, "param": "temperature"})).unwrap();
    s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap();
    let g = grade(&s, c).unwrap();
    let n = g.node(a).unwrap();
    assert!(!n.param("temperature").unwrap().keyframes.is_empty());
    assert!((f64::from(n.float_at("temperature", filmcraft_time::Tick(0))) - tt(&r).0).abs() < 1e-2);
}

fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, p).unwrap_err().to_string()
}

#[test]
fn clipped_spots_are_refused_in_davinci_wide_and_hdr_sequences_too() {
    for ws in ["dwg-di", "rec2100-pq", "rec2100-hlg", "rec709"] {
        let (mut s, c) = matte("#ffffff");
        s.execute("sequence.colorSettings", json!({"workingSpace": ws})).unwrap();
        let a = node(&mut s, c);
        let before = grade(&s, c);
        let e = err(&mut s, "grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5}));
        assert!(e.contains("clipped"), "{ws}: {e}");
        assert!(s.execute("grade.wb.auto", json!({"node": a})).is_err(), "{ws}");
        let e = err(&mut s, "lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5}));
        assert!(e.contains("clipped"), "{ws}: {e}");
        assert!(s.execute("lumetri.wb.auto", json!({"clip": c.0})).is_err(), "{ws}");
        assert_eq!(grade(&s, c), before, "{ws}");
        // one blown channel (a warm white: red at the top) counts too
        let (mut s, c) = matte("#fff0dc");
        s.execute("sequence.colorSettings", json!({"workingSpace": ws})).unwrap();
        let a = node(&mut s, c);
        let e = err(&mut s, "grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5}));
        assert!(e.contains("clipped"), "{ws}: {e}");
    }
}

#[test]
fn unclipped_spots_still_solve_exactly_in_davinci_wide_and_hdr_sequences() {
    for ws in ["dwg-di", "rec2100-pq", "rec2100-hlg"] {
        // a mid and a bright (but unclipped) warm grey
        for hex in ["#a08c78", "#e6d7c8"] {
            let (mut s, c) = matte(hex);
            s.execute("sequence.colorSettings", json!({"workingSpace": ws})).unwrap();
            let a = node(&mut s, c);
            let r = s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5})).unwrap();
            assert!(tt(&r).0 < -2.0, "{ws} {hex}: {r}");
            // the node's output is neutral: a node after it finds nothing to do
            let b = node(&mut s, c);
            let r2 = tt(&s.execute("grade.wb.pick", json!({"node": b, "x": 0.5, "y": 0.5})).unwrap());
            assert!(r2.0.abs() < 0.5 && r2.1.abs() < 0.5, "{ws} {hex}: {r2:?}");
            // Auto and Lumetri's selector agree with the node picker
            let auto = tt(&s.execute("grade.wb.auto", json!({"node": a})).unwrap());
            assert!((auto.0 - tt(&r).0).abs() < 0.05 && (auto.1 - tt(&r).1).abs() < 0.05, "{ws} {hex}: {auto:?} vs {r}");
            s.execute("grade.clear", json!({"clip": c.0})).unwrap();
            let l = tt(&s.execute("lumetri.wb.pick", json!({"clip": c.0, "x": 0.5, "y": 0.5})).unwrap());
            assert!((l.0 - tt(&r).0).abs() < 0.05 && (l.1 - tt(&r).1).abs() < 0.05, "{ws} {hex}: {l:?} vs {r}");
        }
    }
}

#[test]
fn clipping_is_judged_on_the_footage_not_on_what_earlier_nodes_did() {
    for ws in ["rec709", "dwg-di"] {
        // a blown white darkened by an earlier node is still blown
        let (mut s, c) = matte("#ffffff");
        s.execute("sequence.colorSettings", json!({"workingSpace": ws})).unwrap();
        let a = node(&mut s, c);
        s.execute("grade.setParam", json!({"node": a, "param": "offset", "value": [-0.3, -0.3, -0.3]})).unwrap();
        let b = node(&mut s, c);
        let e = err(&mut s, "grade.wb.pick", json!({"node": b, "x": 0.5, "y": 0.5}));
        assert!(e.contains("clipped"), "{ws}: {e}");
        // an unclipped grey pushed past white by an earlier node is fine
        let (mut s, c) = matte("#d2c8be");
        s.execute("sequence.colorSettings", json!({"workingSpace": ws})).unwrap();
        let a = node(&mut s, c);
        s.execute("grade.setParam", json!({"node": a, "param": "offset", "value": [0.4, 0.4, 0.4]})).unwrap();
        let b = node(&mut s, c);
        s.execute("grade.wb.pick", json!({"node": b, "x": 0.5, "y": 0.5})).unwrap();
    }
}

#[test]
fn clipping_of_log_footage_is_its_top_code_value() {
    // the matte interpreted as F-Log2: white is the top of the log signal, greys are not
    for (hex, clipped) in [("#ffffff", true), ("#a08c78", false), ("#d2c8be", false)] {
        let (mut s, c) = matte(hex);
        let item = s.active_sequence().unwrap().find_item(c).unwrap().1.item;
        s.execute("clip.interpretFootage", json!({"items": [item.0], "colorSpace": "flog2-fgamut"})).unwrap();
        s.execute("sequence.colorSettings", json!({"workingSpace": "dwg-di"})).unwrap();
        let a = node(&mut s, c);
        let r = s.execute("grade.wb.pick", json!({"node": a, "x": 0.5, "y": 0.5}));
        assert_eq!(r.is_err(), clipped, "{hex}: {r:?}");
        if let Err(e) = r {
            assert!(e.to_string().contains("clipped"), "{hex}: {e}");
        }
    }
}
