//! Streaming DeepFilterNet3 enhancement at 48 kHz, 480 samples (10 ms) per channel at a time,
//! the way `libDF`'s `DfTract::process` does it with its default settings (both stages on every
//! frame, no post-filter, no attenuation limit): the network sees the newest frame's features and
//! its outputs apply to the frame two hops earlier (the model's lookahead), ERB gains on the whole
//! spectrum, then the deep filter over the last five noisy frames on the first 96 bins. Output is
//! the input delayed by [`DfParams::latency_samples`] (1440 samples, 30 ms).
//!
//! Several channels share one network: it listens to their mean (the mid of a stereo pair) and its
//! gains and filter taps are applied to each channel's own spectrum, so a stereo track costs about
//! one channel's network time and keeps its stereo image. One channel is exactly libDF's mono path.

use crate::ModelError;
use crate::dsp::{self, C32, DfParams, Features, Stft};
use crate::net::Net;
use crate::weights::Weights;

const P: DfParams = DfParams::DFN3;
/// Input beyond ±64 (+36 dBFS) is garbage, not audio; clamping it keeps the f32 maths finite.
const MAX_SAMPLE: f32 = 64.0;
/// Most channels one denoiser takes (7.1 and below).
pub const MAX_CHANNELS: usize = 8;

/// One channel's signal path around the shared network.
struct Chan {
    stft: Stft,
    /// The last `df_order` noisy spectra, oldest first.
    ring: Vec<Vec<C32>>,
    spec: Vec<C32>,
    input: Vec<f32>,
    output: Vec<f32>,
}

/// The enhancer for 1–[`MAX_CHANNELS`] channels.
pub struct Denoiser {
    widths: Vec<usize>,
    feats: Features,
    net: Net,
    chans: Vec<Chan>,
    mid: Vec<C32>,
    erb: Vec<f32>,
    cplx: Vec<C32>,
}

impl Denoiser {
    /// One channel.
    pub fn new(w: &Weights) -> Result<Denoiser, ModelError> {
        Denoiser::with_channels(w, 1)
    }

    /// `channels` (clamped to 1–[`MAX_CHANNELS`]) sharing one network.
    pub fn with_channels(w: &Weights, channels: usize) -> Result<Denoiser, ModelError> {
        let bad = || ModelError("DeepFilterNet3 settings are inconsistent".into());
        let widths = dsp::erb_widths(P.sample_rate, P.fft_size, P.nb_erb, P.min_nb_erb_freqs).ok_or_else(bad)?;
        if w.erb_widths()? != widths {
            return Err(ModelError("the checkpoint's ERB filterbank doesn't match DeepFilterNet3".into()));
        }
        let bins = P.freq_bins();
        let chans = (0..channels.clamp(1, MAX_CHANNELS))
            .map(|_| {
                Ok(Chan {
                    stft: Stft::new(P.fft_size, P.hop_size).ok_or_else(bad)?,
                    ring: vec![vec![C32::ZERO; bins]; P.df_order],
                    spec: vec![C32::ZERO; bins],
                    input: vec![0.0; P.hop_size],
                    output: vec![0.0; P.hop_size],
                })
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        Ok(Denoiser {
            widths,
            feats: Features::new(P.nb_erb, P.nb_df, dsp::norm_alpha(P.sample_rate, P.hop_size, P.norm_tau)),
            net: Net::new(w)?,
            chans,
            mid: vec![C32::ZERO; bins],
            erb: vec![0.0; P.nb_erb],
            cplx: vec![C32::ZERO; P.nb_df],
        })
    }

    /// Samples per channel per call.
    pub fn hop(&self) -> usize {
        P.hop_size
    }

    pub fn channels(&self) -> usize {
        self.chans.len()
    }

    /// Forget the past (a new clip, a seek): silence in, silence out again.
    pub fn reset(&mut self) {
        // in place: `AudioEffect::reset` may run on the audio thread
        self.feats.reset();
        self.net.reset();
        for c in &mut self.chans {
            c.stft.reset();
            for f in &mut c.ring {
                f.fill(C32::ZERO);
            }
        }
    }

    /// Enhance one hop of one channel (`hop()` samples at 48 kHz) into `output`; returns the local
    /// SNR estimate (dB). Non-finite input samples are treated as silence, samples beyond ±64 are
    /// clamped.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<f32, ModelError> {
        self.process_multi(&[input], &mut [output])
    }

    /// Enhance one hop of each of the first `inputs.len()` channels (at most [`channels`](Self::channels);
    /// `outputs` matches `inputs`); one network step for all of them.
    pub fn process_multi(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) -> Result<f32, ModelError> {
        let n = inputs.len();
        if n == 0
            || n > self.chans.len()
            || outputs.len() != n
            || inputs.iter().map(|b| b.len()).chain(outputs.iter().map(|o| o.len())).any(|l| l != P.hop_size)
        {
            return Err(ModelError(format!("expected 1–{} channels of {} samples", self.chans.len(), P.hop_size)));
        }
        for (c, input) in self.chans.iter_mut().zip(inputs) {
            c.input.copy_from_slice(input);
        }
        let lsnr = self.step(n)?;
        for (c, output) in self.chans.iter().zip(outputs.iter_mut()) {
            output.copy_from_slice(&c.output);
        }
        Ok(lsnr)
    }

    /// Channel `ch`'s input buffer for the next [`step`](Self::step) (`hop()` samples).
    pub fn input_mut(&mut self, ch: usize) -> Option<&mut [f32]> {
        self.chans.get_mut(ch).map(|c| c.input.as_mut_slice())
    }

    /// Channel `ch`'s output of the last [`step`](Self::step).
    pub fn output(&self, ch: usize) -> Option<&[f32]> {
        self.chans.get(ch).map(|c| c.output.as_slice())
    }

    /// Enhance the hop in the input buffers of the first `n` channels into their output buffers
    /// (no allocation); returns the local SNR estimate (dB).
    pub fn step(&mut self, n: usize) -> Result<f32, ModelError> {
        if n == 0 || n > self.chans.len() {
            return Err(ModelError(format!("expected 1–{} channels", self.chans.len())));
        }
        self.mid.fill(C32::ZERO);
        let k = 1.0 / n as f32;
        for c in self.chans.iter_mut().take(n) {
            for d in c.input.iter_mut() {
                *d = if d.is_finite() { d.clamp(-MAX_SAMPLE, MAX_SAMPLE) } else { 0.0 };
            }
            c.ring.rotate_left(1);
            let newest = c.ring.last_mut().ok_or_else(|| ModelError("no frames".into()))?;
            if !c.stft.analyse(&c.input, newest) {
                return Err(ModelError("analysis failed".into()));
            }
            for (m, x) in self.mid.iter_mut().zip(newest.iter()) {
                m.re += x.re * k;
                m.im += x.im * k;
            }
        }
        if !self.feats.erb(&self.mid, &self.widths, &mut self.erb) || !self.feats.cplx(&self.mid, &mut self.cplx) {
            return Err(ModelError("feature sizes".into()));
        }
        let out = self.net.step(&self.erb, &self.cplx)?;
        let target = P.df_order - 1 - P.df_lookahead;
        for c in self.chans.iter_mut().take(n) {
            c.spec.copy_from_slice(c.ring.get(target).ok_or_else(|| ModelError("no frames".into()))?);
            let ring: &[Vec<C32>; 5] = c.ring.as_slice().try_into().map_err(|_| ModelError("frame ring size".into()))?;
            let frames = ring.each_ref().map(Vec::as_slice);
            if !dsp::apply_band_gains(&mut c.spec, out.gains, &self.widths) || !dsp::deep_filter(&frames, out.coefs, P.nb_df, &mut c.spec) {
                return Err(ModelError("filter sizes".into()));
            }
            if !c.stft.synthesise(&c.spec, &mut c.output) {
                return Err(ModelError("synthesis failed".into()));
            }
            for v in c.output.iter_mut() {
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
        }
        Ok(out.lsnr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{Device, Tensor};
    use std::path::Path;

    /// Small uniform weights with the right names and shapes, and the real ERB filterbank.
    fn synthetic() -> Weights {
        let widths = dsp::erb_widths(48_000, 960, 32, 2).unwrap();
        let mut fb = vec![0.0f32; 481 * 32];
        let mut start = 0;
        for (b, w) in widths.iter().enumerate() {
            for k in start..start + w {
                fb[k * 32 + b] = 1.0 / *w as f32;
            }
            start += w;
        }
        let v = crate::weights::manifest()
            .into_iter()
            .map(|(n, d)| {
                let t = if n == "erb_fb" {
                    Tensor::from_vec(fb.clone(), d.as_slice(), &Device::Cpu).unwrap()
                } else {
                    Tensor::full(0.01f32, d.as_slice(), &Device::Cpu).unwrap()
                };
                (n, t)
            })
            .collect();
        Weights::from_tensors(v).unwrap()
    }

    #[test]
    fn runs_on_hostile_input_without_panicking() {
        let mut d = Denoiser::new(&synthetic()).unwrap();
        let mut out = vec![0.0; 480];
        for input in [vec![0.0; 480], vec![f32::NAN; 480], vec![1e30; 480], vec![f32::INFINITY; 480], (0..480).map(|i| (i as f32 * 0.1).sin()).collect()] {
            let lsnr = d.process(&input, &mut out).unwrap();
            assert!(lsnr.is_finite() && (-15.0..=35.0).contains(&lsnr));
            assert!(out.iter().all(|v| v.is_finite()));
        }
        assert!(d.process(&[0.0; 479], &mut out).is_err());
        assert!(d.process(&[0.0; 480], &mut out[..10]).is_err());
        d.reset();
        d.process(&[0.0; 480], &mut out).unwrap();
    }

    #[test]
    fn channels_share_the_network_and_identical_channels_match_mono() {
        let w = synthetic();
        let mut mono = Denoiser::new(&w).unwrap();
        let mut stereo = Denoiser::with_channels(&w, 2).unwrap();
        assert_eq!((mono.channels(), stereo.channels(), Denoiser::with_channels(&w, 99).unwrap().channels()), (1, 2, MAX_CHANNELS));
        let (mut m, mut l, mut r) = (vec![0.0; 480], vec![0.0; 480], vec![0.0; 480]);
        for k in 0..20 {
            let x: Vec<f32> = (0..480).map(|i| ((k * 480 + i) as f32 * 0.07).sin() * 0.3).collect();
            mono.process(&x, &mut m).unwrap();
            stereo.process_multi(&[&x, &x], &mut [&mut l, &mut r]).unwrap();
            assert_eq!(m, l);
            assert_eq!(l, r);
        }
        // bad shapes are errors, not panics
        assert!(stereo.process_multi(&[], &mut []).is_err());
        assert!(stereo.process_multi(&[&m, &m, &m], &mut [&mut l]).is_err());
        assert!(stereo.process_multi(&[&m[..10]], &mut [&mut l]).is_err());
        assert!(stereo.step(0).is_err() && stereo.step(3).is_err());
        assert!(stereo.input_mut(2).is_none() && stereo.output(5).is_none());
        stereo.input_mut(0).unwrap().fill(f32::NAN);
        stereo.step(1).unwrap();
        assert!(stereo.output(0).unwrap().iter().all(|v| v.is_finite()));
    }

    fn write_wav16(path: &Path, x: &[i16]) {
        let mut b = Vec::new();
        let data = (x.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&48_000u32.to_le_bytes());
        b.extend_from_slice(&96_000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for v in x {
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    /// 16-bit mono PCM samples of a simple WAV (the `data` chunk).
    fn read_wav16(path: &Path) -> Vec<f32> {
        let b = std::fs::read(path).unwrap();
        let mut i = 12;
        while i + 8 <= b.len() {
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if &b[i..i + 4] == b"data" {
                return b[i + 8..(i + 8 + len).min(b.len())].as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).collect();
            }
            i += 8 + len + (len & 1);
        }
        panic!("no data chunk");
    }

    /// A voice-like test signal: a gliding harmonic tone with syllable-rate bursts, plus noise.
    fn noisy_voice(n: usize) -> Vec<i16> {
        let mut seed = 0x1234_5678u32;
        let mut phase = 0.0f64;
        (0..n)
            .map(|i| {
                let t = i as f64 / 48_000.0;
                let f0 = 140.0 + 30.0 * (t * 1.3).sin();
                phase += 2.0 * std::f64::consts::PI * f0 / 48_000.0;
                let env = (0.5 + 0.5 * (t * 2.0 * std::f64::consts::PI * 3.0).sin()).powi(2);
                let voice: f64 = (1..20).map(|h| (phase * h as f64).sin() / h as f64).sum::<f64>() * env * 0.15;
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = (seed as f64 / u32::MAX as f64 - 0.5) * 0.08;
                ((voice + noise).clamp(-1.0, 1.0) * 32767.0) as i16
            })
            .collect()
    }

    /// With `FILMCRAFT_DFN3_CKPT` (the checkpoint) and `FILMCRAFT_DFN_ORACLE` (DeepFilterNet's own
    /// `deep-filter`, built from source as an external oracle): our output matches the original's.
    #[test]
    fn matches_the_original_when_present() {
        let (Some(ckpt), Some(oracle)) = (std::env::var_os("FILMCRAFT_DFN3_CKPT"), std::env::var_os("FILMCRAFT_DFN_ORACLE")) else {
            eprintln!("FILMCRAFT_DFN3_CKPT / FILMCRAFT_DFN_ORACLE unset: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("fc-voice-parity-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let x = noisy_voice(48_000 * 4);
        let wav = dir.join("noisy.wav");
        write_wav16(&wav, &x);
        let st = std::process::Command::new(&oracle).arg("-o").arg(dir.join("out")).arg(&wav).status().unwrap();
        assert!(st.success());
        let reference = read_wav16(&dir.join("out").join("noisy.wav"));

        let w = Weights::load(Path::new(&ckpt)).unwrap();
        let mut d = Denoiser::new(&w).unwrap();
        let input: Vec<f32> = x.iter().map(|v| *v as f32 / 32768.0).collect();
        let mut ours = Vec::with_capacity(input.len());
        let mut out = vec![0.0; 480];
        for hop in input.as_chunks::<480>().0 {
            d.process(hop, &mut out).unwrap();
            ours.extend_from_slice(&out);
        }
        let n = ours.len().min(reference.len());
        let (mut sig, mut diff) = (0.0f64, 0.0f64);
        for i in 1440..n {
            sig += (reference[i] as f64).powi(2);
            diff += (reference[i] as f64 - ours[i] as f64).powi(2);
        }
        let snr = 10.0 * (sig / diff.max(1e-20)).log10();
        eprintln!("parity with deep-filter: {snr:.1} dB over {} samples", n - 1440);
        std::fs::remove_dir_all(&dir).ok();
        assert!(snr > 25.0, "parity {snr:.1} dB");
    }

    /// Real-time factor on this CPU with the real checkpoint:
    /// `FILMCRAFT_DFN3_CKPT=… cargo test --release -p filmcraft-voice --features dfn rtf -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn rtf() {
        let Some(ckpt) = std::env::var_os("FILMCRAFT_DFN3_CKPT") else {
            return;
        };
        let w = Weights::load(Path::new(&ckpt)).unwrap();
        let mut d = Denoiser::new(&w).unwrap();
        let input: Vec<f32> = noisy_voice(48_000 * 10).iter().map(|v| *v as f32 / 32768.0).collect();
        let mut out = vec![0.0; 480];
        let t0 = std::time::Instant::now();
        for hop in input.as_chunks::<480>().0 {
            d.process(hop, &mut out).unwrap();
        }
        let secs = t0.elapsed().as_secs_f64();
        eprintln!("rtf: {:.3} (10 s of 48 kHz mono in {secs:.2} s, one thread, SIMD {})", secs / 10.0, filmcraft_ml::simd::level());
    }
}
