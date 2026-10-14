//! The voice clean-up test set (fork AUDIO-PLAN A2): generated, deterministic noise of five kinds,
//! mixing at a chosen SNR, scale-invariant SDR, and a clean 2× upsampler (the built-in voices speak
//! at 24 kHz, the model works at 48 kHz). Pure Rust, no model needed; the measurements that use it
//! live in the engine (`voice_isolation_eval`).

use std::f64::consts::PI;

/// A small deterministic generator (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [−1, 1).
    pub fn bipolar(&mut self) -> f64 {
        self.next_f64() * 2.0 - 1.0
    }
}

/// The noise kinds of the test set; the last three change over time ("non-steady").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Noise {
    Fan,
    Room,
    Traffic,
    Keyboard,
    Wind,
}

impl Noise {
    pub const ALL: [Noise; 5] = [Noise::Fan, Noise::Room, Noise::Traffic, Noise::Keyboard, Noise::Wind];

    pub fn name(self) -> &'static str {
        match self {
            Noise::Fan => "fan",
            Noise::Room => "room",
            Noise::Traffic => "traffic",
            Noise::Keyboard => "keyboard",
            Noise::Wind => "wind",
        }
    }

    /// Whether the noise stays the same over time (what classic noise reduction can learn).
    pub fn steady(self) -> bool {
        matches!(self, Noise::Fan | Noise::Room)
    }
}

/// One-pole low-pass coefficient for `hz` at `sr`.
fn lp_coef(hz: f64, sr: f64) -> f64 {
    1.0 - (-2.0 * PI * hz / sr).exp()
}

/// `n` samples of `kind` noise at `sr` Hz, normalised to RMS 1.
pub fn noise(kind: Noise, n: usize, sr: u32, seed: u64) -> Vec<f32> {
    let sr = f64::from(sr.max(1));
    let mut r = Rng::new(seed ^ (kind as u64 * 7919));
    let mut out = vec![0.0f64; n];
    match kind {
        Noise::Fan => {
            // Pink-ish broadband (three one-pole bands) + motor hum and blade-pass tones.
            let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
            let (ka, kb, kc) = (lp_coef(150.0, sr), lp_coef(1200.0, sr), lp_coef(6000.0, sr));
            for (i, o) in out.iter_mut().enumerate() {
                let w = r.bipolar();
                a += ka * (w - a);
                b += kb * (w - b);
                c += kc * (w - c);
                let t = i as f64 / sr;
                *o = 3.0 * a + 1.5 * b + 0.6 * c + 0.05 * (2.0 * PI * 120.0 * t).sin() + 0.03 * (2.0 * PI * 37.0 * 7.0 * t).sin();
            }
        }
        Noise::Room => {
            // Low air-handling rumble, a little hiss and mains hum: steady.
            let (mut a, mut h) = (0.0, 0.0);
            let (ka, kh) = (lp_coef(200.0, sr), lp_coef(9000.0, sr));
            for (i, o) in out.iter_mut().enumerate() {
                let w = r.bipolar();
                a += ka * (w - a);
                h += kh * (w - h);
                let t = i as f64 / sr;
                *o = 4.0 * a + 0.15 * (w - h) + 0.04 * (2.0 * PI * 60.0 * t).sin() + 0.02 * (2.0 * PI * 180.0 * t).sin();
            }
        }
        Noise::Traffic => {
            // Band noise whose level follows passing vehicles (2–5 s swells at random times).
            let mut cars = Vec::new();
            let mut t = r.next_f64() * 1.5;
            let dur = n as f64 / sr;
            while t < dur + 3.0 {
                cars.push((t, 0.8 + r.next_f64() * 2.2, 0.3 + r.next_f64()));
                t += 0.8 + r.next_f64() * 3.0;
            }
            let (mut lo, mut hi) = (0.0, 0.0);
            let (kl, kh) = (lp_coef(1500.0, sr), lp_coef(90.0, sr));
            for (i, o) in out.iter_mut().enumerate() {
                let w = r.bipolar();
                lo += kl * (w - lo);
                hi += kh * (lo - hi);
                let t = i as f64 / sr;
                let env: f64 = 0.05 + cars.iter().map(|(c, w, g)| g * (-((t - c) / w).powi(2)).exp()).sum::<f64>();
                *o = (lo - hi) * env;
            }
        }
        Noise::Keyboard => {
            // Typing bursts with pauses; each key is a short broadband click plus a damped knock.
            let mut keys = Vec::new();
            let mut t = r.next_f64() * 0.5;
            let dur = n as f64 / sr;
            while t < dur {
                let burst_end = t + 1.0 + r.next_f64() * 3.0;
                while t < burst_end.min(dur) {
                    keys.push((t, 0.4 + r.next_f64(), 1200.0 + r.next_f64() * 2500.0));
                    t += 0.06 + r.next_f64() * 0.2;
                }
                t += 0.5 + r.next_f64() * 2.0;
            }
            for (t0, g, f) in keys {
                let start = (t0 * sr) as usize;
                let len = (0.03 * sr) as usize;
                for k in 0..len {
                    let Some(o) = out.get_mut(start + k) else { break };
                    let tt = k as f64 / sr;
                    let click = r.bipolar() * (-tt / 0.002).exp();
                    let knock = (2.0 * PI * f * tt).sin() * (-tt / 0.008).exp() * 0.6;
                    *o += g * (click + knock);
                }
            }
        }
        Noise::Wind => {
            // Low-passed noise with gusts: a slow envelope (four slow sines, random phases) drives
            // level and brightness; exp() makes lulls deep (about 40 dB below the gusts).
            let ph: Vec<(f64, f64)> = [0.11, 0.23, 0.41, 0.67].iter().map(|f| (*f, r.next_f64() * 2.0 * PI)).collect();
            let mut s = 0.0;
            for (i, o) in out.iter_mut().enumerate() {
                let t = i as f64 / sr;
                let g = ph.iter().map(|(f, p)| (2.0 * PI * f * t + p).sin()).sum::<f64>() / 4.0;
                let gust = (2.3 * (g * 2.0 - 1.0)).exp().min(1.0);
                s += lp_coef(150.0 + 900.0 * gust, sr) * (r.bipolar() - s);
                *o = s * gust;
            }
        }
    }
    let rms = (out.iter().map(|v| v * v).sum::<f64>() / n.max(1) as f64).sqrt();
    let k = if rms > 0.0 { 1.0 / rms } else { 0.0 };
    out.iter().map(|v| (v * k) as f32).collect()
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// `clean + noise` with the noise scaled so the whole-signal SNR is `snr_db`.
pub fn mix_at_snr(clean: &[f32], noise: &[f32], snr_db: f64) -> Vec<f32> {
    let (ps, pn) = (rms(clean), rms(noise));
    let k = if pn > 0.0 { ps / pn / 10f64.powf(snr_db / 20.0) } else { 0.0 };
    clean.iter().zip(noise.iter().chain(std::iter::repeat(&0.0))).map(|(s, n)| (f64::from(*s) + f64::from(*n) * k) as f32).collect()
}

/// Scale-invariant signal-to-distortion ratio (dB) of `est` against `reference` (Le Roux et
/// al. 2019): both made zero-mean, `est` projected onto `reference`. Uses the common length.
pub fn si_sdr(est: &[f32], reference: &[f32]) -> f64 {
    let n = est.len().min(reference.len());
    if n == 0 {
        return f64::NEG_INFINITY;
    }
    let me = est.iter().take(n).map(|v| f64::from(*v)).sum::<f64>() / n as f64;
    let mr = reference.iter().take(n).map(|v| f64::from(*v)).sum::<f64>() / n as f64;
    let (mut dot, mut rr) = (0.0, 0.0);
    for (e, r) in est.iter().zip(reference).take(n) {
        let (e, r) = (f64::from(*e) - me, f64::from(*r) - mr);
        dot += e * r;
        rr += r * r;
    }
    if rr <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let a = dot / rr;
    let (mut target, mut resid) = (0.0, 0.0);
    for (e, r) in est.iter().zip(reference).take(n) {
        let (e, r) = (f64::from(*e) - me, f64::from(*r) - mr);
        let t = a * r;
        target += t * t;
        resid += (e - t) * (e - t);
    }
    10.0 * (target / resid.max(1e-30)).log10()
}

/// 2× upsampling with a 63-tap windowed-sinc low-pass (cut-off at the old Nyquist).
pub fn upsample2(x: &[f32]) -> Vec<f32> {
    const TAPS: usize = 63;
    let half = TAPS / 2;
    let h: Vec<f64> = (0..TAPS)
        .map(|k| {
            let m = k as f64 - half as f64;
            let sinc = if m == 0.0 { 1.0 } else { (PI * m / 2.0).sin() / (PI * m / 2.0) };
            let w = 0.42 - 0.5 * (2.0 * PI * k as f64 / (TAPS - 1) as f64).cos() + 0.08 * (4.0 * PI * k as f64 / (TAPS - 1) as f64).cos();
            sinc * w
        })
        .collect();
    let stuffed: Vec<f64> = x.iter().flat_map(|v| [f64::from(*v), 0.0]).collect();
    (0..stuffed.len())
        .map(|i| {
            let mut acc = 0.0;
            for (k, hk) in h.iter().enumerate() {
                if let Some(s) = (i + k).checked_sub(half).and_then(|j| stuffed.get(j)) {
                    acc += s * hk;
                }
            }
            acc as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_normalised_and_finite() {
        for kind in Noise::ALL {
            let a = noise(kind, 48_000, 48_000, 7);
            assert_eq!(a, noise(kind, 48_000, 48_000, 7), "{kind:?}");
            assert!((rms(&a) - 1.0).abs() < 1e-3, "{kind:?}");
            assert!(a.iter().all(|v| v.is_finite()));
            assert_ne!(a, noise(kind, 48_000, 48_000, 8));
        }
        assert!(noise(Noise::Fan, 0, 48_000, 1).is_empty());
        assert_eq!(noise(Noise::Wind, 10, 0, 1).len(), 10);
    }

    /// The non-steady kinds really change level over time; the steady ones don't.
    #[test]
    fn steady_and_non_steady() {
        for kind in Noise::ALL {
            let x = noise(kind, 48_000 * 8, 48_000, 3);
            let levels: Vec<f64> = x.chunks(24_000).map(rms).collect();
            let (lo, hi) = levels.iter().fold((f64::MAX, 0.0f64), |(l, h), v| (l.min(*v), h.max(*v)));
            let spread_db = 20.0 * (hi / lo.max(1e-9)).log10();
            if kind.steady() {
                assert!(spread_db < 3.0, "{kind:?} varies {spread_db:.1} dB");
            } else {
                assert!(spread_db > 10.0, "{kind:?} varies only {spread_db:.1} dB");
            }
        }
    }

    #[test]
    fn snr_mixing_and_si_sdr() {
        let s: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin()).collect();
        let n = noise(Noise::Fan, 48_000, 48_000, 1);
        let m = mix_at_snr(&s, &n, 10.0);
        let resid: Vec<f32> = m.iter().zip(&s).map(|(a, b)| a - b).collect();
        assert!((20.0 * (rms(&s) / rms(&resid)).log10() - 10.0).abs() < 0.01);
        // SI-SDR ignores scale, sees the added noise.
        let scaled: Vec<f32> = s.iter().map(|v| v * 0.3).collect();
        assert!(si_sdr(&scaled, &s) > 100.0);
        assert!((si_sdr(&m, &s) - 10.0).abs() < 0.5);
        assert_eq!(si_sdr(&[], &s), f64::NEG_INFINITY);
        assert_eq!(si_sdr(&s, &[0.0; 10]), f64::NEG_INFINITY);
    }

    #[test]
    fn upsampling_keeps_a_tone_and_its_level() {
        let x: Vec<f32> = (0..24_000).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 24_000.0).sin()).collect();
        let y = upsample2(&x);
        assert_eq!(y.len(), 48_000);
        let reference: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin()).collect();
        // (the filter is symmetric, so no delay)
        assert!(si_sdr(&y[100..47_900], &reference[100..47_900]) > 40.0);
        assert!((rms(&y[100..47_900]) - rms(&reference[100..47_900])).abs() < 0.01);
    }
}
