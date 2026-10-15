//! Review 2026-10-15 (U0.2.5 owed): once the process exits ([`filmcraft_platform::quiesce`]), a
//! decoder still running on another thread submits no new picture to the driver (the drivers'
//! exit handlers must not run under a picture in flight); the submit is refused, and nothing
//! crashes. Its own test binary: exiting is process-wide. Skips without ffmpeg (fixture
//! generator) or without a VA-API driver.
#![cfg(target_os = "linux")]

mod common;

use common::*;

#[test]
fn no_picture_is_submitted_after_quiesce() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let Some(path) = FIXTURES.iter().find_map(|(name, _)| named(&ff, name)) else { return };
    let s = read_stream(&path);
    let mut hw = match filmcraft_platform::vaapi_factory(&s.entry) {
        Some(Ok(d)) if d.name().starts_with("VA-API") => d,
        _ => {
            eprintln!("SKIPPED: no VA-API decoder for this stream on this machine");
            return;
        }
    };
    let n = s.samples.len().min(12);
    assert!(n > 4, "fixture too short");
    for (data, pts) in &s.samples[..2] {
        assert!(hw.decode(data, *pts).is_ok(), "before exiting");
    }
    filmcraft_platform::quiesce();
    let refused = filmcraft_platform::va::work_refused_while_exiting();
    // (the decoder may recover on its own, e.g. in software: only no crash and no VA submit count)
    for (data, pts) in &s.samples[2..n] {
        let _ = hw.decode(data, *pts);
    }
    let _ = hw.flush();
    assert!(filmcraft_platform::va::work_refused_while_exiting() > refused, "pictures were submitted to the driver while exiting");
}
