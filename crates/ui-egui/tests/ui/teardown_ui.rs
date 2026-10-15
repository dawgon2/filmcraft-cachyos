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
    // keep a handle on the frame server to count what its workers do after the close (it only
    // looks: stopping them is the app's drop's job)
    let frames = std::sync::Arc::clone(&h.state().frames);
    let workers = frames.workers() as u64;
    let jobs_before = frames.stats().jobs;
    let t = Instant::now();
    drop(h);
    let took = t.elapsed();
    assert!(took < Duration::from_secs(6), "closing waited {took:?}");
    // How long the frames in hand take to notice the cancellation depends on the machine: in the
    // parallel ci run a hundred other tests' workers share the cores and the last frame can take
    // many seconds (the 3 s bound this test had failed there, 2026-10-15). So the deterministic
    // check is what ran, not how long: only the frames in hand at the close may finish, nothing
    // queued starts; the time limit only catches work that never stops.
    let limit = Instant::now() + Duration::from_secs(120);
    while frames.live_workers() > 0 && Instant::now() < limit {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(frames.live_workers(), 0, "frame workers still running 2 min after the close; threads alive: {}", live_threads());
    let ran = frames.stats().jobs.saturating_sub(jobs_before);
    // + 1: a worker can pick up one more job between reading the count and the close
    assert!(ran <= workers + 1, "{ran} frame jobs ran after the close ({workers} workers): queued work was not dropped");
    drop(frames);
    // every frame worker and background job (the render cache) holds the media pool while it runs
    while media.strong_count() > 0 && Instant::now() < limit {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(media.strong_count(), 0, "work of the dropped app is still running 2 min after the close; threads alive: {}", live_threads());
}

/// The names of this process's threads (Linux; empty elsewhere), so a failure says whose work
/// outlived the app.
fn live_threads() -> String {
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else { return String::new() };
    let mut names: Vec<String> = dir.flatten().filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok()).map(|n| n.trim().to_owned()).collect();
    names.sort();
    names.join(", ")
}
