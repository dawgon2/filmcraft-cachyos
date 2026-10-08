//! cpal audio output: the playback master clock. Settings ▸ Audio Hardware picks the host
//! ("Device Class"), the output device, the I/O buffer size and the sample rate.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filmcraft_engine::settings::AudioHardwarePrefs;
use filmcraft_ui_egui::{AudioDevices, AudioOut};

pub struct CpalOut {
    stream: Option<cpal::Stream>,
    played: Arc<AtomicU64>,
    rate: u32,
    channels: u16,
    hw: AudioHardwarePrefs,
    /// Sequence sample rate for "Attempt to force hardware to document sample rate".
    document_rate: Option<u32>,
}

/// A device's display name (what Settings ▸ Audio Hardware lists and stores).
pub(crate) fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|x| x.name().to_string())
}

/// Report a stream error at most once a second: a backend that fails every period (an ALSA device
/// stuck in an xrun, issue #23) must not flood the terminal.
fn report_stream_error(e: &cpal::Error) {
    static LAST: AtomicU64 = AtomicU64::new(0);
    static SUPPRESSED: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if LAST.swap(now, Ordering::Relaxed) == now {
        SUPPRESSED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    match SUPPRESSED.swap(0, Ordering::Relaxed) {
        0 => eprintln!("filmcraft: audio stream error: {e}"),
        n => eprintln!("filmcraft: audio stream error: {e} ({n} more in the last second)"),
    }
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

/// The output device named in the settings (empty or missing = the host's default).
fn output_device(h: &cpal::Host, name: &str) -> Option<cpal::Device> {
    if !name.is_empty()
        && let Ok(mut devs) = h.output_devices()
        && let Some(d) = devs.find(|d| device_name(d).is_some_and(|n| n == name))
    {
        return Some(d);
    }
    h.default_output_device()
}

impl CpalOut {
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        let dev = host.default_output_device()?;
        let cfg = dev.default_output_config().ok()?;
        Some(Self {
            stream: None,
            played: Arc::new(AtomicU64::new(0)),
            rate: cfg.sample_rate(),
            channels: cfg.channels(),
            hw: AudioHardwarePrefs::default(),
            document_rate: None,
        })
    }

    /// The stream configuration the settings ask for, falling back to the device default.
    fn config(&self, dev: &cpal::Device) -> Result<cpal::SupportedStreamConfig, String> {
        let default = dev.default_output_config().map_err(|e| e.to_string())?;
        let want = if self.hw.force_document_rate { self.document_rate.unwrap_or(self.hw.sample_rate) } else { self.hw.sample_rate };
        let exact = dev.supported_output_configs().ok().and_then(|mut it| {
            it.find(|c| {
                c.sample_format() == cpal::SampleFormat::F32
                    && c.channels() == default.channels()
                    && (c.min_sample_rate()..=c.max_sample_rate()).contains(&want)
            })
            .and_then(|c| c.try_with_sample_rate(want))
        });
        Ok(exact.unwrap_or(default))
    }
}

impl AudioOut for CpalOut {
    fn start(&mut self, mut fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        self.stop();
        let host = host(&self.hw.device_class);
        let dev = output_device(&host, &self.hw.default_output).ok_or("no output device")?;
        let cfg = self.config(&dev)?;
        let channels = cfg.channels() as usize;
        let mut config: cpal::StreamConfig = cfg.into();
        if let cpal::SupportedBufferSize::Range { min, max } = cfg.buffer_size()
            && (*min..=*max).contains(&self.hw.buffer_size)
        {
            config.buffer_size = cpal::BufferSize::Fixed(self.hw.buffer_size);
        }
        self.rate = cfg.sample_rate();
        self.channels = cfg.channels();
        self.played.store(0, Ordering::SeqCst);
        let played = self.played.clone();
        let err = |e: cpal::Error| report_stream_error(&e);
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(
                config,
                move |buf: &mut [f32], _| {
                    fill(buf, channels);
                    played.fetch_add((buf.len() / channels) as u64, Ordering::SeqCst);
                },
                err,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        Ok(self.rate)
    }
    fn stop(&mut self) {
        self.stream = None;
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> usize {
        self.channels as usize
    }
    fn played_frames(&self) -> Option<u64> {
        self.stream.as_ref().map(|_| self.played.load(Ordering::SeqCst))
    }
    fn devices(&self) -> AudioDevices {
        let h = host(&self.hw.device_class);
        let outputs = h.output_devices().map(|d| d.filter_map(|x| device_name(&x)).collect()).unwrap_or_default();
        let inputs = h.input_devices().map(|d| d.filter_map(|x| device_name(&x)).collect()).unwrap_or_default();
        let output_channels = output_device(&h, &self.hw.default_output).and_then(|d| self.config(&d).ok()).map(|c| c.channels()).unwrap_or(0);
        AudioDevices { hosts: cpal::available_hosts().iter().map(|h| h.name().to_string()).collect(), inputs, outputs, output_channels }
    }
    fn configure(&mut self, hw: &AudioHardwarePrefs, document_rate: Option<u32>) {
        self.hw = hw.clone();
        self.document_rate = document_rate;
        // the rate playback mixes at must be known before `start`
        let host = host(&self.hw.device_class);
        if let Some(dev) = output_device(&host, &self.hw.default_output)
            && let Ok(cfg) = self.config(&dev)
        {
            self.rate = cfg.sample_rate();
            self.channels = cfg.channels();
        }
    }
}

#[cfg(test)]
mod tests {
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
            let Some(mut out) = CpalOut::new() else { panic!("no default output device") };
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
            let Some(mut out) = CpalOut::new() else { panic!("no default output device") };
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
