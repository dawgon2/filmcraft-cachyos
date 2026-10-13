//! Fork RS2: the Project Manager home screen's engine side: listing, sorting, searching,
//! summaries and their cache, hostile files, trash, rename / duplicate collisions, names that try
//! to leave the projects folder, startup choice and preferences.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;
use crate::projects::{self, SortKey, StartupAction, StartupShow};
use crate::settings::Workflow;

struct Env {
    s: Session,
    dir: PathBuf,
    root: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn env(tag: &str) -> Env {
    let dir = tmp_dir(&format!("rs2-{tag}"));
    let root = dir.join("Projects");
    let mut s = Session::default();
    s.prefs.general.projects_folder = Some(root.clone());
    std::fs::create_dir_all(dir.join("data")).unwrap();
    s.prefs_path = Some(dir.join("data/preferences.json"));
    s.trash_dir = Some(dir.join("Trash"));
    Env { s, dir, root }
}

/// A saved project with one `w`×`h` sequence at `fps`.
fn save_project(s: &mut Session, path: &Path, w: u32, h: u32, fps: f64) {
    s.execute("file.newProject", json!({"name": "x"})).unwrap();
    s.execute("file.newSequence", json!({"name": "Seq", "width": w, "height": h, "fps": fps})).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    s.execute("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
}

fn list(s: &mut Session, p: Value) -> Value {
    s.execute("projects.list", p).unwrap()
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

fn set_mtime(path: &Path, secs: u64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

#[test]
fn lists_folders_and_projects_hiding_support_folders() {
    let mut e = env("list");
    save_project(&mut e.s, &e.root.join("Beta.fcproj"), 1920, 1080, 25.0);
    save_project(&mut e.s, &e.root.join("alpha.fcproj"), 3840, 2160, 23.976);
    save_project(&mut e.s, &e.root.join("Client/Gamma.fcproj"), 1280, 720, 30.0);
    std::fs::create_dir_all(e.root.join("Auto-Save")).unwrap();
    std::fs::create_dir_all(e.root.join(".hidden")).unwrap();
    std::fs::write(e.root.join("notes.txt"), "x").unwrap();
    let v = list(&mut e.s, json!({}));
    assert_eq!(names(&v, "projects"), ["alpha", "Beta"], "{v}");
    assert_eq!(names(&v, "folders"), ["Client"], "support folders (Auto-Save, previews) and dot folders are hidden");
    assert_eq!(v["folders"][0]["projects"], 1);
    let alpha = &v["projects"][0];
    assert_eq!((alpha["width"].as_u64(), alpha["height"].as_u64(), alpha["sequenceCount"].as_u64()), (Some(3840), Some(2160), Some(1)));
    assert!((alpha["fps"].as_f64().unwrap() - 23.976).abs() < 0.01);
    assert!(alpha["error"].is_null());
    // into the sub-folder
    let v = list(&mut e.s, json!({"folder": "Client"}));
    assert_eq!(names(&v, "projects"), ["Gamma"]);
    assert_eq!(v["breadcrumb"], json!(["Client"]));
    assert!(v["other"].as_array().unwrap().is_empty(), "other locations only on the top level");
}

#[test]
fn sorts_and_searches() {
    let mut e = env("sort");
    for (n, t) in [("b", 300), ("a", 100), ("c", 200)] {
        let p = e.root.join(format!("{n}.fcproj"));
        save_project(&mut e.s, &p, 640, 360, 24.0);
        set_mtime(&p, 1_700_000_000 + t);
    }
    assert_eq!(names(&list(&mut e.s, json!({"sort": "name"})), "projects"), ["a", "b", "c"]);
    assert_eq!(names(&list(&mut e.s, json!({"sort": "name", "descending": true})), "projects"), ["c", "b", "a"]);
    assert_eq!(names(&list(&mut e.s, json!({"sort": "modified"})), "projects"), ["b", "c", "a"], "newest first");
    assert_eq!(names(&list(&mut e.s, json!({"search": "B"})), "projects"), ["b"], "case-insensitive substring");
    assert!(e.s.execute("projects.list", json!({"sort": "size"})).is_err());
}

#[test]
fn garbage_files_are_cant_read_entries_not_failures() {
    let mut e = env("garbage");
    std::fs::create_dir_all(&e.root).unwrap();
    save_project(&mut e.s, &e.root.join("good.fcproj"), 1920, 1080, 25.0);
    let good = std::fs::read(e.root.join("good.fcproj")).unwrap();
    std::fs::write(e.root.join("empty.fcproj"), b"").unwrap();
    std::fs::write(e.root.join("random.fcproj"), (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<_>>()).unwrap();
    std::fs::write(e.root.join("truncated.fcproj"), &good[..good.len() / 2]).unwrap();
    std::fs::write(e.root.join("nonutf8.fcproj"), [0xff, 0xfe, b'{', 0x80, b'}']).unwrap();
    std::fs::write(e.root.join("foreign.fcproj"), br#"{"format":"somebody.else","schema_version":1}"#).unwrap();
    std::fs::write(e.root.join("deep.fcproj"), "[".repeat(100_000)).unwrap();
    std::fs::create_dir_all(e.root.join("dir.fcproj")).unwrap();
    // larger than the cap: never read
    std::fs::File::create(e.root.join("huge.fcproj")).unwrap().set_len(projects::MAX_PROJECT_BYTES + 1).unwrap();
    let v = list(&mut e.s, json!({}));
    let projects = v["projects"].as_array().unwrap();
    let get = |n: &str| projects.iter().find(|p| p["name"] == n).unwrap_or_else(|| panic!("{n} missing: {v}"));
    assert!(get("good")["error"].is_null());
    for bad in ["empty", "random", "truncated", "nonutf8", "foreign", "deep", "huge"] {
        assert!(get(bad)["error"].is_string(), "{bad}: {}", get(bad));
    }
    assert!(get("huge")["error"].as_str().unwrap().contains("MiB"));
    // a folder named like a project is a folder
    assert!(names(&v, "folders").contains(&"dir.fcproj".to_string()));
    // listing twice (cached) gives the same
    assert_eq!(list(&mut e.s, json!({}))["projects"], v["projects"]);
}

#[test]
fn summaries_are_cached_and_recomputed_when_the_file_changes() {
    let mut e = env("cache");
    let path = e.root.join("p.fcproj");
    save_project(&mut e.s, &path, 1920, 1080, 25.0);
    let cache = projects::cache_dir(&e.s).unwrap();
    let key = projects::cache_key(&path);
    assert!(cache.join(format!("{key}.json")).is_file(), "saving writes the summary");
    assert_eq!(e.s.poster_requests, [path.to_string_lossy().to_string()], "and asks for a poster");
    let s1 = projects::summary(Some(&cache), &path);
    assert_eq!((s1.width, s1.height), (1920, 1080));
    // a cache entry that lies but matches the file is believed (that's the point of the cache)
    let mut entry: Value = serde_json::from_slice(&std::fs::read(cache.join(format!("{key}.json"))).unwrap()).unwrap();
    entry["summary"]["width"] = json!(7);
    std::fs::write(cache.join(format!("{key}.json")), serde_json::to_vec(&entry).unwrap()).unwrap();
    assert_eq!(projects::summary(Some(&cache), &path).width, 7);
    // the file changes (another build saved it): stale, read again
    let mut other = Session::default();
    save_project(&mut other, &path, 1280, 720, 30.0);
    set_mtime(&path, 1_800_000_000);
    let s2 = projects::summary(Some(&cache), &path);
    assert_eq!((s2.width, s2.height), (1280, 720));
    // a garbage cache entry is ignored
    std::fs::write(cache.join(format!("{key}.json")), b"\x00garbage").unwrap();
    assert_eq!(projects::summary(Some(&cache), &path).width, 1280);
    // no cache at all (headless) still works
    assert_eq!(projects::summary(None, &path).width, 1280);
}

#[test]
fn posters_round_trip_downscaled_and_garbage_is_ignored() {
    let e = env("poster");
    let cache = e.dir.join("cache");
    let project = e.dir.join("p.fcproj");
    std::fs::write(&project, b"x").unwrap();
    let (w, h) = (1920u32, 1080u32);
    let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, 0, 0, 255]).collect();
    projects::store_poster(&cache, &project, w, h, &rgba, Some("color")).unwrap();
    let png = cache.join(format!("{}.png", projects::cache_key(&project)));
    let (pw, ph, px) = projects::load_poster(&png).unwrap();
    assert_eq!((pw, ph), (480, 270));
    assert_eq!(px.len(), 480 * 270 * 4);
    let s = projects::summary(Some(&cache), &project);
    assert_eq!(s.page.as_deref(), Some("color"));
    assert_eq!(s.poster.as_deref(), Some(png.to_string_lossy().as_ref()));
    // malformed input is refused, garbage PNGs don't decode
    assert!(projects::store_poster(&cache, &project, 10, 10, &[0; 3], None).is_err());
    assert!(projects::downscale(0, 10, &[], 480).is_none());
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nnot really").unwrap();
    assert!(projects::load_poster(&png).is_none());
}

#[test]
fn names_cannot_leave_the_folder() {
    let mut e = env("names");
    for bad in ["../x", "a/b", "a\\b", "..", ".", ".hidden", "", "   ", "x\u{0}y", "tab\there", &"n".repeat(300)] {
        assert!(projects::check_name(bad).is_err(), "{bad:?} accepted");
        assert!(e.s.execute("projects.newFolder", json!({"name": bad})).is_err(), "newFolder {bad:?}");
        assert!(e.s.execute("projects.new", json!({"name": bad})).is_err(), "new {bad:?}");
    }
    assert_eq!(projects::check_name(" Cut 1.fcproj ").unwrap(), "Cut 1");
    // a leading slash is still relative to the projects folder
    assert!(projects::resolve_folder(&e.root, "/etc").unwrap().starts_with(&e.root));
    for bad in ["..", "../..", "a/../..", "a/.git", "a/./b"] {
        assert!(e.s.execute("projects.list", json!({"folder": bad})).is_err(), "list folder {bad:?}");
        assert!(e.s.execute("projects.new", json!({"name": "ok", "folder": bad})).is_err(), "new in {bad:?}");
    }
    assert!(!e.dir.join("ok.fcproj").exists() && !Path::new("/etc/ok.fcproj").exists());
    // a project in a nested folder is fine
    e.s.execute("projects.newFolder", json!({"name": "A"})).unwrap();
    e.s.execute("projects.newFolder", json!({"name": "B", "folder": "A"})).unwrap();
    assert!(e.root.join("A/B").is_dir());
    assert!(e.s.execute("projects.newFolder", json!({"name": "A"})).is_err(), "exists");
}

#[test]
fn new_project_is_created_opened_and_never_overwrites() {
    let mut e = env("new");
    let r = e.s.execute("projects.new", json!({"name": "Holiday"})).unwrap();
    let path = e.root.join("Holiday.fcproj");
    assert_eq!(r["path"], json!(path.to_string_lossy()));
    assert!(path.is_file(), "the projects folder is made on first use");
    assert_eq!(e.s.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert_eq!(e.s.prefs.general.recent_projects.first().map(String::as_str), Some(path.to_string_lossy().as_ref()));
    assert!(e.s.execute("projects.new", json!({"name": "Holiday"})).is_err(), "never overwrites");
    // unsaved changes are kept unless the caller says to discard them
    e.s.execute("file.newSequence", json!({"name": "S"})).unwrap();
    assert!(e.s.is_dirty());
    assert!(e.s.execute("projects.new", json!({"name": "Other"})).is_err());
    e.s.execute("projects.new", json!({"name": "Other", "discard": true})).unwrap();
    assert!(e.root.join("Other.fcproj").is_file());
    // projects.open lands back on Holiday
    let r = e.s.execute("projects.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(r["page"].is_null(), "no page recorded yet");
    assert_eq!(e.s.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert!(e.s.execute("projects.open", json!({"path": e.root.join("nope.fcproj").to_string_lossy()})).is_err());
}

#[test]
fn rename_and_duplicate_refuse_collisions() {
    let mut e = env("rename");
    save_project(&mut e.s, &e.root.join("One.fcproj"), 640, 360, 24.0);
    save_project(&mut e.s, &e.root.join("Two.fcproj"), 640, 360, 24.0);
    let one = e.root.join("One.fcproj");
    // Two is open: it can't be renamed or trashed
    assert!(e.s.execute("projects.rename", json!({"path": e.root.join("Two.fcproj").to_string_lossy(), "name": "Three"})).is_err());
    assert!(e.s.execute("projects.rename", json!({"path": one.to_string_lossy(), "name": "Two"})).is_err(), "collision");
    assert!(e.s.execute("projects.rename", json!({"path": one.to_string_lossy(), "name": "../Up"})).is_err());
    let r = e.s.execute("projects.rename", json!({"path": one.to_string_lossy(), "name": "Uno"})).unwrap();
    let uno = e.root.join("Uno.fcproj");
    assert_eq!(r["path"], json!(uno.to_string_lossy()));
    assert!(uno.is_file() && !one.exists());
    assert!(e.s.prefs.general.recent_projects.iter().any(|p| Path::new(p) == uno), "recents follow the rename");
    assert!(!e.s.prefs.general.recent_projects.iter().any(|p| Path::new(p) == one));
    // duplicates: "Uno copy", "Uno copy 2"; a named one can't collide
    let d1 = e.s.execute("projects.duplicate", json!({"path": uno.to_string_lossy()})).unwrap();
    let d2 = e.s.execute("projects.duplicate", json!({"path": uno.to_string_lossy()})).unwrap();
    assert_eq!((d1["name"].as_str(), d2["name"].as_str()), (Some("Uno copy"), Some("Uno copy 2")));
    assert_eq!(std::fs::read(&uno).unwrap(), std::fs::read(e.root.join("Uno copy.fcproj")).unwrap());
    assert!(e.s.execute("projects.duplicate", json!({"path": uno.to_string_lossy(), "name": "Two"})).is_err());
    // folders inside the projects folder rename too; ones outside don't
    e.s.execute("projects.newFolder", json!({"name": "F"})).unwrap();
    e.s.execute("projects.rename", json!({"path": e.root.join("F").to_string_lossy(), "name": "G"})).unwrap();
    assert!(e.root.join("G").is_dir());
    assert!(e.s.execute("projects.rename", json!({"path": e.dir.join("data").to_string_lossy(), "name": "x"})).is_err());
    assert!(e.s.execute("projects.rename", json!({"path": e.root.to_string_lossy(), "name": "x"})).is_err(), "not the root itself");
}

#[test]
fn trash_moves_into_the_trash_with_an_info_file() {
    let mut e = env("trash");
    let p = e.root.join("Gone.fcproj");
    save_project(&mut e.s, &p, 640, 360, 24.0);
    e.s.execute("file.newProject", json!({"name": "other"})).unwrap();
    let r = e.s.execute("projects.trash", json!({"path": p.to_string_lossy()})).unwrap();
    let trash = e.dir.join("Trash");
    assert!(!p.exists());
    assert_eq!(r["trashed"], json!(trash.join("files/Gone.fcproj").to_string_lossy()));
    assert!(trash.join("files/Gone.fcproj").is_file());
    let info = std::fs::read_to_string(trash.join("info/Gone.fcproj.trashinfo")).unwrap();
    assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
    assert!(info.contains("Gone.fcproj\nDeletionDate=") && info.contains('T'), "{info}");
    assert!(!e.s.prefs.general.recent_projects.iter().any(|x| Path::new(x) == p), "gone from the recents");
    // the same name again: a second, numbered entry
    save_project(&mut e.s, &p, 640, 360, 24.0);
    e.s.execute("file.newProject", json!({"name": "other"})).unwrap();
    e.s.execute("projects.trash", json!({"path": p.to_string_lossy()})).unwrap();
    assert!(trash.join("files/Gone.2.fcproj").is_file());
    assert!(trash.join("info/Gone.2.fcproj.trashinfo").is_file());
    // the open project, a non-project file and folders outside the projects folder stay
    let open = e.root.join("Open.fcproj");
    save_project(&mut e.s, &open, 640, 360, 24.0);
    assert!(e.s.execute("projects.trash", json!({"path": open.to_string_lossy()})).is_err());
    std::fs::write(e.root.join("notes.txt"), "x").unwrap();
    assert!(e.s.execute("projects.trash", json!({"path": e.root.join("notes.txt").to_string_lossy()})).is_err());
    assert!(e.s.execute("projects.trash", json!({"path": e.dir.join("data").to_string_lossy()})).is_err());
    assert!(open.is_file() && e.dir.join("data").is_dir());
    // a folder inside the projects folder goes whole
    e.s.execute("projects.newFolder", json!({"name": "Old"})).unwrap();
    e.s.execute("projects.trash", json!({"path": e.root.join("Old").to_string_lossy()})).unwrap();
    assert!(trash.join("files/Old").is_dir() && !e.root.join("Old").exists());
}

#[test]
fn trash_info_dates_and_paths() {
    assert_eq!(projects::iso_local(0, 0), "1970-01-01T00:00:00");
    assert_eq!(projects::iso_local(1_760_000_000, 0), "2025-10-09T08:53:20");
    assert_eq!(projects::iso_local(1_760_000_000, -7 * 3600), "2025-10-09T01:53:20");
    assert_eq!(projects::iso_local(951_782_400, 0), "2000-02-29T00:00:00", "leap day");
}

#[test]
fn other_locations_list_recents_outside_the_folder() {
    let mut e = env("other");
    let elsewhere = e.dir.join("Elsewhere/Far.fcproj");
    save_project(&mut e.s, &elsewhere, 1920, 1080, 25.0);
    let inside = e.root.join("Near.fcproj");
    save_project(&mut e.s, &inside, 1920, 1080, 25.0);
    let gone = e.dir.join("Gone.fcproj");
    let mut prefs = e.s.prefs.clone();
    prefs.general.recent_projects.push(gone.to_string_lossy().into_owned());
    e.s.set_prefs(prefs).unwrap();
    let v = list(&mut e.s, json!({}));
    assert_eq!(names(&v, "projects"), ["Near"]);
    let other = v["other"].as_array().unwrap();
    assert_eq!(other.len(), 2, "{v}");
    let far = other.iter().find(|o| o["name"] == "Far").unwrap();
    assert_eq!(far["missing"], false);
    let g = other.iter().find(|o| o["name"] == "Gone").unwrap();
    assert_eq!(g["missing"], true);
    // locate it, then remove it from the list
    let moved = e.dir.join("Moved.fcproj");
    std::fs::copy(&elsewhere, &moved).unwrap();
    e.s.execute("projects.locate", json!({"path": gone.to_string_lossy(), "newPath": moved.to_string_lossy()})).unwrap();
    assert!(e.s.prefs.general.recent_projects.iter().any(|p| Path::new(p) == moved));
    let r = e.s.execute("projects.removeFromList", json!({"path": moved.to_string_lossy()})).unwrap();
    assert_eq!(r["removed"], 1);
    assert!(moved.is_file(), "removing from the list leaves the file");
    let v = list(&mut e.s, json!({}));
    assert_eq!(names(&v, "other"), ["Far"]);
}

#[test]
fn import_copies_projects_and_refuses_garbage() {
    let mut e = env("import");
    let src = e.dir.join("In/Shoot.fcproj");
    save_project(&mut e.s, &src, 1920, 1080, 25.0);
    let r = e.s.execute("projects.import", json!({"path": src.to_string_lossy()})).unwrap();
    assert_eq!(r["path"], json!(e.root.join("Shoot.fcproj").to_string_lossy()));
    let r = e.s.execute("projects.import", json!({"path": src.to_string_lossy()})).unwrap();
    assert_eq!(r["path"], json!(e.root.join("Shoot 2.fcproj").to_string_lossy()), "never overwrites");
    let junk = e.dir.join("junk.fcproj");
    std::fs::write(&junk, b"not json").unwrap();
    assert!(e.s.execute("projects.import", json!({"path": junk.to_string_lossy()})).is_err());
    // an archive folder comes in whole (a background copy; `wait` for the test)
    std::fs::write(e.dir.join("In/media.bin"), [1u8; 64]).unwrap();
    let r = e.s.execute("projects.import", json!({"path": e.dir.join("In").to_string_lossy(), "wait": true})).unwrap();
    assert!(r["job"].is_u64());
    assert!(e.root.join("In/Shoot.fcproj").is_file() && e.root.join("In/media.bin").is_file());
    // a folder without a project, or the projects folder itself, is refused
    std::fs::create_dir_all(e.dir.join("Empty")).unwrap();
    assert!(e.s.execute("projects.import", json!({"path": e.dir.join("Empty").to_string_lossy()})).is_err());
    assert!(e.s.execute("projects.import", json!({"path": e.root.to_string_lossy()})).is_err());
}

#[test]
fn export_without_sequences_writes_the_project_into_the_folder() {
    let mut e = env("export");
    e.s.execute("projects.new", json!({"name": "Bare"})).unwrap();
    let dest = e.dir.join("Archive");
    let r = e.s.execute("projects.export", json!({"path": e.root.join("Bare.fcproj").to_string_lossy(), "destination": dest.to_string_lossy()})).unwrap();
    assert!(dest.join("Bare.fcproj").is_file(), "{r}");
    assert!(
        e.s.execute("projects.export", json!({"path": e.root.join("Bare.fcproj").to_string_lossy(), "destination": dest.to_string_lossy()})).is_err(),
        "never overwrites"
    );
    // with a sequence: the collect engine (File ▸ Project Manager) writes the archive
    let p = e.root.join("Seq.fcproj");
    save_project(&mut e.s, &p, 640, 360, 24.0);
    let dest2 = e.dir.join("Archive2");
    e.s.execute("projects.export", json!({"path": p.to_string_lossy(), "destination": dest2.to_string_lossy(), "wait": true})).unwrap();
    assert!(dest2.join("Seq.fcproj").is_file());
}

#[test]
fn startup_follows_the_flag_then_the_setting_then_the_workflow() {
    let mut p = crate::autosave::Preferences { workflow: Workflow::Resolve, ..Default::default() };
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::ProjectManager, "Resolve: the home screen");
    assert_eq!(projects::startup_action(&p, true, true), StartupAction::Demo, "--demo wins");
    assert_eq!(projects::startup_action(&p, true, false), StartupAction::NewEmpty, "--empty wins");
    p.workflow = Workflow::Premiere;
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::Demo, "Premiere: today's Show Home");
    p.general.at_startup = "openMostRecent".into();
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::LastProject);
    p.general.at_startup = "emptyProject".into();
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::NewEmpty);
    p.general.startup_show = Some(StartupShow::ProjectManager);
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::ProjectManager, "chosen: either workflow");
    p.workflow = Workflow::Resolve;
    p.general.startup_show = Some(StartupShow::LastProject);
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::LastProject);
    p.general.startup_show = Some(StartupShow::NewEmpty);
    assert_eq!(projects::startup_action(&p, false, true), StartupAction::NewEmpty);
}

#[test]
fn startup_and_folder_prefs_load_old_garbage_and_round_trip() {
    let dir = tmp_dir("rs2-prefs");
    let path = dir.join("preferences.json");
    // a v3 file from before RS2
    std::fs::write(&path, r#"{"version":3,"general":{"atStartup":"openMostRecent"}}"#).unwrap();
    let p = crate::autosave::Preferences::load(&path);
    assert_eq!(p.general.at_startup, "openMostRecent");
    assert_eq!((p.general.startup_show, p.general.projects_folder.clone()), (None, None));
    // garbage values fall back without losing the rest of the file
    std::fs::write(&path, r#"{"version":3,"general":{"atStartup":"openMostRecent","startupShow":42,"projectsFolder":["x"]}}"#).unwrap();
    let p = crate::autosave::Preferences::load(&path);
    assert_eq!(p.general.at_startup, "openMostRecent");
    assert_eq!((p.general.startup_show, p.general.projects_folder.clone()), (None, None));
    // through prefs.set and back
    let mut s = Session { prefs_path: Some(path.clone()), ..Default::default() };
    s.execute("prefs.set", json!({"key": "general.startupShow", "value": "lastProject"})).unwrap();
    s.execute("prefs.set", json!({"key": "general.projectsFolder", "value": "/tmp/fc projects"})).unwrap();
    assert!(s.execute("prefs.set", json!({"key": "general.startupShow", "value": "sometimes"})).is_err());
    let p = crate::autosave::Preferences::load(&path);
    assert_eq!(p.general.startup_show, Some(StartupShow::LastProject));
    assert_eq!(p.general.projects_folder, Some(PathBuf::from("/tmp/fc projects")));
    s.execute("prefs.set", json!({"key": "general.startupShow", "value": "workflowDefault"})).unwrap();
    s.execute("prefs.set", json!({"key": "general.projectsFolder", "value": ""})).unwrap();
    let p = crate::autosave::Preferences::load(&path);
    assert_eq!((p.general.startup_show, p.general.projects_folder), (None, None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn projects_show_sits_after_open_and_resolve_maps_shift_1() {
    let ids: Vec<&str> = crate::commands::command_specs().iter().map(|c| c.id).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap();
    assert_eq!(pos("projects.show"), pos("file.open") + 1);
    let sc = crate::shortcuts::Shortcuts::new();
    let resolve = sc.builtin(crate::shortcuts::RESOLVE_PRESET).unwrap();
    assert!(resolve.iter().any(|b| b.command == "projects.show" && b.keys == "Shift+1"));
    assert!(!resolve.iter().any(|b| b.command != "projects.show" && b.keys == "Shift+1" && b.panel.is_none()), "Shift+1 is the Project Manager only");
}

#[test]
fn listing_without_the_folder_yet_is_empty_not_an_error() {
    let mut e = env("absent");
    let v = list(&mut e.s, json!({}));
    assert_eq!(v["exists"], false);
    assert!(!e.root.exists(), "listing doesn't create the folder");
    assert_eq!(SortKey::parse("modified"), Some(SortKey::Modified));
}

// ------------------------------------------------------------------ review fixes (RS2)

#[test]
fn a_broad_or_relative_projects_folder_is_refused() {
    let home = PathBuf::from("/home/someone");
    let homes = [home.clone()];
    assert!(projects::unsafe_root(Path::new("relative/Projects"), &homes).is_some(), "relative");
    assert!(projects::unsafe_root(Path::new("/"), &homes).is_some(), "the file system root");
    assert!(projects::unsafe_root(&home, &homes).is_some(), "home itself");
    assert!(projects::unsafe_root(Path::new("/home"), &homes).is_some(), "a parent of home");
    assert!(projects::unsafe_root(Path::new("/home/someone/../someone/Videos"), &homes).is_some(), "`..`");
    assert!(projects::unsafe_root(&home.join("Videos/FilmCraft Projects"), &homes).is_none());
    assert!(projects::unsafe_root(Path::new("/srv/projects"), &homes).is_none());
    // a session with an unsafe preference falls back to the default folder and says why
    let mut e = env("broad");
    for bad in [PathBuf::from("Projects"), PathBuf::from("/")] {
        e.s.prefs.general.projects_folder = Some(bad.clone());
        assert_ne!(projects::projects_root(&e.s), Some(bad.clone()));
        assert!(projects::root_warning(&e.s).is_some());
        let v = list(&mut e.s, json!({}));
        assert!(v["warning"].is_string(), "{v}");
        // nothing outside a real projects folder can be renamed through it
        assert!(e.s.execute("projects.rename", json!({"path": e.dir.join("data").to_string_lossy(), "name": "x"})).is_err());
    }
    if let Some(h) = crate::media_browser::std_home_dir() {
        e.s.prefs.general.projects_folder = Some(PathBuf::from(&h));
        assert_ne!(projects::projects_root(&e.s), Some(PathBuf::from(&h)), "the real home folder is refused");
    }
}

#[test]
fn hidden_and_support_folders_are_not_renamed_or_trashed() {
    let mut e = env("support");
    for d in [".hidden", "Auto-Save", "Proxies", "Client/Proxies"] {
        std::fs::create_dir_all(e.root.join(d)).unwrap();
        for cmd in ["projects.rename", "projects.trash"] {
            let r = e.s.execute(cmd, json!({"path": e.root.join(d).to_string_lossy(), "name": "x"}));
            assert!(r.is_err(), "{cmd} {d}");
        }
        assert!(e.root.join(d).is_dir());
    }
    save_project(&mut e.s, &e.root.join("Auto-Save/Auto.fcproj"), 64, 36, 24.0);
    e.s.execute("file.newProject", json!({"name": "other"})).unwrap();
    assert!(e.s.execute("projects.trash", json!({"path": e.root.join("Auto-Save/Auto.fcproj").to_string_lossy()})).is_err());
}

#[cfg(unix)]
#[test]
fn links_inside_the_folder_are_acted_on_never_their_targets() {
    let mut e = env("links");
    let outside = e.dir.join("Outside");
    save_project(&mut e.s, &outside.join("Far.fcproj"), 64, 36, 24.0);
    e.s.execute("file.newProject", json!({"name": "other"})).unwrap();
    std::fs::create_dir_all(&e.root).unwrap();
    std::os::unix::fs::symlink(&outside, e.root.join("LinkDir")).unwrap();
    std::os::unix::fs::symlink(outside.join("Far.fcproj"), e.root.join("Linked.fcproj")).unwrap();
    // rename: the link is renamed, the target keeps its name
    e.s.execute("projects.rename", json!({"path": e.root.join("LinkDir").to_string_lossy(), "name": "Renamed"})).unwrap();
    assert!(std::fs::symlink_metadata(e.root.join("Renamed")).unwrap().file_type().is_symlink());
    assert!(outside.is_dir() && outside.join("Far.fcproj").is_file());
    e.s.execute("projects.rename", json!({"path": e.root.join("Linked.fcproj").to_string_lossy(), "name": "Other"})).unwrap();
    assert!(std::fs::symlink_metadata(e.root.join("Other.fcproj")).unwrap().file_type().is_symlink());
    assert!(outside.join("Far.fcproj").is_file());
    // trash: the links go, the targets stay
    e.s.execute("projects.trash", json!({"path": e.root.join("Renamed").to_string_lossy()})).unwrap();
    e.s.execute("projects.trash", json!({"path": e.root.join("Other.fcproj").to_string_lossy()})).unwrap();
    assert!(outside.join("Far.fcproj").is_file(), "the target outside is untouched");
    assert!(std::fs::symlink_metadata(e.dir.join("Trash/files/Renamed")).unwrap().file_type().is_symlink());
    // a link pointing outside, reached by a path through it, is not a folder inside the root
    std::os::unix::fs::symlink(&outside, e.root.join("Again")).unwrap();
    std::fs::create_dir_all(outside.join("Sub")).unwrap();
    assert!(e.s.execute("projects.trash", json!({"path": e.root.join("Again/Sub").to_string_lossy()})).is_err());
    assert!(outside.join("Sub").is_dir());
}

#[cfg(unix)]
#[test]
fn a_dangling_link_counts_as_taken() {
    let mut e = env("dangling");
    save_project(&mut e.s, &e.root.join("One.fcproj"), 64, 36, 24.0);
    e.s.execute("file.newProject", json!({"name": "other"})).unwrap();
    let decoy = e.dir.join("decoy-target");
    std::os::unix::fs::symlink(&decoy, e.root.join("Two.fcproj")).unwrap();
    std::os::unix::fs::symlink(&decoy, e.root.join("One copy.fcproj")).unwrap();
    let one = e.root.join("One.fcproj");
    assert!(e.s.execute("projects.rename", json!({"path": one.to_string_lossy(), "name": "Two"})).is_err());
    assert!(e.s.execute("projects.duplicate", json!({"path": one.to_string_lossy(), "name": "Two"})).is_err());
    let d = e.s.execute("projects.duplicate", json!({"path": one.to_string_lossy()})).unwrap();
    assert_eq!(d["name"], "One copy 2");
    assert!(!decoy.exists(), "nothing was written through the link");
    // write_new never replaces
    assert_eq!(projects::write_new(&one, b"x").unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    assert!(projects::move_no_replace(&e.root.join("One copy 2.fcproj"), &one).is_err());
    assert!(e.root.join("One copy 2.fcproj").is_file());
}

#[test]
fn folder_renames_and_trash_carry_recents_and_cache() {
    let mut e = env("prefix");
    let a = e.root.join("Client/A.fcproj");
    save_project(&mut e.s, &a, 64, 36, 24.0);
    e.s.execute("file.newProject", json!({"name": "other"})).unwrap();
    let cache = projects::cache_dir(&e.s).unwrap();
    let old_key = projects::cache_key(&a);
    assert!(cache.join(format!("{old_key}.json")).is_file());
    std::fs::write(cache.join(format!("{old_key}.png")), b"png").unwrap();
    // onto an existing folder: refused
    std::fs::create_dir_all(e.root.join("Taken")).unwrap();
    assert!(e.s.execute("projects.rename", json!({"path": e.root.join("Client").to_string_lossy(), "name": "Taken"})).is_err());
    e.s.execute("projects.rename", json!({"path": e.root.join("Client").to_string_lossy(), "name": "Customer"})).unwrap();
    let moved = e.root.join("Customer/A.fcproj");
    assert!(e.s.prefs.general.recent_projects.iter().any(|p| Path::new(p) == moved), "{:?}", e.s.prefs.general.recent_projects);
    assert!(!e.s.prefs.general.recent_projects.iter().any(|p| Path::new(p).starts_with(e.root.join("Client"))));
    let new_key = projects::cache_key(&moved);
    assert!(cache.join(format!("{new_key}.json")).is_file() && cache.join(format!("{new_key}.png")).is_file());
    assert!(!cache.join(format!("{old_key}.json")).exists() && !cache.join(format!("{old_key}.png")).exists());
    // trash: recents and cache entries under it go
    e.s.execute("projects.trash", json!({"path": e.root.join("Customer").to_string_lossy()})).unwrap();
    assert!(!e.s.prefs.general.recent_projects.iter().any(|p| Path::new(p).starts_with(e.root.join("Customer"))));
    assert!(!cache.join(format!("{new_key}.json")).exists() && !cache.join(format!("{new_key}.png")).exists());
}

#[test]
fn a_folder_holding_the_open_project_is_not_trashed_or_renamed() {
    let mut e = env("openfolder");
    let p = e.root.join("Work/Open.fcproj");
    save_project(&mut e.s, &p, 64, 36, 24.0);
    assert!(e.s.execute("projects.trash", json!({"path": e.root.join("Work").to_string_lossy()})).is_err());
    assert!(e.s.execute("projects.rename", json!({"path": e.root.join("Work").to_string_lossy(), "name": "W"})).is_err());
    assert!(p.is_file());
}

#[test]
fn importing_the_projects_folder_or_a_folder_inside_it_is_refused() {
    let mut e = env("importself");
    save_project(&mut e.s, &e.root.join("Sub/A.fcproj"), 64, 36, 24.0);
    for path in [e.root.clone(), e.root.join("Sub"), e.dir.clone()] {
        assert!(e.s.execute("projects.import", json!({"path": path.to_string_lossy(), "wait": true})).is_err(), "{}", path.display());
    }
    // a hostile file never panics the import
    let junk = e.dir.join("junk.fcproj");
    std::fs::write(&junk, [0xffu8, b'{', 0, 0x80]).unwrap();
    assert!(e.s.execute("projects.import", json!({"path": junk.to_string_lossy()})).is_err());
    // removing something that isn't listed removes nothing
    assert_eq!(e.s.execute("projects.removeFromList", json!({"path": "/nowhere.fcproj"})).unwrap()["removed"], 0);
}

#[cfg(unix)]
#[test]
fn a_planted_top_level_trash_folder_is_refused() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let e = env("topdir");
    let uid = std::fs::metadata(&e.dir).unwrap().uid();
    // missing: made with mode 0700
    let t = e.dir.join(".Trash-new");
    projects::checked_trash_dir(&t, uid).unwrap();
    assert_eq!(std::fs::metadata(&t).unwrap().mode() & 0o7777, 0o700);
    // a planted link to somewhere else
    let elsewhere = e.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, e.dir.join(".Trash-link")).unwrap();
    assert!(projects::checked_trash_dir(&e.dir.join(".Trash-link"), uid).is_err());
    // a world-writable folder, or someone else's
    let open = e.dir.join(".Trash-open");
    std::fs::create_dir_all(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(projects::checked_trash_dir(&open, uid).is_err());
    assert!(projects::checked_trash_dir(&t, uid.wrapping_add(1)).is_err());
    // a file
    std::fs::write(e.dir.join(".Trash-file"), b"x").unwrap();
    assert!(projects::checked_trash_dir(&e.dir.join(".Trash-file"), uid).is_err());
}
