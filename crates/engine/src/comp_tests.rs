//! `comp.*` commands (fork C1): building a comp, graph rules, parameters, undo, hostile params.

use super::*;
use filmcraft_project::comp::{CompKind, MAX_LABEL};
use filmcraft_project::{ItemKind, ParamValue};
use serde_json::json;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn media(s: &Session) -> u64 {
    s.project.items.values().find(|i| matches!(i.kind, ItemKind::Media(_)) && i.has_video()).unwrap().id.0
}

fn add(s: &mut Session, comp: u64, params: serde_json::Value) -> u64 {
    let mut p = params;
    p["comp"] = json!(comp);
    s.execute("comp.addNode", p).unwrap()["node"].as_u64().unwrap()
}

fn connect(s: &mut Session, comp: u64, from: u64, to: u64, port: u8) {
    s.execute("comp.connect", json!({"comp": comp, "from": from, "to": to, "toPort": port})).unwrap();
}

fn get(s: &mut Session, comp: u64) -> serde_json::Value {
    s.execute("comp.get", json!({"comp": comp})).unwrap()
}

/// The command fails and changes nothing (project and undo history).
fn refused(s: &mut Session, command: &str, params: serde_json::Value) -> String {
    let (before, undo) = (s.project.clone(), s.history.undo.len());
    let e = s.execute(command, params).expect_err(command).to_string();
    assert!(Arc::ptr_eq(&before, &s.project), "{command} left the project alone");
    assert_eq!(s.history.undo.len(), undo, "{command} added no undo step");
    e
}

/// Acceptance comp A's shape: Media In → Key → Merge over a Solid → Color → Out.
fn comp_a(s: &mut Session) -> (u64, [u64; 5]) {
    let item = media(s);
    let c = s.execute("comp.new", json!({"name": "A"})).unwrap()["comp"].as_u64().unwrap();
    let m = add(s, c, json!({"kind": "mediaIn", "item": item}));
    let k = add(s, c, json!({"kind": "key", "effect": "luma_key"}));
    let so = add(s, c, json!({"kind": "solid"}));
    let mg = add(s, c, json!({"kind": "merge"}));
    let col = add(s, c, json!({"kind": "color"}));
    connect(s, c, m, k, 0);
    connect(s, c, so, mg, 0);
    connect(s, c, k, mg, 1);
    connect(s, c, mg, col, 0);
    connect(s, c, col, 1, 0);
    (c, [m, k, so, mg, col])
}

#[test]
fn comp_a_builds_and_validates() {
    let mut s = demo();
    let (c, [m, k, so, mg, col]) = comp_a(&mut s);
    let g = get(&mut s, c);
    assert_eq!(g["error"], serde_json::Value::Null);
    assert_eq!(g["order"], json!([m, k, so, mg, col, 1]));
    assert_eq!(g["width"], json!(s.active_sequence().unwrap().settings.width));
    // a comp item is a project item with a picture
    let it = s.project.item(ItemId(c)).unwrap();
    assert!(it.has_video() && it.type_label() == "Comp");
}

#[test]
fn add_after_splices_and_remove_bridges() {
    let mut s = demo();
    let (c, [m, k, ..]) = comp_a(&mut s);
    let b = add(&mut s, c, json!({"kind": "blur", "after": m}));
    let g = get(&mut s, c);
    let edges = g["edges"].as_array().unwrap();
    assert!(edges.contains(&json!({"from": {"node": m, "index": 0}, "to": {"node": b, "index": 0}})));
    assert!(edges.contains(&json!({"from": {"node": b, "index": 0}, "to": {"node": k, "index": 0}})));
    s.execute("comp.removeNode", json!({"comp": c, "nodes": [b]})).unwrap();
    let g = get(&mut s, c);
    assert!(g["edges"].as_array().unwrap().contains(&json!({"from": {"node": m, "index": 0}, "to": {"node": k, "index": 0}})));
    assert_eq!(g["error"], serde_json::Value::Null);
}

#[test]
fn cycles_bad_ports_and_unknown_nodes_are_refused() {
    let mut s = demo();
    let (c, [m, k, _, mg, _]) = comp_a(&mut s);
    let e = refused(&mut s, "comp.connect", json!({"comp": c, "from": mg, "to": k}));
    assert!(e.contains("cycle"), "{e}");
    refused(&mut s, "comp.connect", json!({"comp": c, "from": m, "to": mg, "toPort": 2}));
    refused(&mut s, "comp.connect", json!({"comp": c, "from": 999, "to": mg}));
    refused(&mut s, "comp.addNode", json!({"comp": c, "kind": "warp"}));
    refused(&mut s, "comp.addNode", json!({"comp": c, "kind": "key", "effect": "nope"}));
    refused(&mut s, "comp.addNode", json!({"comp": c, "kind": "blur", "pos": [1e300, 0]}));
    refused(&mut s, "comp.removeNode", json!({"comp": c, "nodes": [999]}));
    refused(&mut s, "comp.disconnect", json!({"comp": c, "to": m}));
    refused(&mut s, "comp.setOutput", json!({"comp": c, "node": 999}));
    refused(&mut s, "comp.setSize", json!({"comp": c, "width": 0}));
    let m = media(&s);
    refused(&mut s, "comp.get", json!({"comp": m}));
    refused(&mut s, "comp.new", json!({"width": 100000}));
}

#[test]
fn a_comp_cannot_read_itself() {
    let mut s = demo();
    let (c, _) = comp_a(&mut s);
    let e = refused(&mut s, "comp.addNode", json!({"comp": c, "kind": "mediaIn", "item": c}));
    assert!(e.contains("nested inside itself"), "{e}");
}

#[test]
fn parameters_are_typed_clamped_and_one_undo_step_per_drag() {
    let mut s = demo();
    let (c, [_, k, so, mg, col]) = comp_a(&mut s);
    let undo = s.history.undo.len();
    s.execute("comp.setParam", json!({"comp": c, "node": mg, "param": "opacity", "value": 30.0})).unwrap();
    s.execute("comp.setParam", json!({"comp": c, "node": mg, "param": "opacity", "value": 250.0})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1, "a drag is one undo step");
    assert_eq!(
        get(&mut s, c)["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(mg)).unwrap()["params"]["opacity"]["value"],
        json!({"Float": 100.0})
    );
    // wrong types, out-of-range choices, unknown keys
    refused(&mut s, "comp.setParam", json!({"comp": c, "node": mg, "param": "opacity", "value": "x"}));
    refused(&mut s, "comp.setParam", json!({"comp": c, "node": mg, "param": "blend", "value": 9999}));
    refused(&mut s, "comp.setParam", json!({"comp": c, "node": mg, "param": "nope", "value": 1}));
    refused(&mut s, "comp.setParam", json!({"comp": c, "node": col, "param": "tnr.luma", "value": 1}));
    // a Color node takes the grade's keys, a Key node its effect's, a Solid a colour
    s.execute("comp.setParam", json!({"comp": c, "node": col, "param": "saturation", "value": 0.5})).unwrap();
    s.execute("comp.setParam", json!({"comp": c, "node": k, "param": "threshold", "value": 40.0})).unwrap();
    s.execute("comp.setParam", json!({"comp": c, "node": so, "param": "color", "value": "#ff8000"})).unwrap();
    let Some(ItemKind::Comp(cc)) = s.project.item(ItemId(c)).map(|i| &i.kind) else { panic!("comp") };
    let CompKind::Color { node } = &cc.node(col).unwrap().kind else { panic!("color") };
    assert_eq!(node.params["saturation"].value, ParamValue::Float(0.5));
    // reset removes the stored value
    s.execute("comp.setParam", json!({"comp": c, "node": mg, "param": "opacity", "reset": true})).unwrap();
    assert!(get(&mut s, c)["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(mg)).unwrap()["params"].get("opacity").is_none());
}

#[test]
fn keyframes_text_and_animation() {
    let mut s = demo();
    let c = s.execute("comp.new", json!({})).unwrap()["comp"].as_u64().unwrap();
    let t = add(&mut s, c, json!({"kind": "text"}));
    let tr = add(&mut s, c, json!({"kind": "transform", "after": t}));
    s.execute("comp.setParam", json!({"comp": c, "node": t, "param": "text", "value": "Title"})).unwrap();
    s.execute(
        "comp.setParam",
        json!({"comp": c, "node": tr, "param": "position", "keyframes": [{"time": 1000, "value": [50, 20]}, {"time": 0, "value": [10, 20]}]}),
    )
    .unwrap();
    let Some(ItemKind::Comp(cc)) = s.project.item(ItemId(c)).map(|i| &i.kind) else { panic!("comp") };
    let keys = &cc.node(tr).unwrap().params["position"].keyframes;
    assert_eq!(keys.iter().map(|k| k.time.0).collect::<Vec<_>>(), vec![0, 1000], "sorted");
    let CompKind::Text { layer } = &cc.node(t).unwrap().kind else { panic!("text") };
    assert_eq!(layer.params["text"].value, ParamValue::Text("Title".into()));
    let r = s.execute("comp.toggleAnimation", json!({"comp": c, "node": tr, "param": "rotation", "time": 0})).unwrap();
    assert_eq!(r["animated"], json!(true));
    refused(&mut s, "comp.setParam", json!({"comp": c, "node": tr, "param": "position", "keyframes": [{"time": 0.5, "value": [1, 1]}]}));
}

#[test]
fn undo_redo_and_labels() {
    let mut s = demo();
    let (c, [m, ..]) = comp_a(&mut s);
    let label = "x".repeat(500);
    s.execute("comp.labelNode", json!({"comp": c, "node": m, "label": label})).unwrap();
    let Some(ItemKind::Comp(cc)) = s.project.item(ItemId(c)).map(|i| &i.kind) else { panic!("comp") };
    assert_eq!(cc.node(m).unwrap().label.chars().count(), MAX_LABEL);
    let before = (*s.project).clone();
    s.execute("comp.bypass", json!({"comp": c, "node": m})).unwrap();
    s.execute("comp.moveNode", json!({"comp": c, "node": m, "pos": [10, 20]})).unwrap();
    s.execute("comp.moveNode", json!({"comp": c, "node": m, "pos": [30, 40]})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.project, before, "bypass + one merged move undone");
    s.execute("edit.redo", json!({})).unwrap();
    let Some(ItemKind::Comp(cc)) = s.project.item(ItemId(c)).map(|i| &i.kind) else { panic!("comp") };
    assert!(cc.node(m).unwrap().bypass);
}

#[test]
fn new_clip_places_a_comp_in_one_undo_step() {
    let mut s = demo();
    let before = (*s.project).clone();
    let undo = s.history.undo.len();
    let r = s.execute("comp.newClip", json!({"track": "V2", "time": 0})).unwrap();
    let comp = ItemId(r["comp"].as_u64().unwrap());
    assert_eq!(s.history.undo.len(), undo + 1);
    let q = s.active_sequence().unwrap();
    assert!(q.video_tracks[1].items.iter().any(|i| i.item == comp && i.start == Tick::ZERO));
    // the selected comp clip is the default `comp`
    let clip = q.video_tracks[1].items.iter().find(|i| i.item == comp).unwrap().id;
    s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
    assert_eq!(s.execute("comp.get", json!({})).unwrap()["comp"], json!(comp.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.project, before);
    // a bad track: nothing made, nothing to undo or redo
    let undo = s.history.undo.len();
    let items = s.project.items.len();
    assert!(s.execute("comp.newClip", json!({"track": "V99"})).is_err());
    assert_eq!((s.history.undo.len(), s.project.items.len()), (undo, items));
}

#[test]
fn mask_nodes_take_masks() {
    let mut s = demo();
    let c = s.execute("comp.new", json!({})).unwrap()["comp"].as_u64().unwrap();
    let m = add(&mut s, c, json!({"kind": "mask"}));
    let mask = filmcraft_project::Mask::linear("m", filmcraft_geom::Vec2::new(0.0, 0.0), filmcraft_geom::Vec2::new(10.0, 10.0));
    s.execute("comp.setMasks", json!({"comp": c, "node": m, "masks": [serde_json::to_value(&mask).unwrap()]})).unwrap();
    let Some(ItemKind::Comp(cc)) = s.project.item(ItemId(c)).map(|i| &i.kind) else { panic!("comp") };
    assert!(matches!(&cc.node(m).unwrap().kind, CompKind::Mask { masks } if masks.len() == 1));
    refused(&mut s, "comp.setMasks", json!({"comp": c, "node": m, "masks": [{"bogus": 1}]}));
    refused(&mut s, "comp.setMasks", json!({"comp": c, "node": 1, "masks": []}));
    let many: Vec<_> = (0..17).map(|_| serde_json::to_value(&mask).unwrap()).collect();
    refused(&mut s, "comp.setMasks", json!({"comp": c, "node": m, "masks": many}));
}

/// A 1280x720 sequence with the demo footage (1920x1080 or so: fitted into the comp) on V1 and V2
/// over the same second range, picture only. Returns the two clips (V1, V2).
fn stacked(s: &mut Session) -> [u64; 2] {
    let item = media(s);
    let q = s.execute("file.newSequence", json!({"name": "Stack", "width": 1280, "height": 720})).unwrap()["sequence"].as_u64().unwrap();
    s.execute("sequence.open", json!({"item": q})).unwrap();
    let put = |s: &mut Session, track: &str| {
        s.execute("timeline.place", json!({"item": item, "track": track, "seconds": 0.0, "audio": false})).unwrap()["clips"][0].as_u64().unwrap()
    };
    [put(s, "V1"), put(s, "V2")]
}

fn frame(s: &Session, t: Tick) -> filmcraft_render::Image {
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let opts = filmcraft_render::RenderOptions { scale: 0.25, ..Default::default() };
    filmcraft_render::render_sequence(&s.project, s.state.active_sequence.unwrap(), t, opts, &provider)
}

#[test]
fn convert_to_comp_clip_keeps_the_picture_and_is_one_undo_step() {
    let mut s = demo();
    let [v1, v2] = stacked(&mut s);
    for (param, value) in [("scale", json!(40.0)), ("position", json!([300.0, 200.0])), ("rotation", json!(15.0))] {
        s.execute("effects.setParam", json!({"clip": v2, "effect": "motion", "param": param, "value": value})).unwrap();
    }
    s.execute("effects.setParam", json!({"clip": v2, "effect": "opacity", "param": "opacity", "value": 60.0})).unwrap();
    let t = Tick(filmcraft_time::TICKS_PER_SECOND / 2);
    let before_img = frame(&s, t);
    let (before, undo) = (s.project.clone(), s.history.undo.len());
    let r = s.execute("comp.convertToCompClip", json!({"clips": [v2, v1], "name": "Stack comp"})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1);
    let comp = r["comp"].as_u64().unwrap();
    let clip = filmcraft_project::ClipId(r["clip"].as_u64().unwrap());
    assert_eq!(s.state.selection, vec![clip]);
    let q = s.active_sequence().unwrap();
    assert_eq!(q.video_tracks[0].items.len(), 1);
    assert!(q.video_tracks[1].items.is_empty());
    assert_eq!(q.video_tracks[0].items[0].id, clip);
    let g = get(&mut s, comp);
    let kinds: Vec<&str> = g["nodes"].as_array().unwrap().iter().filter_map(|n| n["kind"]["type"].as_str()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "mediaIn").count(), 2, "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "transform").count(), 2, "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "merge").count(), 1, "{kinds:?}");
    // the same frame as the stack (resampled through the comp's fitted Media In)
    let after_img = frame(&s, t);
    assert_eq!((before_img.w, before_img.h), (after_img.w, after_img.h));
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in (2..after_img.h.saturating_sub(2)).step_by(3) {
        for x in (2..after_img.w.saturating_sub(2)).step_by(3) {
            let (a, b) = (before_img.get(x, y), after_img.get(x, y));
            sum += a.iter().zip(&b).map(|(p, q)| f64::from((p - q).abs())).sum::<f64>();
            n += 4;
        }
    }
    let mean = sum / n.max(1) as f64;
    let lit = (0..after_img.h).step_by(7).flat_map(|y| (0..after_img.w).step_by(7).map(move |x| (x, y))).filter(|&(x, y)| after_img.get(x, y)[3] > 0.5).count();
    assert!(lit > 20, "the frame shows the footage");
    assert!(mean < 0.02, "mean difference {mean}");
    // one undo step brings the stack back
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.project, *before);
}

#[test]
fn convert_to_comp_clip_refuses_what_it_would_lose() {
    let mut s = demo();
    let [v1, v2] = stacked(&mut s);
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": []})).contains("select"));
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [987654321u64]})).contains("no clip"));
    // an effect on one of them
    s.execute("effects.apply", json!({"clips": [v2], "effect": "gaussian_blur"})).unwrap();
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [v1, v2]})).contains("effect"));
    s.execute("edit.undo", json!({})).unwrap();
    // animated Motion
    s.execute("effects.toggleAnimation", json!({"clip": v2, "effect": "motion", "param": "scale"})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(v2)).unwrap().1.effect("motion").unwrap().param("scale").unwrap().is_animated());
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [v1, v2]})).contains("animated"));
    s.execute("edit.undo", json!({})).unwrap();
    // different times
    let item = media(&s);
    let late = s.execute("timeline.place", json!({"item": item, "track": "V3", "seconds": 60.0, "audio": false})).unwrap()["clips"][0].as_u64().unwrap();
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [v1, late]})).contains("same time"));
    // two on one track
    let next = s.execute("timeline.place", json!({"item": item, "track": "V1", "seconds": 60.0, "audio": false})).unwrap()["clips"][0].as_u64().unwrap();
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [v1, next]})).contains("stacked"));
    // linked sound
    let linked = s.execute("timeline.place", json!({"item": item, "track": "V2", "seconds": 600.0})).unwrap()["clips"].clone();
    let ids: Vec<u64> = linked.as_array().unwrap().iter().filter_map(serde_json::Value::as_u64).collect();
    let q = s.active_sequence().unwrap();
    let pic = ids.iter().copied().find(|c| q.video_tracks.iter().any(|t| t.items.iter().any(|i| i.id.0 == *c))).unwrap();
    let snd = ids.iter().copied().find(|c| *c != pic).unwrap();
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [pic]})).contains("linked"));
    assert!(refused(&mut s, "comp.convertToCompClip", json!({"clips": [snd]})).contains("sound clip"));
}

/// `tools/comp-accept.json` (the `command_batch` that builds acceptance comps A and B over the
/// footage on V1, DESIGN-C1 §5) runs step by step on the demo, as the MCP server runs a batch.
#[test]
fn the_acceptance_batch_builds_comps_a_and_b() {
    let mut s = demo();
    let batch: serde_json::Value = serde_json::from_str(include_str!("../../../tools/comp-accept.json")).unwrap();
    for step in batch["steps"].as_array().unwrap() {
        let id = step["id"].as_str().unwrap();
        s.execute(id, step.get("params").cloned().unwrap_or(json!({}))).unwrap_or_else(|e| panic!("{id} {}: {e}", step["params"]));
    }
    let q = s.active_sequence().unwrap();
    let comps: Vec<(usize, ItemId)> = q
        .video_tracks
        .iter()
        .enumerate()
        .flat_map(|(n, t)| t.items.iter().map(move |i| (n, i.item)))
        .filter(|(_, i)| matches!(s.project.item(*i).map(|it| &it.kind), Some(ItemKind::Comp(_))))
        .collect();
    assert_eq!(comps.iter().map(|c| c.0).collect::<Vec<_>>(), vec![1, 2], "comp A on V2, comp B on V3");
    for ((_, item), want) in
        comps.iter().zip([vec!["mediaOut", "mediaIn", "key", "solid", "merge", "color"], vec!["mediaOut", "text", "transform", "mediaIn", "merge"]])
    {
        let g = get(&mut s, item.0);
        let kinds: Vec<&str> = g["nodes"].as_array().unwrap().iter().filter_map(|n| n["kind"]["type"].as_str()).collect();
        assert_eq!(kinds, want);
        let Some(ItemKind::Comp(c)) = s.project.item(*item).map(|i| &i.kind) else { panic!("comp") };
        assert!(filmcraft_project::dag::validate(&c.nodes, &c.edges, 1).is_ok(), "{:?}", c.edges);
        assert_eq!(c.edges.len(), want.len() - 1, "every node feeds Media Out (a tree)");
    }
    // both render: the comps' pictures are in the frame
    let img = frame(&s, Tick(filmcraft_time::TICKS_PER_SECOND));
    assert!(img.get(img.w / 2, img.h / 2)[3] > 0.5);
}

/// C1 item 7: `comp.explain` describes comps A and B node by node in flow order, names only nodes
/// that exist, lists changed parameters by their real labels, and reports unconnected nodes and
/// broken graphs instead of failing.
#[test]
fn comp_explain_describes_the_acceptance_comps() {
    let mut s = demo();
    let batch: serde_json::Value = serde_json::from_str(include_str!("../../../tools/comp-accept.json")).unwrap();
    for step in batch["steps"].as_array().unwrap() {
        let id = step["id"].as_str().unwrap();
        s.execute(id, step.get("params").cloned().unwrap_or(json!({}))).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let q = s.active_sequence().unwrap();
    let comps: Vec<ItemId> = q
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter().map(|i| i.item))
        .filter(|i| matches!(s.project.item(*i).map(|it| &it.kind), Some(ItemKind::Comp(_))))
        .collect();
    assert_eq!(comps.len(), 2);
    for item in &comps {
        let e = s.execute("comp.explain", json!({"comp": item.0})).unwrap();
        let lines = e["lines"].as_array().unwrap();
        let Some(ItemKind::Comp(c)) = s.project.item(*item).map(|i| &i.kind) else { panic!("comp") };
        assert_eq!(lines.len(), c.nodes.len(), "{e}");
        // flow order: sources first, Media Out last
        assert!(lines.last().unwrap()["text"].as_str().unwrap().starts_with("is the comp's result"), "{e}");
        let names: Vec<String> = c.nodes.iter().map(crate::comp_explain::node_name).collect();
        for l in lines {
            assert!(names.contains(&l["name"].as_str().unwrap().to_string()), "{l}");
        }
        assert!(e["unused"].as_array().unwrap().is_empty());
        assert!(e["summary"].as_str().unwrap().contains("Media Out"), "{e}");
    }
    // comp A: the Merge names its two inputs as foreground and background
    let a = s.execute("comp.explain", json!({"comp": comps[0].0})).unwrap();
    assert!(a["lines"].as_array().unwrap().iter().any(|l| l["text"].as_str().unwrap().contains("(foreground) over")), "{a}");
    // a node that doesn't reach Media Out is listed as unused; a changed Transform names its labels
    let id = comps[1].0;
    let n = s.execute("comp.addNode", json!({"comp": id, "kind": "blur"})).unwrap();
    let blur = n["node"].as_u64().unwrap_or_else(|| n.as_u64().unwrap());
    let e = s.execute("comp.explain", json!({"comp": id})).unwrap();
    assert_eq!(e["unused"][0]["node"], json!(blur), "{e}");
    assert!(e["unused"][0]["text"].as_str().unwrap().contains("isn't connected"));
    // a cycle: the summary says why, no lines
    let Some(ItemKind::Comp(c)) = s.project.item(comps[1]).map(|i| &i.kind) else { panic!("comp") };
    let mut broken = (**c).clone();
    let first = broken.nodes.iter().find(|n| n.kind.inputs() > 0).map(|n| n.id).unwrap();
    broken.edges.push(filmcraft_project::dag::Edge::new(filmcraft_project::dag::Port::new(first, 0), filmcraft_project::dag::Port::new(first, 0)));
    let e = crate::comp_explain::explain(&broken, &s.project, Tick::ZERO);
    assert!(e.lines.is_empty() && e.summary.contains("can't be shown"), "{e:?}");
    // hostile: an unknown comp id is an error, not a panic
    assert!(s.execute("comp.explain", json!({"comp": 999_999})).is_err());
}

/// A new comp's Media Out sits on the right, and nodes added without a position don't land on
/// top of each other (seen on screen 2026-10-14: Media Out hidden under the first node).
#[test]
fn new_nodes_do_not_stack_on_each_other() {
    let mut s = demo();
    let id = s.execute("comp.new", json!({})).unwrap()["comp"].as_u64().unwrap();
    for kind in ["solid", "blur", "merge"] {
        s.execute("comp.addNode", json!({"comp": id, "kind": kind})).unwrap();
    }
    let Some(ItemKind::Comp(c)) = s.project.item(ItemId(id)).map(|i| &i.kind) else { panic!("comp") };
    let pos: Vec<[f32; 2]> = c.nodes.iter().map(|n| n.pos).collect();
    assert_eq!(pos[0], [filmcraft_project::comp::MEDIA_OUT_X, 0.0]);
    for (i, a) in pos.iter().enumerate() {
        assert!(pos.iter().skip(i + 1).all(|b| a != b), "{pos:?}");
    }
}

/// `comp.setMasks` with the same `gesture` merges into one undo step (a drag on the monitor).
#[test]
fn set_masks_merges_one_gesture_into_one_undo_step() {
    let mut s = demo();
    let id = s.execute("comp.new", json!({})).unwrap()["comp"].as_u64().unwrap();
    let node = s.execute("comp.addNode", json!({"comp": id, "kind": "mask"})).unwrap()["node"].as_u64().unwrap();
    let mask = |x: f64| json!([filmcraft_project::Mask::new("m".to_string(), filmcraft_project::MaskPath::rect(x, 0.0, x + 10.0, 10.0))]);
    let depth = s.history.undo.len();
    for x in [1.0, 2.0, 3.0] {
        s.execute("comp.setMasks", json!({"comp": id, "node": node, "masks": mask(x), "gesture": 7})).unwrap();
    }
    assert_eq!(s.history.undo.len(), depth + 1);
    s.execute("comp.setMasks", json!({"comp": id, "node": node, "masks": mask(4.0), "gesture": 8})).unwrap();
    s.execute("comp.setMasks", json!({"comp": id, "node": node, "masks": mask(5.0)})).unwrap();
    assert_eq!(s.history.undo.len(), depth + 3);
}
