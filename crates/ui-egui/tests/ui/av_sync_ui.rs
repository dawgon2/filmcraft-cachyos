//! AUDIO-PLAN A1.4 (fork): audio and picture stay within one frame of each other, measured from
//! the clock, not by ear: after 100 random seeks while playing, and at 2× (tape-style varispeed).
//!
//! The sequence plays a WAV whose samples say where they are: the left channel is the frame's
//! own index (scaled), the right a constant marker, so left / right gives the position whatever
//! gain the mix applied. A stand-in device plays the app's real play-ahead mix in 512-frame
//! periods, one UI step's worth (50 ms) each time the clock is read, and remembers the last
//! sample it played; the playhead the clock puts on screen must match that sample's position.
//! The playhead shows whole frames, so the picture trails the sound by 0 to just under one frame
//! (a sawtooth that never accumulates); compared as whole frames, the frame on screen must be the
//! one the sound is in: measured 0 frames in every reading (AUDIO-PLAN allows one). The stand-in
//! has no output latency (the clock's latency compensation is tested in the desktop app's audio
//! module); the probe checks the start, seek and 2× mapping from played frames to the picture.
//! (4× plays no sound by design: the clock alone drives it there.)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::Workflow;
use filmcraft_time::Tick;
use filmcraft_ui_egui::{AudioOut, FilmcraftApp};
use serde_json::json;

const SR: u32 = 48_000;
const SECONDS: u32 = 20;
/// Left = frame index × SCALE (20 s = 960 000 frames stays under 0.46, exact in f32).
const SCALE: f32 = 1.0 / 2_097_152.0;
const MARKER: f32 = 0.5;
const PERIOD: usize = 512;
const STEP_S: f32 = 0.05;

type Fill = Box<dyn FnMut(&mut [f32], usize) + Send>;

/// The position (file frame) the last played sample says it is at, and the speed it played at.
#[derive(Default)]
struct Heard {
    last: Option<f64>,
    silent_periods: u64,
}

struct ProbeOut {
    fill: Mutex<Option<Fill>>,
    played: AtomicU64,
    heard: Arc<Mutex<Heard>>,
    /// Frames the clock reports ahead of what was played (0; the self-check skews it to prove
    /// the probe sees a clock that's off).
    skew: u64,
}

impl AudioOut for ProbeOut {
    fn start(&mut self, fill: Fill) -> Result<u32, String> {
        *self.fill.lock().unwrap() = Some(fill);
        self.played.store(0, Ordering::SeqCst);
        *self.heard.lock().unwrap() = Heard::default();
        Ok(SR)
    }
    fn stop(&mut self) {
        *self.fill.lock().unwrap() = None;
    }
    fn sample_rate(&self) -> u32 {
        SR
    }
    /// Plays one UI step's worth of periods (the mixer runs ahead on its own thread; give it a
    /// moment so this stand-in, which plays faster than real time, doesn't outrun it).
    fn played_frames(&self) -> Option<u64> {
        let mut fill = self.fill.lock().unwrap();
        let fill = fill.as_mut()?;
        let periods = (SR as f32 * STEP_S) as usize / PERIOD;
        let mut buf = vec![0.0f32; PERIOD * 2];
        for _ in 0..periods {
            std::thread::sleep(Duration::from_micros(500));
            fill(&mut buf, 2);
            self.played.fetch_add(PERIOD as u64, Ordering::SeqCst);
            let mut heard = self.heard.lock().unwrap();
            match buf.get(buf.len() - 2..) {
                Some([l, r]) if *r > MARKER * 0.25 => heard.last = Some(f64::from(*l / *r * MARKER / SCALE)),
                _ => heard.silent_periods += 1,
            }
        }
        Some(self.played.load(Ordering::SeqCst) + self.skew)
    }
}

fn session(workflow: Workflow) -> Session {
    let mut s = Session::default();
    s.prefs.workflow = workflow;
    s.execute("file.newSequence", json!({"name": "Sync", "audio": 1, "video": 1})).unwrap();
    let inter: Vec<f32> = (0..SR * SECONDS).flat_map(|i| [i as f32 * SCALE, MARKER]).collect();
    let bytes: Arc<[u8]> = filmcraft_engine::previews::write_wav_f32(&inter, SR).into();
    let item = filmcraft_engine::commands::import_bytes(&mut s, "/position.wav", bytes, None).unwrap();
    s.execute("timeline.place", json!({"item": item.0, "audioTrack": "A1", "seconds": 0.0})).unwrap();
    s
}

/// Steps until the audio clock runs (the preroll waits for the first pictures), then `measure`
/// steps; returns the largest gap between the playhead and the sound heard, in video frames.
fn drift_after(h: &mut Harness<'static, FilmcraftApp>, heard: &Arc<Mutex<Heard>>, speed: f64, measure: usize) -> f64 {
    for _ in 0..200 {
        if h.state().playback.audio_clock {
            break;
        }
        h.step();
    }
    assert!(h.state().playback.audio_clock, "the device clock drives playback");
    let fps = h.state().session.sequence_rate().as_f64();
    let mut worst = 0.0f64;
    let mut readings = 0;
    for _ in 0..measure {
        h.step();
        let Some(at) = heard.lock().unwrap().last else { continue };
        readings += 1;
        // the playhead shows the frame after the last sample played; at 2× each device frame is
        // two file frames
        // compared as whole frames: the frame the sound is in (floor) against the frame on screen
        // (the playhead is snapped to frames; round only absorbs float error)
        let sound = ((at + speed) / f64::from(SR) * fps + 1e-6).floor();
        let shown = (h.state().session.playhead().seconds() * fps).round();
        worst = worst.max((shown - sound).abs());
    }
    assert!(readings > 0, "the device played the position-coded sound");
    eprintln!("A/V drift at {speed}×: {worst:.3} frames over {readings} readings");
    worst
}

fn harness(workflow: Workflow, skew: u64) -> (Harness<'static, FilmcraftApp>, Arc<Mutex<Heard>>) {
    let heard = Arc::new(Mutex::new(Heard::default()));
    let mut app = FilmcraftApp::new(session(workflow));
    app.audio = Some(Box::new(ProbeOut { fill: Mutex::new(None), played: AtomicU64::new(0), heard: heard.clone(), skew }));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(STEP_S).with_max_steps(100_000).build_eframe(move |_cc| app);
    h.step();
    (h, heard)
}

/// The probe itself: a clock two frames ahead of the sound is seen as two frames off.
#[test]
fn the_sync_probe_sees_a_clock_that_is_off() {
    let (mut h, heard) = harness(Workflow::Premiere, 2 * u64::from(SR) / 24 + 100);
    h.state_mut().session.set_playhead(Tick::from_seconds_f64(2.0));
    h.state_mut().play(1.0);
    let off = drift_after(&mut h, &heard, 1.0, 10);
    assert!(off >= 2.0, "a skewed clock must show: {off}");
    h.state_mut().stop();
}

/// (`realtime` in the name: `cargo xtask ci` runs it alone after the parallel tests, because the
/// play-ahead mixer thread falls behind under a fully loaded machine: 3 frames once in a full run.)
#[test]
fn sound_and_picture_stay_within_a_frame_after_seeks_and_at_2x_realtime() {
    for workflow in [Workflow::Premiere, Workflow::Resolve] {
        let (mut h, heard) = harness(workflow, 0);

        let mut worst = 0.0f64;
        // 100 seeks to seeded random places while playing (a click in the ruler: playhead, then
        // play on from there)
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..100 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed % u64::from((SECONDS - 2) * 1000)) as f64 / 1000.0;
            h.state_mut().session.set_playhead(Tick::from_seconds_f64(at));
            h.state_mut().play(1.0);
            worst = worst.max(drift_after(&mut h, &heard, 1.0, 3));
        }
        assert!(worst == 0.0, "{workflow:?}: after 100 seeks the sound was {worst:.2} frames off the picture");

        // a long stretch at 1×: no drift builds up
        h.state_mut().session.set_playhead(Tick::from_seconds_f64(3.0));
        h.state_mut().play(1.0);
        let long = drift_after(&mut h, &heard, 1.0, 80);
        assert!(long == 0.0, "{workflow:?}: over 4 s at 1× the sound was {long:.2} frames off the picture");

        // 2× forward (L twice), tape-style sound
        h.state_mut().session.set_playhead(Tick::from_seconds_f64(1.0));
        h.state_mut().play(2.0);
        let at_2x = drift_after(&mut h, &heard, 2.0, 40);
        assert!(at_2x == 0.0, "{workflow:?}: at 2× the sound was {at_2x:.2} frames off the picture");
        assert_eq!(heard.lock().unwrap().silent_periods, 0, "{workflow:?}: no period played silence");
        h.state_mut().stop();
    }
}
