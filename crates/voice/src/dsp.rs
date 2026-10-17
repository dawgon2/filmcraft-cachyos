//! DeepFilterNet's signal path around the network: the Vorbis-window STFT, ERB band layout, the
//! two normalised input features, ERB gains and the deep-filter operation. Ported from the
//! MIT / Apache-2.0 `libDF` (DeepFilterNet, Hendrik Schröter) so the network sees exactly the
//! features it was trained on.

use crate::fft::MixedFft;

/// One complex STFT bin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct C32 {
    pub re: f32,
    pub im: f32,
}

impl C32 {
    pub const ZERO: C32 = C32 { re: 0.0, im: 0.0 };

    pub fn norm(self) -> f32 {
        self.re.hypot(self.im)
    }
}

impl std::ops::Mul for C32 {
    type Output = C32;

    fn mul(self, o: C32) -> C32 {
        C32 { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
}

/// The DeepFilterNet3 model's signal settings (its `config.ini [df]` section).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DfParams {
    pub sample_rate: usize,
    pub fft_size: usize,
    pub hop_size: usize,
    pub nb_erb: usize,
    pub nb_df: usize,
    pub min_nb_erb_freqs: usize,
    /// Feature normalisation time constant, seconds.
    pub norm_tau: f32,
    pub df_order: usize,
    pub df_lookahead: usize,
    pub conv_lookahead: usize,
    pub lsnr_min: f32,
    pub lsnr_max: f32,
}

impl DfParams {
    /// DeepFilterNet3 as released (`models/DeepFilterNet3.zip`).
    pub const DFN3: DfParams = DfParams {
        sample_rate: 48_000,
        fft_size: 960,
        hop_size: 480,
        nb_erb: 32,
        nb_df: 96,
        min_nb_erb_freqs: 2,
        norm_tau: 1.0,
        df_order: 5,
        df_lookahead: 2,
        conv_lookahead: 2,
        lsnr_min: -15.0,
        lsnr_max: 35.0,
    };

    pub fn freq_bins(&self) -> usize {
        self.fft_size / 2 + 1
    }

    /// Samples from input to output: the STFT overlap plus the model's frame lookahead.
    pub fn latency_samples(&self) -> usize {
        (self.fft_size - self.hop_size) + self.conv_lookahead.max(self.df_lookahead) * self.hop_size
    }
}

fn freq2erb(hz: f32) -> f32 {
    9.265 * (hz / (24.7 * 9.265)).ln_1p()
}

fn erb2freq(erb: f32) -> f32 {
    24.7 * 9.265 * ((erb / 9.265).exp() - 1.0)
}

/// Bins per ERB band (`libDF::erb_fb`): `nb_bands` widths summing to `fft_size / 2 + 1`, each at
/// least `min_nb_freqs`. `None` for settings that can't make such a layout.
pub fn erb_widths(sample_rate: usize, fft_size: usize, nb_bands: usize, min_nb_freqs: usize) -> Option<Vec<usize>> {
    let bins = fft_size.checked_div(2)?.checked_add(1)?;
    if sample_rate == 0 || fft_size < 2 || nb_bands == 0 || nb_bands.checked_mul(min_nb_freqs.max(1))? > bins {
        return None;
    }
    let freq_width = sample_rate as f32 / fft_size as f32;
    let erb_high = freq2erb((sample_rate / 2) as f32);
    let step = erb_high / nb_bands as f32;
    let mut widths = vec![0usize; nb_bands];
    let (mut prev, mut over) = (0i64, 0i64);
    let min = i64::try_from(min_nb_freqs).ok()?;
    for (i, w) in widths.iter_mut().enumerate() {
        let fb = (erb2freq((i + 1) as f32 * step) / freq_width).round() as i64;
        let mut nb = fb - prev - over;
        if nb < min {
            over = min - nb;
            nb = min;
        } else {
            over = 0;
        }
        *w = usize::try_from(nb).ok()?;
        prev = fb;
    }
    let last = widths.last_mut()?;
    *last += 1;
    let sum: usize = widths.iter().sum();
    let too_large = sum.checked_sub(bins)?;
    let last = widths.last_mut()?;
    *last = last.checked_sub(too_large).filter(|w| *w > 0)?;
    Some(widths)
}

/// The Vorbis power-complementary window `sin(π/2 · sin²(π(n + ½)/(N/2)))`.
pub fn vorbis_window(n: usize) -> Vec<f32> {
    let half = (n / 2).max(1) as f64;
    (0..n)
        .map(|i| {
            let s = (0.5 * std::f64::consts::PI * (i as f64 + 0.5) / half).sin();
            (0.5 * std::f64::consts::PI * s * s).sin() as f32
        })
        .collect()
}

/// Exponential smoothing factor of the feature normalisation (`libDF::calc_norm_alpha`):
/// `exp(−hop / (sr · τ))`, rounded to the fewest decimals that keep it below 1.
pub fn norm_alpha(sample_rate: usize, hop_size: usize, tau: f32) -> f32 {
    let dt = hop_size as f32 / sample_rate.max(1) as f32;
    let alpha = if tau > 0.0 { (-dt / tau).exp() } else { 0.0 };
    if !alpha.is_finite() {
        return 0.0;
    }
    for precision in 3..10 {
        let p = 10f32.powi(precision);
        let a = (alpha * p).round() / p;
        if a < 1.0 {
            return a;
        }
    }
    alpha.min(0.999_999)
}

/// One channel's streaming STFT: `hop` samples in → `fft/2 + 1` bins out, and back, with
/// libDF's scaling (window normalisation applied in analysis only), so analysis followed by
/// synthesis reconstructs the input delayed by `fft − hop` samples.
pub struct Stft {
    fft: MixedFft,
    window: Vec<f32>,
    wnorm: f32,
    hop: usize,
    analysis_mem: Vec<f32>,
    synthesis_mem: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Stft {
    /// `None` unless `2 · hop ≤ fft` and the FFT size is supported.
    pub fn new(fft_size: usize, hop_size: usize) -> Option<Stft> {
        if hop_size == 0 || hop_size.checked_mul(2)? > fft_size {
            return None;
        }
        let fft = MixedFft::new(fft_size)?;
        let wnorm = (2 * hop_size) as f32 / (fft_size as f32 * fft_size as f32);
        Some(Stft {
            fft,
            window: vorbis_window(fft_size),
            wnorm,
            hop: hop_size,
            analysis_mem: vec![0.0; fft_size - hop_size],
            synthesis_mem: vec![0.0; fft_size - hop_size],
            re: vec![0.0; fft_size],
            im: vec![0.0; fft_size],
        })
    }

    pub fn reset(&mut self) {
        self.analysis_mem.fill(0.0);
        self.synthesis_mem.fill(0.0);
    }

    pub fn bins(&self) -> usize {
        self.window.len() / 2 + 1
    }

    /// Analyse one hop. `false` (nothing changed) if `input` isn't `hop` samples or `out` is
    /// shorter than `bins()`.
    pub fn analyse(&mut self, input: &[f32], out: &mut [C32]) -> bool {
        let (n, hop, bins) = (self.window.len(), self.hop, self.bins());
        if input.len() != hop || out.len() < bins {
            return false;
        }
        let keep = n - hop;
        for i in 0..keep {
            self.re[i] = self.analysis_mem[i] * self.window[i];
        }
        for (i, x) in input.iter().enumerate() {
            self.re[keep + i] = x * self.window[keep + i];
        }
        self.im.fill(0.0);
        // Shift the memory by one hop and append the new input.
        self.analysis_mem.rotate_left(hop.min(keep));
        let tail = keep.saturating_sub(hop);
        for (m, x) in self.analysis_mem[tail..].iter_mut().zip(input.iter().skip(hop.saturating_sub(keep))) {
            *m = *x;
        }
        if !self.fft.forward(&mut self.re, &mut self.im) {
            return false;
        }
        for (k, o) in out.iter_mut().take(bins).enumerate() {
            *o = C32 { re: self.re[k] * self.wnorm, im: self.im[k] * self.wnorm };
        }
        true
    }

    /// Synthesise one hop from `bins()` bins (overlap-add). `false` (nothing changed) on wrong
    /// lengths.
    pub fn synthesise(&mut self, spec: &[C32], out: &mut [f32]) -> bool {
        let (n, hop, bins) = (self.window.len(), self.hop, self.bins());
        if spec.len() < bins || out.len() != hop {
            return false;
        }
        // Hermitian extension; the imaginary parts of DC / Nyquist only affect the (dropped)
        // imaginary output.
        for k in 0..bins {
            self.re[k] = spec[k].re;
            self.im[k] = spec[k].im;
        }
        for k in bins..n {
            self.re[k] = spec[n - k].re;
            self.im[k] = -spec[n - k].im;
        }
        if !self.fft.inverse(&mut self.re, &mut self.im) {
            return false;
        }
        for (x, w) in self.re.iter_mut().zip(&self.window) {
            *x *= w;
        }
        let keep = n - hop;
        for i in 0..hop {
            out[i] = self.re[i] + self.synthesis_mem.get(i).copied().unwrap_or(0.0);
        }
        // Next memory: the old memory shifted by a hop plus this frame's tail.
        self.synthesis_mem.rotate_left(hop.min(keep));
        let split = keep.saturating_sub(hop);
        for i in 0..keep {
            let x = self.re[hop + i];
            if i < split {
                self.synthesis_mem[i] += x;
            } else {
                self.synthesis_mem[i] = x;
            }
        }
        true
    }
}

/// The normalisation state behind the two input features (`libDF::DFState` norm states).
pub struct Features {
    alpha: f32,
    mean_norm: Vec<f32>,
    unit_norm: Vec<f32>,
}

impl Features {
    pub fn new(nb_erb: usize, nb_df: usize, alpha: f32) -> Features {
        let mut f = Features { alpha, mean_norm: vec![0.0; nb_erb], unit_norm: vec![0.0; nb_df] };
        f.reset();
        f
    }

    /// Back to the starting norm states, in place: no allocation, so it may run on the audio thread.
    pub fn reset(&mut self) {
        fn ramp(v: &mut [f32], a: f32, b: f32) {
            let n = v.len();
            let step = if n > 1 { (b - a) / (n - 1) as f32 } else { 0.0 };
            for (i, x) in v.iter_mut().enumerate() {
                *x = a + i as f32 * step;
            }
        }
        ramp(&mut self.mean_norm, -60.0, -90.0);
        ramp(&mut self.unit_norm, 0.001, 0.0001);
    }

    /// ERB feature: band power in dB, minus its running mean, / 40. `false` on wrong lengths.
    pub fn erb(&mut self, spec: &[C32], widths: &[usize], out: &mut [f32]) -> bool {
        if out.len() != self.mean_norm.len()
            || widths.len() != out.len()
            || widths.iter().try_fold(0usize, |a, w| a.checked_add(*w)).is_none_or(|t| t > spec.len())
        {
            return false;
        }
        let a = self.alpha;
        let mut start = 0;
        for ((w, o), s) in widths.iter().zip(out.iter_mut()).zip(self.mean_norm.iter_mut()) {
            let band = &spec[start..start + w];
            let p = band.iter().map(|x| x.re * x.re + x.im * x.im).sum::<f32>() / (*w).max(1) as f32;
            let db = (p + 1e-10).log10() * 10.0;
            // Hostile input (Inf / NaN) must not latch into the running mean.
            let db = if db.is_finite() {
                db
            } else if db > 0.0 {
                400.0
            } else {
                -100.0
            };
            *s = db * (1.0 - a) + *s * a;
            *o = (db - *s) / 40.0;
            start += w;
        }
        true
    }

    /// Complex feature: the first `nb_df` bins divided by the square root of their running
    /// magnitude. `false` on wrong lengths.
    pub fn cplx(&mut self, spec: &[C32], out: &mut [C32]) -> bool {
        let n = self.unit_norm.len();
        if spec.len() < n || out.len() != n {
            return false;
        }
        let a = self.alpha;
        for ((x, s), o) in spec.iter().zip(self.unit_norm.iter_mut()).zip(out.iter_mut()) {
            let finite = x.re.is_finite() && x.im.is_finite();
            let m = if finite { x.norm() } else { 0.0 };
            *s = m.min(f32::MAX / 2.0) * (1.0 - a) + *s * a;
            let d = s.sqrt();
            *o = if finite && d > 0.0 { C32 { re: x.re / d, im: x.im / d } } else { C32::ZERO };
        }
        true
    }
}

/// Multiply every bin by its ERB band's gain. `false` (untouched) on wrong lengths.
pub fn apply_band_gains(spec: &mut [C32], gains: &[f32], widths: &[usize]) -> bool {
    if gains.len() != widths.len() || widths.iter().try_fold(0usize, |a, w| a.checked_add(*w)).is_none_or(|t| t > spec.len()) {
        return false;
    }
    let mut start = 0;
    for (w, g) in widths.iter().zip(gains) {
        for x in &mut spec[start..start + w] {
            x.re *= g;
            x.im *= g;
        }
        start += w;
    }
    true
}

/// Deep filtering: bin `f < nb_df` of `out` becomes `Σ_i frames[i][f] · coefs[f·order + i]` over
/// the last `order` noisy frames (oldest first); bins from `nb_df` up are left alone. `false`
/// (untouched) on wrong lengths.
pub fn deep_filter(frames: &[&[C32]], coefs: &[C32], nb_df: usize, out: &mut [C32]) -> bool {
    let order = frames.len();
    if order == 0 || out.len() < nb_df || frames.iter().any(|f| f.len() < nb_df) || coefs.len() != nb_df.saturating_mul(order) {
        return false;
    }
    for (f, o) in out.iter_mut().take(nb_df).enumerate() {
        let mut acc = C32::ZERO;
        for (i, fr) in frames.iter().enumerate() {
            let p = fr[f] * coefs[f * order + i];
            acc.re += p.re;
            acc.im += p.im;
        }
        *o = acc;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: DfParams = DfParams::DFN3;

    #[test]
    fn features_reset_restores_the_start_in_place() {
        let alpha = norm_alpha(P.sample_rate, P.hop_size, P.norm_tau);
        let fresh = Features::new(4, 3, alpha);
        let mut f = Features::new(4, 3, alpha);
        let spec = vec![C32 { re: 3.0, im: -1.0 }; 8];
        let mut out = vec![0.0; 4];
        assert!(f.erb(&spec, &[2, 2, 2, 2], &mut out));
        assert_ne!(f.mean_norm, fresh.mean_norm, "the running mean moved");
        let (m, u) = (f.mean_norm.as_ptr(), f.unit_norm.as_ptr());
        f.reset();
        assert_eq!((f.mean_norm.clone(), f.unit_norm.clone()), (fresh.mean_norm.clone(), fresh.unit_norm.clone()));
        assert_eq!((f.mean_norm.as_ptr(), f.unit_norm.as_ptr()), (m, u), "no new buffers (audio thread)");
        // one band / empty: no division by zero
        let mut one = Features::new(1, 0, alpha);
        one.reset();
        assert_eq!(one.mean_norm, vec![-60.0]);
    }

    #[test]
    fn dfn3_erb_layout() {
        let w = erb_widths(P.sample_rate, P.fft_size, P.nb_erb, P.min_nb_erb_freqs).unwrap();
        assert_eq!(w.len(), 32);
        assert_eq!(w.iter().sum::<usize>(), 481);
        assert!(w.iter().all(|&b| b >= 2));
        // Low bands are clamped to the minimum width, high bands widen.
        assert_eq!(&w[..4], &[2, 2, 2, 2]);
        assert!(w[31] > w[20]);
    }

    #[test]
    fn hostile_erb_settings_give_none() {
        assert!(erb_widths(0, 960, 32, 2).is_none());
        assert!(erb_widths(48_000, 0, 32, 2).is_none());
        assert!(erb_widths(48_000, 960, 0, 2).is_none());
        assert!(erb_widths(48_000, 64, 32, 2).is_none());
        assert!(erb_widths(48_000, 960, usize::MAX, usize::MAX).is_none());
    }

    #[test]
    fn norm_alpha_matches_libdf() {
        // exp(−0.01) = 0.99005 → 0.99 at 3 decimals.
        assert_eq!(norm_alpha(48_000, 480, 1.0), 0.99);
        assert_eq!(norm_alpha(0, 480, 0.0), 0.0);
        assert!(norm_alpha(48_000, 1, 1e9) < 1.0);
    }

    #[test]
    fn stft_reconstructs_the_input_delayed_by_one_hop() {
        let mut st = Stft::new(960, 480).unwrap();
        let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.013).sin() * 0.5 + ((i * 7919) % 211) as f32 / 1000.0).collect();
        let mut y = Vec::new();
        let mut spec = vec![C32::ZERO; st.bins()];
        let mut out = vec![0.0; 480];
        for hop in x.as_chunks::<480>().0 {
            assert!(st.analyse(hop, &mut spec));
            assert!(st.synthesise(&spec, &mut out));
            y.extend_from_slice(&out);
        }
        for i in 960..x.len() {
            assert!((y[i] - x[i - 480]).abs() < 1e-4, "sample {i}: {} vs {}", y[i], x[i - 480]);
        }
    }

    #[test]
    fn stft_refuses_wrong_lengths() {
        assert!(Stft::new(960, 0).is_none());
        assert!(Stft::new(960, 481).is_none());
        assert!(Stft::new(4096 * 64, 480).is_none());
        let mut st = Stft::new(960, 480).unwrap();
        let mut spec = vec![C32::ZERO; 481];
        assert!(!st.analyse(&[0.0; 479], &mut spec));
        assert!(!st.analyse(&[0.0; 480], &mut spec[..480]));
        assert!(!st.synthesise(&spec[..10], &mut [0.0; 480]));
        assert!(!st.synthesise(&spec, &mut [0.0; 10]));
    }

    #[test]
    fn features_are_finite_on_silence_and_hostile_values() {
        let w = erb_widths(48_000, 960, 32, 2).unwrap();
        let mut f = Features::new(32, 96, 0.99);
        let mut erb = vec![0.0; 32];
        let mut cp = vec![C32::ZERO; 96];
        let nan = C32 { re: f32::NAN, im: f32::INFINITY };
        for spec in [vec![C32::ZERO; 481], vec![C32 { re: 1e30, im: -1e30 }; 481], vec![nan; 481], vec![C32 { re: 0.1, im: 0.0 }; 481]] {
            assert!(f.erb(&spec, &w, &mut erb));
            assert!(f.cplx(&spec, &mut cp));
            assert!(erb.iter().all(|v| v.is_finite()));
            assert!(cp.iter().all(|v| v.re.is_finite() && v.im.is_finite()));
        }
        assert!(!f.erb(&[C32::ZERO; 10], &w, &mut erb));
        assert!(!f.cplx(&[C32::ZERO; 10], &mut cp));
    }

    #[test]
    fn band_gains_and_deep_filter() {
        let w = [2usize, 3];
        let mut spec = vec![C32 { re: 1.0, im: 1.0 }; 5];
        assert!(apply_band_gains(&mut spec, &[0.5, 2.0], &w));
        assert_eq!(spec[1], C32 { re: 0.5, im: 0.5 });
        assert_eq!(spec[4], C32 { re: 2.0, im: 2.0 });
        assert!(!apply_band_gains(&mut spec[..4], &[0.5, 2.0], &w));

        // A unit tap on the newest frame copies it; other bins stay.
        let a = vec![C32 { re: 1.0, im: 0.0 }; 4];
        let b = vec![C32 { re: 0.0, im: 3.0 }; 4];
        let mut coefs = vec![C32::ZERO; 2 * 2];
        coefs[1] = C32 { re: 1.0, im: 0.0 };
        coefs[3] = C32 { re: 0.0, im: 1.0 };
        let mut out = vec![C32 { re: 9.0, im: 9.0 }; 4];
        assert!(deep_filter(&[&a, &b], &coefs, 2, &mut out));
        assert_eq!(out[0], C32 { re: 0.0, im: 3.0 });
        assert_eq!(out[1], C32 { re: -3.0, im: 0.0 });
        assert_eq!(out[2], C32 { re: 9.0, im: 9.0 });
        assert!(!deep_filter(&[], &coefs, 2, &mut out));
        assert!(!deep_filter(&[&a, &b], &coefs[..3], 2, &mut out));
    }

    #[test]
    fn latency_is_one_hop_plus_lookahead() {
        assert_eq!(P.latency_samples(), 480 + 2 * 480);
    }
}
