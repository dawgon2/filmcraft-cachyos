//! User shader effects loading (fork Y3.4 step 3): files in `<data dir>/Effects`, errors with their
//! line, the registry, a project using an effect whose file is gone.

use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

const GOOD: &str = "// @effect \"Soft Glow\"\n// @param amount: float = 0.5 [0.0, 1.0] \"Amount\"\nfn effect(px: vec2<f32>) -> vec4<f32> {\n    let c = sample(px);\n    return vec4(mix(c.rgb, vec3(1.0), P.amount * 0.2), c.a);\n}\n";

/// One test: the registry is process-wide.
#[test]
fn user_effects_load_register_and_survive_a_missing_file() {
    let dir = tmp_dir("user-effects");
    let fx = dir.join("Effects");
    std::fs::create_dir_all(&fx).unwrap();
    std::fs::write(fx.join("Soft Glow.wgsl"), GOOD).unwrap();
    std::fs::write(fx.join("broken.wgsl"), "fn effect(px: vec2<f32>) -> vec4<f32> {\n    return sample(px);\n    let = 3;\n}\n").unwrap();
    std::fs::write(fx.join("huge.wgsl"), "a".repeat(filmcraft_usershader::MAX_SOURCE + 1)).unwrap();
    std::fs::write(fx.join("notes.txt"), "not an effect").unwrap();
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.prefs_path = Some(dir.join("preferences.json"));
    let r = s.execute("effects.reloadUser", json!({})).unwrap();
    let all = r.as_array().unwrap();
    assert_eq!(all.len(), 3 + crate::user_effects::EXAMPLES.len(), "{r}");
    for id in ["example.chromatic-aberration", "example.glitch", "example.vhs", "example.light-leak"] {
        assert!(all.iter().any(|e| e["id"] == id && e["ok"] == true && e["path"].is_null()), "{id}: {r}");
        let def = filmcraft_project::find_effect(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(def.category, ["Video Effects", "User", "Stylize"], "{id}");
    }
    let by = |id: &str| all.iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("{id}: {r}")).clone();
    assert_eq!(by("user.soft-glow")["ok"], true, "{r}");
    assert_eq!(by("user.soft-glow")["name"], "Soft Glow");
    let broken = by("user.broken");
    assert_eq!(broken["ok"], false);
    assert!(broken["error"].as_str().unwrap().starts_with("line 3:"), "{broken}");
    assert!(by("user.huge")["error"].as_str().unwrap().contains("larger"));
    let def = filmcraft_project::find_effect("user.soft-glow").expect("registered");
    assert_eq!(def.params.len(), 1);
    assert!(filmcraft_project::find_effect("user.broken").is_none());
    let list = s.execute("effects.listUser", json!({})).unwrap();
    assert!(list["folder"].as_str().unwrap().ends_with("Effects"));

    // quarantine (the GPU stopped while it ran): turned off until the file changes or unquarantine
    use crate::user_effects::{QUARANTINE_FILE, quarantine, read_quarantine};
    let hash = filmcraft_usershader::parse(GOOD, "Soft Glow").unwrap().hash;
    quarantine(&fx, &[("user.soft-glow".into(), hash), ("blur".into(), 1), ("user.soft-glow".into(), hash)]).unwrap();
    assert_eq!(read_quarantine(&fx), [("user.soft-glow".to_string(), hash)], "built-in ids and duplicates are left out");
    let r = s.execute("effects.reloadUser", json!({})).unwrap();
    let q = r.as_array().unwrap().iter().find(|e| e["id"] == "user.soft-glow").unwrap().clone();
    assert_eq!(q["ok"], false, "{q}");
    assert!(q["error"].as_str().unwrap().contains("GPU stopped"), "{q}");
    assert!(filmcraft_project::find_effect("user.soft-glow").is_none());
    let r = s.execute("effects.unquarantine", json!({"id": "user.soft-glow"})).unwrap();
    assert!(r.as_array().unwrap().iter().any(|e| e["id"] == "user.soft-glow" && e["ok"] == true), "{r}");
    assert!(read_quarantine(&fx).is_empty());
    // hostile quarantine files are ignored
    for bad in ["not json", "{\"id\": 1}", "[{\"id\": \"user.soft-glow\", \"hash\": 12}]", "[{\"id\": \"user.soft-glow\", \"hash\": \"-1\"}]"] {
        std::fs::write(fx.join(QUARANTINE_FILE), bad).unwrap();
        assert!(read_quarantine(&fx).is_empty(), "{bad}");
    }
    std::fs::write(fx.join(QUARANTINE_FILE), "[".repeat(300 * 1024)).unwrap();
    assert!(read_quarantine(&fx).is_empty(), "too large");
    s.execute("effects.unquarantine", json!({})).unwrap();
    assert!(filmcraft_project::find_effect("user.soft-glow").is_some());

    // apply it to a clip, save, then open where the file is gone
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
    s.execute("effects.apply", json!({"clips": [clip.0], "effect": "user.soft-glow"})).unwrap();
    s.execute("effects.setParam", json!({"clip": clip.0, "effect": "user.soft-glow", "param": "amount", "value": 0.8})).unwrap();
    let path = dir.join("uses-user-effect.fcproj");
    s.execute("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
    std::fs::remove_file(fx.join("Soft Glow.wgsl")).unwrap();
    s.execute("effects.reloadUser", json!({})).unwrap();
    assert!(filmcraft_project::find_effect("user.soft-glow").is_none());
    let mut t = Session { prefs_path: Some(dir.join("preferences.json")), ..Default::default() };
    t.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    let it = t.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
    let e = it.effects.iter().find(|e| e.effect == "user.soft-glow").expect("the instance is kept");
    assert_eq!(e.param("amount").map(|p| p.value.clone()), Some(filmcraft_project::ParamValue::Float(0.8)), "with its values");
    assert!(e.def().is_none(), "shown as a missing effect");
    // it renders (as pass-through), no panic
    t.set_playhead(it.start);
    assert!(t.render_program(0.25).is_some());
    filmcraft_project::effect::register_user_effects(&[]);
}
