//! Mixed-radix complex FFT for sizes `2^p × m` (`m` odd and small), allocation-free after
//! planning. DeepFilterNet works on 960-point frames (`2^6 × 15`), which the power-of-two FFT in
//! `filmcraft-audio-dsp` can't do. The size splits into `m` radix-2 FFTs of `2^p` points, a
//! twiddle step and `2^p` direct DFTs of `m` points (`m ≤ MAX_ODD`, so that part stays cheap).

use std::f64::consts::PI;

/// Largest odd factor accepted (the direct DFTs cost `n × m`).
pub const MAX_ODD: usize = 63;
/// Largest transform accepted.
pub const MAX_LEN: usize = 1 << 16;

/// A planned FFT of fixed size `n = p2 × m`.
#[derive(Clone, Debug)]
pub struct MixedFft {
    n: usize,
    /// Power-of-two part.
    p2: usize,
    /// Odd part.
    m: usize,
    /// e^{-2πik/n} for k in 0..n.
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
    /// Bit reversal of 0..p2.
    rev: Vec<usize>,
    s_re: Vec<f32>,
    s_im: Vec<f32>,
}

impl MixedFft {
    /// Plan an FFT of `n` points; `None` unless `n = 2^p × m` with `m` odd, `m ≤ MAX_ODD`,
    /// `2 ≤ n ≤ MAX_LEN`.
    pub fn new(n: usize) -> Option<MixedFft> {
        if !(2..=MAX_LEN).contains(&n) {
            return None;
        }
        let p2 = 1usize << n.trailing_zeros();
        let m = n / p2;
        if m > MAX_ODD {
            return None;
        }
        let bits = p2.trailing_zeros();
        let rev = (0..p2).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        let (tw_re, tw_im) = (0..n)
            .map(|k| {
                let a = -2.0 * PI * k as f64 / n as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .unzip();
        Some(MixedFft { n, p2, m, tw_re, tw_im, rev, s_re: vec![0.0; n], s_im: vec![0.0; n] })
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Forward transform (e^{-iωt}), unscaled, in place. `false` (data untouched) if a slice is
    /// shorter than `n`; only the first `n` values are used.
    pub fn forward(&mut self, re: &mut [f32], im: &mut [f32]) -> bool {
        self.run(re, im, false)
    }

    /// Inverse transform (e^{+iωt}), unscaled (divide by `n` yourself).
    pub fn inverse(&mut self, re: &mut [f32], im: &mut [f32]) -> bool {
        self.run(re, im, true)
    }

    fn run(&mut self, re: &mut [f32], im: &mut [f32], inverse: bool) -> bool {
        let (n, p2, m) = (self.n, self.p2, self.m);
        let (Some(re), Some(im)) = (re.get_mut(..n), im.get_mut(..n)) else {
            return false;
        };
        // e^{±2πi k/n}: the inverse uses the conjugate twiddles.
        let sign = if inverse { -1.0 } else { 1.0 };
        // 1. Gather x[q·m + r] into block r (p2 points each) in bit-reversed order, then radix-2.
        for r in 0..m {
            for q in 0..p2 {
                let dst = r * p2 + self.rev[q];
                self.s_re[dst] = re[q * m + r];
                self.s_im[dst] = im[q * m + r];
            }
            let (br, bi) = (&mut self.s_re[r * p2..(r + 1) * p2], &mut self.s_im[r * p2..(r + 1) * p2]);
            let mut len = 2;
            while len <= p2 {
                let half = len / 2;
                // W_len^k = W_n^{k·n/len}.
                let stride = n / len;
                let mut start = 0;
                while start < p2 {
                    for k in 0..half {
                        let wr = self.tw_re[k * stride];
                        let wi = sign * self.tw_im[k * stride];
                        let (a, b) = (start + k, start + k + half);
                        let xr = br[b] * wr - bi[b] * wi;
                        let xi = br[b] * wi + bi[b] * wr;
                        br[b] = br[a] - xr;
                        bi[b] = bi[a] - xi;
                        br[a] += xr;
                        bi[a] += xi;
                    }
                    start += len;
                }
                len *= 2;
            }
        }
        if m == 1 {
            re.copy_from_slice(&self.s_re);
            im.copy_from_slice(&self.s_im);
            return true;
        }
        // 2. Twiddle A[r][k2] by W_n^{r·k2}, 3. m-point DFTs: X[k2 + p2·k1] = Σ_r A[r][k2]·W_m^{r·k1}.
        for r in 1..m {
            for k2 in 0..p2 {
                let t = (r * k2) % n;
                let (wr, wi) = (self.tw_re[t], sign * self.tw_im[t]);
                let i = r * p2 + k2;
                let (xr, xi) = (self.s_re[i], self.s_im[i]);
                self.s_re[i] = xr * wr - xi * wi;
                self.s_im[i] = xr * wi + xi * wr;
            }
        }
        for k2 in 0..p2 {
            for k1 in 0..m {
                let (mut ar, mut ai) = (0.0f32, 0.0f32);
                for r in 0..m {
                    // W_m^{r·k1} = W_n^{r·k1·p2}.
                    let t = (r * k1 % m) * p2;
                    let (wr, wi) = (self.tw_re[t], sign * self.tw_im[t]);
                    let (xr, xi) = (self.s_re[r * p2 + k2], self.s_im[r * p2 + k2]);
                    ar += xr * wr - xi * wi;
                    ai += xr * wi + xi * wr;
                }
                re[k2 + p2 * k1] = ar;
                im[k2 + p2 * k1] = ai;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(x_re: &[f32], x_im: &[f32], inverse: bool) -> (Vec<f32>, Vec<f32>) {
        let n = x_re.len();
        let s = if inverse { 1.0 } else { -1.0 };
        (0..n)
            .map(|k| {
                let (mut a, mut b) = (0.0f64, 0.0f64);
                for t in 0..n {
                    let w = s * 2.0 * std::f64::consts::PI * (k * t % n) as f64 / n as f64;
                    a += x_re[t] as f64 * w.cos() - x_im[t] as f64 * w.sin();
                    b += x_re[t] as f64 * w.sin() + x_im[t] as f64 * w.cos();
                }
                (a as f32, b as f32)
            })
            .unzip()
    }

    fn signal(n: usize) -> (Vec<f32>, Vec<f32>) {
        let re = (0..n).map(|i| ((i * 37 % 101) as f32 / 50.0 - 1.0) + (i as f32 * 0.03).sin()).collect();
        let im = (0..n).map(|i| ((i * 13 % 29) as f32 / 14.0 - 1.0) * 0.5).collect();
        (re, im)
    }

    #[test]
    fn matches_the_direct_dft() {
        for n in [2, 15, 30, 64, 120, 960] {
            let (re0, im0) = signal(n);
            for inverse in [false, true] {
                let mut f = MixedFft::new(n).unwrap();
                let (mut re, mut im) = (re0.clone(), im0.clone());
                assert!(if inverse { f.inverse(&mut re, &mut im) } else { f.forward(&mut re, &mut im) });
                let (er, ei) = naive(&re0, &im0, inverse);
                let tol = 2e-4 * n as f32;
                for k in 0..n {
                    assert!((re[k] - er[k]).abs() < tol && (im[k] - ei[k]).abs() < tol, "n {n} k {k} inverse {inverse}");
                }
            }
        }
    }

    #[test]
    fn round_trip() {
        let n = 960;
        let (re0, im0) = signal(n);
        let mut f = MixedFft::new(n).unwrap();
        let (mut re, mut im) = (re0.clone(), im0.clone());
        f.forward(&mut re, &mut im);
        f.inverse(&mut re, &mut im);
        for k in 0..n {
            assert!((re[k] / n as f32 - re0[k]).abs() < 1e-4 && (im[k] / n as f32 - im0[k]).abs() < 1e-4);
        }
    }

    #[test]
    fn hostile_sizes_and_short_slices_are_refused() {
        for n in [0, 1, 65 * 2, 127, MAX_LEN * 2, usize::MAX] {
            assert!(MixedFft::new(n).is_none(), "{n}");
        }
        let mut f = MixedFft::new(960).unwrap();
        let (mut re, mut im) = (vec![1.0; 959], vec![0.0; 960]);
        assert!(!f.forward(&mut re, &mut im));
        assert!(re.iter().all(|v| *v == 1.0));
    }
}
