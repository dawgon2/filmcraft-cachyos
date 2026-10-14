//! Quantized weight blocks and their dot products, with run-time SIMD dispatch (AI-PLAN AI1.1).
//!
//! The block layouts and the reference maths are ported from candle 0.9.2
//! (`candle-core/src/quantized/k_quants.rs`, MIT / Apache-2.0, © the candle authors), which follows
//! ggml's reference kernels. candle picks its SIMD kernels at *compile* time (`target_feature =
//! "avx2"`), which is why the fork had to build for x86-64-v3 (OWNER-QUESTIONS Q8). Here the
//! kernels are plain safe Rust run inside [`pulp::Arch::dispatch`], which detects AVX-512 / AVX2 /
//! NEON at run time and compiles the same loop once per level, so one binary is fast on every CPU.
//!
//! Integer block sums are exact and the per-block float accumulation keeps its order, so every
//! dispatch level gives bit-identical results (tested).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use half::f16;

use crate::LlmError;

/// Values per Q8_0 block.
pub const QK8_0: usize = 32;
/// Bytes per Q8_0 block in a GGUF file: an f16 scale then 32 signed bytes.
pub const Q8_0_BYTES: usize = 2 + QK8_0;

/// One Q8_0 block, scale already widened to f32 (exact: every f16 is an f32).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockQ8_0 {
    pub d: f32,
    pub qs: [i8; QK8_0],
}

pub(crate) fn err(msg: impl Into<String>) -> LlmError {
    LlmError::Inference(msg.into())
}

/// Which code path the kernels use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdLevel {
    /// The best the CPU offers, detected at run time.
    Auto,
    /// No run-time SIMD: the build's baseline (SSE2 on x86-64; the fork no longer builds for x86-64-v3, AI1.2).
    /// For tests and for timing the slowest supported CPU.
    Baseline,
}

const LEVEL_UNSET: u8 = 0;
const LEVEL_AUTO: u8 = 1;
const LEVEL_BASELINE: u8 = 2;
static FORCED_LEVEL: AtomicU8 = AtomicU8::new(LEVEL_UNSET);

/// Forces a dispatch level for the whole process (tests, timing runs).
pub fn set_simd_level(level: SimdLevel) {
    let v = match level {
        SimdLevel::Auto => LEVEL_AUTO,
        SimdLevel::Baseline => LEVEL_BASELINE,
    };
    FORCED_LEVEL.store(v, Ordering::Relaxed);
}

/// The level in use: [`set_simd_level`], else `FILMCRAFT_LLM_SIMD=baseline` (anything else, or
/// unset, means auto), read once.
pub fn simd_level() -> SimdLevel {
    match FORCED_LEVEL.load(Ordering::Relaxed) {
        LEVEL_AUTO => SimdLevel::Auto,
        LEVEL_BASELINE => SimdLevel::Baseline,
        _ => {
            static FROM_ENV: OnceLock<SimdLevel> = OnceLock::new();
            *FROM_ENV.get_or_init(|| level_from_env(std::env::var("FILMCRAFT_LLM_SIMD").ok().as_deref()))
        }
    }
}

/// `FILMCRAFT_LLM_SIMD`: `baseline` (any case, spaces ignored) forces the portable loop; anything
/// else, or unset, is auto.
fn level_from_env(v: Option<&str>) -> SimdLevel {
    match v {
        Some(v) if v.trim().eq_ignore_ascii_case("baseline") => SimdLevel::Baseline,
        _ => SimdLevel::Auto,
    }
}

pub(crate) fn arch(level: SimdLevel) -> pulp::Arch {
    match level {
        SimdLevel::Auto => pulp::Arch::new(),
        SimdLevel::Baseline => pulp::Arch::Scalar,
    }
}

/// Parses a row of GGUF Q8_0 bytes into `out` (cleared first). A length that isn't whole blocks
/// or a non-finite scale is an error.
pub fn parse_q8_0(bytes: &[u8], out: &mut Vec<BlockQ8_0>) -> Result<(), LlmError> {
    if !bytes.len().is_multiple_of(Q8_0_BYTES) {
        return Err(err(format!("Q8_0 data of {} bytes is not whole {Q8_0_BYTES}-byte blocks", bytes.len())));
    }
    out.clear();
    out.reserve(bytes.len() / Q8_0_BYTES);
    for b in bytes.as_chunks::<Q8_0_BYTES>().0 {
        let Some((scale, qs)) = b.split_first_chunk::<2>() else {
            return Err(err("short Q8_0 block"));
        };
        let d = f16::from_le_bytes(*scale).to_f32();
        if !d.is_finite() {
            return Err(err("Q8_0 block with a non-finite scale"));
        }
        let mut block = BlockQ8_0 { d, qs: [0; QK8_0] };
        for (q, &byte) in block.qs.iter_mut().zip(qs) {
            *q = i8::from_le_bytes([byte]);
        }
        out.push(block);
    }
    Ok(())
}

/// Bytes per Q5_0 block: f16 scale, 32 high bits, 16 bytes of low nibbles.
pub const Q5_0_BYTES: usize = 2 + 4 + QK8_0 / 2;

/// Parses GGUF Q5_0 bytes (Qwen2.5 Q4_K_M files store `token_embd` this way) into Q8_0 blocks
/// (cleared first). The 5-bit values −16..=15 and the scale carry over exactly, so every Q8_0
/// kernel gives Q5_0's reference result (`dequantize_row_q5_0` / `vec_dot_q5_0_q8_0`, ported
/// from candle 0.9.2), at 34 instead of 22 bytes per block.
pub fn parse_q5_0_as_q8_0(bytes: &[u8], out: &mut Vec<BlockQ8_0>) -> Result<(), LlmError> {
    if !bytes.len().is_multiple_of(Q5_0_BYTES) {
        return Err(err(format!("Q5_0 data of {} bytes is not whole {Q5_0_BYTES}-byte blocks", bytes.len())));
    }
    out.clear();
    out.reserve(bytes.len() / Q5_0_BYTES);
    for b in bytes.as_chunks::<Q5_0_BYTES>().0 {
        let short = || err("short Q5_0 block");
        let (scale, rest) = b.split_first_chunk::<2>().ok_or_else(short)?;
        let (qh, qs) = rest.split_first_chunk::<4>().ok_or_else(short)?;
        let d = f16::from_le_bytes(*scale).to_f32();
        if !d.is_finite() {
            return Err(err("Q5_0 block with a non-finite scale"));
        }
        let qh = u32::from_le_bytes(*qh);
        let mut block = BlockQ8_0 { d, qs: [0; QK8_0] };
        let (lo, hi) = block.qs.split_at_mut(QK8_0 / 2);
        for (j, ((l, h), &q)) in lo.iter_mut().zip(hi.iter_mut()).zip(qs).enumerate() {
            let h0 = (((qh >> j) << 4) & 0x10) as u8;
            let h1 = ((qh >> (j + 12)) & 0x10) as u8;
            // 0..=31 − 16 fits i8.
            *l = ((q & 0x0F) | h0) as i8 - 16;
            *h = ((q >> 4) | h1) as i8 - 16;
        }
        out.push(block);
    }
    Ok(())
}

/// Quantizes activations to Q8_0 like ggml's `quantize_row_q8_0` (scale rounded through f16, so
/// results match the reference path). `xs.len()` must be whole blocks and finite.
pub fn quantize_q8_0(xs: &[f32], out: &mut Vec<BlockQ8_0>) -> Result<(), LlmError> {
    if !xs.len().is_multiple_of(QK8_0) {
        return Err(err(format!("{} values are not whole {QK8_0}-value blocks", xs.len())));
    }
    out.clear();
    out.reserve(xs.len() / QK8_0);
    for chunk in xs.as_chunks::<QK8_0>().0 {
        let amax = chunk.iter().fold(0f32, |m, x| m.max(x.abs()));
        if !amax.is_finite() || chunk.iter().any(|x| !x.is_finite()) {
            return Err(err("non-finite activation"));
        }
        let d = amax / 127.0;
        let id = if d != 0.0 { 1.0 / d } else { 0.0 };
        let mut block = BlockQ8_0 { d: f16::from_f32(d).to_f32(), qs: [0; QK8_0] };
        for (q, &x) in block.qs.iter_mut().zip(chunk) {
            // `as` saturates; |x * id| ≤ 127 anyway.
            *q = (x * id).round() as i8;
        }
        out.push(block);
    }
    Ok(())
}

/// Dequantizes blocks to f32 (appends to `out`).
pub fn dequantize_q8_0(xs: &[BlockQ8_0], out: &mut Vec<f32>) {
    out.reserve(xs.len().saturating_mul(QK8_0));
    for b in xs {
        out.extend(b.qs.iter().map(|&q| f32::from(q) * b.d));
    }
}

#[inline(always)]
fn dot_q8_0_kernel(xs: &[BlockQ8_0], ys: &[BlockQ8_0]) -> f32 {
    let mut sum = 0f32;
    for (x, y) in xs.iter().zip(ys) {
        let mut s = 0i32;
        for (a, b) in x.qs.iter().zip(y.qs.iter()) {
            s += i32::from(*a) * i32::from(*b);
        }
        // Same order as the reference: (Σ · dx) · dy. |s| ≤ 32 · 128² fits f32 exactly.
        sum += s as f32 * x.d * y.d;
    }
    sum
}

/// Dot product of two equal-length Q8_0 rows.
pub fn vec_dot_q8_0(xs: &[BlockQ8_0], ys: &[BlockQ8_0]) -> Result<f32, LlmError> {
    vec_dot_q8_0_at(simd_level(), xs, ys)
}

fn vec_dot_q8_0_at(level: SimdLevel, xs: &[BlockQ8_0], ys: &[BlockQ8_0]) -> Result<f32, LlmError> {
    if xs.len() != ys.len() {
        return Err(err(format!("Q8_0 dot of {} and {} blocks", xs.len(), ys.len())));
    }
    if xs.is_empty() {
        return Ok(0.0);
    }
    let mut out = [0f32];
    rows_at::<Q8>(level, xs, ys, ys.len().max(1), &mut out)?;
    Ok(out[0])
}

/// A dot kernel as a type, so it inlines into each SIMD context (a function pointer or a plain
/// closure does not: measured, the dispatch then gains nothing).
pub(crate) trait Kernel {
    /// Weight block.
    type W: Sync;
    /// Activation block.
    type A: Sync;
    fn dot(w: &[Self::W], a: &[Self::A]) -> f32;
    /// The same dot with explicit AVX2 instructions (x86-64-v3, detected at run time); defaults
    /// to the portable loop.
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn dot_v3(_simd: pulp::x86::V3, w: &[Self::W], a: &[Self::A]) -> f32 {
        Self::dot(w, a)
    }
    /// One weight row against four activation rows (each weight block loaded once); every lane
    /// equals [`Self::dot_v3`] of that row.
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn dot4_v3(simd: pulp::x86::V3, w: &[Self::W], a: [&[Self::A]; 4]) -> [f32; 4] {
        a.map(|a| Self::dot_v3(simd, w, a))
    }
}

pub(crate) struct Q8;
impl Kernel for Q8 {
    type W = BlockQ8_0;
    type A = BlockQ8_0;
    #[inline(always)]
    fn dot(w: &[BlockQ8_0], a: &[BlockQ8_0]) -> f32 {
        dot_q8_0_kernel(w, a)
    }
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn dot_v3(simd: pulp::x86::V3, w: &[BlockQ8_0], a: &[BlockQ8_0]) -> f32 {
        dot_q8_0_v3(simd, w, a)
    }
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn dot4_v3(simd: pulp::x86::V3, w: &[BlockQ8_0], a: [&[BlockQ8_0]; 4]) -> [f32; 4] {
        dot4_q8_0_v3(simd, w, a)
    }
}

/// [`dot_q8_0_v3`] for four activation rows at once: the weight block's |w| and sign are formed
/// once and reused; each of the four accumulators sees exactly the operations of a single dot,
/// so every lane is bit-identical to [`dot_q8_0_v3`].
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn dot4_q8_0_v3(simd: pulp::x86::V3, xs: &[BlockQ8_0], ys: [&[BlockQ8_0]; 4]) -> [f32; 4] {
    use core::arch::x86_64::{__m256, __m256i};
    let (avx, avx2, fma) = (simd.avx, simd.avx2, simd.fma);
    let ones = avx._mm256_set1_epi16(1);
    let mut acc = [avx._mm256_setzero_ps(); 4];
    let [y0, y1, y2, y3] = ys;
    for (i, x) in xs.iter().enumerate() {
        let (Some(b0), Some(b1), Some(b2), Some(b3)) = (y0.get(i), y1.get(i), y2.get(i), y3.get(i)) else {
            break;
        };
        let xv: __m256i = bytemuck::cast(x.qs);
        let ax = avx2._mm256_sign_epi8(xv, xv);
        for (a, y) in acc.iter_mut().zip([b0, b1, b2, b3]) {
            let yv: __m256i = bytemuck::cast(y.qs);
            let p16 = avx2._mm256_maddubs_epi16(ax, avx2._mm256_sign_epi8(yv, xv));
            let f = avx._mm256_cvtepi32_ps(avx2._mm256_madd_epi16(p16, ones));
            *a = fma._mm256_fmadd_ps(avx._mm256_set1_ps(x.d * y.d), f, *a);
        }
    }
    acc.map(|a| bytemuck::cast::<__m256, [f32; 8]>(a).iter().sum())
}

/// Q8_0 · Q8_0 with AVX2, the sign trick of llama.cpp / candle's `avx.rs` (MIT): `vpmaddubsw` needs
/// an unsigned operand, so it multiplies |w| by a·sign(w) (same products), then `vpmaddwd` with
/// ones widens the pair sums to i32, and eight f32 lanes accumulate `d_w·d_a·Σ` across blocks
/// (one horizontal sum per row, not per block). Pair sums saturate only for two −128 weights in
/// a row (|−128·−128·2| = 32768); `quantize_q8_0` never writes −128, real Q8_0 files neither.
/// The lane order differs from the portable loop's, so results agree to rounding, not bitwise.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn dot_q8_0_v3(simd: pulp::x86::V3, xs: &[BlockQ8_0], ys: &[BlockQ8_0]) -> f32 {
    use core::arch::x86_64::{__m256, __m256i};
    let (avx, avx2, fma) = (simd.avx, simd.avx2, simd.fma);
    let ones = avx._mm256_set1_epi16(1);
    let mut acc = avx._mm256_setzero_ps();
    for (x, y) in xs.iter().zip(ys) {
        let xv: __m256i = bytemuck::cast(x.qs);
        let yv: __m256i = bytemuck::cast(y.qs);
        let ax = avx2._mm256_sign_epi8(xv, xv);
        let sy = avx2._mm256_sign_epi8(yv, xv);
        let p16 = avx2._mm256_maddubs_epi16(ax, sy);
        let p32 = avx2._mm256_madd_epi16(p16, ones);
        let f = avx._mm256_cvtepi32_ps(p32);
        acc = fma._mm256_fmadd_ps(avx._mm256_set1_ps(x.d * y.d), f, acc);
    }
    let lanes: [f32; 8] = bytemuck::cast::<__m256, [f32; 8]>(acc);
    lanes.iter().sum()
}

/// Weight rows × `t` activation rows; `out` is rows × t. The whole loop runs inside one dispatch.
struct Rows<'a, K: Kernel> {
    weights: &'a [K::W],
    xs: &'a [K::A],
    cols: usize,
    out: &'a mut [f32],
    /// Set when the CPU has AVX2 and the level allows it: use the kernel's explicit V3 dot.
    #[cfg(target_arch = "x86_64")]
    v3: Option<pulp::x86::V3>,
}

impl<K: Kernel> pulp::WithSimd for Rows<'_, K> {
    type Output = ();
    #[inline(always)]
    fn with_simd<S: pulp::Simd>(self, _simd: S) {
        let t = self.xs.len().checked_div(self.cols).unwrap_or(0);
        if t == 0 {
            return;
        }
        for (row, o) in self.weights.chunks_exact(self.cols).zip(self.out.chunks_exact_mut(t)) {
            #[cfg(target_arch = "x86_64")]
            if let Some(simd) = self.v3 {
                let mut xs = self.xs.chunks_exact(self.cols);
                let (o4s, rest) = o.as_chunks_mut::<4>();
                for o4 in o4s {
                    let (Some(a), Some(b), Some(c), Some(d)) = (xs.next(), xs.next(), xs.next(), xs.next()) else {
                        break;
                    };
                    *o4 = K::dot4_v3(simd, row, [a, b, c, d]);
                }
                for (v, x) in rest.iter_mut().zip(xs) {
                    *v = K::dot_v3(simd, row, x);
                }
                continue;
            }
            for (x, v) in self.xs.chunks_exact(self.cols).zip(o.iter_mut()) {
                *v = K::dot(row, x);
            }
        }
    }
}

/// Rows per parallel task; below two tasks' worth the product stays on the calling thread.
const ROWS_PER_TASK: usize = 32;

/// `out[r·t + i] = weights[r] · xs[i]` for `t = xs.len() / cols` activation rows of `cols` blocks.
/// Rows are independent, so the thread split never changes a result, and each dot is the same as
/// in a matvec (bit-identical to `t` separate matvecs).
pub(crate) fn rows_at<K: Kernel>(level: SimdLevel, weights: &[K::W], xs: &[K::A], cols: usize, out: &mut [f32]) -> Result<(), LlmError> {
    use rayon::prelude::*;
    let t = xs.len().checked_div(cols).filter(|t| *t > 0 && xs.len().is_multiple_of(cols));
    let rows = t.and_then(|t| out.len().checked_div(t).filter(|_| out.len().is_multiple_of(t)));
    let (Some(t), Some(rows)) = (t, rows) else {
        return Err(err(format!("matmul: {} activation blocks / {cols} per row into {} outputs", xs.len(), out.len())));
    };
    let (Some(task_blocks), Some(task_out)) = (cols.checked_mul(ROWS_PER_TASK), t.checked_mul(ROWS_PER_TASK)) else {
        return Err(err("matmul too large"));
    };
    if Some(weights.len()) != rows.checked_mul(cols) {
        return Err(err(format!("matmul: {} weight blocks are not {rows} rows of {cols}", weights.len())));
    }
    #[cfg(target_arch = "x86_64")]
    let v3 = match level {
        SimdLevel::Auto => pulp::x86::V3::try_new(),
        SimdLevel::Baseline => None,
    };
    let run = |weights: &[K::W], out: &mut [f32]| {
        let rows = Rows::<K> {
            weights,
            xs,
            cols,
            out,
            #[cfg(target_arch = "x86_64")]
            v3,
        };
        #[cfg(target_arch = "x86_64")]
        if let Some(simd) = v3 {
            return <pulp::x86::V3 as pulp::Simd>::vectorize(simd, rows);
        }
        arch(level).dispatch(rows)
    };
    if rows < 2 * ROWS_PER_TASK {
        run(weights, out);
        return Ok(());
    }
    out.par_chunks_mut(task_out).zip(weights.par_chunks(task_blocks)).for_each(|(out, weights)| run(weights, out));
    Ok(())
}

/// `out[r] = weights[r] · x` for a row-major matrix of `out.len()` rows of `x.len()` blocks each.
pub fn matvec_q8_0(weights: &[BlockQ8_0], x: &[BlockQ8_0], out: &mut [f32]) -> Result<(), LlmError> {
    rows_at::<Q8>(simd_level(), weights, x, x.len(), out)
}

/// `out[r·t + i] = weights[r] · xs[i]`: `t = xs.len() / cols` activation rows at once, so each weight
/// row is read from memory once per batch (prompt prefill).
pub fn matmul_q8_0(weights: &[BlockQ8_0], xs: &[BlockQ8_0], cols: usize, out: &mut [f32]) -> Result<(), LlmError> {
    rows_at::<Q8>(simd_level(), weights, xs, cols, out)
}

#[cfg(test)]
fn matvec_q8_0_at(level: SimdLevel, weights: &[BlockQ8_0], x: &[BlockQ8_0], out: &mut [f32]) -> Result<(), LlmError> {
    rows_at::<Q8>(level, weights, x, x.len(), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator (xorshift), so tests need no extra dependency.
    fn values(n: usize, seed: u64) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 4.0
            })
            .collect()
    }

    fn to_bytes(blocks: &[BlockQ8_0]) -> Vec<u8> {
        let mut v = Vec::new();
        for b in blocks {
            v.extend_from_slice(&f16::from_f32(b.d).to_le_bytes());
            v.extend(b.qs.iter().map(|q| q.to_le_bytes()[0]));
        }
        v
    }

    #[test]
    fn quantize_dequantize_round_trip_is_close() {
        let xs = values(32 * 8, 7);
        let mut q = Vec::new();
        quantize_q8_0(&xs, &mut q).unwrap();
        let mut back = Vec::new();
        dequantize_q8_0(&q, &mut back);
        for (a, b) in xs.iter().zip(&back) {
            assert!((a - b).abs() <= 2.0 / 127.0 + 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn parse_reads_what_quantize_wrote() {
        let mut q = Vec::new();
        quantize_q8_0(&values(64, 3), &mut q).unwrap();
        let mut parsed = Vec::new();
        parse_q8_0(&to_bytes(&q), &mut parsed).unwrap();
        assert_eq!(parsed, q);
    }

    #[test]
    fn dot_matches_an_f64_reference() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        quantize_q8_0(&values(32 * 30, 11), &mut a).unwrap();
        quantize_q8_0(&values(32 * 30, 12), &mut b).unwrap();
        let mut reference = 0f64;
        for (x, y) in a.iter().zip(&b) {
            for (p, q) in x.qs.iter().zip(&y.qs) {
                reference += f64::from(*p) * f64::from(x.d) * f64::from(*q) * f64::from(y.d);
            }
        }
        let got = f64::from(vec_dot_q8_0(&a, &b).unwrap());
        assert!((got - reference).abs() <= 1e-4 * reference.abs().max(1.0), "{got} vs {reference}");
    }

    #[test]
    fn every_dispatch_level_agrees() {
        let (rows, cols) = (17, 24);
        let (mut w, mut x) = (Vec::new(), Vec::new());
        quantize_q8_0(&values(rows * cols * QK8_0, 5), &mut w).unwrap();
        quantize_q8_0(&values(cols * QK8_0, 6), &mut x).unwrap();
        let mut auto = vec![0f32; rows];
        let mut base = vec![0f32; rows];
        matvec_q8_0_at(SimdLevel::Auto, &w, &x, &mut auto).unwrap();
        matvec_q8_0_at(SimdLevel::Baseline, &w, &x, &mut base).unwrap();
        // The explicit AVX2 dot sums its float lanes in another order: equal to rounding.
        for (a, b) in auto.iter().zip(&base) {
            assert!((a - b).abs() <= 1e-5 * b.abs().max(1.0), "{a} vs {b}");
        }
        let row0 = w.get(..cols).unwrap();
        assert_eq!(vec_dot_q8_0_at(SimdLevel::Baseline, row0, &x).unwrap().to_bits(), base[0].to_bits());
    }

    #[test]
    fn parallel_rows_match_one_thread() {
        let (rows, cols) = (2 * ROWS_PER_TASK + 37, 5);
        let (mut w, mut x) = (Vec::new(), Vec::new());
        quantize_q8_0(&values(rows * cols * QK8_0, 21), &mut w).unwrap();
        quantize_q8_0(&values(cols * QK8_0, 22), &mut x).unwrap();
        let mut par = vec![0f32; rows];
        matvec_q8_0(&w, &x, &mut par).unwrap();
        let one: Vec<f32> = w.chunks_exact(cols).map(|r| vec_dot_q8_0(r, &x).unwrap()).collect();
        assert_eq!(par.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), one.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
    }

    #[test]
    fn matmul_equals_separate_matvecs() {
        let (rows, cols, t) = (2 * ROWS_PER_TASK + 9, 3, 5);
        let (mut w, mut xs) = (Vec::new(), Vec::new());
        quantize_q8_0(&values(rows * cols * QK8_0, 41), &mut w).unwrap();
        quantize_q8_0(&values(t * cols * QK8_0, 42), &mut xs).unwrap();
        let mut out = vec![0f32; rows * t];
        matmul_q8_0(&w, &xs, cols, &mut out).unwrap();
        for (i, x) in xs.chunks_exact(cols).enumerate() {
            let mut one = vec![0f32; rows];
            matvec_q8_0(&w, x, &mut one).unwrap();
            for r in 0..rows {
                assert_eq!(out[r * t + i].to_bits(), one[r].to_bits(), "row {r} token {i}");
            }
        }
        assert!(matmul_q8_0(&w, &xs, 0, &mut out).is_err());
        assert!(matmul_q8_0(&w, &xs[..cols * t - 1], cols, &mut out).is_err());
        assert!(matmul_q8_0(&w, &xs, cols, &mut out[..rows * t - 1]).is_err());
    }

    #[test]
    fn hostile_inputs_are_errors_not_panics() {
        let mut out = Vec::new();
        assert!(parse_q8_0(&[0u8; Q8_0_BYTES + 1], &mut out).is_err());
        let mut nan = [0u8; Q8_0_BYTES];
        nan[..2].copy_from_slice(&f16::NAN.to_le_bytes());
        assert!(parse_q8_0(&nan, &mut out).is_err());
        assert!(quantize_q8_0(&[1.0; 31], &mut out).is_err());
        let mut inf = vec![0.0f32; 32];
        inf[3] = f32::INFINITY;
        assert!(quantize_q8_0(&inf, &mut out).is_err());
        let mut ok = Vec::new();
        quantize_q8_0(&values(64, 1), &mut ok).unwrap();
        assert!(vec_dot_q8_0(&ok, ok.get(..1).unwrap()).is_err());
        assert!(matvec_q8_0(&ok, &[], &mut [0.0]).is_err());
        assert!(matvec_q8_0(&ok, ok.get(..1).unwrap(), &mut [0.0; 3]).is_err());
        assert!(matvec_q8_0(&[], &[], &mut []).is_err());
    }

    #[test]
    fn q5_0_widens_exactly_to_candles_values() {
        let mut raw: Vec<u8> = (0..3 * Q5_0_BYTES).map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3]).collect();
        for b in raw.as_chunks_mut::<Q5_0_BYTES>().0 {
            b[..2].copy_from_slice(&f16::from_f32(0.05).to_le_bytes());
        }
        let mut q = Vec::new();
        parse_q5_0_as_q8_0(&raw, &mut q).unwrap();
        let mut ours = Vec::new();
        dequantize_q8_0(&q, &mut ours);
        let dev = candle_core::Device::Cpu;
        let t = candle_core::quantized::ggml_file::qtensor_from_ggml(candle_core::quantized::GgmlDType::Q5_0, &raw, vec![3 * QK8_0], &dev).unwrap();
        assert_eq!(ours, t.dequantize(&dev).unwrap().to_vec1::<f32>().unwrap());
        assert!(parse_q5_0_as_q8_0(&raw[..Q5_0_BYTES + 1], &mut q).is_err());
        raw[..2].copy_from_slice(&f16::NAN.to_le_bytes());
        assert!(parse_q5_0_as_q8_0(&raw, &mut q).is_err());
    }

    #[test]
    fn all_zero_activations_quantize_to_zero() {
        let mut q = Vec::new();
        quantize_q8_0(&[0.0; 32], &mut q).unwrap();
        assert_eq!(q, vec![BlockQ8_0 { d: 0.0, qs: [0; QK8_0] }]);
    }

    #[test]
    fn env_level_parsing_defaults_to_auto() {
        // Pure parsing: the global override is never touched in tests (it would race with
        // FILMCRAFT_LLM_SIMD=baseline runs of the other tests).
        assert_eq!(level_from_env(None), SimdLevel::Auto);
        assert_eq!(level_from_env(Some(" BaseLine ")), SimdLevel::Baseline);
        assert_eq!(level_from_env(Some("avx512")), SimdLevel::Auto);
    }

    /// Timing check for PREP-AI1's first risk (run with `--ignored --nocapture`, release build):
    /// a 960 × 2560 Q8_0 matvec (SmolLM2-360M's MLP size), auto vs baseline.
    #[test]
    #[ignore]
    fn time_matvec_levels() {
        let (rows, cols) = (2560, 960 / QK8_0);
        let (mut w, mut x) = (Vec::new(), Vec::new());
        quantize_q8_0(&values(rows * cols * QK8_0, 9), &mut w).unwrap();
        quantize_q8_0(&values(cols * QK8_0, 10), &mut x).unwrap();
        let mut out = vec![0f32; rows];
        for level in [SimdLevel::Auto, SimdLevel::Baseline] {
            let t = std::time::Instant::now();
            for _ in 0..200 {
                matvec_q8_0_at(level, &w, &x, &mut out).unwrap();
            }
            eprintln!("{level:?}: {:.1} µs per matvec", t.elapsed().as_secs_f64() * 1e6 / 200.0);
        }
        // candle's QMatMul on the same sizes (compare threads with RAYON_NUM_THREADS).
        use candle_core::{Device, Module, Tensor, quantized};
        let dev = Device::Cpu;
        let wt = Tensor::from_vec(values(rows * cols * QK8_0, 9), (rows, cols * QK8_0), &dev).unwrap();
        let mm = quantized::QMatMul::from_qtensor(quantized::QTensor::quantize(&wt, quantized::GgmlDType::Q8_0).unwrap()).unwrap();
        let xt = Tensor::from_vec(values(cols * QK8_0, 10), (1, cols * QK8_0), &dev).unwrap();
        let t = std::time::Instant::now();
        for _ in 0..200 {
            mm.forward(&xt).unwrap();
        }
        eprintln!("candle: {:.1} µs per matvec", t.elapsed().as_secs_f64() * 1e6 / 200.0);
    }
}
