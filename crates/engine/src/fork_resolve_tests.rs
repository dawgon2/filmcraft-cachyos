//! FORK-ONLY (RESOLVE-UI-PLAN RS1): the default workflow per feature, the one-time preferences
//! migration, and the keyboard + theme round trip between the workflows.

use serde_json::json;

use crate::Session;
use crate::autosave::{AutosaveConfig, Preferences};
use crate::media_test_util::tmp_dir;
use crate::settings::Workflow;

fn start(dir: &std::path::Path) -> Session {
    let mut s = Session::default();
    s.start_autosave(AutosaveConfig::new(dir)).unwrap();
    s
}

#[test]
#[cfg(feature = "fork-resolve-default")]
fn default_workflow_is_resolve_with_the_fork_feature() {
    assert_eq!(Workflow::default(), Workflow::Resolve);
    assert_eq!(Preferences::default().workflow, Workflow::Resolve);
}

#[test]
#[cfg(not(feature = "fork-resolve-default"))]
fn default_workflow_is_premiere_without_the_fork_feature() {
    assert_eq!(Workflow::default(), Workflow::Premiere);
    assert_eq!(Preferences::default().workflow, Workflow::Premiere);
}

#[test]
fn switching_back_and_forth_restores_keymap_and_theme() {
    let dir = tmp_dir("rs1-roundtrip");
    let mut s = start(&dir);
    s.set_workflow(Workflow::Premiere).unwrap();
    // (one automatic round trip first: a switch must have set a theme before a different one counts as hand-picked)
    s.set_workflow(Workflow::Resolve).unwrap();
    s.set_workflow(Workflow::Premiere).unwrap();
    s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": "dark"})).unwrap();
    s.execute("shortcuts.loadPreset", json!({"name": "Avid Media Composer Compatible"})).unwrap();
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(s.shortcuts.preset, "DaVinci Resolve Compatible");
    assert_eq!(s.prefs.appearance.dark_theme, "dark", "a theme picked by hand follows the switch");
    assert_eq!(s.prefs.appearance.color_theme, "dark");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(s.shortcuts.preset, "Avid Media Composer Compatible");
    assert_eq!(s.prefs.appearance.dark_theme, "dark");
    assert_eq!(s.prefs.workflow_return_theme, "");
    // a theme picked by hand while in Resolve follows you back to Premiere and on again
    s.set_workflow(Workflow::Resolve).unwrap();
    s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": "darkest"})).unwrap();
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(s.prefs.appearance.dark_theme, "darkest");
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(s.prefs.appearance.dark_theme, "darkest");
    s.set_workflow(Workflow::Premiere).unwrap();
    // the bookkeeping can't be set around the switch
    assert!(s.execute("prefs.set", json!({"key": "workflowReturnTheme", "value": "dark"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A hand-picked theme follows the user across workflow switches, both directions; only when
/// the theme is the one the last switch set itself does the switch swap it. The Resolve theme
/// picked in Premiere survives a Premiere -> Resolve -> Premiere round trip (the original bug).
#[test]
fn a_hand_picked_theme_follows_the_user_across_workflow_switches() {
    let dir = tmp_dir("rs1-theme-roundtrip");
    let mut s = start(&dir);
    s.set_workflow(Workflow::Premiere).unwrap();
    let theme = |s: &Session| s.prefs.appearance.dark_theme.clone();
    let pick = |s: &mut Session, t: &str| {
        s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": t})).unwrap();
    };
    // the Resolve theme chosen in Premiere stays through the round trip
    pick(&mut s, "resolve");
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(theme(&s), "resolve");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(theme(&s), "resolve");
    assert_eq!(s.prefs.appearance.color_theme, "resolve");
    // a Premiere theme chosen in Premiere follows into Resolve and back
    pick(&mut s, "dark");
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(theme(&s), "dark", "hand-picked: kept in Resolve");
    assert_eq!(s.prefs.appearance.color_theme, "dark");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(theme(&s), "dark");
    // and so does one chosen in Resolve, also across a restart
    s.set_workflow(Workflow::Resolve).unwrap();
    pick(&mut s, "darkest");
    drop(s);
    let mut s = start(&dir);
    assert_eq!((s.prefs.workflow, theme(&s).as_str()), (Workflow::Resolve, "darkest"));
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(theme(&s), "darkest");
    drop(s);
    let mut s = start(&dir);
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(theme(&s), "darkest");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without a hand-picked theme the switch swaps: the Resolve theme in Resolve, the previous one
/// back in Premiere (also across a restart); a remembered theme that no longer exists falls back
/// to the default.
#[test]
fn without_a_hand_picked_theme_the_switch_swaps_and_restores() {
    let dir = tmp_dir("rs1-theme-auto");
    let mut s = start(&dir);
    s.set_workflow(Workflow::Premiere).unwrap();
    let theme = |s: &Session| s.prefs.appearance.dark_theme.clone();
    let before = theme(&s);
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!((theme(&s).as_str(), s.prefs.workflow_return_theme.as_str()), ("resolve", before.as_str()));
    drop(s);
    let mut s = start(&dir);
    assert_eq!(theme(&s), "resolve");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(theme(&s), before, "the theme from before comes back");
    // the swap repeats while nothing was picked by hand
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(theme(&s), "resolve");
    // a remembered theme that no longer exists falls back to the default
    s.prefs.workflow_return_theme = "no-such-theme".into();
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(theme(&s), "darkest");
    // once kept, a hand-picked theme is never swapped away again
    s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": "dark"})).unwrap();
    s.set_workflow(Workflow::Resolve).unwrap();
    s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": "darkest"})).unwrap();
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(theme(&s), "darkest");
    // the bookkeeping can't be set around the switch
    assert!(s.execute("prefs.set", json!({"key": "workflowAutoTheme", "value": "dark"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn garbage_auto_theme_is_dropped_and_switching_still_works() {
    let dir = tmp_dir("rs1-garbage-auto");
    std::fs::write(
        dir.join("preferences.json"),
        serde_json::to_vec(
            &json!({"version": 3, "workflow": "premiere", "workflowAutoTheme": 7, "workflowReturnTheme": ["x"], "appearance": {"darkTheme": "dark"}}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut s = start(&dir);
    assert_eq!((s.prefs.workflow_auto_theme.as_str(), s.prefs.workflow_return_theme.as_str()), ("", ""));
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(s.prefs.appearance.dark_theme, "resolve");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(s.prefs.appearance.dark_theme, "dark");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn garbage_return_theme_is_dropped_and_the_way_back_still_works() {
    let dir = tmp_dir("rs1-garbage-theme");
    std::fs::write(
        dir.join("preferences.json"),
        serde_json::to_vec(&json!({"version": 3, "workflow": "resolve", "workflowReturnTheme": 7, "appearance": {"darkTheme": "resolve"}})).unwrap(),
    )
    .unwrap();
    let mut s = start(&dir);
    assert_eq!(s.prefs.workflow_return_theme, "");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(s.prefs.appearance.dark_theme, "darkest");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(not(feature = "fork-resolve-default"))]
fn upstream_build_never_migrates() {
    let dir = tmp_dir("rs1-upstream");
    std::fs::write(dir.join("preferences.json"), serde_json::to_vec(&json!({"version": 2, "workflow": "premiere"})).unwrap()).unwrap();
    let s = start(&dir);
    assert_eq!(s.prefs.workflow, Workflow::Premiere);
    assert_eq!(s.shortcuts.preset, "FilmCraft Default");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fork-resolve-default")]
mod fork {
    use super::*;
    use serde_json::Value;

    fn file(dir: &std::path::Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join("preferences.json")).unwrap()).unwrap()
    }

    /// A session that never gets a data directory (headless, MCP) used to report the Resolve
    /// workflow with the FilmCraft Default keys and theme.
    #[test]
    fn a_session_without_a_data_dir_is_consistently_resolve() {
        let mut s = Session::default();
        assert_eq!((s.prefs.workflow, s.shortcuts.preset.as_str()), (Workflow::Resolve, "FilmCraft Default"), "the old state");
        crate::fork_resolve::without_data_dir(&mut s);
        assert_eq!(s.shortcuts.preset, "DaVinci Resolve Compatible");
        assert!(!s.shortcuts.modified);
        assert_eq!(s.shortcuts.bindings, s.shortcuts.builtin("DaVinci Resolve Compatible").unwrap());
        assert_eq!((s.prefs.appearance.dark_theme.as_str(), s.prefs.appearance.color_theme.as_str()), ("resolve", "resolve"));
        let w = s.execute("settings.getWorkflow", json!({})).unwrap();
        assert_eq!((w["workflow"].as_str(), w["keyboardPreset"].as_str()), (Some("resolve"), Some("DaVinci Resolve Compatible")), "{w}");
        // twice changes nothing
        let before = (s.shortcuts.bindings.clone(), s.prefs.clone());
        crate::fork_resolve::without_data_dir(&mut s);
        assert_eq!((s.shortcuts.bindings.clone(), s.prefs.clone()), before);
        // the way back works without a data directory, and forth again
        s.set_workflow(Workflow::Premiere).unwrap();
        assert_eq!((s.shortcuts.preset.as_str(), s.prefs.appearance.dark_theme.as_str()), ("FilmCraft Default", "darkest"));
        s.set_workflow(Workflow::Resolve).unwrap();
        assert_eq!((s.shortcuts.preset.as_str(), s.prefs.appearance.dark_theme.as_str()), ("DaVinci Resolve Compatible", "resolve"));
        // Premiere chosen explicitly: left alone
        s.set_workflow(Workflow::Premiere).unwrap();
        crate::fork_resolve::without_data_dir(&mut s);
        assert_eq!((s.prefs.workflow, s.shortcuts.preset.as_str()), (Workflow::Premiere, "FilmCraft Default"));
        // an edited keyboard is never thrown away
        let mut s = Session::default();
        s.execute("shortcuts.set", json!({"command": "edit.undo", "keys": "F12"})).unwrap();
        assert!(s.shortcuts.modified);
        let keys = s.shortcuts.bindings.clone();
        crate::fork_resolve::without_data_dir(&mut s);
        assert_eq!(s.shortcuts.bindings, keys);
    }

    /// With a data directory its preferences and saved keyboard decide, as before.
    #[test]
    fn a_session_with_a_data_dir_is_left_to_its_preferences() {
        let dir = tmp_dir("rs1-nodir-premiere");
        std::fs::write(dir.join("preferences.json"), serde_json::to_vec(&json!({"version": 3, "workflow": "premiere"})).unwrap()).unwrap();
        let mut s = start(&dir);
        crate::fork_resolve::without_data_dir(&mut s);
        assert_eq!((s.prefs.workflow, s.shortcuts.preset.as_str()), (Workflow::Premiere, "FilmCraft Default"));
        let _ = std::fs::remove_dir_all(&dir);
        let dir = tmp_dir("rs1-nodir-resolve");
        let mut s = start(&dir);
        s.execute("shortcuts.loadPreset", json!({"name": "Avid Media Composer Compatible"})).unwrap();
        crate::fork_resolve::without_data_dir(&mut s);
        assert_eq!((s.prefs.workflow, s.shortcuts.preset.as_str()), (Workflow::Resolve, "Avid Media Composer Compatible"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_preferences_switch_to_resolve_once() {
        let dir = tmp_dir("rs1-migrate");
        std::fs::write(
            dir.join("preferences.json"),
            serde_json::to_vec(&json!({"version": 2, "workflow": "premiere", "timeline": {"stillImageDuration": 7}})).unwrap(),
        )
        .unwrap();
        let mut s = start(&dir);
        assert_eq!(s.prefs.workflow, Workflow::Resolve);
        assert_eq!(s.shortcuts.preset, "DaVinci Resolve Compatible");
        assert_eq!(s.prefs.workflow_return_preset, "FilmCraft Default", "the previous keymap is remembered");
        assert_eq!(s.prefs.appearance.dark_theme, "resolve");
        assert_eq!(s.prefs.workflow_return_theme, "darkest");
        assert_eq!(s.prefs.timeline.still_image_duration, 7.0, "other preferences are kept");
        assert!(!s.prefs.pending_resolve_migration);
        let f = file(&dir);
        assert_eq!((f["version"].as_u64(), f["workflow"].as_str()), (Some(3), Some("resolve")));
        assert!(f.get("pendingResolveMigration").is_none());
        // an explicit switch back is respected on every later launch
        s.set_workflow(Workflow::Premiere).unwrap();
        assert_eq!(s.shortcuts.preset, "FilmCraft Default");
        assert_eq!(s.prefs.appearance.dark_theme, "darkest");
        drop(s);
        for _ in 0..2 {
            let s = start(&dir);
            assert_eq!(s.prefs.workflow, Workflow::Premiere);
            assert_eq!(s.shortcuts.preset, "FilmCraft Default");
            assert_eq!(file(&dir)["workflow"], "premiere");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An older build (pre-RS1) rewrites the version to 2 on save: the user's later Premiere
    /// choice still stands (the marker file remembers the switch ran); deleting the preferences
    /// starts fresh in Resolve again.
    #[test]
    fn an_older_build_rewriting_the_version_does_not_migrate_again() {
        let dir = tmp_dir("rs1-downgrade");
        std::fs::write(dir.join("preferences.json"), serde_json::to_vec(&json!({"version": 2, "workflow": "premiere"})).unwrap()).unwrap();
        let mut s = start(&dir);
        assert_eq!(s.prefs.workflow, Workflow::Resolve);
        assert!(dir.join(crate::fork_resolve::MARKER).exists());
        s.set_workflow(Workflow::Premiere).unwrap();
        drop(s);
        // what the phase-6 build writes back
        let mut f = file(&dir);
        f["version"] = json!(2);
        std::fs::write(dir.join("preferences.json"), serde_json::to_vec(&f).unwrap()).unwrap();
        let s = start(&dir);
        assert_eq!(s.prefs.workflow, Workflow::Premiere);
        assert_eq!(s.shortcuts.preset, "FilmCraft Default");
        drop(s);
        std::fs::remove_file(dir.join("preferences.json")).unwrap();
        assert_eq!(start(&dir).prefs.workflow, Workflow::Resolve, "a fresh start is Resolve");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_already_in_resolve_keeps_its_keyboard_and_gains_the_theme() {
        let dir = tmp_dir("rs1-already");
        std::fs::write(
            dir.join("preferences.json"),
            serde_json::to_vec(&json!({"version": 2, "workflow": "resolve", "workflowReturnPreset": "Mine"})).unwrap(),
        )
        .unwrap();
        let p = Preferences::load(&dir.join("preferences.json"));
        assert_eq!((p.workflow, p.workflow_return_preset.as_str(), p.pending_resolve_migration), (Workflow::Resolve, "Mine", false));
        assert_eq!((p.appearance.dark_theme.as_str(), p.workflow_return_theme.as_str()), ("resolve", "darkest"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_1_files_and_a_fresh_install_start_in_resolve() {
        let dir = tmp_dir("rs1-v1");
        std::fs::write(dir.join("preferences.json"), serde_json::to_vec(&json!({"trim": {"loopTrim": true}})).unwrap()).unwrap();
        assert_eq!(start(&dir).prefs.workflow, Workflow::Resolve);
        let fresh = tmp_dir("rs1-fresh");
        let s = start(&fresh);
        assert_eq!((s.prefs.workflow, s.shortcuts.preset.as_str()), (Workflow::Resolve, "DaVinci Resolve Compatible"));
        assert_eq!(file(&fresh)["workflow"], "resolve");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&fresh);
    }

    #[test]
    fn garbage_preferences_fall_back_safely() {
        for (i, bytes) in
            [b"not json at all".to_vec(), b"[1,2,3]".to_vec(), b"{\"version\": \"x\", \"workflow\": 5}".to_vec(), Vec::new()].into_iter().enumerate()
        {
            let dir = tmp_dir(&format!("rs1-garbage-{i}"));
            std::fs::write(dir.join("preferences.json"), bytes).unwrap();
            let s = start(&dir);
            assert_eq!(s.prefs.workflow, Workflow::Resolve, "case {i}");
            assert_eq!(s.shortcuts.preset, "DaVinci Resolve Compatible", "case {i}");
            let _ = std::fs::remove_dir_all(&dir);
        }
        // a current-version file is never touched
        let dir = tmp_dir("rs1-v3");
        std::fs::write(dir.join("preferences.json"), serde_json::to_vec(&json!({"version": 3, "workflow": "premiere"})).unwrap()).unwrap();
        let s = start(&dir);
        assert_eq!((s.prefs.workflow, s.shortcuts.preset.as_str()), (Workflow::Premiere, "FilmCraft Default"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
