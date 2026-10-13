//! L2.1: XDG base directories (`crate::dirs`).

use crate::Session;
use crate::dirs::{AppDirs, xdg_platform};
use crate::media_test_util::tmp_dir;
use serde_json::json;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let m: HashMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
    move |k| m.get(k).cloned()
}

#[test]
fn default_linux_install_splits_into_xdg_dirs() {
    let env = env_of(&[("HOME", "/home/u")]);
    let d = AppDirs::for_data_dir_with(Path::new("/home/u/.local/share/filmcraft"), true, env);
    assert_eq!(d.data, PathBuf::from("/home/u/.local/share/filmcraft"));
    assert_eq!(d.config, PathBuf::from("/home/u/.config/filmcraft"));
    assert_eq!(d.cache, PathBuf::from("/home/u/.cache/filmcraft"));
    assert_eq!(d.state, PathBuf::from("/home/u/.local/state/filmcraft"));
    assert_eq!(d.logs(), PathBuf::from("/home/u/.local/state/filmcraft/Logs"));
    assert_eq!(d.media_cache(), PathBuf::from("/home/u/.cache/filmcraft/Media Cache"));
}

#[test]
fn xdg_variables_are_honoured() {
    let env = env_of(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/d"), ("XDG_CONFIG_HOME", "/c"), ("XDG_CACHE_HOME", "/k"), ("XDG_STATE_HOME", "/s/")]);
    let d = AppDirs::for_data_dir_with(Path::new("/d/filmcraft"), true, env);
    assert_eq!(d.config, PathBuf::from("/c/filmcraft"));
    assert_eq!(d.cache, PathBuf::from("/k/filmcraft"));
    assert_eq!(d.state, PathBuf::from("/s/filmcraft"));
    // a trailing slash on the data directory is the same directory
    let env = env_of(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/d")]);
    assert_eq!(AppDirs::for_data_dir_with(Path::new("/d/filmcraft/"), true, env).cache, PathBuf::from("/home/u/.cache/filmcraft"));
}

#[test]
fn relative_or_empty_xdg_variables_are_ignored() {
    // the specification: a relative path is invalid and must be ignored
    let env = env_of(&[("HOME", "/home/u"), ("XDG_CACHE_HOME", "cache"), ("XDG_STATE_HOME", ""), ("XDG_CONFIG_HOME", "../x")]);
    let d = AppDirs::for_data_dir_with(Path::new("/home/u/.local/share/filmcraft"), true, env);
    assert_eq!(d.cache, PathBuf::from("/home/u/.cache/filmcraft"));
    assert_eq!(d.state, PathBuf::from("/home/u/.local/state/filmcraft"));
    assert_eq!(d.config, PathBuf::from("/home/u/.config/filmcraft"));
}

#[test]
fn an_explicit_data_dir_keeps_everything_in_one_folder() {
    // --data-dir (the fork launcher's ~/.local/share/filmcraft-fork), a portable install, tests
    let env = env_of(&[("HOME", "/home/u")]);
    let d = AppDirs::for_data_dir_with(Path::new("/home/u/.local/share/filmcraft-fork"), true, env);
    assert_eq!(d, AppDirs::single(Path::new("/home/u/.local/share/filmcraft-fork")));
    assert_eq!(d.logs(), PathBuf::from("/home/u/.local/share/filmcraft-fork/Logs"));
    assert_eq!(d.media_cache(), PathBuf::from("/home/u/.local/share/filmcraft-fork/Media Cache"));
    // FILMCRAFT_DATA_DIR pointing at the default path still means "one folder, on purpose"
    let env = env_of(&[("HOME", "/home/u"), ("FILMCRAFT_DATA_DIR", "/home/u/.local/share/filmcraft")]);
    let p = Path::new("/home/u/.local/share/filmcraft");
    assert_eq!(AppDirs::for_data_dir_with(p, true, env), AppDirs::single(p));
}

#[test]
fn non_xdg_platforms_and_missing_home_keep_one_folder() {
    let p = Path::new("/home/u/.local/share/filmcraft");
    assert_eq!(AppDirs::for_data_dir_with(p, false, env_of(&[("HOME", "/home/u")])), AppDirs::single(p));
    assert_eq!(AppDirs::for_data_dir_with(p, true, env_of(&[])), AppDirs::single(p));
    assert_eq!(AppDirs::for_data_dir_with(p, true, env_of(&[("HOME", "")])), AppDirs::single(p));
    // garbage data directories never panic
    for weird in ["", "/", "relative/dir", "\u{0}", "../../.."] {
        let d = AppDirs::for_data_dir_with(Path::new(weird), true, env_of(&[("HOME", "/home/u")]));
        assert_eq!(d, AppDirs::single(Path::new(weird)));
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_are_kept_exactly() {
    use std::os::unix::ffi::OsStringExt;
    let home = OsString::from_vec(b"/home/\xffu".to_vec());
    let h2 = home.clone();
    let env = move |k: &str| (k == "HOME").then(|| h2.clone());
    let data = PathBuf::from(home).join(".local/share/filmcraft");
    let d = AppDirs::for_data_dir_with(&data, true, env);
    assert!(d.cache.ends_with(".cache/filmcraft"));
    assert_eq!(d.cache.parent().and_then(Path::parent), data.parent().and_then(Path::parent).and_then(Path::parent));
    // to_json is lossy but never fails
    assert!(d.to_json()["cache"].as_str().is_some());
}

fn split_in(root: &Path) -> AppDirs {
    AppDirs { data: root.join("data"), config: root.join("config"), cache: root.join("cache"), state: root.join("state") }
}

#[test]
fn an_older_media_cache_and_logs_are_moved_once() {
    let root = tmp_dir("dirs-migrate");
    let d = split_in(&root);
    std::fs::create_dir_all(d.data.join("Media Cache/Previews")).unwrap();
    std::fs::write(d.data.join("Media Cache/Previews/a.mov"), b"x").unwrap();
    std::fs::create_dir_all(d.data.join("Logs")).unwrap();
    std::fs::write(d.data.join("Logs/filmcraft.log"), b"log").unwrap();
    // before the move the old folders are what's used (Media Cache Info / Delete still see them)
    assert_eq!(d.media_cache(), d.data.join("Media Cache"));
    assert_eq!(d.logs(), d.data.join("Logs"));
    let lines = d.migrate();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|l| l.starts_with("moved")), "{lines:?}");
    assert_eq!(std::fs::read(d.cache.join("Media Cache/Previews/a.mov")).unwrap(), b"x");
    assert_eq!(std::fs::read(d.state.join("Logs/filmcraft.log")).unwrap(), b"log");
    assert!(!d.data.join("Media Cache").exists() && !d.data.join("Logs").exists());
    assert_eq!(d.media_cache(), d.cache.join("Media Cache"));
    assert_eq!(d.logs(), d.state.join("Logs"));
    // second start: nothing to do
    assert!(d.migrate().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn migration_never_overwrites_and_one_folder_is_left_alone() {
    let root = tmp_dir("dirs-migrate-both");
    let d = split_in(&root);
    // both old and new exist: nothing moves, the new one is used, the old one stays
    std::fs::create_dir_all(d.data.join("Media Cache")).unwrap();
    std::fs::write(d.data.join("Media Cache/old"), b"o").unwrap();
    std::fs::create_dir_all(d.cache.join("Media Cache")).unwrap();
    assert!(d.migrate().is_empty());
    assert!(d.data.join("Media Cache/old").exists());
    assert_eq!(d.media_cache(), d.cache.join("Media Cache"));
    // an old *file* (not a folder) named like the cache is not touched
    std::fs::write(d.data.join("Logs"), b"not a folder").unwrap();
    assert!(d.migrate().is_empty());
    assert_eq!(d.logs(), d.state.join("Logs"));
    // everything in one folder: never moves
    let one = AppDirs::single(&d.data);
    assert!(one.migrate().is_empty());
    assert_eq!(one.media_cache(), d.data.join("Media Cache"));
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_moved_stays_in_use() {
    use std::os::unix::fs::PermissionsExt;
    let root = tmp_dir("dirs-migrate-ro");
    let d = split_in(&root);
    std::fs::create_dir_all(d.data.join("Media Cache")).unwrap();
    // the cache base can't be created: a file is in the way
    std::fs::write(&d.cache, b"in the way").unwrap();
    let lines = d.migrate();
    assert!(lines.iter().any(|l| l.starts_with("kept")), "{lines:?}");
    assert_eq!(d.media_cache(), d.data.join("Media Cache"), "the old folder stays in use");
    let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn app_paths_command_reports_the_folders() {
    // no data directory (tests, CLI, web): folders are null
    let mut s = Session::default();
    let v = s.execute("app.paths", json!({})).unwrap();
    assert!(v["data"].is_null() && v["logs"].is_null() && v["mediaCache"].is_null(), "{v}");
    assert_eq!(v["xdg"], json!(xdg_platform()));
    // an explicit data directory keeps everything there (logs, media cache)
    let dir = tmp_dir("dirs-app-paths");
    let data = dir.join("data");
    s.start_autosave(crate::autosave::AutosaveConfig::new(&data)).unwrap();
    let v = s.execute("app.paths", json!({})).unwrap();
    let p = |k: &str| PathBuf::from(v[k].as_str().unwrap_or_default());
    assert_eq!(p("data"), data);
    assert_eq!(p("cache"), data);
    assert_eq!(p("state"), data);
    assert_eq!(p("logs"), data.join("Logs"));
    assert_eq!(p("mediaCache"), data.join("Media Cache"));
    s.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
