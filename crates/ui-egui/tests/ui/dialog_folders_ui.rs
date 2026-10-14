//! LINUX-PLAN L1.5 (fork): file dialogs shown through the host's non-blocking runner start in the
//! folder that kind of dialog was last used in (the project's own folder for Save), and remember
//! the chosen one; a cancelled dialog, a folder that's gone or a Folder dialog with its own start
//! leave that alone; closing the window waits for a dialog parented to it. Both workflows.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::native_dialogs::FileDialog;
use serde_json::json;

/// Dialogs the app asked for: its kind, the folder it starts in, and the sender that answers it.
type Asked = Arc<Mutex<Vec<(&'static str, Option<String>, Sender<Vec<String>>)>>>;

fn harness(workflow: Workflow) -> (Harness<'static, FilmcraftApp>, Asked) {
    let mut session = Session::default();
    session.prefs.workflow = workflow;
    session.execute("file.openDemoProject", json!({})).expect("demo project");
    let mut app = FilmcraftApp::new(session);
    let asked: Asked = Arc::default();
    let a = asked.clone();
    app.hooks.run_dialog_from = Some(Box::new(move |dialog: FileDialog, start: Option<String>| -> Receiver<Vec<String>> {
        let (tx, rx) = channel();
        a.lock().unwrap().push((dialog.folder_kind(), start, tx));
        rx
    }));
    let h = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    (h, asked)
}

/// Ask for `dialog`, check where it starts, answer `paths` and let the app take the answer.
fn ask(h: &mut Harness<'static, FilmcraftApp>, asked: &Asked, dialog: FileDialog, paths: Vec<String>) -> Option<String> {
    let r = h.state_mut().pick(dialog, |_, paths| Ok(json!(paths)));
    assert_eq!(r, Ok(json!({"dialog": "open"})));
    let (_, start, tx) = asked.lock().unwrap().pop().expect("the dialog went to the runner");
    tx.send(paths).unwrap();
    for _ in 0..3 {
        h.step();
    }
    assert!(!h.state().native_dialog_open());
    start
}

fn dir(root: &Path, name: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn dialogs_start_where_their_kind_was_last_used_in_both_workflows() {
    for (n, workflow) in [Workflow::Premiere, Workflow::Resolve].into_iter().enumerate() {
        let root = std::env::temp_dir().join(format!("fc-dialog-folders-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (card, exports, archive, projects) = (dir(&root, "card"), dir(&root, "exports"), dir(&root, "archive"), dir(&root, "projects"));
        let (mut h, asked) = harness(workflow);
        h.step();

        // nothing remembered yet: the host's default
        assert_eq!(ask(&mut h, &asked, FileDialog::open_files(&["wav"]), vec![s(&card.join("a.wav"))]), None);
        assert_eq!(h.state().session.prefs.dialogs.last_folder("import"), Some(s(&card).as_str()));
        // the next import starts there; other kinds keep their own
        assert_eq!(ask(&mut h, &asked, FileDialog::open_files(&["wav"]), vec![]), Some(s(&card)), "{workflow:?}");
        assert_eq!(ask(&mut h, &asked, FileDialog::save_as("Video", &["mp4"], "x.mp4"), vec![s(&exports.join("x.mp4"))]), None);
        assert_eq!(ask(&mut h, &asked, FileDialog::save_as("Video", &["mp4"], "y.mp4"), vec![]), Some(s(&exports)));
        // a cancelled dialog changed nothing
        assert_eq!(h.state().session.prefs.dialogs.last_folder("import"), Some(s(&card).as_str()));

        // a Folder dialog remembers the folder itself; one with its own start keeps that
        assert_eq!(ask(&mut h, &asked, FileDialog::Folder { at: None }, vec![s(&archive)]), None);
        assert_eq!(ask(&mut h, &asked, FileDialog::Folder { at: None }, vec![]), Some(s(&archive)));
        assert_eq!(ask(&mut h, &asked, FileDialog::Folder { at: Some(s(&card)) }, vec![]), None);

        // Save of a saved project starts in its folder, whatever was remembered
        h.state_mut().session.path = Some(s(&projects.join("film.fcproj")));
        assert_eq!(ask(&mut h, &asked, FileDialog::SaveProject { name: "film.fcproj".into() }, vec![]), Some(s(&projects)));

        // the host checks the folder on its own thread (a dead mount mustn't hold the UI): the app
        // hands over a remembered folder that's gone (an ejected card) as it is
        std::fs::remove_dir_all(&card).unwrap();
        assert_eq!(ask(&mut h, &asked, FileDialog::open_files(&["wav"]), vec![]), Some(s(&card)));

        // closing the window waits while a parented dialog is open
        assert!(!h.state().close_waits_for_dialog());
        let _ = h.state_mut().pick(FileDialog::OpenProject, |_, _| Ok(json!(null)));
        assert!(h.state().close_waits_for_dialog());
        let (_, _, tx) = asked.lock().unwrap().pop().unwrap();
        drop(tx);
        for _ in 0..3 {
            h.step();
        }
        assert!(!h.state().close_waits_for_dialog(), "a lost dialog counts as closed");

        let _ = std::fs::remove_dir_all(&root);
    }
}
