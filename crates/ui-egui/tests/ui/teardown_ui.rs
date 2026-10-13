//! A closed app leaves nothing running: its frame workers and background jobs (the render cache)
//! stop with it instead of rendering on for seconds.
//!
//! Fork, 2026-10-13: every test in this binary opens the demo project and drops its app at the
//! end, and the dropped apps' work kept running (each holding its frames and media), so running
//! the binary on one thread grew by about 1 GB per test until it was killed for memory.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::json;

#[test]
fn dropping_the_app_stops_its_workers_and_frees_its_media() {
    let mut session = Session::default();
    session.execute("file.openDemoProject", json!({})).expect("demo project");
    let app = FilmcraftApp::new(session);
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
    // long enough for the monitors and thumbnails to ask for frames and for the render cache to
    // start on the demo's effects (both render procedural demo frames: slow, on every core)
    let busy_until = Instant::now() + Duration::from_secs(10);
    for _ in 0..60 {
        h.step();
        if !h.state().session.background.is_idle() || Instant::now() > busy_until {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let media = std::sync::Arc::downgrade(&h.state().session.media);
    let t = Instant::now();
    drop(h);
    let took = t.elapsed();
    // every frame worker and background job holds the media pool while it runs. Closing waits a
    // bounded time for the frame in hand; on a loaded machine (the parallel ci run) that frame can
    // still be finishing, so allow it a moment more, far less than the 5–15 s the work used to run on
    let settle = Instant::now() + Duration::from_secs(3);
    while media.strong_count() > 0 && Instant::now() < settle {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(media.strong_count(), 0, "work of the dropped app is still running");
    assert!(took < Duration::from_secs(6), "closing waited {took:?}");
}
