//! cpal audio output: the playback master clock. Settings ▸ Audio Hardware picks the host
//! ("Device Class"), the output device, the I/O buffer size and the sample rate.
//!
//! A stream the device drops while playing (an ALSA xrun the backend does not recover from, which
//! cpal 0.18 reports as an `Xrun` error, 0.17 as `poll() returned POLLERR`; an unplugged device; a
//! restarted audio server) is restarted by
//! [`AudioOut::recover`], keeping the played-frame clock and the play-ahead mixer, so sound
//! resumes where it stopped instead of playback going silent (#840).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filmcraft_engine::settings::AudioHardwarePrefs;
use filmcraft_ui_egui::{AudioDevices, AudioOut};

/// The playback mixer callback handed to [`AudioOut::start`].
type Fill = Box<dyn FnMut(&mut [f32], usize) + Send>;

pub struct CpalOut {
    stream: Option<cpal::Stream>,
    /// The running playback's mixer, shared with the stream so a restarted stream continues it.
    fill: Option<Arc<Mutex<Fill>>>,
    /// Fork A1.3a: a restart failed to open the device (a restarting audio server is often still
    /// down on the same frame); the watchdog retries while the restart budget lasts, and the
    /// clock holds still meanwhile instead of being given up.
    reopen: bool,
    played: Arc<AtomicU64>,
    /// Fork A1.4a: frames handed to the device that it hasn't played yet (its output latency plus
    /// the buffer just written), from the callback's timestamps; the clock subtracts it.
    latency: Arc<AtomicU64>,
    /// Fork A1.4a: the last clock reading handed out, so a latency that changes never moves the
    /// clock backwards.
    heard: AtomicU64,
    health: Arc<Health>,
    watchdog: Watchdog,
    /// Time base for the watchdog.
    epoch: Instant,
    rate: u32,
    channels: u16,
    hw: AudioHardwarePrefs,
    /// Sequence sample rate for "Attempt to force hardware to document sample rate".
    document_rate: Option<u32>,
    /// Status-bar note: the host's default device could not be opened and a fallback device is
    /// in use, or no output device was found (playing without sound).
    note: Option<Note>,
    /// The fallback picked when the default device could not be opened, as (host, device) names,
    /// so later lookups (`devices`, `configure`, `start`) reuse it instead of probing every PCM.
    fallback: Mutex<Option<(String, String)>>,
}

/// A hardware note for the status bar, translated when it is shown.
#[derive(Clone, Debug, PartialEq)]
enum Note {
    /// The default output device cannot be opened; this named device is used instead.
    Fallback(String),
    /// No output device at all: playback runs without sound.
    NoDevice,
}

impl Note {
    fn text(&self) -> String {
        use filmcraft_ui_egui::i18n::{fmt, t};
        match self {
            Note::Fallback(device) => fmt(t("Audio hardware: the default output device cannot be opened; using '{device}'"), &[("device", device)]),
            Note::NoDevice => t("No audio output device found: playing without sound (check Settings ▸ Audio Hardware)").to_string(),
        }
    }
}

/// A device's display name (what Settings ▸ Audio Hardware lists and stores).
pub(crate) fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|x| x.name().to_string())
}

/// The host named in the settings (empty or unknown = the default host).
fn host(name: &str) -> cpal::Host {
    if !name.is_empty()
        && let Some(id) = cpal::available_hosts().into_iter().find(|h| h.name() == name)
        && let Ok(h) = cpal::host_from_id(id)
    {
        return h;
    }
    cpal::default_host()
}

/// The output device the settings ask for, with a status-bar note when a fallback is in use.
struct DeviceChoice {
    device: cpal::Device,
    /// The host's default device could not be opened: this device is the fallback.
    fallback: Option<Note>,
}

/// Fallback order when the default device cannot be opened: the sound-server bridges first
/// (`pipewire`, `pulse`: they share the card with every other app), then other named PCMs, and
/// raw `hw:`/`plughw:` devices last (opening one takes the card exclusively and silences the
/// rest of the desktop).
fn fallback_rank(name: &str) -> u8 {
    let n = name.to_ascii_lowercase();
    if n == "pipewire" || n.starts_with("pipewire:") {
        0
    } else if n == "pulse" || n.starts_with("pulse:") {
        1
    } else if n.starts_with("hw:") || n.starts_with("plughw:") {
        3
    } else {
        2
    }
}

/// The output device named in the settings (empty or missing = the host's default). When the
/// default cannot be opened — a broken ALSA `default` (missing `99-pipewire-default.conf`,
/// #23/#106) — a fallback that can, so the app is not silently mute. The fallback list is probed
/// only after the default fails, in [`fallback_rank`] order, and the pick is cached in `cache`.
fn output_device(h: &cpal::Host, name: &str, cache: &Mutex<Option<(String, String)>>) -> Option<DeviceChoice> {
    if !name.is_empty()
        && let Ok(mut devs) = h.output_devices()
        && let Some(d) = devs.find(|d| device_name(d).is_some_and(|n| n == name))
    {
        return Some(DeviceChoice { device: d, fallback: None });
    }
    if let Some(d) = h.default_output_device()
        && d.default_output_config().is_ok()
    {
        return Some(DeviceChoice { device: d, fallback: None });
    }
    let host_name = h.id().name().to_string();
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let mut devs: Vec<(String, cpal::Device)> = h.output_devices().ok()?.filter_map(|d| device_name(&d).map(|n| (n, d))).collect();
    if let Some((cached_host, cached)) = cache.as_ref()
        && *cached_host == host_name
        && let Some(i) = devs.iter().position(|(n, _)| n == cached)
    {
        let (found, d) = devs.swap_remove(i);
        return Some(DeviceChoice { device: d, fallback: Some(Note::Fallback(found)) });
    }
    devs.sort_by_key(|(n, _)| fallback_rank(n));
    let (found, d) = devs.into_iter().find(|(_, d)| d.default_output_config().is_ok())?;
    log::warn!("the default audio output device cannot be opened; falling back to '{found}'");
    *cache = Some((host_name, found.clone()));
    Some(DeviceChoice { device: d, fallback: Some(Note::Fallback(found)) })
}

impl CpalOut {
    pub fn new() -> Self {
        let host = cpal::default_host();
        let fallback = Mutex::new(None);
        let choice = output_device(&host, "", &fallback);
        let cfg = choice.as_ref().and_then(|c| c.device.default_output_config().ok());
        let note = match choice {
            Some(c) => c.fallback,
            None => {
                log::warn!("no audio output device found");
                Some(Note::NoDevice)
            }
        };
        Self {
            stream: None,
            fill: None,
            reopen: false,
            played: Arc::new(AtomicU64::new(0)),
            latency: Arc::new(AtomicU64::new(0)),
            heard: AtomicU64::new(0),
            health: Arc::new(Health::default()),
            watchdog: Watchdog::default(),
            epoch: Instant::now(),
            rate: cfg.as_ref().map_or(48_000, |c| c.sample_rate()),
            channels: cfg.as_ref().map_or(2, |c| c.channels()),
            hw: AudioHardwarePrefs::default(),
            document_rate: None,
            note,
            fallback,
        }
    }

    /// The stream configuration the settings ask for, falling back to the device default.
    /// `expect` (a restart, fork A1.3a): the rate and channel count the running mixer was set up
    /// for are asked for first, so a new default sink whose own layout differs (a 6-channel HDMI
    /// sink) still plays the stream as it was.
    fn config(&self, dev: &cpal::Device, expect: Option<(u32, u16)>) -> Result<cpal::SupportedStreamConfig, String> {
        let default = dev.default_output_config().map_err(|e| e.to_string())?;
        let settings = if self.hw.force_document_rate { self.document_rate.unwrap_or(self.hw.sample_rate) } else { self.hw.sample_rate };
        let (want, channels) = wanted_format(settings, default.channels(), expect);
        let exact = dev.supported_output_configs().ok().and_then(|it| {
            it.filter(|c| c.channels() > 0 && (c.min_sample_rate()..=c.max_sample_rate()).contains(&want))
                .max_by_key(|c| (c.channels() == channels, c.sample_format() == cpal::SampleFormat::F32, c.sample_format() == default.sample_format()))
                .and_then(|c| c.try_with_sample_rate(want))
        });
        Ok(exact.unwrap_or(default))
    }

    fn converted_stream<T>(&self, dev: &cpal::Device, config: &cpal::StreamConfig, fill: Arc<Mutex<Fill>>) -> Result<cpal::Stream, cpal::Error>
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let channels = usize::from(config.channels);
        let rate = config.sample_rate;
        let played = self.played.clone();
        let latency = self.latency.clone();
        let health = self.health.clone();
        let error_health = health.clone();
        let mut scratch = Vec::new();
        dev.build_output_stream(
            *config,
            move |buf: &mut [T], info: &cpal::OutputCallbackInfo| {
                // Reuse the conversion buffer. A pathological driver buffer must not panic the audio thread.
                if buf.len() > 1_048_576 || scratch.try_reserve(buf.len().saturating_sub(scratch.len())).is_err() {
                    buf.fill(T::EQUILIBRIUM);
                    if !health.failed.swap(true, Ordering::SeqCst) {
                        eprintln!("filmcraft: unable to allocate the audio conversion buffer");
                    }
                    return;
                }
                scratch.resize(buf.len(), 0.0);
                render(&fill, &mut scratch, channels, &played, &health);
                latency.store(unplayed_frames(info, rate, buf.len() / channels.max(1)), Ordering::Relaxed);
                convert_samples(&scratch, buf);
            },
            move |e| stream_error(&error_health, e),
            None,
        )
    }

    /// Open the configured output device and play `fill` on it. `expect` is the (sample rate,
    /// channels) a restarted stream must keep: the mixer was set up for them.
    fn open(&mut self, fill: Arc<Mutex<Fill>>, expect: Option<(u32, u16)>) -> Result<(), String> {
        let host = host(&self.hw.device_class);
        let DeviceChoice { device: dev, fallback } = output_device(&host, &self.hw.default_output, &self.fallback).ok_or("no output device")?;
        self.note = fallback;
        let cfg = self.config(&dev, expect)?;
        let channels = cfg.channels() as usize;
        if channels == 0 || cfg.sample_rate() == 0 {
            return Err("the output device returned an invalid channel count or sample rate".into());
        }
        if let Some((rate, ch)) = expect
            && (rate, ch) != (cfg.sample_rate(), cfg.channels())
        {
            return Err(format!("the output device now runs at {} Hz with {} channels", cfg.sample_rate(), cfg.channels()));
        }
        let mut config: cpal::StreamConfig = cfg.into();
        if let cpal::SupportedBufferSize::Range { min, max } = cfg.buffer_size()
            && (*min..=*max).contains(&self.hw.buffer_size)
        {
            config.buffer_size = cpal::BufferSize::Fixed(self.hw.buffer_size);
        }
        self.rate = cfg.sample_rate();
        self.channels = cfg.channels();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => {
                let played = self.played.clone();
                let latency = self.latency.clone();
                let rate = config.sample_rate;
                let health = self.health.clone();
                let error_health = health.clone();
                dev.build_output_stream(
                    config,
                    move |buf: &mut [f32], info: &cpal::OutputCallbackInfo| {
                        render(&fill, buf, channels, &played, &health);
                        latency.store(unplayed_frames(info, rate, buf.len() / channels.max(1)), Ordering::Relaxed);
                    },
                    move |e| stream_error(&error_health, e),
                    None,
                )
            }
            cpal::SampleFormat::I8 => self.converted_stream::<i8>(&dev, &config, fill),
            cpal::SampleFormat::I16 => self.converted_stream::<i16>(&dev, &config, fill),
            cpal::SampleFormat::I24 => self.converted_stream::<cpal::I24>(&dev, &config, fill),
            cpal::SampleFormat::I32 => self.converted_stream::<i32>(&dev, &config, fill),
            cpal::SampleFormat::I64 => self.converted_stream::<i64>(&dev, &config, fill),
            cpal::SampleFormat::U8 => self.converted_stream::<u8>(&dev, &config, fill),
            cpal::SampleFormat::U16 => self.converted_stream::<u16>(&dev, &config, fill),
            cpal::SampleFormat::U32 => self.converted_stream::<u32>(&dev, &config, fill),
            cpal::SampleFormat::U64 => self.converted_stream::<u64>(&dev, &config, fill),
            cpal::SampleFormat::F64 => self.converted_stream::<f64>(&dev, &config, fill),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        Ok(())
    }

    /// [`AudioOut::recover`] at `now`, with `restart` for [`Self::restart`] (tests inject a
    /// failing or succeeding one).
    fn recover_with(&mut self, now: Duration, mut restart: impl FnMut(&mut Self) -> Result<(), String>) {
        if (self.stream.is_none() && !self.reopen) || self.health.failed.load(Ordering::SeqCst) {
            return;
        }
        let lost = self.health.lost.swap(false, Ordering::SeqCst);
        // a pending reopen counts as an error with no samples flowing: retried every
        // REOPEN_RETRY while the restart budget lasts (fork A1.3a)
        let errored = self.health.errored.swap(false, Ordering::SeqCst) || self.reopen;
        self.watchdog.reopening = self.reopen;
        match self.watchdog.check(now, self.played.load(Ordering::SeqCst), errored, lost) {
            Verdict::Healthy => {}
            Verdict::Restart => {
                log::warn!("the audio output stream {}; restarting it", if lost { "lost its device" } else { "stopped after an error" });
                match restart(self) {
                    Ok(()) => self.reopen = false,
                    Err(e) => {
                        log::warn!("the audio output stream could not be restarted (will retry): {e}");
                        self.stream = None;
                        self.reopen = self.fill.is_some();
                    }
                }
            }
            Verdict::GiveUp => {
                log::warn!("the audio output stream keeps failing ({MAX_RESTARTS} restarts within {} s)", RESTART_WINDOW.as_secs());
                self.reopen = false;
                self.health.failed.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Replace a dropped stream with a new one on the same mixer, clock and format.
    fn restart(&mut self) -> Result<(), String> {
        let fill = self.fill.clone().ok_or("nothing is playing")?;
        let expect = (self.rate, self.channels);
        // The old stream (and its audio thread) goes first: only one stream ever runs the mixer.
        self.stream = None;
        self.health.clear_errors();
        self.open(fill, Some(expect))
    }
}

/// Fork A1.3a: the (rate, channel count) to ask a device for: a restart keeps the running
/// stream's, else the settings' rate and the device's default channel count.
fn wanted_format(settings_rate: u32, default_channels: u16, expect: Option<(u32, u16)>) -> (u32, u16) {
    expect.unwrap_or((settings_rate, default_channels))
}

/// What the audio thread reports to the UI thread.
#[derive(Default)]
struct Health {
    /// The device went away (unplugged): restart the stream at once.
    lost: AtomicBool,
    /// A backend error since the last check (cpal keeps the stream alive after these): restart
    /// only if the stream also stopped consuming samples.
    errored: AtomicBool,
    /// Unrecoverable: the mixer panicked, a buffer could not be allocated, or restarts ran out.
    /// [`AudioOut::played_frames`] reports `None` and playback goes on without sound.
    failed: AtomicBool,
    /// The first error of a stream has been printed.
    reported: AtomicBool,
}

impl Health {
    fn clear_errors(&self) {
        self.lost.store(false, Ordering::SeqCst);
        self.errored.store(false, Ordering::SeqCst);
        self.reported.store(false, Ordering::SeqCst);
    }
}

/// How long a stream that reported an error may go without consuming samples before it is
/// restarted. Well under the UI's `AUDIO_STALL_S`, which gives up on the device clock.
const STALL_RESTART: Duration = Duration::from_millis(300);
/// Fork A1.3a: how often a reopen that failed is retried: the budget ([`MAX_RESTARTS`]) is spent
/// ~600 ms after the loss, well under the UI's `AUDIO_STALL_S` (1 s from the last sample).
const REOPEN_RETRY: Duration = Duration::from_millis(200);
/// At most this many restarts within [`RESTART_WINDOW`]; a device that keeps failing gives up.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(10);

/// What [`Watchdog::check`] decides about the running stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Healthy,
    Restart,
    GiveUp,
}

/// Decides when a stream that reported errors is restarted (UI thread, every frame).
#[derive(Default)]
struct Watchdog {
    /// Played-frame count last seen, and when it last changed.
    seen: (u64, Duration),
    /// An error was reported and the stream has not consumed samples since.
    pending_error: bool,
    /// When recent restarts happened (at most [`MAX_RESTARTS`] kept).
    restarts: Vec<Duration>,
    /// Fork A1.3a: a failed reopen is pending: retry after [`REOPEN_RETRY`], not [`STALL_RESTART`].
    reopening: bool,
}

impl Watchdog {
    fn reset(&mut self, now: Duration) {
        *self = Self { seen: (0, now), ..Self::default() };
    }

    fn check(&mut self, now: Duration, played: u64, errored: bool, lost: bool) -> Verdict {
        if played != self.seen.0 {
            self.seen = (played, now);
            self.pending_error = false;
        }
        // An error that arrives with the stream still consuming samples is only a hiccup.
        self.pending_error |= errored;
        let stalled = now.saturating_sub(self.seen.1) >= if self.reopening { REOPEN_RETRY } else { STALL_RESTART };
        if !lost && !(self.pending_error && stalled) {
            return Verdict::Healthy;
        }
        self.restarts.retain(|t| now.saturating_sub(*t) < RESTART_WINDOW);
        if self.restarts.len() >= MAX_RESTARTS {
            return Verdict::GiveUp;
        }
        self.restarts.push(now);
        self.seen = (played, now);
        self.pending_error = false;
        Verdict::Restart
    }
}

/// One device callback: run the shared mixer into `buf` and advance the clock. Never panics: a
/// panicking mixer plays silence and fails the stream, and a mixer held by another stream (never
/// expected: a restart drops the old stream first) plays silence without advancing the clock.
fn render(fill: &Mutex<Fill>, buf: &mut [f32], channels: usize, played: &AtomicU64, health: &Health) {
    let mut guard = match fill.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            buf.fill(0.0);
            return;
        }
    };
    if health.failed.load(Ordering::SeqCst) || catch_unwind(AssertUnwindSafe(|| (*guard)(buf, channels))).is_err() {
        buf.fill(0.0);
        if !health.failed.swap(true, Ordering::SeqCst) {
            eprintln!("filmcraft: the audio mixer failed; playing without sound");
        }
        return;
    }
    played.fetch_add((buf.len() / channels.max(1)) as u64, Ordering::SeqCst);
}

/// Fork A1.4a: how many frames the device holds unplayed right after this callback: the time from
/// the callback to when its first frame plays, plus the `frames` it was just given. A backend that
/// reports nonsense is capped at half a second.
fn unplayed_frames(info: &cpal::OutputCallbackInfo, rate: u32, frames: usize) -> u64 {
    let ts = info.timestamp();
    let ahead = ts.playback.saturating_duration_since(ts.callback).as_secs_f64().min(0.5);
    ((ahead * f64::from(rate)) as u64).saturating_add(frames as u64)
}

fn stream_error(health: &Health, e: cpal::Error) {
    // (cpal 0.18's error kinds, fork: the device or its server is gone, or the stream must be
    // rebuilt: restart at once; a reroute to the new default device or a refused real-time
    // priority leave the stream playing; anything else counts as an error, restarted only when
    // no samples flow)
    match e.kind() {
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable | cpal::ErrorKind::StreamInvalidated => {
            health.lost.store(true, Ordering::SeqCst)
        }
        cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied => {}
        _ => health.errored.store(true, Ordering::SeqCst),
    }
    if !health.reported.swap(true, Ordering::SeqCst) {
        eprintln!("filmcraft: audio stream error: {e}");
    }
}

fn convert_samples<T: cpal::Sample + cpal::FromSample<f32>>(input: &[f32], output: &mut [T]) {
    // Integer conversion represents [-1, 1). In particular I24 cannot represent +1 exactly.
    let upper = f32::from_bits(1.0_f32.to_bits() - 1);
    for (sample, converted) in input.iter().zip(output) {
        *converted = T::from_sample(if sample.is_finite() { sample.clamp(-1.0, upper) } else { 0.0 });
    }
}

impl AudioOut for CpalOut {
    fn start(&mut self, fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        self.stop();
        self.played.store(0, Ordering::SeqCst);
        self.latency.store(0, Ordering::Relaxed);
        self.heard.store(0, Ordering::Relaxed);
        self.health = Arc::new(Health::default());
        self.watchdog.reset(self.epoch.elapsed());
        self.reopen = false;
        let fill = Arc::new(Mutex::new(fill));
        self.open(fill.clone(), None)?;
        self.fill = Some(fill);
        Ok(self.rate)
    }
    fn stop(&mut self) {
        self.stream = None;
        self.fill = None;
        self.reopen = false;
    }
    fn recover(&mut self) {
        self.recover_with(self.epoch.elapsed(), Self::restart);
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> usize {
        self.channels as usize
    }
    fn played_frames(&self) -> Option<u64> {
        // (while a reopen is pending the clock holds still: the UI waits up to AUDIO_STALL_S)
        (!self.health.failed.load(Ordering::SeqCst) && (self.stream.is_some() || self.reopen)).then(|| {
            // the frames heard, not the frames handed over (fork A1.4a); never backwards
            let heard = self.played.load(Ordering::SeqCst).saturating_sub(self.latency.load(Ordering::Relaxed));
            self.heard.fetch_max(heard, Ordering::Relaxed).max(heard)
        })
    }
    fn devices(&self) -> AudioDevices {
        let h = host(&self.hw.device_class);
        let outputs = h.output_devices().map(|d| d.filter_map(|x| device_name(&x)).collect()).unwrap_or_default();
        let inputs = h.input_devices().map(|d| d.filter_map(|x| device_name(&x)).collect()).unwrap_or_default();
        let output_channels =
            output_device(&h, &self.hw.default_output, &self.fallback).and_then(|c| self.config(&c.device, None).ok()).map(|cfg| cfg.channels()).unwrap_or(0);
        AudioDevices { hosts: cpal::available_hosts().iter().map(|h| h.name().to_string()).collect(), inputs, outputs, output_channels }
    }
    fn configure(&mut self, hw: &AudioHardwarePrefs, document_rate: Option<u32>) {
        self.hw = hw.clone();
        self.document_rate = document_rate;
        // the rate playback mixes at must be known before `start`
        let host = host(&self.hw.device_class);
        if let Some(choice) = output_device(&host, &self.hw.default_output, &self.fallback)
            && let Ok(cfg) = self.config(&choice.device, None)
        {
            self.rate = cfg.sample_rate();
            self.channels = cfg.channels();
            self.note = choice.fallback;
        }
    }
    fn note(&self) -> Option<String> {
        self.note.as_ref().map(Note::text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::Sample;

    #[test]
    fn fallback_prefers_sound_servers_over_raw_hardware() {
        let mut names = vec!["plughw:CARD=PCH,DEV=0", "hw:CARD=PCH,DEV=0", "sysdefault:CARD=PCH", "pulse", "pipewire"];
        names.sort_by_key(|n| fallback_rank(n));
        assert_eq!(names, ["pipewire", "pulse", "sysdefault:CARD=PCH", "plughw:CARD=PCH,DEV=0", "hw:CARD=PCH,DEV=0"]);
    }

    #[test]
    fn notes_name_the_fallback_device() {
        assert!(Note::Fallback("pipewire".into()).text().contains("'pipewire'"));
        assert!(!Note::NoDevice.text().is_empty());
    }

    #[test]
    fn integer_conversion_clips_and_preserves_interleaved_channels() {
        let mut signed = [0_i16; 8];
        convert_samples(&[-2.0, 2.0, -0.5, 0.5, 0.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY], &mut signed);
        assert_eq!(signed, [i16::MIN, i16::MAX, -16_384, 16_384, 0, 0, 0, 0]);
        let mut unsigned = [0_u16; 3];
        convert_samples(&[-1.0, 0.0, 1.0], &mut unsigned);
        assert_eq!(unsigned, [0, 32_768, u16::MAX]);
    }

    #[test]
    fn full_scale_24_bit_samples_remain_representable() {
        let mut samples = [cpal::I24::EQUILIBRIUM; 3];
        convert_samples(&[-1.0, 0.0, 1.0], &mut samples);
        assert_eq!(samples.map(|s| s.inner()), [-8_388_608, 0, 8_388_607]);
    }

    /// Fork A1.4a: the clock counts the frames heard: the device's reported output latency and the
    /// buffer just written are not played yet; a nonsense latency is capped.
    #[test]
    fn unplayed_frames_are_the_output_latency_plus_the_buffer() {
        let at = |ms: u64| cpal::StreamInstant::ZERO.checked_add(Duration::from_millis(ms)).unwrap();
        let info = |cb: u64, play: u64| cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp { callback: at(cb), playback: at(play) });
        assert_eq!(unplayed_frames(&info(1_000, 1_020), 48_000, 480), 960 + 480, "20 ms latency + the buffer");
        assert_eq!(unplayed_frames(&info(1_000, 1_000), 48_000, 512), 512);
        assert_eq!(unplayed_frames(&info(1_000, 900), 48_000, 512), 512, "a timestamp in the past counts as none");
        assert_eq!(unplayed_frames(&info(0, 3_600_000), 48_000, 512), 24_000 + 512, "capped at 0.5 s");
    }

    /// Fork A1.4a: the clock subtracts what the device hasn't played yet and never runs backwards
    /// when that amount grows (a restarted stream with a longer latency).
    #[test]
    fn the_clock_counts_heard_frames_and_never_goes_back() {
        let mut out = CpalOut::new();
        out.reopen = true; // (no stream needed to read the clock)
        out.fill = Some(Arc::new(Mutex::new(Box::new(|buf: &mut [f32], _: usize| buf.fill(0.0)))));
        out.played.store(48_000, Ordering::SeqCst);
        out.latency.store(1_440, Ordering::Relaxed);
        assert_eq!(out.played_frames(), Some(46_560));
        out.latency.store(4_800, Ordering::Relaxed);
        assert_eq!(out.played_frames(), Some(46_560), "held, not backwards");
        out.played.store(52_800, Ordering::SeqCst);
        assert_eq!(out.played_frames(), Some(48_000));
        out.latency.store(60_000, Ordering::Relaxed);
        assert_eq!(out.played_frames(), Some(48_000), "a latency larger than the clock saturates");
    }

    /// Fork A1.1 (cpal 0.18): a reroute to the new default device or a refused real-time
    /// priority leaves the stream alone; an invalidated stream or a gone audio server restarts at
    /// once, like an unplugged device.
    #[test]
    fn cpal_error_kinds_map_to_restart_or_nothing() {
        for kind in [cpal::ErrorKind::DeviceChanged, cpal::ErrorKind::RealtimeDenied] {
            let health = Health::default();
            stream_error(&health, cpal::Error::new(kind));
            assert!(!health.errored.load(Ordering::SeqCst) && !health.lost.load(Ordering::SeqCst), "{kind:?}");
        }
        for kind in [cpal::ErrorKind::StreamInvalidated, cpal::ErrorKind::HostUnavailable, cpal::ErrorKind::DeviceNotAvailable] {
            let health = Health::default();
            stream_error(&health, cpal::Error::new(kind));
            assert!(health.lost.load(Ordering::SeqCst), "{kind:?}");
        }
        for kind in [cpal::ErrorKind::Xrun, cpal::ErrorKind::DeviceBusy, cpal::ErrorKind::Other] {
            let health = Health::default();
            stream_error(&health, cpal::Error::new(kind));
            assert!(health.errored.load(Ordering::SeqCst) && !health.lost.load(Ordering::SeqCst), "{kind:?}");
        }
    }

    #[test]
    fn stream_errors_ask_for_a_restart_instead_of_failing_playback() {
        let health = Health::default();
        stream_error(&health, cpal::Error::new(cpal::ErrorKind::Xrun));
        assert!(health.errored.load(Ordering::SeqCst) && !health.lost.load(Ordering::SeqCst));
        stream_error(&health, cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable));
        assert!(health.lost.load(Ordering::SeqCst));
        assert!(!health.failed.load(Ordering::SeqCst), "a stream error alone must not silence playback");
        let output = CpalOut::new();
        assert_eq!(output.played_frames(), None, "no stream, no clock");
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn an_error_while_samples_keep_flowing_is_only_a_hiccup() {
        let mut w = Watchdog::default();
        w.reset(ms(0));
        assert_eq!(w.check(ms(10), 480, true, false), Verdict::Healthy);
        // the stream keeps consuming samples after the error: never restarted
        for (i, t) in (20..2_000).step_by(20).enumerate() {
            assert_eq!(w.check(ms(t), 480 * (i as u64 + 2), false, false), Verdict::Healthy);
        }
    }

    #[test]
    fn a_stream_that_errors_and_stops_consuming_is_restarted() {
        let mut w = Watchdog::default();
        w.reset(ms(0));
        assert_eq!(w.check(ms(10), 480, false, false), Verdict::Healthy);
        // POLLERR: the backend reports errors and no longer calls back
        assert_eq!(w.check(ms(20), 480, true, false), Verdict::Healthy);
        assert_eq!(w.check(ms(200), 480, true, false), Verdict::Healthy);
        assert_eq!(w.check(ms(310), 480, false, false), Verdict::Restart);
        // the restarted stream gets a fresh grace period
        assert_eq!(w.check(ms(400), 480, false, false), Verdict::Healthy);
        // a stall without any reported error is left to the UI's own watchdog
        assert_eq!(w.check(ms(2_000), 480, false, false), Verdict::Healthy);
    }

    /// Fork A1.3a: the audio server restarts (the stream is invalidated) and is still down when
    /// the first reopen runs: `recover` keeps the reopen pending, which the watchdog sees as an
    /// error with no samples flowing, so it retries every STALL_RESTART: well before the UI's
    /// own 1 s stall limit, and within the restart budget.
    #[test]
    fn a_failed_reopen_is_retried_on_the_watchdog_cadence_then_given_up() {
        let mut w = Watchdog::default();
        w.reset(ms(0));
        assert_eq!(w.check(ms(10), 960, false, false), Verdict::Healthy);
        assert_eq!(w.check(ms(20), 960, false, true), Verdict::Restart, "invalidated: at once");
        // reopen failed: pending (errored = true, reopening), the clock holds at 960
        w.reopening = true;
        assert_eq!(w.check(ms(100), 960, true, false), Verdict::Healthy);
        assert_eq!(w.check(ms(220), 960, true, false), Verdict::Restart, "second try after 200 ms");
        assert_eq!(w.check(ms(420), 960, true, false), Verdict::Restart, "third try");
        assert_eq!(w.check(ms(620), 960, true, false), Verdict::GiveUp, "budget spent");
        assert!(ms(620).as_secs_f64() + 0.2 < filmcraft_ui_egui::AUDIO_STALL_S, "well before the UI's own stall limit");
        // the server came back in time: the reopened stream consumes samples again
        let mut w = Watchdog::default();
        w.reset(ms(0));
        assert_eq!(w.check(ms(20), 960, false, true), Verdict::Restart);
        w.reopening = true;
        assert_eq!(w.check(ms(220), 960, true, false), Verdict::Restart);
        w.reopening = false;
        for (i, t) in (240..3_000).step_by(20).enumerate() {
            assert_eq!(w.check(ms(t), 960 + 480 * (i as u64 + 1), false, false), Verdict::Healthy);
        }
    }

    /// Fork A1.3a, through `recover`: a pending reopen keeps the (frozen) clock, is retried until
    /// the budget is spent, then gives the clock up; a reopen that succeeds clears it.
    #[test]
    fn recover_keeps_the_clock_while_a_reopen_is_retried() {
        let mut out = CpalOut::new();
        out.fill = Some(Arc::new(Mutex::new(Box::new(|buf: &mut [f32], _: usize| buf.fill(0.0)))));
        out.played.store(960, Ordering::SeqCst);
        out.watchdog.reset(ms(0));
        // the device went away and the first reopen failed
        out.health.lost.store(true, Ordering::SeqCst);
        out.reopen = true;
        let mut tries = 0;
        let mut failing = |_: &mut CpalOut| -> Result<(), String> {
            tries += 1;
            Err("connection refused".into())
        };
        for t in (20..=1_000).step_by(20) {
            out.recover_with(ms(t), &mut failing);
            if t < 600 {
                assert_eq!(out.played_frames(), Some(960), "the clock holds while retrying (t = {t} ms)");
            }
        }
        assert_eq!(tries, 3, "the restart budget");
        assert_eq!(out.played_frames(), None, "given up: playback goes on without sound");
        assert!(!out.reopen);

        let mut out = CpalOut::new();
        out.fill = Some(Arc::new(Mutex::new(Box::new(|buf: &mut [f32], _: usize| buf.fill(0.0)))));
        out.watchdog.reset(ms(0));
        out.reopen = true;
        out.recover_with(ms(250), |_| Ok(()));
        assert!(!out.reopen, "a reopen that worked is no longer pending");
    }

    /// Fork A1.3a: a restart asks the device for the running stream's rate and channel count, not
    /// the new device's default layout; a fresh start asks for the settings' rate.
    #[test]
    fn a_restart_asks_for_the_running_format() {
        assert_eq!(wanted_format(48_000, 6, Some((44_100, 2))), (44_100, 2));
        assert_eq!(wanted_format(48_000, 6, None), (48_000, 6));
    }

    #[test]
    fn a_lost_device_restarts_at_once_and_repeated_failures_give_up() {
        let mut w = Watchdog::default();
        w.reset(ms(0));
        assert_eq!(w.check(ms(5), 0, false, true), Verdict::Restart);
        assert_eq!(w.check(ms(10), 0, false, true), Verdict::Restart);
        assert_eq!(w.check(ms(15), 0, false, true), Verdict::Restart);
        assert_eq!(w.check(ms(20), 0, false, true), Verdict::GiveUp);
        // sporadic failures far apart keep being recovered
        assert_eq!(w.check(ms(10_006), 0, false, true), Verdict::Restart);
    }

    #[test]
    fn the_device_callback_never_panics_and_counts_played_frames() {
        let (played, health) = (AtomicU64::new(0), Health::default());
        let fill: Mutex<Fill> = Mutex::new(Box::new(|buf: &mut [f32], _| buf.fill(0.25)));
        let mut buf = [1.0_f32; 8];
        render(&fill, &mut buf, 2, &played, &health);
        assert_eq!((buf, played.load(Ordering::SeqCst)), ([0.25; 8], 4));
        // a mixer still held elsewhere plays silence without moving the clock
        {
            let _held = fill.lock().unwrap();
            render(&fill, &mut buf, 2, &played, &health);
        }
        assert_eq!((buf, played.load(Ordering::SeqCst)), ([0.0; 8], 4));
        // zero channels from a broken driver must not divide by zero
        render(&fill, &mut buf, 0, &played, &health);
        assert_eq!(played.load(Ordering::SeqCst), 12);
    }

    #[test]
    fn a_panicking_mixer_plays_silence_and_fails_the_stream() {
        let (played, health) = (AtomicU64::new(0), Health::default());
        let fill: Mutex<Fill> = Mutex::new(Box::new(|buf: &mut [f32], _| {
            buf.fill(0.5);
            panic!("mixer bug");
        }));
        let mut buf = [1.0_f32; 4];
        render(&fill, &mut buf, 2, &played, &health);
        assert_eq!(buf, [0.0; 4]);
        assert_eq!(played.load(Ordering::SeqCst), 0);
        assert!(health.failed.load(Ordering::SeqCst));
        // later callbacks stay silent instead of re-entering the broken mixer
        buf = [1.0; 4];
        render(&fill, &mut buf, 2, &played, &health);
        assert_eq!(buf, [0.0; 4]);
    }
}

#[cfg(test)]
mod hardware_tests {
    use super::*;
    use filmcraft_ui_egui::AudioOut;

    /// Plays 1.5 s of silence on every host's default output (PulseAudio / PipeWire and ALSA on
    /// Linux) and checks each stream advances without errors piling up (issue #23: ALSA under
    /// PipeWire looped on POLLERR). Needs audio hardware or a server:
    /// `cargo test -p filmcraft -- --ignored audio_output`.
    #[test]
    #[ignore]
    fn audio_output_plays_on_every_host() {
        eprintln!("default host: {:?}", cpal::default_host().id());
        for id in cpal::available_hosts() {
            let mut out = CpalOut::new();
            let prefs = AudioHardwarePrefs { device_class: id.name().to_string(), ..AudioHardwarePrefs::default() };
            out.configure(&prefs, None);
            let rate = out.start(Box::new(|buf: &mut [f32], _| buf.fill(0.0))).unwrap_or_else(|e| panic!("{}: {e}", id.name()));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let played = out.played_frames().unwrap_or(0);
            out.stop();
            eprintln!("{}: {rate} Hz, {} channels, played {played} frames", id.name(), out.channels());
            assert!(played as f64 > rate as f64, "{}: stream advanced ({played} frames at {rate} Hz)", id.name());
        }
    }

    /// Issue #23: after an underrun (the callback stalls once), playback must continue and the
    /// error must not repeat every period. `cargo test -p filmcraft -- --ignored xrun_recovers`.
    #[test]
    #[ignore]
    fn xrun_recovers() {
        for id in cpal::available_hosts() {
            let mut out = CpalOut::new();
            let prefs = AudioHardwarePrefs { device_class: id.name().to_string(), ..AudioHardwarePrefs::default() };
            out.configure(&prefs, None);
            let mut calls = 0u32;
            let rate = out
                .start(Box::new(move |buf: &mut [f32], _| {
                    calls += 1;
                    if calls == 20 {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                    buf.fill(0.0);
                }))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(800));
            let before = out.played_frames().unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let after = out.played_frames().unwrap_or(0);
            out.stop();
            eprintln!("XRUN {}: {rate} Hz, played {} frames in the 1.5 s after the stall", id.name(), after.saturating_sub(before));
        }
    }
}
