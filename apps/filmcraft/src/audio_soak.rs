//! AUDIO-PLAN A1 acceptance soak (fork A1.5): the desktop app's real audio output ([`CpalOut`])
//! plays a 20-track timeline for a long stretch with seeks and 2× shuttles, while the audio device
//! is taken away and given back, and reports underruns, A/V drift and recovery. `#[ignore]`: it
//! needs an audio server. Run it with `tools/fork/a1-audio-soak.sh` (Linux, PipeWire): that script
//! creates a null sink, points the stream at it (`PULSE_SINK`) so nothing is heard, removes and
//! restores it and switches the card's profile off and back during the run, and cleans up.
//!
//! The sound says where it is: track A1 carries a WAV whose left channel encodes each frame's
//! position within a 20 s loop and whose right channel is a constant marker (left / right cancels
//! any gain; everything plays at −90 dB, inaudible should the stream land on a real output for a
//! moment), the other 19 tracks overlapping clips of silence (decoded and mixed like any clip, and
//! exactly zero, so the code stays exact). A tap on the device callback records the position of
//! the last frame written; with the clock's frames heard it gives the position being heard, which
//! must be on the frame the playhead shows.
//!
//! Environment: `FC_SOAK_SECONDS` (default 60), `FC_SOAK_DEVICE` (the output to play on, as
//! Settings ▸ Audio Hardware lists it: the null sink's description), `FC_SOAK_NULL_SINK` (sink
//! name; with it the sink is removed and restored mid-run), `FC_SOAK_CARD` and `FC_SOAK_PROFILE` (switched off and back),
//! `FC_SOAK_REPORT` (a JSON summary is written there), `FC_SOAK_VOICE_MODEL` (a DeepFilterNet3
//! checkpoint: Voice Isolation runs on `FC_SOAK_VOICE_TRACKS` silence tracks, default 4; needs a
//! scratch `FILMCRAFT_DATA_DIR`, fork A2.6).

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_engine::settings::AudioHardwarePrefs;
use filmcraft_time::Tick;
use filmcraft_ui_egui::{AudioDevices, AudioOut, FilmcraftApp};
use serde_json::json;

use crate::audio::CpalOut;

const SR: u32 = 48_000;
const LOOP_S: u32 = 20;
const SCALE: f32 = 1.0 / 2_097_152.0;
const MARKER: f32 = 0.5;
/// −90 dB.
const GAIN: f32 = 3.162_277_7e-5;
const TRACKS: usize = 20;
const STEP_S: f32 = 1.0 / 60.0;

/// What the device callback wrote last: the position code of its last frame (file frame within
/// the loop, as f64 bits; `u64::MAX`: none) and the frames written so far.
#[derive(Default)]
struct Tap {
    pos: AtomicU64,
    written: AtomicU64,
}

/// [`CpalOut`] with a tap on its callback.
struct Tapped {
    out: CpalOut,
    tap: Arc<Tap>,
}

impl AudioOut for Tapped {
    fn start(&mut self, mut fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        let tap = self.tap.clone();
        tap.pos.store(u64::MAX, Ordering::SeqCst);
        tap.written.store(0, Ordering::SeqCst);
        self.out.start(Box::new(move |buf: &mut [f32], ch: usize| {
            fill(buf, ch);
            let ch = ch.max(1);
            tap.written.fetch_add((buf.len() / ch) as u64, Ordering::SeqCst);
            let last = buf.len().saturating_sub(ch);
            if let (Some(l), Some(r)) = (buf.get(last), buf.get(last + 1))
                && *r > MARKER * GAIN * 0.25
            {
                tap.pos.store(f64::from(*l / *r * MARKER / SCALE).to_bits(), Ordering::SeqCst);
            } else {
                tap.pos.store(u64::MAX, Ordering::SeqCst);
            }
        }))
    }
    fn stop(&mut self) {
        self.out.stop();
    }
    fn sample_rate(&self) -> u32 {
        self.out.sample_rate()
    }
    fn channels(&self) -> usize {
        self.out.channels()
    }
    fn played_frames(&self) -> Option<u64> {
        self.out.played_frames()
    }
    fn recover(&mut self) {
        self.out.recover();
    }
    fn devices(&self) -> AudioDevices {
        self.out.devices()
    }
    fn configure(&mut self, hw: &AudioHardwarePrefs, document_rate: Option<u32>) {
        self.out.configure(hw, document_rate);
    }
    fn note(&self) -> Option<String> {
        self.out.note()
    }
}

fn session(seconds: u32, device: &str) -> Session {
    let mut s = Session::default();
    s.prefs.audio_hardware.default_output = device.to_string();
    // 25 fps: the 20 s code loop is a whole number of frames, so every loop's clip starts exactly on
    // a multiple of 20 s
    s.execute("file.newSequence", json!({"name": "Soak", "audio": TRACKS, "video": 1, "fps": 25.0, "width": 640, "height": 360})).unwrap();
    let code: Vec<f32> = (0..SR * LOOP_S).flat_map(|i| [i as f32 * SCALE * GAIN, MARKER * GAIN]).collect();
    let code = filmcraft_engine::commands::import_bytes(&mut s, "/position.wav", filmcraft_engine::previews::write_wav_f32(&code, SR).into(), None).unwrap();
    let silence = vec![0.0f32; (SR * 30 * 2) as usize];
    let silence =
        filmcraft_engine::commands::import_bytes(&mut s, "/silence.wav", filmcraft_engine::previews::write_wav_f32(&silence, SR).into(), None).unwrap();
    let length = seconds + 60;
    for k in 0..length / LOOP_S + 1 {
        s.execute("timeline.place", json!({"item": code.0, "audioTrack": "A1", "seconds": f64::from(k * LOOP_S)})).unwrap();
    }
    // overlapping clips: each track's 30 s clips start at a different offset
    for t in 2..=TRACKS {
        let offset = (t as f64 * 1.37) % 30.0;
        let mut at = offset;
        while at < f64::from(length) {
            s.execute("timeline.place", json!({"item": silence.0, "audioTrack": format!("A{t}"), "seconds": at})).unwrap();
            at += 30.0;
        }
    }
    // Fork A2.6: Voice Isolation on the first `FC_SOAK_VOICE_TRACKS` silence tracks (default 4),
    // loaded from `FC_SOAK_VOICE_MODEL`. Silence through the model stays exactly zero, so the
    // position code still decodes; the network runs on every frame like on a voice clip.
    if let Some(model) = std::env::var_os("FC_SOAK_VOICE_MODEL") {
        assert!(std::env::var_os("FILMCRAFT_DATA_DIR").is_some(), "FC_SOAK_VOICE_MODEL needs a scratch FILMCRAFT_DATA_DIR (the model is installed there)");
        let st = s.execute("audio.voiceIsolate.installFile", json!({"path": model.to_string_lossy()})).unwrap();
        assert_eq!(st["loaded"], true, "{st}");
        let n: usize = std::env::var("FC_SOAK_VOICE_TRACKS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
        let clips: Vec<u64> = s.active_sequence().unwrap().audio_tracks.iter().skip(1).take(n).flat_map(|t| t.items.iter().map(|i| i.id.0)).collect();
        s.execute("effects.apply", json!({"clips": clips, "effect": "voice_isolate"})).unwrap();
        eprintln!("soak: Voice Isolation on {n} tracks ({} clips)", clips.len());
    }
    s
}

fn pactl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("pactl").args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).into_owned()) } else { Err(String::from_utf8_lossy(&out.stderr).into_owned()) }
}

/// The module id of the null sink named `sink`.
fn null_sink_module(sink: &str) -> Option<String> {
    let list = pactl(&["list", "short", "modules"]).ok()?;
    list.lines()
        .find(|l| l.contains("module-null-sink") && l.contains(&format!("sink_name={sink}")))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
}

/// A device event in the run: what, when (s from the start).
#[derive(Debug)]
enum Event {
    SinkGone,
    SinkBack,
    CardOff,
    CardBack,
}

#[derive(Default)]
struct Totals {
    underruns: u64,
    last_seen: u64,
    worst_drift: f64,
    readings: u64,
    no_reading: u64,
    seeks: u32,
    shuttles: u32,
    clock_lost: u32,
}

impl Totals {
    /// Underruns counted across plays (each play's mixer starts its own counters).
    fn add_underruns(&mut self, now: u64) {
        self.underruns += if now < self.last_seen { now } else { now - self.last_seen };
        self.last_seen = now;
    }
}

#[test]
#[ignore]
fn audio_soak() {
    let seconds: u32 = std::env::var("FC_SOAK_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let device = std::env::var("FC_SOAK_DEVICE").unwrap_or_default();
    let outputs = CpalOut::new().devices().outputs;
    eprintln!("soak: outputs {outputs:?}, playing on {device:?}");
    assert!(device.is_empty() || outputs.contains(&device), "the soak's output {device:?} exists");
    let sink = std::env::var("FC_SOAK_NULL_SINK").ok().filter(|s| !s.is_empty());
    let card = std::env::var("FC_SOAK_CARD").ok().filter(|s| !s.is_empty());
    let profile = std::env::var("FC_SOAK_PROFILE").ok().filter(|s| !s.is_empty());
    let third = f64::from(seconds) / 3.0;
    let mut events: Vec<(f64, Event)> = Vec::new();
    if sink.is_some() {
        events.push((third, Event::SinkGone));
        events.push((third + 3.0, Event::SinkBack));
    }
    // (after SinkGone the stream falls back to the default output, on this card, and stays there
    // when the sink comes back: switching the card off then takes the stream's device away)
    if card.is_some() && profile.is_some() {
        events.push((third + 6.0, Event::CardOff));
        events.push((third + 11.0, Event::CardBack));
    }
    let mut recovered: Vec<serde_json::Value> = Vec::new();

    let tap = Arc::new(Tap::default());
    let mut app = FilmcraftApp::new(session(seconds, &device));
    app.audio = Some(Box::new(Tapped { out: CpalOut::new(), tap: tap.clone() }));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(STEP_S).with_max_steps(u64::MAX).build_eframe(move |_cc| app);
    h.step();
    h.state_mut().session.set_playhead(Tick::ZERO);
    h.state_mut().play(1.0);

    let fps = h.state().session.sequence_rate().as_f64();
    let started = Instant::now();
    let mut totals = Totals::default();
    let mut quiet_until = 2.0; // no measuring while playback (re)starts
    let mut next_seek = 23.0;
    let mut next_shuttle = 37.0;
    let mut speed = 1.0;
    let mut back_to_1x = f64::MAX;
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut step = 0u64;
    let mut pending_events = events;
    // event, when, last clock reading, frames heard since (across restarts of playback)
    let mut watches: Vec<(String, f64, u64, u64)> = Vec::new();
    let trace = std::env::var_os("FC_SOAK_TRACE").is_some();
    let mut next_trace = 0.0;
    loop {
        let t = started.elapsed().as_secs_f64();
        if t >= f64::from(seconds) {
            break;
        }
        // keep the UI steps on the wall clock (the device runs in real time)
        step += 1;
        let due = Duration::from_secs_f64(step as f64 * f64::from(STEP_S));
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
        // device events
        let due_events: Vec<Event> = {
            let (now, later): (Vec<_>, Vec<_>) = pending_events.drain(..).partition(|(at, _)| *at <= t);
            pending_events = later;
            now.into_iter().map(|(_, e)| e).collect()
        };
        for e in due_events {
            let done = match (&e, &sink, &card, &profile) {
                (Event::SinkGone, Some(sink), _, _) => {
                    null_sink_module(sink).ok_or("no null sink module".to_string()).and_then(|id| pactl(&["unload-module", &id]))
                }
                (Event::SinkBack, Some(sink), _, _) => {
                    pactl(&["load-module", "module-null-sink", &format!("sink_name={sink}"), &format!("sink_properties=device.description={sink}")])
                }
                (Event::CardOff, _, Some(card), Some(_)) => pactl(&["set-card-profile", card, "off"]),
                (Event::CardBack, _, Some(card), Some(profile)) => pactl(&["set-card-profile", card, profile]),
                _ => Ok(String::new()),
            };
            eprintln!("soak {t:7.2} s: {e:?} {}", if done.is_ok() { "done" } else { "FAILED" });
            let heard = h.state().audio.as_ref().and_then(|a| a.played_frames()).unwrap_or(0);
            watches.push((format!("{e:?}"), t, heard, 0));
            quiet_until = t + 2.0;
        }
        // seeks and shuttles, as an editor would
        if t >= next_seek {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let to = (seed % u64::from(seconds.max(10) * 1000)) as f64 / 1000.0;
            h.state_mut().session.set_playhead(Tick::from_seconds_f64(to));
            h.state_mut().play(1.0);
            speed = 1.0;
            totals.seeks += 1;
            next_seek = t + 23.0;
            quiet_until = t + 2.0;
        }
        if t >= next_shuttle {
            h.state_mut().play(2.0);
            speed = 2.0;
            totals.shuttles += 1;
            back_to_1x = t + 4.0;
            next_shuttle = t + 37.0;
            quiet_until = t + 2.0;
        }
        if t >= back_to_1x {
            h.state_mut().play(1.0);
            speed = 1.0;
            back_to_1x = f64::MAX;
            quiet_until = t + 2.0;
        }
        // the clock as the step will read it (first thing in the frame; the step then draws for
        // a while, during which the device plays on)
        // (re-read until no callback ran in between: position, clock and frames written belong
        // to the same callback)
        let mut snapshot = (u64::MAX, None, 0);
        for _ in 0..4 {
            let before = tap.written.load(Ordering::SeqCst);
            snapshot = (tap.pos.load(Ordering::SeqCst), h.state().audio.as_ref().and_then(|a| a.played_frames()), tap.written.load(Ordering::SeqCst));
            if snapshot.2 == before {
                break;
            }
        }
        h.step();
        let app = h.state();
        if trace && !watches.is_empty() {
            let heard = app.audio.as_ref().and_then(|a| a.played_frames());
            eprintln!("trace {t:8.3} event-watch heard {heard:?} written {} clock {}", tap.written.load(Ordering::SeqCst), app.playback.audio_clock);
        }
        let before = totals.underruns;
        totals.add_underruns(app.playback.audio_stats.underruns.load(Ordering::Relaxed));
        if trace && totals.underruns > before {
            eprintln!("trace {t:8.3} underruns +{} (total {}) speed {speed}", totals.underruns - before, totals.underruns);
        }
        if !app.playback.audio_clock {
            totals.clock_lost += 1;
        }
        // a device event: the clock must keep (or be back to) counting within 3 s, without a
        // restart of playback
        let heard_now = app.audio.as_ref().and_then(|a| a.played_frames()).unwrap_or(0);
        watches.retain_mut(|(what, at, last, progress)| {
            // (a new play starts its clock at 0: count from there)
            *progress += if heard_now < *last { heard_now } else { heard_now - *last };
            *last = heard_now;
            if app.playback.audio_clock && *progress > u64::from(SR) / 2 {
                recovered.push(json!({"event": what, "seconds": t - *at, "ok": true}));
                false
            } else if t - *at > 3.0 {
                recovered.push(json!({"event": what, "seconds": t - *at, "ok": false, "audioClock": app.playback.audio_clock}));
                false
            } else {
                true
            }
        });
        if t < quiet_until || !app.playback.audio_clock {
            continue;
        }
        // drift: the position heard now against the frame on screen (loop-relative)
        let (pos, Some(heard), written) = snapshot else { continue };
        if pos == u64::MAX {
            totals.no_reading += 1;
            continue;
        }
        let last = f64::from_bits(pos);
        let unplayed = written.saturating_sub(heard) as f64;
        let hearing = last + speed - unplayed * speed;
        // whole frames, as the sync probe: the frame the sound is in against the frame on screen
        let loop_frames = f64::from(LOOP_S) * fps;
        let sound = (hearing / f64::from(SR) * fps + 1e-6).floor().rem_euclid(loop_frames);
        let shown = (app.session.playhead().seconds() * fps).round().rem_euclid(loop_frames);
        let mut d = (shown - sound).abs();
        d = d.min(loop_frames - d);
        totals.worst_drift = totals.worst_drift.max(d);
        totals.readings += 1;
        if trace && (t >= next_trace || d > 1.0) {
            next_trace = t + 0.5;
            eprintln!(
                "trace {t:8.3} heard {heard} written {written} last {last:.1} unplayed {unplayed} speed {speed} shown {shown:.3} sound {sound:.3} playhead {:.5} drift {d:.3}",
                app.session.playhead().seconds()
            );
        }
    }
    h.state_mut().stop();
    let stats = h.state().playback.audio_stats.to_json(SR);
    let report = json!({
        "seconds": seconds,
        "tracks": TRACKS,
        "underruns": totals.underruns,
        "worstDriftFrames": totals.worst_drift,
        "readings": totals.readings,
        "noReading": totals.no_reading,
        "seeks": totals.seeks,
        "shuttles": totals.shuttles,
        "framesWithoutAudioClock": totals.clock_lost,
        "deviceEvents": recovered,
        "lastPlay": stats,
        "voiceIsolationTracks": std::env::var_os("FC_SOAK_VOICE_MODEL").map(|_| std::env::var("FC_SOAK_VOICE_TRACKS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(4)),
    });
    eprintln!("SOAK {report}");
    if let Ok(path) = std::env::var("FC_SOAK_REPORT") {
        let _ = std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap_or_default());
    }
    assert_eq!(totals.underruns, 0, "underruns: {report}");
    assert!(totals.readings > 0, "the position code was heard: {report}");
    assert!(totals.worst_drift <= 1.0, "drift: {report}");
    assert!(recovered.iter().all(|r| r["ok"] == true), "device events: {report}");
}
