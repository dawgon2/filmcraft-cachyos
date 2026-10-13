//! L1.3: the main window's size and maximised state (`crate::window_state`).

use crate::media_test_util::tmp_dir;
use crate::window_state::{DEFAULT_SIZE, MAX_SIDE, MIN_SIZE, WindowState, WindowTracker};

#[test]
fn saved_state_round_trips_through_the_state_dir() {
    let dir = tmp_dir("window-state");
    let path = WindowState::path(&dir.join("nested"));
    let s = WindowState { inner_size: [1280.0, 720.0], maximized: true };
    s.save(&path).unwrap();
    assert_eq!(WindowState::load(&path), Some(s));
    assert!(!path.with_extension("json.tmp").exists());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"innerSize\"") && text.contains("\"maximized\":true"), "{text}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_or_broken_files_mean_the_default_size() {
    let dir = tmp_dir("window-state-broken");
    let path = WindowState::path(&dir);
    assert_eq!(WindowState::load(&path), None);
    for bad in ["", "{", "null", "[]", "{\"innerSize\":[1,2,3],\"maximized\":false}", "{\"innerSize\":\"big\"}", "\u{0}\u{1}"] {
        std::fs::write(&path, bad).unwrap();
        assert_eq!(WindowState::load(&path), None, "{bad:?}");
    }
    // a file far larger than ours is not read at all
    std::fs::write(&path, " ".repeat(1 << 20)).unwrap();
    assert_eq!(WindowState::load(&path), None);
    // a directory where the file should be
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(WindowState::load(&path), None);
    assert!(WindowState::default().save(&path).is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn hostile_sizes_are_clamped_or_refused() {
    let dir = tmp_dir("window-state-hostile");
    let path = WindowState::path(&dir);
    std::fs::write(&path, "{\"innerSize\":[1e30,-5],\"maximized\":false}").unwrap();
    assert_eq!(WindowState::load(&path), Some(WindowState { inner_size: [MAX_SIDE, MIN_SIZE[1]], maximized: false }));
    std::fs::write(&path, "{\"innerSize\":[0,0],\"maximized\":true}").unwrap();
    assert_eq!(WindowState::load(&path), Some(WindowState { inner_size: MIN_SIZE, maximized: true }));
    assert_eq!(WindowState { inner_size: [f32::NAN, 600.0], maximized: false }.sanitized(), None);
    assert_eq!(WindowState { inner_size: [1000.0, f32::INFINITY], maximized: false }.sanitized(), None);
    // saving a non-finite size writes the default instead of `null`s
    WindowState { inner_size: [f32::NAN, f32::NEG_INFINITY], maximized: true }.save(&path).unwrap();
    assert_eq!(WindowState::load(&path), Some(WindowState::default()));
    assert_eq!(WindowState::default().inner_size, DEFAULT_SIZE);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_tracker_keeps_the_normal_size_while_maximised() {
    let mut t = WindowTracker::new(WindowState::default());
    t.observe(Some([1200.0, 800.0]), Some(false), Some(false), Some(false));
    assert_eq!(t.state(), WindowState { inner_size: [1200.0, 800.0], maximized: false });
    // maximising reports the screen-sized rect: the normal size stays
    t.observe(Some([2560.0, 1400.0]), Some(true), Some(false), Some(false));
    assert_eq!(t.state(), WindowState { inner_size: [1200.0, 800.0], maximized: true });
    // full screen and minimised frames change nothing
    t.observe(Some([2560.0, 1440.0]), Some(false), Some(true), Some(false));
    t.observe(Some([1.0, 1.0]), Some(false), Some(false), Some(true));
    assert_eq!(t.state(), WindowState { inner_size: [1200.0, 800.0], maximized: true });
    // back to normal, resized
    t.observe(Some([1000.0, 700.0]), Some(false), None, None);
    assert_eq!(t.state(), WindowState { inner_size: [1000.0, 700.0], maximized: false });
}

#[test]
fn the_tracker_ignores_unknown_and_hostile_frames() {
    let start = WindowState { inner_size: [1300.0, 900.0], maximized: true };
    let mut t = WindowTracker::new(start);
    // no rect yet (first frames): nothing changes
    t.observe(None, None, None, None);
    t.observe(None, Some(false), None, None);
    assert_eq!(t.state(), WindowState { inner_size: [1300.0, 900.0], maximized: false });
    // a NaN rect is ignored; a tiny one is clamped to the minimum
    t.observe(Some([f32::NAN, 10.0]), Some(false), None, None);
    assert_eq!(t.state().inner_size, [1300.0, 900.0]);
    t.observe(Some([10.0, 10.0]), Some(false), None, None);
    assert_eq!(t.state().inner_size, MIN_SIZE);
    // maximised state unknown: the size is followed only while not known to be maximised
    t.observe(Some([1400.0, 950.0]), None, None, None);
    assert_eq!(t.state(), WindowState { inner_size: [1400.0, 950.0], maximized: false });
    let mut t = WindowTracker::new(start);
    t.observe(Some([2560.0, 1400.0]), None, None, None);
    assert_eq!(t.state(), start);
}

#[test]
fn the_maximised_size_reported_a_frame_early_is_not_the_normal_size() {
    let mut t = WindowTracker::new(WindowState { inner_size: [1200.0, 800.0], maximized: false });
    t.observe(Some([1200.0, 800.0]), Some(false), None, None);
    // the compositor resizes first and says "maximised" a frame later
    t.observe(Some([2560.0, 1400.0]), Some(false), None, None);
    t.observe(Some([2560.0, 1400.0]), Some(true), None, None);
    assert_eq!(t.state(), WindowState { inner_size: [1200.0, 800.0], maximized: true });
    // a resize seen for one frame just before maximising is kept
    let mut t = WindowTracker::new(WindowState::default());
    t.observe(Some([1000.0, 700.0]), Some(false), None, None);
    t.observe(Some([2560.0, 1400.0]), Some(true), None, None);
    assert_eq!(t.state(), WindowState { inner_size: [1000.0, 700.0], maximized: true });
}
