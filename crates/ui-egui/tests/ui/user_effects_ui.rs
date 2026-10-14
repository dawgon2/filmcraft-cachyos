//! User shader effects in the UI (fork Y3.4, ADR 0006): Video Effects ▸ User lists the bundled
//! examples and the user's files, files that didn't load show with their error, Reload / Open
//! Effects Folder are there, and a clip using an effect whose file is gone shows a "Missing effect"
//! row in Effect Controls.
use egui::vec2;
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::json;

fn step(h: &mut Harness<'static, FilmcraftApp>) {
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    eframe::App::raw_input_hook(h.state_mut(), &ctx, &mut raw);
    *h.input_mut() = raw;
    ctx.request_repaint();
    h.step();
}

fn has(h: &Harness<'static, FilmcraftApp>, id: &str) -> bool {
    h.state().auto.elements.iter().any(|e| e.id == id)
}

const SOFT: &str = "// @effect \"Soft Test\"\n// @param amount: float = 0.5 [0.0, 1.0] \"Amount\"\nfn effect(px: vec2<f32>) -> vec4<f32> {\n    return sample(px) * P.amount;\n}\n";

#[test]
fn user_effects_folder_lists_examples_broken_files_and_missing_effects() {
    let dir = std::env::temp_dir().join(format!("fc-user-effects-ui-{}", std::process::id()));
    let fx = dir.join("Effects");
    std::fs::create_dir_all(&fx).unwrap();
    std::fs::write(fx.join("soft-test.wgsl"), SOFT).unwrap();
    std::fs::write(fx.join("broken-test.wgsl"), "fn effect(px: vec2<f32>) -> vec4<f32> {\n    let = 1;\n}\n").unwrap();
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.prefs_path = Some(dir.join("preferences.json"));
    s.execute("effects.reloadUser", json!({})).unwrap();
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
    s.execute("playhead.set", json!({"time": clip.start.0})).unwrap();
    s.execute("timeline.select", json!({"clips": [clip.id.0]})).unwrap();
    s.execute("effects.apply", json!({"clips": [clip.id.0], "effect": "user.soft-test"})).unwrap();
    let mut app = FilmcraftApp::new(s);
    app.set_workspace("Effects");
    // a search opens the folders it matches
    app.ui.effects_search = "test".into();
    let mut h = Harness::builder().with_size(vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    for _ in 0..4 {
        step(&mut h);
    }
    assert!(has(&h, "effects.item.user.soft-test"), "the user's effect is listed");
    assert!(has(&h, "effects.user.broken.user.broken-test"), "a file that didn't load is listed");
    assert!(has(&h, "effects.user.reload") && has(&h, "effects.user.openFolder"));
    assert!(has(&h, "effectControls.effect.user.soft-test"), "Effect Controls shows the applied user effect");
    h.state_mut().ui.effects_search = "glitch".into();
    step(&mut h);
    assert!(has(&h, "effects.item.example.glitch"), "bundled examples are listed");
    // the file goes away: the clip keeps the effect, shown as missing
    std::fs::remove_file(fx.join("soft-test.wgsl")).unwrap();
    h.state_mut().session.execute("effects.reloadUser", json!({})).unwrap();
    step(&mut h);
    step(&mut h);
    assert!(has(&h, "effectControls.missing.user.soft-test"), "a missing effect row");
    assert!(!has(&h, "effectControls.effect.user.soft-test"));
    filmcraft_project::effect::register_user_effects(&[]);
    let _ = std::fs::remove_dir_all(&dir);
}
