//! Choosing the next token from the logits (AI-PLAN AI1.1f): greedy or temperature sampling, and
//! the repeat penalty.
//!
//! Greedy and the repeat penalty match candle 0.9.2 exactly (`generation::LogitsProcessor` with
//! `Sampling::ArgMax`: the first maximum; `utils::apply_repeat_penalty`, MIT / Apache-2.0, © the
//! candle authors). Temperature sampling draws from the same softmax with our own seeded
//! generator, so it is deterministic per seed but not candle's random sequence (greedy is the
//! default and what the explainers are evaluated with).

use std::collections::HashSet;

/// Divides positive logits of tokens seen in `context` by `penalty` and multiplies negative ones
/// (each distinct token once), as candle's `apply_repeat_penalty`. Out-of-range ids are ignored.
pub fn apply_repeat_penalty(logits: &mut [f32], penalty: f32, context: &[u32]) {
    let mut seen = HashSet::new();
    for &t in context {
        if !seen.insert(t) {
            continue;
        }
        if let Some(l) = usize::try_from(t).ok().and_then(|i| logits.get_mut(i)) {
            if *l >= 0.0 {
                *l /= penalty;
            } else {
                *l *= penalty;
            }
        }
    }
}

/// The index of the first largest logit (NaN never wins); `None` for an empty or all-NaN slice.
pub fn argmax(logits: &[f32]) -> Option<u32> {
    let mut best: Option<(usize, f32)> = None;
    for (i, &v) in logits.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        if best.is_none_or(|(_, b)| v > b) {
            best = Some((i, v));
        }
    }
    best.and_then(|(i, _)| u32::try_from(i).ok())
}

/// Greedy below a temperature of 1e-7 (as candle), else softmax sampling with a seeded generator.
pub struct Sampler {
    temperature: Option<f64>,
    state: u64,
}

impl Sampler {
    pub fn new(seed: u64, temperature: Option<f64>) -> Sampler {
        let temperature = temperature.filter(|t| t.is_finite() && *t >= 1e-7);
        Sampler { temperature, state: seed }
    }

    /// splitmix64: small, well distributed, no dependency.
    fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The next token; `None` when the logits are empty or all NaN.
    pub fn sample(&mut self, logits: &[f32]) -> Option<u32> {
        let Some(t) = self.temperature else {
            return argmax(logits);
        };
        let max = logits.iter().copied().filter(|v| !v.is_nan()).fold(f32::NEG_INFINITY, f32::max);
        if !max.is_finite() {
            return argmax(logits);
        }
        let weights: Vec<f64> = logits.iter().map(|&v| if v.is_nan() { 0.0 } else { ((f64::from(v) - f64::from(max)) / t).exp() }).collect();
        let total: f64 = weights.iter().sum();
        if !(total > 0.0 && total.is_finite()) {
            return argmax(logits);
        }
        let mut r = self.next_f64() * total;
        let mut last = None;
        for (i, w) in weights.iter().enumerate() {
            if *w > 0.0 {
                last = Some(i);
                if r < *w {
                    return u32::try_from(i).ok();
                }
                r -= w;
            }
        }
        last.and_then(|i| u32::try_from(i).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_penalty_matches_candle() {
        let base = vec![2.0f32, -1.0, 0.5, 0.0, 3.0];
        let ctx = [0u32, 1, 1, 3, 99];
        let mut ours = base.clone();
        apply_repeat_penalty(&mut ours, 1.3, &ctx);
        let dev = candle_core::Device::Cpu;
        let t = candle_core::Tensor::from_vec(base, 5, &dev).unwrap();
        let want = candle_transformers::utils::apply_repeat_penalty(&t, 1.3, &ctx).unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(ours, want);
    }

    #[test]
    fn greedy_matches_candle_including_ties() {
        let logits = vec![0.1f32, 3.0, -2.0, 3.0, 1.0];
        let mut lp = candle_transformers::generation::LogitsProcessor::new(1, None, None);
        let t = candle_core::Tensor::from_vec(logits.clone(), 5, &candle_core::Device::Cpu).unwrap();
        assert_eq!(Sampler::new(1, None).sample(&logits), Some(lp.sample(&t).unwrap()));
        assert_eq!(Sampler::new(1, Some(0.0)).sample(&logits), Some(1));
    }

    #[test]
    fn sampling_is_seeded_and_follows_the_distribution() {
        let logits = vec![0.0f32, (3f32).ln(), f32::NAN, f32::NEG_INFINITY];
        let mut a = Sampler::new(7, Some(1.0));
        let mut b = Sampler::new(7, Some(1.0));
        let draws: Vec<u32> = (0..4000).map(|_| a.sample(&logits).unwrap()).collect();
        assert_eq!(draws, (0..4000).map(|_| b.sample(&logits).unwrap()).collect::<Vec<_>>());
        let ones = draws.iter().filter(|&&d| d == 1).count() as f64 / 4000.0;
        assert!((ones - 0.75).abs() < 0.03, "{ones}");
        assert!(draws.iter().all(|&d| d < 2), "NaN and −∞ are never drawn");
    }

    #[test]
    fn degenerate_logits_never_panic() {
        assert_eq!(argmax(&[]), None);
        assert_eq!(argmax(&[f32::NAN, f32::NAN]), None);
        assert_eq!(Sampler::new(0, Some(0.8)).sample(&[]), None);
        assert_eq!(Sampler::new(0, Some(0.8)).sample(&[f32::NEG_INFINITY, f32::NEG_INFINITY]), Some(0));
        assert_eq!(Sampler::new(0, Some(1e-300)).sample(&[1.0, 2.0]), Some(1));
        let mut l = vec![1.0f32];
        apply_repeat_penalty(&mut l, 2.0, &[u32::MAX, 0]);
        assert_eq!(l, vec![0.5]);
    }
}
