//! LINUX-PLAN L2.6: Settings ▸ Playback ▸ Graphics device lists the host's GPUs (Auto first),
//! stores the chosen one by its PCI ids, keeps a choice whose GPU is gone visible, and
//! `graphics.listDevices` answers agents. Both workflows.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::graphics_device::GpuChoice;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new(workflow: Workflow) -> Self {
        let mut session = Session::default();
        session.prefs.workflow = workflow;
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(session).with_control(rx);
        app.hooks.graphics_devices = Some(Box::new(|| {
            vec![
                GpuChoice { id: "8086:4680".into(), name: "Intel(R) UHD Graphics 770".into(), kind: "integrated".into() },
                GpuChoice { id: "1002:73df".into(), name: "AMD Radeon RX 6700 XT".into(), kind: "discrete".into() },
            ]
        }));
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
            if let Ok(r) = reply.try_recv() {
                assert_eq!(r["ok"], true, "{method} {params}: {r}");
                return r["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }
    fn menu(&mut self, id: &str) -> Value {
        let r = self.ok("ui.menu.invoke", json!({"id": id}));
        self.frames(3);
        r
    }
    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }
    fn has(&mut self, id: &str) -> bool {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        v.as_array().unwrap().iter().any(|e| e["id"] == id)
    }
    fn pref(&mut self) -> Value {
        self.ok("engine.execute", json!({"command": "prefs.get", "params": {"key": "playback.graphicsDevice"}}))
    }
}

#[test]
fn graphics_device_is_chosen_from_the_hosts_gpus_in_both_workflows() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let mut d = Driver::new(workflow);
        assert_eq!(d.pref(), "", "Auto by default ({workflow:?})");

        d.menu("app.settings.playback");
        d.click("settings.playback.graphicsDevice");
        for id in ["", "8086:4680", "1002:73df"] {
            assert!(d.has(&format!("settings.playback.graphicsDevice.{id}")), "option {id:?} ({workflow:?})");
        }
        d.click("settings.playback.graphicsDevice.1002:73df");
        d.click("settings.ok");
        assert_eq!(d.pref(), "1002:73df", "{workflow:?}");

        // agents: the list, the stored choice, the GPU drawing now (none headless)
        let r = d.menu("graphics.listDevices");
        assert_eq!(r["current"], "1002:73df");
        assert_eq!(r["devices"].as_array().map(Vec::len), Some(2));
        assert_eq!(r["devices"][1]["kind"], "discrete");
        assert!(r["inUse"].is_null());

        // another computer's choice (a GPU that's gone) stays visible and can be set back to Auto
        d.ok("engine.execute", json!({"command": "prefs.set", "params": {"key": "playback.graphicsDevice", "value": "10de:2684"}}));
        d.menu("app.settings.playback");
        d.click("settings.playback.graphicsDevice");
        assert!(d.has("settings.playback.graphicsDevice.10de:2684"), "{workflow:?}");
        d.click("settings.playback.graphicsDevice.");
        d.click("settings.ok");
        assert_eq!(d.pref(), "", "{workflow:?}");
    }
}
