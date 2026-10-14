//! Fork C1: the Compositor panel, opened from Window ▸ in both workflows, shows a comp's graph on
//! the node canvas and edits it through `comp.*` commands.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn demo(workflow: Workflow) -> Self {
        let mut session = Session::default();
        session.prefs.workflow = workflow;
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn has(&mut self, id: &str) -> bool {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array().unwrap().iter().any(|e| e["id"] == json!(id))
    }
}

fn compositor_round_trip(workflow: Workflow) {
    let mut d = Driver::demo(workflow);
    d.ok("ui.menu.invoke", json!({"id": "window.panel.Compositor"}));
    d.frames(3);
    assert!(d.has("panel.tab.Compositor"), "{workflow:?}: no Compositor tab");
    // empty state: New Comp Clip makes a comp, places it and shows it
    assert!(d.has("comp.newClip"));
    d.ok("ui.click", json!({"id": "comp.newClip"}));
    d.frames(3);
    assert!(d.has("comp.canvas") && d.has("comp.node.1"), "{workflow:?}: the new comp's Media Out is on the canvas");
    let comp = d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["comp"].as_u64().expect("the panel remembers the comp");
    // nodes and connections made by commands show up with their ids
    let solid = d.exec("comp.addNode", json!({"comp": comp, "kind": "solid", "pos": [-200, 0]}))["node"].as_u64().unwrap();
    d.exec("comp.connect", json!({"comp": comp, "from": solid, "to": 1}));
    d.frames(3);
    assert!(d.has(&format!("comp.node.{solid}")) && d.has(&format!("comp.port.{solid}.out.0")) && d.has("comp.port.1.in.0"));
    assert!(d.has(&format!("comp.edge.{solid}-1.0")));
    // the inspector: select a node and its parameters show as rows under the canvas
    assert!(!d.has("comp.param.color"));
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"selection": [solid]}}}}));
    d.frames(3);
    assert!(d.has("comp.param.color"), "{workflow:?}: the Solid's colour row");
    // the node viewer: `comp.view` picks the node the Program monitor shows; unknown nodes are refused
    assert_eq!(d.ok("ui.menu.invoke", json!({"id": "comp.view", "params": {"node": solid}}))["node"], json!(solid));
    d.frames(3);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["canvas"]["viewed"], json!(solid));
    assert!(d.has("comp.viewer"), "{workflow:?}: the Show picker");
    let (req, reply) = ControlRequest::new("ui.menu.invoke", json!({"id": "comp.view", "params": {"node": 987_654u64}}));
    d.tx.send(req).unwrap();
    let v = (0..600).find_map(|_| {
        d.frames(1);
        reply.try_recv().ok()
    });
    assert_eq!(v.expect("a reply")["ok"], json!(false));
    d.ok("ui.menu.invoke", json!({"id": "comp.view", "params": {}}));
    d.frames(2);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["canvas"]["viewed"], json!(null));
    let merge = d.exec("comp.addNode", json!({"comp": comp, "kind": "merge", "pos": [-200, 160]}))["node"].as_u64().unwrap();
    d.frames(2);
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"selection": [merge]}}}}));
    d.frames(3);
    assert!(d.has("comp.param.opacity") && d.has("comp.param.blend") && !d.has("comp.param.color"), "{workflow:?}: the Merge's rows");
    // a value set by command shows in its row
    d.exec("comp.setParam", json!({"comp": comp, "node": merge, "param": "opacity", "value": 40.0}));
    d.frames(2);
    let row = d.ok("ui.elements", json!({"prefix": "comp.param.opacity"}));
    assert!(row.as_array().unwrap().iter().any(|e| e["label"].as_str().is_some_and(|l| l.starts_with("40"))), "{row}");
    // a Color node shows its numeric grade settings; a value set by command shows in its row
    let color = d.exec("comp.addNode", json!({"comp": comp, "kind": "color", "pos": [-200, 240]}))["node"].as_u64().unwrap();
    d.exec("comp.setParam", json!({"comp": comp, "node": color, "param": "saturation", "value": 1.5}));
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"selection": [color]}}}}));
    d.frames(3);
    assert!(d.has("comp.param.saturation") && d.has("comp.param.temperature") && d.has("comp.param.contrast"), "{workflow:?}: the Color node's rows");
    let row = d.ok("ui.elements", json!({"prefix": "comp.param.saturation"}));
    assert!(row.as_array().unwrap().iter().any(|e| e["label"].as_str().is_some_and(|l| l.starts_with("1.5"))), "{row}");
    // its wheels: a click off-centre on the Gamma wheel moves gamma's colour balance
    for key in ["lift", "gamma", "gain", "offset"] {
        assert!(d.has(&format!("comp.wheel.{key}")), "{workflow:?}: the {key} wheel");
    }
    d.ok("ui.click", json!({"id": "comp.wheel.gamma", "fx": 0.85, "fy": 0.5}));
    d.frames(3);
    let g = d.exec("comp.get", json!({"comp": comp}));
    let node = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(color)).cloned().unwrap();
    let gamma = &node["kind"]["node"]["params"]["gamma"];
    assert!(!gamma.is_null(), "{workflow:?}: gamma set by the wheel: {node}");
    // its curve: dragging on the master curve adds a point and lifts it
    assert!(d.has("comp.curve") && d.has("comp.curve.master") && d.has("comp.curve.r"), "{workflow:?}: the curve editor");
    // (the inspector scrolls: bring the curve into view, as a user would)
    for _ in 0..8 {
        let r = d.ok("ui.elements", json!({"prefix": "comp.curve"}));
        let bottom = r
            .as_array()
            .and_then(|a| a.iter().find(|e| e["id"] == json!("comp.curve")))
            .map_or(0.0, |e| e["rect"][1].as_f64().unwrap_or(0.0) + e["rect"][3].as_f64().unwrap_or(0.0));
        let panel = d.ok("ui.elements", json!({"prefix": "panel.Compositor"}));
        let limit = panel[0]["rect"][1].as_f64().unwrap_or(0.0) + panel[0]["rect"][3].as_f64().unwrap_or(0.0);
        if bottom <= limit {
            break;
        }
        let (px, py) = (panel[0]["rect"][0].as_f64().unwrap_or(0.0) + 40.0, limit - 30.0);
        d.ok("ui.scroll", json!({"x": px, "y": py, "dx": 0.0, "dy": -120.0}));
        d.frames(2);
    }
    let cr = d.ok("ui.elements", json!({"prefix": "comp.curve"}));
    let cr = cr.as_array().and_then(|a| a.iter().find(|e| e["id"] == json!("comp.curve")).map(|e| e["rect"].clone())).expect("curve rect");
    let (x, y, w, h) = (cr[0].as_f64().unwrap(), cr[1].as_f64().unwrap(), cr[2].as_f64().unwrap(), cr[3].as_f64().unwrap());
    d.ok("ui.drag", json!({"from": {"x": x + w * 0.5, "y": y + h * 0.5}, "to": {"x": x + w * 0.5, "y": y + h * 0.35}, "steps": 6}));
    d.frames(3);
    let g = d.exec("comp.get", json!({"comp": comp}));
    let node = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(color)).cloned().unwrap();
    assert!(!node["kind"]["node"]["params"]["curve.master"].is_null(), "{workflow:?}: master curve set: {node}");
    // back to the top of the inspector for the next node
    let panel = d.ok("ui.elements", json!({"prefix": "panel.Compositor"}));
    let (px, py) =
        (panel[0]["rect"][0].as_f64().unwrap_or(0.0) + 40.0, panel[0]["rect"][1].as_f64().unwrap_or(0.0) + panel[0]["rect"][3].as_f64().unwrap_or(0.0) - 30.0);
    d.ok("ui.scroll", json!({"x": px, "y": py, "dx": 0.0, "dy": 4000.0}));
    d.frames(2);
    // a Text node shows the graphic text controls; its text set by command shows in its row
    let text = d.exec("comp.addNode", json!({"comp": comp, "kind": "text", "pos": [-200, 320]}))["node"].as_u64().unwrap();
    d.exec("comp.setParam", json!({"comp": comp, "node": text, "param": "text", "value": "Hello comp"}));
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"selection": [text]}}}}));
    d.frames(3);
    assert!(d.has("comp.param.text") && d.has("comp.param.size"), "{workflow:?}: the Text node's rows");
    let row = d.ok("ui.elements", json!({"prefix": "comp.param.text"}));
    assert!(row.as_array().unwrap().iter().any(|e| e["label"] == json!("Hello comp")), "{row}");
    // typing into the field changes the node's text
    d.ok("ui.click", json!({"id": "comp.param.text"}));
    d.ok("ui.type", json!({"text": "!"}));
    d.frames(3);
    let g = d.exec("comp.get", json!({"comp": comp}));
    let tn = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(text)).unwrap().to_string();
    assert!(tn.contains("Hello comp!"), "{tn}");
    // a Mask node shows its mask list: add an ellipse, invert it, delete it
    let mask = d.exec("comp.addNode", json!({"comp": comp, "kind": "mask", "pos": [-200, 480]}))["node"].as_u64().unwrap();
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"selection": [mask]}}}}));
    d.frames(3);
    assert!(d.has("comp.mask.ellipse") && d.has("comp.mask.rect") && !d.has("comp.mask0.inverted"), "{workflow:?}: the Mask node's list");
    let masks_of = |d: &mut Driver| {
        let g = d.exec("comp.get", json!({"comp": comp}));
        g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(mask)).unwrap()["kind"]["masks"].clone()
    };
    d.ok("ui.click", json!({"id": "comp.mask.ellipse"}));
    d.frames(3);
    assert_eq!(masks_of(&mut d).as_array().map(Vec::len), Some(1));
    assert!(d.has("comp.mask0.inverted") && d.has("comp.mask0.feather") && d.has("comp.mask0.opacity") && d.has("comp.mask0.delete"));
    // on the Program monitor: dragging inside the ellipse moves it, one undo step per drag
    assert!(d.has("comp.maskOverlay"), "{workflow:?}: the mask overlay on the Program monitor");
    let before = masks_of(&mut d)[0]["path"].clone();
    let ov = d.ok("ui.elements", json!({"prefix": "comp.maskOverlay"}));
    let r = ov[0]["rect"].clone();
    let (cx, cy) = (r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0, r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0);
    let undo0 = d.exec("history.list", json!({}))["undo"].as_array().map_or(0, Vec::len);
    d.ok("ui.drag", json!({"from": {"x": cx, "y": cy}, "to": {"x": cx + 30.0, "y": cy + 10.0}, "steps": 6}));
    d.frames(3);
    assert_ne!(masks_of(&mut d)[0]["path"], before, "{workflow:?}: the drag moved the mask");
    let undo1 = d.exec("history.list", json!({}))["undo"].as_array().map_or(0, Vec::len);
    assert!(undo1 <= undo0 + 1, "{workflow:?}: one undo step for the drag ({undo0} → {undo1})");
    d.ok("ui.click", json!({"id": "comp.mask0.inverted"}));
    d.frames(3);
    assert_eq!(masks_of(&mut d)[0]["inverted"], json!(true));
    d.ok("ui.click", json!({"id": "comp.mask0.delete"}));
    d.frames(3);
    assert_eq!(masks_of(&mut d).as_array().map(Vec::len), Some(0));
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"selection": [merge]}}}}));
    d.frames(2);
    // the stopwatch turns keyframes on for that parameter
    d.ok("ui.click", json!({"id": "comp.param.opacity.stopwatch"}));
    d.frames(2);
    let g = d.exec("comp.get", json!({"comp": comp}));
    let node = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(merge)).unwrap().clone();
    assert!(node["params"]["opacity"]["keyframes"].as_array().is_some_and(|k| !k.is_empty()), "{node}");
    // the canvas view state is readable and settable through the control channel
    d.ok("ui.set", json!({"panels": {"comp": {"canvas": {"zoom": 2.0}}}}));
    d.frames(2);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["canvas"]["zoom"], json!(2.0));
    // Clip ▸ Compositor ▸ Open in Compositor: another comp, by item or by its clip
    let other = d.exec("comp.new", json!({"name": "Other"}))["comp"].as_u64().unwrap();
    d.exec("timeline.select", json!({"clips": []}));
    d.ok("ui.menu.invoke", json!({"id": "window.closePanel", "params": {"panel": "Compositor"}}));
    d.frames(2);
    assert_eq!(d.ok("ui.menu.invoke", json!({"id": "comp.open", "params": {"comp": other}}))["comp"], json!(other));
    d.frames(3);
    assert!(d.has("panel.tab.Compositor"), "{workflow:?}: comp.open shows the panel");
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["comp"], json!(other));
    let placed = d.exec("comp.newClip", json!({"comp": comp, "track": "V3", "seconds": 300.0}))["clip"].clone();
    let clip = placed.get("clips").and_then(|c| c.get(0)).cloned().unwrap_or(placed);
    assert_eq!(d.ok("ui.menu.invoke", json!({"id": "comp.open", "params": {"clip": clip}}))["comp"], json!(comp));
    // an opened comp stays shown while the selection is unchanged, even with a comp clip selected
    d.exec("timeline.select", json!({"clips": [clip]}));
    d.ok("ui.menu.invoke", json!({"id": "comp.open", "params": {"comp": other}}));
    d.frames(3);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["comp"], json!(other));
    // selecting something else hands the panel back to the selection
    d.exec("timeline.select", json!({"clips": []}));
    d.frames(2);
    d.exec("timeline.select", json!({"clips": [clip]}));
    d.frames(3);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["panels"]["comp"]["comp"], json!(comp));
    // a clip that isn't a comp clip, or no clip at all, is refused
    for params in [json!({"clip": 987_654_321u64}), json!({"comp": 987_654_321u64})] {
        let (req, reply) = ControlRequest::new("ui.menu.invoke", json!({"id": "comp.open", "params": params}));
        d.tx.send(req).unwrap();
        let v = (0..600).find_map(|_| {
            d.frames(1);
            reply.try_recv().ok()
        });
        assert_eq!(v.expect("a reply")["ok"], json!(false));
    }
}

#[test]
fn compositor_panel_premiere() {
    compositor_round_trip(Workflow::Premiere);
}

#[test]
fn compositor_panel_resolve() {
    compositor_round_trip(Workflow::Resolve);
}

/// Premiere workflow: Window ▸ Workspaces ▸ Compositing; Resolve workflow: the Fusion page between
/// Edit and Color. Both show the Compositor beside the Program monitor.
#[test]
fn compositing_workspace_and_fusion_page() {
    let mut d = Driver::demo(Workflow::Premiere);
    d.ok("ui.menu.invoke", json!({"id": "window.workspace.compositing"}));
    d.frames(3);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Compositing"));
    assert!(d.has("panel.tab.Compositor") && d.has("comp.newClip"));
    let mut d = Driver::demo(Workflow::Resolve);
    assert!(d.has("page.fusion"), "the page bar shows Fusion");
    let x = |d: &mut Driver, id: &str| d.ok("ui.elements", json!({"prefix": id}))[0]["rect"][0].as_f64().unwrap();
    let (edit, fusion, color) = (x(&mut d, "page.edit"), x(&mut d, "page.fusion"), x(&mut d, "page.color"));
    assert!(edit < fusion && fusion < color, "Fusion sits between Edit and Color");
    d.ok("ui.click", json!({"id": "page.fusion"}));
    d.frames(3);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Compositing"));
    assert!(d.has("panel.tab.Compositor"));
}
