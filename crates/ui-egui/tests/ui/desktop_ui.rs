//! LINUX-PLAN L1.7 / L2.3: the desktop host hook sees the app's state every frame, and its
//! requests (a second launch's files, media keys) run as commands. Both workflows.

use std::cell::RefCell;
use std::rc::Rc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::desktop::{DesktopHost, DesktopRequest, DesktopState, FinishedJob};
use serde_json::json;

#[derive(Default)]
struct Log {
    states: Vec<DesktopState>,
    finished: Vec<FinishedJob>,
    pending: Vec<DesktopRequest>,
}

struct FakeHost(Rc<RefCell<Log>>);

impl DesktopHost for FakeHost {
    fn update(&mut self, _ctx: &egui::Context, state: &DesktopState, finished: &[FinishedJob]) {
        let mut l = self.0.borrow_mut();
        l.states.push(state.clone());
        l.finished.extend_from_slice(finished);
    }
    fn take_requests(&mut self) -> Vec<DesktopRequest> {
        std::mem::take(&mut self.0.borrow_mut().pending)
    }
}

fn harness(workflow: Workflow) -> (Harness<'static, FilmcraftApp>, Rc<RefCell<Log>>) {
    let mut session = Session::default();
    session.prefs.workflow = workflow;
    session.execute("file.openDemoProject", json!({})).expect("demo project");
    let mut app = FilmcraftApp::new(session);
    let log = Rc::new(RefCell::new(Log::default()));
    app.hooks.desktop = Some(Box::new(FakeHost(log.clone())));
    let h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(10_000).build_eframe(move |_| app);
    (h, log)
}

fn frames(h: &mut Harness<'_, FilmcraftApp>, n: usize) {
    for _ in 0..n {
        h.step();
        assert!(h.state().ui_error.is_none(), "the app caught a UI panic: {:?}", h.state().ui_error);
    }
}

fn last(log: &Rc<RefCell<Log>>) -> DesktopState {
    log.borrow().states.last().cloned().expect("the host was called")
}

fn request(h: &mut Harness<'_, FilmcraftApp>, log: &Rc<RefCell<Log>>, r: DesktopRequest) {
    log.borrow_mut().pending.push(r);
    frames(h, 3);
}

#[test]
fn desktop_host_sees_the_state_and_runs_requests_in_both_workflows() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let (mut h, log) = harness(workflow);
        frames(&mut h, 3);
        let s = last(&log);
        assert!(!s.playing && !s.busy && s.progress.is_none(), "{workflow:?}: {s:?}");
        assert_eq!(s.project, h.state().session.project.name);
        assert!(s.sequence.is_some(), "{workflow:?}: the demo has a sequence");
        assert!(s.duration_us > 0 && s.frame_us > 0, "{s:?}");
        // a media key: play / pause
        request(&mut h, &log, DesktopRequest::Command("playback.toggle".into(), json!({"monitor": "program"})));
        assert!(last(&log).playing, "{workflow:?}: playing after playback.toggle");
        request(&mut h, &log, DesktopRequest::Command("playback.stop".into(), json!({})));
        assert!(!last(&log).playing, "{workflow:?}: stopped");
        // step a frame (MPRIS Next)
        let before = h.state().session.playhead();
        request(&mut h, &log, DesktopRequest::Command("playhead.stepForward".into(), json!({})));
        assert!(h.state().session.playhead() > before, "{workflow:?}: stepped forward");
        assert!(last(&log).position_us > 0);
        // a raise without files changes nothing in the project
        let name = h.state().session.project.name.clone();
        request(&mut h, &log, DesktopRequest::Raise);
        assert_eq!(h.state().session.project.name, name);
    }
}

#[test]
fn hostile_desktop_requests_never_panic() {
    let (mut h, log) = harness(Workflow::Resolve);
    frames(&mut h, 2);
    let long = "x".repeat(10_000);
    request(&mut h, &log, DesktopRequest::Open(vec![]));
    request(&mut h, &log, DesktopRequest::Open(vec!["".into(), " ".into(), "\0".into(), long.clone(), format!("{long}.FCPROJ")]));
    request(&mut h, &log, DesktopRequest::Open(vec!["/nonexistent/folder/film.fcproj".into(), "/nonexistent/clip.mov".into()]));
    request(&mut h, &log, DesktopRequest::Command("no.such.command".into(), json!({"x": [1, 2, 3]})));
    request(&mut h, &log, DesktopRequest::Command("".into(), serde_json::Value::Null));
    // the demo project is still open
    assert!(h.state().session.active_sequence().is_some());
}

#[test]
fn a_second_launch_opens_its_project() {
    let dir = std::env::temp_dir().join(format!("filmcraft-desktop-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Other Film.fcproj").to_string_lossy().into_owned();
    // write a project to open
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    let (mut h, log) = harness(Workflow::Premiere);
    frames(&mut h, 2);
    request(&mut h, &log, DesktopRequest::Open(vec![path.clone()]));
    assert_eq!(h.state().session.path.as_deref(), Some(path.as_str()));
    assert_eq!(last(&log).project, "Other Film");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_launch_never_replaces_unsaved_changes() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let dir = std::env::temp_dir().join(format!("filmcraft-desktop-dirty-{}-{workflow:?}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Other.fcproj").to_string_lossy().into_owned();
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s.execute("file.saveAs", json!({"path": path})).unwrap();
        let (mut h, log) = harness(workflow);
        frames(&mut h, 2);
        h.state_mut().session.execute("file.newBin", json!({"name": "Unsaved"})).unwrap();
        assert!(h.state().session.is_dirty());
        let before = h.state().session.path.clone();
        request(&mut h, &log, DesktopRequest::Open(vec![path.clone()]));
        assert_eq!(h.state().session.path, before, "{workflow:?}: the unsaved project stays open");
        assert!(h.state().ui.status.contains("Other.fcproj"), "{workflow:?}: {}", h.state().ui.status);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// L2.8: the error window's "Report an Issue…" shows the crash log and opens the tracker
/// (nothing is uploaded). Both workflows.
#[test]
fn error_window_reports_an_issue_with_the_crash_log() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let (mut h, _log) = harness(workflow);
        frames(&mut h, 2);
        let opened: Rc<RefCell<Vec<(String, bool)>>> = Rc::default();
        let urls: Rc<RefCell<Vec<String>>> = Rc::default();
        let (o, u) = (opened.clone(), urls.clone());
        h.state_mut().hooks.open_path = Some(Box::new(move |p: &str, reveal: bool| {
            o.borrow_mut().push((p.to_string(), reveal));
            Ok(())
        }));
        h.state_mut().hooks.open_url = Some(Box::new(move |url: &str| {
            u.borrow_mut().push(url.to_string());
            Ok(())
        }));
        h.state_mut().ui_error = Some("test error".into());
        // a new window is laid out invisibly on its first frame: clicks land on it from the second
        h.step();
        h.step();
        h.get_by_label("Report an Issue…").click();
        h.step();
        h.step();
        assert!(urls.borrow().iter().any(|u| u.contains("issues")), "{workflow:?}: {:?}", urls.borrow());
        // the crash log is revealed when there is one (the desktop app installs its folder)
        if filmcraft_ui_egui::crash::log_path().is_some() {
            assert!(opened.borrow().iter().any(|(p, reveal)| *reveal && p.contains("crash-")), "{:?}", opened.borrow());
        }
        assert!(h.state().ui_error.is_some(), "the window stays until Continue");
    }
}

#[test]
fn shutdown_reaches_the_host() {
    struct Closing(Rc<RefCell<bool>>);
    impl DesktopHost for Closing {
        fn update(&mut self, _: &egui::Context, _: &DesktopState, _: &[FinishedJob]) {}
        fn take_requests(&mut self) -> Vec<DesktopRequest> {
            Vec::new()
        }
        fn shutdown(&mut self) {
            *self.0.borrow_mut() = true;
        }
    }
    let closed = Rc::new(RefCell::new(false));
    let mut app = FilmcraftApp::new(Session::default());
    app.hooks.desktop = Some(Box::new(Closing(closed.clone())));
    eframe::App::on_exit(&mut app);
    assert!(*closed.borrow());
}
