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

/// Effects folder edge cases (cloud USERFX-FOLDER). Scans and parses only: nothing is registered,
/// so this can't race the registry test above.
#[test]
fn the_effects_folder_survives_odd_entries() {
    use crate::user_effects::{MAX_FILES, scan, statuses};
    let dir = tmp_dir("user-effects-folder");
    let fx = dir.join("Effects");
    std::fs::create_dir_all(fx.join("a directory.wgsl")).unwrap();
    std::fs::write(fx.join("a directory.wgsl").join("inside.wgsl"), GOOD).unwrap();
    std::fs::write(fx.join("empty.wgsl"), "").unwrap();
    std::fs::write(fx.join("latin1.wgsl"), b"// @effect \"Caf\xe9\"\nfn effect(px: vec2<f32>) -> vec4<f32> { return sample(px); }\n").unwrap();
    std::fs::write(fx.join("My Glow.wgsl"), GOOD).unwrap();
    std::fs::write(fx.join("my-glow.wgsl"), GOOD).unwrap();
    std::fs::write(fx.join("SHOUT.WGSL"), GOOD.replace("Soft Glow", "Shout")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fx.join("loop.wgsl"), fx.join("loop.wgsl")).unwrap();
        std::os::unix::fs::symlink(fx.join("nowhere"), fx.join("dangling.wgsl")).unwrap();
    }
    // a FIFO would block a plain read forever (skipped where mkfifo is missing)
    #[cfg(unix)]
    let fifo = std::process::Command::new("mkfifo").arg(fx.join("pipe.wgsl")).status().is_ok_and(|s| s.success());
    let out = statuses(scan(&fx));
    let by = |id: &str| out.iter().find(|o| o.id == id).unwrap_or_else(|| panic!("{id}: {out:?}"));
    assert!(!out.iter().any(|o| o.path.as_deref().is_some_and(|p| p.contains("a directory"))), "a directory is not an effect file: {out:?}");
    assert!(by("user.empty").error.as_deref().unwrap().contains("fn effect"), "{out:?}");
    assert_eq!(by("user.latin1").error.as_deref(), Some("the file is not UTF-8 text"));
    assert!(by("user.shout").error.is_none(), "upper-case .WGSL loads: {out:?}");
    // `My Glow` and `my-glow` give the same id: the first (sorted) loads, the second says why not
    let glows: Vec<_> = out.iter().filter(|o| o.id == "user.my-glow").collect();
    assert_eq!(glows.len(), 2, "{out:?}");
    assert!(glows[0].error.is_none() && glows[0].path.as_deref().is_some_and(|p| p.ends_with("My Glow.wgsl")), "{glows:?}");
    assert!(glows[1].error.as_deref().unwrap().contains("already uses the id `user.my-glow`"), "{glows:?}");
    #[cfg(unix)]
    {
        assert!(by("user.loop").error.is_some(), "a symlink loop is reported, not followed: {out:?}");
        assert!(by("user.dangling").error.is_some(), "{out:?}");
        if fifo {
            assert_eq!(by("user.pipe").error.as_deref(), Some("not a regular file"));
        }
    }

    // 300 files: the first MAX_FILES (sorted) are read, the rest left out (and logged)
    let many = dir.join("Many");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..300 {
        std::fs::write(many.join(format!("fx{i:03}.wgsl")), "").unwrap();
    }
    let found = scan(&many);
    assert_eq!(found.len(), MAX_FILES);
    assert_eq!(found.first().map(|f| f.0.as_str()), Some("user.fx000"));
    assert_eq!(found.last().map(|f| f.0.clone()), Some(format!("user.fx{:03}", MAX_FILES - 1)));
    // a missing folder is no effects, not an error
    assert!(scan(&dir.join("missing")).is_empty());
}

/// Every WGSL example in the guide's shader-effects page (the Warm Spot sample and the Recipes,
/// cloud GUIDE-SHADER-COOKBOOK) is a file FilmCraft loads: it parses and validates.
#[test]
fn the_guide_examples_parse_and_validate() {
    let page = include_str!("../../../docs/guide/shader-effects.md");
    let blocks: Vec<&str> = page.split("```wgsl\n").skip(1).filter_map(|b| b.split("```").next()).collect();
    assert!(blocks.len() >= 4, "the sample and the three recipes: {}", blocks.len());
    for b in &blocks {
        let fx = filmcraft_usershader::parse(b, "Guide example").unwrap_or_else(|e| panic!("{e:?}\n{b}"));
        assert_eq!(filmcraft_usershader::validate(&fx), Ok(()), "{b}");
        assert!(!fx.params.is_empty(), "every example has controls: {b}");
    }
    let names: Vec<String> = blocks.iter().filter_map(|b| filmcraft_usershader::parse(b, "x").ok()).map(|fx| fx.name).collect();
    for n in ["Soft Vignette", "Duotone", "Pulse"] {
        assert!(names.iter().any(|x| x == n), "{n}: {names:?}");
    }
}
