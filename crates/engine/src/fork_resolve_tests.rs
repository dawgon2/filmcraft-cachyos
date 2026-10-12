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
    s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": "dark"})).unwrap();
    s.execute("shortcuts.loadPreset", json!({"name": "Avid Media Composer Compatible"})).unwrap();
    s.set_workflow(Workflow::Resolve).unwrap();
    assert_eq!(s.shortcuts.preset, "DaVinci Resolve Compatible");
    assert_eq!(s.prefs.appearance.dark_theme, "resolve");
    assert_eq!(s.prefs.appearance.color_theme, "resolve", "the legacy theme follows");
    assert_eq!(s.prefs.workflow_return_theme, "dark");
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(s.shortcuts.preset, "Avid Media Composer Compatible");
    assert_eq!(s.prefs.appearance.dark_theme, "dark");
    assert_eq!(s.prefs.workflow_return_theme, "");
    // a theme picked by hand while in Resolve stays
    s.set_workflow(Workflow::Resolve).unwrap();
    s.execute("prefs.set", json!({"key": "appearance.darkTheme", "value": "darkest"})).unwrap();
    s.set_workflow(Workflow::Premiere).unwrap();
    assert_eq!(s.prefs.appearance.dark_theme, "darkest");
    // the bookkeeping can't be set around the switch
    assert!(s.execute("prefs.set", json!({"key": "workflowReturnTheme", "value": "dark"})).is_err());
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
