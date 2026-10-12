//! Headless UI tests of the Color Grading workspace (fork GRADE-PLAN G1): the Nodes strip and the
//! Primaries panel, driven by automation id through the control channel.
//!
//! Set `FILMCRAFT_UI_SNAPSHOT_DIR=<dir>` to also render offscreen with wgpu and write `grade-*.png`.

#![allow(dead_code)]

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    snapshots: Option<std::path::PathBuf>,
}

impl Driver {
    fn demo() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        Self::from_session(session)
    }

    fn from_session(session: Session) -> Self {
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let snapshots = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").map(std::path::PathBuf::from);
        let mut b = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000);
        if snapshots.is_some() {
            b = b.wgpu();
        }
        let harness = b.build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx, snapshots };
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

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(2);
    }

    fn element(&mut self, id: &str) -> Option<[f32; 4]> {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array()?.iter().find(|e| e["id"] == id).map(|e| {
            let r = &e["rect"];
            [r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32, r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32]
        })
    }

    fn ids(&mut self, prefix: &str) -> Vec<String> {
        let v = self.ok("ui.elements", json!({"prefix": prefix}));
        v.as_array().unwrap().iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect()
    }

    /// Offscreen render of the window (or the bounding box of the elements under `prefix`).
    fn snapshot(&mut self, name: &str, prefix: Option<&str>) {
        let Some(dir) = self.snapshots.clone() else { return };
        let crop = prefix.map(|p| {
            let v = self.ok("ui.elements", json!({"prefix": p}));
            let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for e in v.as_array().unwrap() {
                let r = &e["rect"];
                let (x, y, w, h) = (r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32, r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32);
                bb = [bb[0].min(x), bb[1].min(y), bb[2].max(x + w), bb[3].max(y + h)];
            }
            bb
        });
        self.frames(2);
        let img = match self.harness.render() {
            Ok(i) => i,
            Err(e) => {
                eprintln!("snapshot {name} skipped: {e}");
                return;
            }
        };
        let ppp = img.width() as f32 / 1600.0;
        let img = match crop {
            Some(bb) => {
                let x0 = ((bb[0] - 24.0) * ppp).max(0.0) as u32;
                let y0 = ((bb[1] - 30.0) * ppp).max(0.0) as u32;
                let x1 = (((bb[2] + 16.0) * ppp) as u32).min(img.width());
                let y1 = (((bb[3] + 46.0) * ppp) as u32).min(img.height());
                image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image()
            }
            None => img,
        };
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        img.save(&path).unwrap();
        eprintln!("snapshot: {}", path.display());
    }
}

/// Select the first clip on V1 and park the playhead on it.
fn select_first_clip(d: &mut Driver) -> u64 {
    let seq = d.exec("sequence.inspect", json!({}));
    let first = seq["video"][0]["items"][0].clone();
    let clip = first["clip"].as_u64().unwrap_or_else(|| panic!("no clip: {seq}"));
    d.exec("timeline.select", json!({"clips": [clip]}));
    clip
}

fn grade(d: &mut Driver, clip: u64) -> Value {
    d.exec("grade.get", json!({"clip": clip}))["grade"].clone()
}

#[test]
fn node_strip_and_primaries_edit_the_grade() {
    let mut d = Driver::demo();
    d.ok("ui.set", json!({"workspace": "Color Grading"}));
    d.frames(3);
    let clip = select_first_clip(&mut d);
    d.frames(3);
    // no grade yet: the Primaries panel offers Add Node
    d.click("grade.addFirst");
    let g = grade(&mut d, clip);
    assert_eq!(g["nodes"].as_array().map(Vec::len), Some(1), "{g}");
    // two more serial nodes from the strip
    d.click("grade.addNode");
    d.click("grade.addNode");
    d.frames(2);
    let g = grade(&mut d, clip);
    let ids: Vec<u64> = g["nodes"].as_array().unwrap().iter().map(|n| n["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 3, "{g}");
    let strip = d.ids("grade.node.");
    for id in &ids {
        assert!(strip.iter().any(|s| s == &format!("grade.node.{id}")), "{strip:?}");
    }
    // select the first node and check the wheels and sliders are there
    d.click(&format!("grade.node.{}", ids[0]));
    d.frames(2);
    let ids_now = d.ids("grade.");
    for id in [
        "grade.wheel.lift",
        "grade.wheel.gamma",
        "grade.wheel.gain",
        "grade.wheel.offset",
        "grade.param.contrast",
        "grade.param.saturation",
        "grade.param.grain.strength",
    ] {
        assert!(ids_now.iter().any(|i| i == id), "{id} missing from {ids_now:?}");
    }
    // clicking the Offset wheel's rim moves its puck: a colour offset on the selected node
    assert!(d.element("grade.wheel.offset").is_some());
    d.ok("ui.click", json!({"id": "grade.wheel.offset", "fx": 0.8, "fy": 0.5}));
    d.frames(2);
    let g = grade(&mut d, clip);
    let first = &g["nodes"][0];
    assert!(first["params"]["offset"].is_object(), "offset set on node 1: {g}");
    d.snapshot("grade-primaries", None);
    // curves tab
    d.click("grade.tab.curves");
    d.frames(2);
    assert!(d.ids("grade.curve.").iter().any(|i| i == "grade.curve.r"));
    d.snapshot("grade-curves", None);
    // disable and delete from the strip
    d.click("grade.toggleNode");
    let g = grade(&mut d, clip);
    assert_eq!(g["nodes"][0]["enabled"], json!(false), "{g}");
    d.click("grade.removeNode");
    let g = grade(&mut d, clip);
    assert_eq!(g["nodes"].as_array().map(Vec::len), Some(2), "{g}");
    d.ok("ui.set", json!({"workspace": "Color Grading"}));
    d.frames(2);
    d.snapshot("grade-workspace", None);
}

#[test]
fn clicks_are_separate_undo_steps_and_full_strips_scroll() {
    let mut d = Driver::demo();
    d.ok("ui.set", json!({"workspace": "Color Grading"}));
    d.frames(3);
    let clip = select_first_clip(&mut d);
    d.frames(2);
    d.click("grade.addFirst");
    d.ok("ui.click", json!({"id": "grade.wheel.offset", "fx": 0.8, "fy": 0.5}));
    d.frames(2);
    d.ok("ui.click", json!({"id": "grade.wheel.offset", "fx": 0.5, "fy": 0.2}));
    d.frames(2);
    let g = grade(&mut d, clip);
    let second = g["nodes"][0]["params"]["offset"].clone();
    d.exec("edit.undo", json!({}));
    let g = grade(&mut d, clip);
    assert!(g["nodes"][0]["params"]["offset"].is_object(), "one undo removes only the second click: {g}");
    assert_ne!(g["nodes"][0]["params"]["offset"], second);
    // a full grade (64 nodes, long labels) draws without panicking and keeps ids on screen only
    for _ in 0..63 {
        d.exec("grade.addNode", json!({"clip": clip}));
    }
    let first = grade(&mut d, clip)["nodes"][0]["id"].as_u64().unwrap();
    d.exec("grade.labelNode", json!({"clip": clip, "node": first, "label": "x".repeat(64)}));
    d.frames(3);
    let strip = d.ids("grade.node.");
    assert!(!strip.is_empty() && strip.len() < 64, "only visible nodes register: {}", strip.len());
    d.snapshot("grade-full-strip", None);
}

#[test]
fn rename_names_a_node_in_place_and_escape_cancels() {
    let mut d = Driver::demo();
    d.ok("ui.set", json!({"workspace": "Color Grading"}));
    d.frames(3);
    let clip = select_first_clip(&mut d);
    d.frames(2);
    d.click("grade.addFirst");
    d.click("grade.addNode");
    let first = grade(&mut d, clip)["nodes"][0]["id"].as_u64().unwrap();
    d.click(&format!("grade.node.{first}"));
    // Rename opens a field on the node; Enter keeps the name (one undo step)
    d.click("grade.renameNode");
    assert!(d.element("grade.renameNode.field").is_some(), "rename field shown");
    d.ok("ui.type", json!({"text": "Balance"}));
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(2);
    assert!(d.element("grade.renameNode.field").is_none(), "field closed");
    assert_eq!(grade(&mut d, clip)["nodes"][0]["label"], json!("Balance"));
    d.snapshot("grade-node-renamed", Some("grade.node."));
    // Esc leaves the name as it was
    d.click("grade.renameNode");
    d.ok("ui.type", json!({"text": " more"}));
    d.ok("ui.key", json!({"key": "Escape"}));
    d.frames(2);
    assert!(d.element("grade.renameNode.field").is_none());
    assert_eq!(grade(&mut d, clip)["nodes"][0]["label"], json!("Balance"));
    d.exec("edit.undo", json!({}));
    assert_eq!(grade(&mut d, clip)["nodes"][0]["label"].as_str().unwrap_or(""), "");
}

#[test]
fn color_page_clips_strip_and_split_screen() {
    let mut d = Driver::demo();
    d.ok("ui.set", json!({"workspace": "Color Grading"}));
    d.frames(4);
    // Resolve's Color page: the clips strip spans the width with no meters or tools beside it
    let ids = d.ids("");
    assert!(ids.iter().any(|i| i == "clips.strip" || i.starts_with("clips.card.")), "clips strip shown");
    assert!(!ids.iter().any(|i| i.starts_with("audioMeters.")), "no audio meters on the Color page");
    let seq = d.exec("sequence.inspect", json!({}));
    let items = seq["video"][0]["items"].as_array().cloned().unwrap_or_default();
    assert!(items.len() >= 2, "demo has two clips: {seq}");
    let (a, b) = (items[0]["clip"].as_u64().unwrap(), items[1]["clip"].as_u64().unwrap());
    // a card selects its clip and moves the playhead into it
    d.click(&format!("clips.card.{b}"));
    let st = d.exec("sequence.inspect", json!({}));
    assert!(st["selection"].as_array().is_some_and(|v| v.contains(&json!(b)) && !v.contains(&json!(a))), "clip {b} selected: {}", st["selection"]);
    let (start, end) = (items[1]["start"].as_i64().unwrap(), items[1]["end"].as_i64().unwrap());
    let ph = st["playhead"].as_i64().unwrap();
    assert!(start <= ph && ph < end, "playhead {ph} in clip {b} ({start}..{end})");
    // the mini timeline seeks
    let ph0 = d.exec("sequence.inspect", json!({}))["playhead"].as_i64().unwrap_or(-1);
    d.ok("ui.click", json!({"id": "clips.mini", "fx": 0.02, "fy": 0.5}));
    d.frames(2);
    let ph1 = d.exec("sequence.inspect", json!({}))["playhead"].as_i64().unwrap_or(-1);
    assert_ne!(ph0, ph1, "mini timeline click moved the playhead");
    d.click("grade.addFirst");
    d.frames(3);
    d.snapshot("color-page", None);
    // split screen: adjacent clips tile the Program monitor; a tile click selects that clip
    d.ok("ui.menu.invoke", json!({"id": "view.splitScreen.adjacent"}));
    d.frames(3);
    let tiles = d.ids("program.split.");
    assert!(tiles.iter().any(|t| t == &format!("program.split.{a}")) && tiles.iter().any(|t| t == &format!("program.split.{b}")), "{tiles:?}");
    d.snapshot("color-page-split", None);
    d.click(&format!("program.split.{a}"));
    let st = d.exec("sequence.inspect", json!({}));
    assert!(st["selection"].as_array().is_some_and(|v| v.contains(&json!(a)) && !v.contains(&json!(b))), "tile selects clip {a}: {}", st["selection"]);
    // Ctrl+Alt+W toggles it off
    d.ok("ui.menu.invoke", json!({"id": "view.splitScreen.toggle"}));
    d.frames(2);
    assert!(d.ids("program.split.").is_empty(), "split screen off");
}

#[test]
fn resolve_workflow_shows_pages_and_color_opens_the_node_grader() {
    let mut d = Driver::demo();
    d.frames(2);
    assert!(d.ids("page.").is_empty(), "no page bar in the Premiere workflow");
    d.exec("settings.setWorkflow", json!({"workflow": "resolve"}));
    d.frames(3);
    let pages = d.ids("page.");
    assert_eq!(pages.len(), 5, "{pages:?}");
    d.click("page.color");
    d.frames(2);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Color Grading"), "Color page");
    d.click("page.edit");
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Editing"));
    // Window ▸ Workspaces ▸ Color opens the node grader in the Resolve workflow
    d.ok("ui.set", json!({"workspace": "Color"}));
    d.frames(2);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Color Grading"));
    d.click("page.edit");
    d.snapshot("grade-resolve-pages", None);
    // Resolve's Shift+6 (Color page) and Shift+2 (Media) land on the same pages as the bar
    d.ok("ui.key", json!({"key": "Shift+6"}));
    d.frames(2);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Color Grading"));
    d.ok("ui.key", json!({"key": "Shift+2"}));
    d.frames(2);
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["workspace"], json!("Assembly"));
    assert!(d.call("engine.execute", json!({"command": "page.bogus", "params": {}})).get("error").is_some());
    d.click("page.deliver");
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["mode"], json!("Export"));
    d.click("page.edit");
    d.exec("settings.setWorkflow", json!({"workflow": "premiere"}));
    d.frames(3);
    assert!(d.ids("page.").is_empty(), "back to Premiere: no page bar");
}

/// A local-AI stand-in (G2.11): answers with `f(prompt)`.
struct FakeAi(fn(&str) -> Result<String, String>);

impl filmcraft_engine::ai_explain::ExplainBackend for FakeAi {
    fn ready(&self, _: &filmcraft_engine::ai_explain::ModelInfo) -> Result<(), (filmcraft_engine::ai_explain::Fallback, String)> {
        Ok(())
    }
    fn generate(
        &self,
        _: &filmcraft_engine::ai_explain::ModelInfo,
        _: &str,
        user: &str,
        _: filmcraft_engine::ai_explain::GenOptions,
    ) -> Result<String, String> {
        (self.0)(user)
    }
}

/// The template body from the prompt: a text that passes the fact check.
fn echo_template(user: &str) -> Result<String, String> {
    Ok(user.split("Reference sentence:\n").nth(1).and_then(|r| r.split("\n\n").next()).unwrap_or_default().to_string())
}

fn wait_for(d: &mut Driver, id: &str) -> bool {
    for _ in 0..300 {
        if d.ids(id).iter().any(|i| i == id) {
            return true;
        }
        d.frames(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

#[test]
fn ai_explanations_setting_and_checked_badge() {
    let mut session = Session::default();
    session.execute("file.openDemoProject", json!({})).expect("demo project");
    session.ai.set_backend(std::sync::Arc::new(FakeAi(echo_template)));
    let mut d = Driver::from_session(session);
    // Settings ▸ Color ▸ AI explanations: off by default; the switch applies on OK
    d.ok("ui.menu.invoke", json!({"id": "app.settings.color"}));
    d.frames(3);
    for id in ["settings.color.ai.enabled", "settings.color.ai.model", "settings.color.ai.download", "settings.color.ai.remove"] {
        assert!(d.element(id).is_some(), "{id} missing: {:?}", d.ids("settings.color"));
    }
    assert_eq!(d.exec("prefs.get", json!({"key": "color.ai.enabled"})), json!(false));
    d.click("settings.color.ai.enabled");
    d.click("settings.ok");
    assert_eq!(d.exec("prefs.get", json!({"key": "color.ai.enabled"})), json!(true));
    // the node explainer shows the checked AI text with its badge
    d.ok("ui.set", json!({"workspace": "Color Grading"}));
    d.frames(3);
    let clip = select_first_clip(&mut d);
    let node = d.exec("grade.addNode", json!({"clip": clip}))["node"].as_u64().unwrap();
    d.exec("grade.setParam", json!({"clip": clip, "node": node, "param": "gain", "value": [0.0, 0.0, 0.0, 0.3]}));
    d.exec("grade.selectNode", json!({"clip": clip, "node": node}));
    d.frames(2);
    d.click("grade.tab.match");
    d.click("grade.match.explainNode");
    assert!(wait_for(&mut d, "grade.match.aiChecked"), "badge: {:?}", d.ids("grade.match"));
    assert!(!d.ids("grade.match").iter().any(|i| i == "grade.match.aiFallback"));
    d.snapshot("grade-ai-checked", Some("grade.match"));
    // a text that fails the check: the template, with the reason behind a hint
    d.harness.state_mut().session.ai.set_backend(std::sync::Arc::new(FakeAi(|_| Ok("It makes the picture much darker.".into()))));
    d.click("grade.match.explainNode");
    assert!(wait_for(&mut d, "grade.match.aiFallback"), "fallback hint: {:?}", d.ids("grade.match"));
    assert!(!d.ids("grade.match").iter().any(|i| i == "grade.match.aiChecked"));
}

/// A fine-tuned explainer built on this computer: selectable like any model, its Download stays
/// off and the note says where it comes from instead of a download source.
#[test]
fn ai_settings_show_local_tuned_models_without_download() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut d = Driver::demo();
    let models = d.exec("ai.models", json!({}));
    let local = models["models"].as_array().unwrap().iter().find(|m| m["local"] == true).expect("a local entry")["id"].as_str().unwrap().to_string();
    d.ok("ui.menu.invoke", json!({"id": "app.settings.color"}));
    d.frames(3);
    d.exec("ai.select", json!({"model": local}));
    d.frames(3);
    assert_eq!(d.exec("ai.models", json!({}))["selected"], json!(local));
    assert!(d.harness.get_by_label("Download").accesskit_node().is_disabled(), "no download for a local model");
    assert!(d.harness.query_by_label_contains("Built on this computer (FilmCraft fine-tune of").is_some());
    assert!(d.harness.query_by_label_contains("downloaded from https://").is_none());
}
