//! K-quant weight blocks (Q4_K, Q6_K) and their Q8_K activation dots (AI-PLAN AI1.1c), with the
//! same run-time SIMD dispatch as [`crate::quant`]. Q4_K_M model files mix Q4_K and Q6_K tensors.
//!
//! Block layouts, the scale unpacking and the reference dot / quantize maths are ported from candle
//! 0.9.2 (`candle-core/src/quantized/k_quants.rs` and `utils.rs`, MIT / Apache-2.0, © the candle
//! authors), which follows ggml's `k_quants.c`. Weights stay packed in memory (decode is
//! memory-bound); the kernels unpack one 256-value super-block at a time on the stack.

use half::f16;

use crate::LlmError;
use crate::quant::{Kernel, SimdLevel, err, rows_at, simd_level};

/// Values per K-quant super-block.
pub const QK_K: usize = 256;
/// Bytes per Q4_K block in a GGUF file: f16 d, f16 dmin, 12 packed 6-bit scales / mins, 128 nibbles.
pub const Q4K_BYTES: usize = 2 + 2 + 12 + QK_K / 2;
/// Bytes per Q6_K block: 128 low nibbles, 64 bytes of high 2-bit pairs, 16 i8 scales, f16 d.
pub const Q6K_BYTES: usize = QK_K / 2 + QK_K / 4 + QK_K / 16 + 2;

/// One Q4_K super-block; the 6-bit scales and mins are unpacked at parse time.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockQ4K {
    pub d: f32,
    pub dmin: f32,
    pub scales: [u8; 8],
    pub mins: [u8; 8],
    pub qs: [u8; QK_K / 2],
}

/// One Q6_K super-block (packed as in the file, scale widened to f32).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockQ6K {
    pub ql: [u8; QK_K / 2],
    pub qh: [u8; QK_K / 4],
    pub scales: [i8; QK_K / 16],
    pub d: f32,
}

/// Activations quantized for the K-quant dots: one f32 scale, 256 values, sums per 16.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockQ8K {
    pub d: f32,
    pub qs: [i8; QK_K],
    pub bsums: [i16; QK_K / 16],
}

fn f16_at(bytes: [u8; 2]) -> Result<f32, LlmError> {
    let v = f16::from_le_bytes(bytes).to_f32();
    if v.is_finite() { Ok(v) } else { Err(err("K-quant block with a non-finite scale")) }
}

fn whole_blocks(len: usize, block: usize, what: &str) -> Result<usize, LlmError> {
    if len.is_multiple_of(block) { Ok(len / block) } else { Err(err(format!("{what} data of {len} bytes is not whole {block}-byte blocks"))) }
}

/// ggml's `get_scale_min_k4` for all eight sub-blocks at once.
fn unpack_scales_mins(s: &[u8; 12]) -> ([u8; 8], [u8; 8]) {
    let (mut sc, mut m) = ([0u8; 8], [0u8; 8]);
    for j in 0..4 {
        sc[j] = s[j] & 63;
        m[j] = s[j + 4] & 63;
    }
    for j in 4..8 {
        sc[j] = (s[j + 4] & 0xF) | ((s[j - 4] >> 6) << 4);
        m[j] = (s[j + 4] >> 4) | ((s[j] >> 6) << 4);
    }
    (sc, m)
}

/// Parses GGUF Q4_K bytes into `out` (cleared first).
pub fn parse_q4k(bytes: &[u8], out: &mut Vec<BlockQ4K>) -> Result<(), LlmError> {
    out.clear();
    out.reserve(whole_blocks(bytes.len(), Q4K_BYTES, "Q4_K")?);
    for b in bytes.as_chunks::<Q4K_BYTES>().0 {
        let (head, qs) = b.split_first_chunk::<16>().ok_or_else(|| err("short Q4_K block"))?;
        let (d, rest) = head.split_first_chunk::<2>().ok_or_else(|| err("short Q4_K block"))?;
        let (dmin, packed) = rest.split_first_chunk::<2>().ok_or_else(|| err("short Q4_K block"))?;
        let packed: &[u8; 12] = packed.try_into().map_err(|_| err("short Q4_K block"))?;
        let (scales, mins) = unpack_scales_mins(packed);
        out.push(BlockQ4K { d: f16_at(*d)?, dmin: f16_at(*dmin)?, scales, mins, qs: qs.try_into().map_err(|_| err("short Q4_K block"))? });
    }
    Ok(())
}

/// Parses GGUF Q6_K bytes into `out` (cleared first).
pub fn parse_q6k(bytes: &[u8], out: &mut Vec<BlockQ6K>) -> Result<(), LlmError> {
    out.clear();
    out.reserve(whole_blocks(bytes.len(), Q6K_BYTES, "Q6_K")?);
    for b in bytes.as_chunks::<Q6K_BYTES>().0 {
        let short = || err("short Q6_K block");
        let (ql, rest) = b.split_first_chunk::<{ QK_K / 2 }>().ok_or_else(short)?;
        let (qh, rest) = rest.split_first_chunk::<{ QK_K / 4 }>().ok_or_else(short)?;
        let (sc, d) = rest.split_first_chunk::<{ QK_K / 16 }>().ok_or_else(short)?;
        let d: [u8; 2] = d.try_into().map_err(|_| short())?;
        out.push(BlockQ6K { ql: *ql, qh: *qh, scales: sc.map(|v| i8::from_le_bytes([v])), d: f16_at(d)? });
    }
    Ok(())
}

/// The 256 Q4_K values (0..=15) in index order.
#[inline(always)]
fn unpack_q4k(x: &BlockQ4K) -> [u8; QK_K] {
    let mut v = [0u8; QK_K];
    for (out, q) in v.as_chunks_mut::<64>().0.iter_mut().zip(x.qs.as_chunks::<32>().0) {
        let (lo, hi) = out.split_at_mut(32);
        for ((l, h), &b) in lo.iter_mut().zip(hi.iter_mut()).zip(q) {
            *l = b & 0xF;
            *h = b >> 4;
        }
    }
    v
}

/// The 256 Q6_K values (−32..=31) in index order.
#[inline(always)]
fn unpack_q6k(x: &BlockQ6K) -> [i8; QK_K] {
    let mut v = [0i8; QK_K];
    for ((out, ql), qh) in v.as_chunks_mut::<128>().0.iter_mut().zip(x.ql.as_chunks::<64>().0).zip(x.qh.as_chunks::<32>().0) {
        let (ql0, ql1) = ql.split_at(32);
        let (o01, o23) = out.split_at_mut(64);
        let (o0, o1) = o01.split_at_mut(32);
        let (o2, o3) = o23.split_at_mut(32);
        for l in 0..32 {
            let (a, b, h) = (ql0[l], ql1[l], qh[l]);
            // 6-bit values fit i8 after the −32 offset; `as` reinterprets 0..=63 exactly.
            o0[l] = ((a & 0xF) | ((h & 3) << 4)) as i8 - 32;
            o1[l] = ((b & 0xF) | (((h >> 2) & 3) << 4)) as i8 - 32;
            o2[l] = ((a >> 4) | (((h >> 4) & 3) << 4)) as i8 - 32;
            o3[l] = ((b >> 4) | (((h >> 6) & 3) << 4)) as i8 - 32;
        }
    }
    v
}

/// Dequantizes Q4_K blocks to f32 (appends to `out`), as ggml's `dequantize_row_q4_K`.
pub fn dequantize_q4k(xs: &[BlockQ4K], out: &mut Vec<f32>) {
    out.reserve(xs.len().saturating_mul(QK_K));
    for x in xs {
        let v = unpack_q4k(x);
        for ((chunk, &sc), &m) in v.as_chunks::<32>().0.iter().zip(&x.scales).zip(&x.mins) {
            let (d, min) = (x.d * f32::from(sc), x.dmin * f32::from(m));
            out.extend(chunk.iter().map(|&q| d * f32::from(q) - min));
        }
    }
}

/// Dequantizes Q6_K blocks to f32 (appends to `out`), as ggml's `dequantize_row_q6_K`.
pub fn dequantize_q6k(xs: &[BlockQ6K], out: &mut Vec<f32>) {
    out.reserve(xs.len().saturating_mul(QK_K));
    for x in xs {
        let v = unpack_q6k(x);
        for (chunk, &sc) in v.as_chunks::<16>().0.iter().zip(&x.scales) {
            let d = x.d * f32::from(sc);
            out.extend(chunk.iter().map(|&q| d * f32::from(q)));
        }
    }
}

/// Quantizes activations to Q8_K like ggml's `quantize_row_q8_K`. `xs.len()` must be whole
/// 256-value blocks and finite.
pub fn quantize_q8k(xs: &[f32], out: &mut Vec<BlockQ8K>) -> Result<(), LlmError> {
    if !xs.len().is_multiple_of(QK_K) {
        return Err(err(format!("{} values are not whole {QK_K}-value blocks", xs.len())));
    }
    out.clear();
    out.reserve(xs.len() / QK_K);
    for chunk in xs.as_chunks::<QK_K>().0 {
        if chunk.iter().any(|x| !x.is_finite()) {
            return Err(err("non-finite activation"));
        }
        let (mut max, mut amax) = (0f32, 0f32);
        for &x in chunk {
            if amax < x.abs() {
                amax = x.abs();
                max = x;
            }
        }
        let mut b = BlockQ8K { d: 0.0, qs: [0; QK_K], bsums: [0; QK_K / 16] };
        if amax != 0.0 {
            let iscale = -128.0 / max;
            for (q, &x) in b.qs.iter_mut().zip(chunk) {
                // `as` saturates at −128; the max element maps to −128 exactly.
                *q = (iscale * x).round().min(127.0) as i8;
            }
            for (s, group) in b.bsums.iter_mut().zip(b.qs.as_chunks::<16>().0) {
                // |Σ| ≤ 16 · 128 fits i16.
                *s = group.iter().map(|&q| i16::from(q)).sum();
            }
            b.d = 1.0 / iscale;
        }
        out.push(b);
    }
    Ok(())
}

// The float part keeps the reference's order exactly (eight lanes, lane `l` holding the indices
// ≡ l mod 8, scaled per block, summed at the end), so results are bit-identical to candle's and
// ggml's scalar path: an order change of ~1e-7 flips 8-bit activation roundings downstream and
// grows through the layers (measured on SmolLM2-1.7B). The integer lane sums are exact in any
// order, so they are formed per scale group first and scaled once (fast); Q6_K's reference adds
// its lanes in f32, which equals our i32 sums while |sum| < 2^24 (≤ 32 · 127 · 4096 ≈ 2^24 at the
// very worst; real blocks are far below).
#[inline(always)]
fn dot_q4k_kernel(xs: &[BlockQ4K], ys: &[BlockQ8K]) -> f32 {
    let mut sums = [0f32; 8];
    let mut sumf = 0f32;
    for (x, y) in xs.iter().zip(ys) {
        let v = unpack_q4k(x);
        let mut aux32 = [0i32; 8];
        for ((q, yq), &sc) in v.as_chunks::<32>().0.iter().zip(y.qs.as_chunks::<32>().0).zip(&x.scales) {
            let mut p = [0i32; 8];
            for (qh, yh) in q.as_chunks::<8>().0.iter().zip(yq.as_chunks::<8>().0) {
                // Products fit i16 (≤ 15 · 128), as in the reference.
                for l in 0..8 {
                    p[l] += i32::from(i16::from(yh[l]) * i16::from(qh[l]));
                }
            }
            for l in 0..8 {
                aux32[l] += i32::from(sc) * p[l];
            }
        }
        let mut summ = 0i32;
        for (pair, &m) in y.bsums.as_chunks::<2>().0.iter().zip(&x.mins) {
            summ += (i32::from(pair[0]) + i32::from(pair[1])) * i32::from(m);
        }
        let d = x.d * y.d;
        for (s, &a) in sums.iter_mut().zip(&aux32) {
            *s += d * a as f32;
        }
        sumf -= x.dmin * y.d * summ as f32;
    }
    sumf + sums.iter().sum::<f32>()
}

#[inline(always)]
fn dot_q6k_kernel(xs: &[BlockQ6K], ys: &[BlockQ8K]) -> f32 {
    let mut sums = [0f32; 8];
    for (x, y) in xs.iter().zip(ys) {
        let v = unpack_q6k(x);
        let mut aux32 = [0i32; 8];
        for ((q, yq), &sc) in v.as_chunks::<16>().0.iter().zip(y.qs.as_chunks::<16>().0).zip(&x.scales) {
            let mut p = [0i32; 8];
            for (qh, yh) in q.as_chunks::<8>().0.iter().zip(yq.as_chunks::<8>().0) {
                // Products fit i16 (≤ 32 · 128), as in the reference.
                for l in 0..8 {
                    p[l] += i32::from(i16::from(yh[l]) * i16::from(qh[l]));
                }
            }
            for l in 0..8 {
                aux32[l] += i32::from(sc) * p[l];
            }
        }
        let d = x.d * y.d;
        for (s, &a) in sums.iter_mut().zip(&aux32) {
            *s += a as f32 * d;
        }
    }
    sums.iter().sum()
}

pub(crate) struct Q4K;
impl Kernel for Q4K {
    type W = BlockQ4K;
    type A = BlockQ8K;
    #[inline(always)]
    fn dot(w: &[BlockQ4K], x: &[BlockQ8K]) -> f32 {
        dot_q4k_kernel(w, x)
    }
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn dot_v3(simd: pulp::x86::V3, w: &[BlockQ4K], x: &[BlockQ8K]) -> f32 {
        dot_q4k_v3(simd, w, x)
    }
}

/// Q4_K · Q8_K with AVX2 (after llama.cpp / candle `avx.rs`, MIT): the nibbles are unsigned, so
/// `vpmaddubsw` takes them as they are (pair sums ≤ 2 · 15 · 128); `vpmaddwd` by the 32-value
/// scale widens to i32 (≤ 4 · 15 · 128 · 63 per lane and vector, ≤ 2^23 per block); eight f32 lanes
/// accumulate across blocks; the mins term stays scalar as in the portable loop.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn dot_q4k_v3(simd: pulp::x86::V3, xs: &[BlockQ4K], ys: &[BlockQ8K]) -> f32 {
    use core::arch::x86_64::{__m256, __m256i};
    let (avx, avx2, fma) = (simd.avx, simd.avx2, simd.fma);
    let low = avx._mm256_set1_epi8(0x0F);
    let mut acc = avx._mm256_setzero_ps();
    let mut mins = 0f32;
    for (x, y) in xs.iter().zip(ys) {
        let mut isum = avx._mm256_setzero_si256();
        for ((q, yq), sc) in x.qs.as_chunks::<32>().0.iter().zip(y.qs.as_chunks::<64>().0).zip(x.scales.as_chunks::<2>().0) {
            let qv: __m256i = bytemuck::cast(*q);
            let (ylo, yhi) = yq.split_at(32);
            let (Ok(ylo), Ok(yhi)) = (<[i8; 32]>::try_from(ylo), <[i8; 32]>::try_from(yhi)) else {
                continue;
            };
            let lo = avx2._mm256_and_si256(qv, low);
            let hi = avx2._mm256_and_si256(avx2._mm256_srli_epi16::<4>(qv), low);
            let plo = avx2._mm256_maddubs_epi16(lo, bytemuck::cast(ylo));
            let phi = avx2._mm256_maddubs_epi16(hi, bytemuck::cast(yhi));
            let slo = avx2._mm256_madd_epi16(plo, avx._mm256_set1_epi16(i16::from(sc[0])));
            let shi = avx2._mm256_madd_epi16(phi, avx._mm256_set1_epi16(i16::from(sc[1])));
            isum = avx2._mm256_add_epi32(isum, avx2._mm256_add_epi32(slo, shi));
        }
        acc = fma._mm256_fmadd_ps(avx._mm256_set1_ps(x.d * y.d), avx._mm256_cvtepi32_ps(isum), acc);
        let mut summ = 0i32;
        for (pair, &m) in y.bsums.as_chunks::<2>().0.iter().zip(&x.mins) {
            summ += (i32::from(pair[0]) + i32::from(pair[1])) * i32::from(m);
        }
        mins += x.dmin * y.d * summ as f32;
    }
    bytemuck::cast::<__m256, [f32; 8]>(acc).iter().sum::<f32>() - mins
}

pub(crate) struct Q6K;
impl Kernel for Q6K {
    type W = BlockQ6K;
    type A = BlockQ8K;
    #[inline(always)]
    fn dot(w: &[BlockQ6K], x: &[BlockQ8K]) -> f32 {
        dot_q6k_kernel(w, x)
    }
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn dot_v3(simd: pulp::x86::V3, w: &[BlockQ6K], x: &[BlockQ8K]) -> f32 {
        dot_q6k_v3(simd, w, x)
    }
}

/// Q6_K · Q8_K with AVX2 (after llama.cpp / candle `avx.rs`, MIT): no sign trick, because Q8_K
/// activations reach −128 and negating that overflows; instead the unsigned values 0..=63 go into
/// `vpmaddubsw` (pair sums ≤ 2 · 63 · 128) and the −32 offset is taken off once per block through
/// the 16-value activation sums (`bsums`). `vpmaddwd` by a vector holding each 16-value group's
/// scale (the low eight i16 lanes of a 32-byte vector come from its first 16 bytes) widens to i32
/// (≤ 2^24 per block); eight f32 lanes accumulate across blocks.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn dot_q6k_v3(simd: pulp::x86::V3, xs: &[BlockQ6K], ys: &[BlockQ8K]) -> f32 {
    use core::arch::x86_64::{__m256, __m256i};
    let (avx, avx2, fma) = (simd.avx, simd.avx2, simd.fma);
    let mut acc = avx._mm256_setzero_ps();
    let mut offset = 0f32;
    for (x, y) in xs.iter().zip(ys) {
        // −32..=31 back to the stored 0..=63.
        let v = unpack_q6k(x).map(|q| q.wrapping_add(32) as u8);
        let mut isum = avx._mm256_setzero_si256();
        for ((q, yq), sc) in v.as_chunks::<32>().0.iter().zip(y.qs.as_chunks::<32>().0).zip(x.scales.as_chunks::<2>().0) {
            let p16 = avx2._mm256_maddubs_epi16(bytemuck::cast(*q), bytemuck::cast(*yq));
            let (a, b) = (i16::from(sc[0]), i16::from(sc[1]));
            let scales: __m256i = bytemuck::cast([a, a, a, a, a, a, a, a, b, b, b, b, b, b, b, b]);
            isum = avx2._mm256_add_epi32(isum, avx2._mm256_madd_epi16(p16, scales));
        }
        acc = fma._mm256_fmadd_ps(avx._mm256_set1_ps(x.d * y.d), avx._mm256_cvtepi32_ps(isum), acc);
        let corr: i32 = x.scales.iter().zip(&y.bsums).map(|(&sc, &b)| i32::from(sc) * i32::from(b)).sum();
        offset += x.d * y.d * (32 * corr) as f32;
    }
    bytemuck::cast::<__m256, [f32; 8]>(acc).iter().sum::<f32>() - offset
}

fn matvec_at<K: Kernel<A = BlockQ8K>>(level: SimdLevel, weights: &[K::W], x: &[BlockQ8K], out: &mut [f32]) -> Result<(), LlmError> {
    rows_at::<K>(level, weights, x, x.len(), out)
}

/// `out[r] = weights[r] · x` for Q4_K weights.
pub fn matvec_q4k(weights: &[BlockQ4K], x: &[BlockQ8K], out: &mut [f32]) -> Result<(), LlmError> {
    matvec_at::<Q4K>(simd_level(), weights, x, out)
}

/// `out[r] = weights[r] · x` for Q6_K weights.
pub fn matvec_q6k(weights: &[BlockQ6K], x: &[BlockQ8K], out: &mut [f32]) -> Result<(), LlmError> {
    matvec_at::<Q6K>(simd_level(), weights, x, out)
}

/// Batched Q4_K product (see [`crate::quant::matmul_q8_0`] for the layout).
pub fn matmul_q4k(weights: &[BlockQ4K], xs: &[BlockQ8K], cols: usize, out: &mut [f32]) -> Result<(), LlmError> {
    rows_at::<Q4K>(simd_level(), weights, xs, cols, out)
}

/// Batched Q6_K product.
pub fn matmul_q6k(weights: &[BlockQ6K], xs: &[BlockQ8K], cols: usize, out: &mut [f32]) -> Result<(), LlmError> {
    rows_at::<Q6K>(simd_level(), weights, xs, cols, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s >> 56) as u8
            })
            .collect()
    }

    fn values(n: usize, seed: u64) -> Vec<f32> {
        bytes(n * 3, seed).as_chunks::<3>().0.iter().map(|b| (f32::from(u16::from_le_bytes([b[0], b[1]])) / 65536.0 - 0.5) * 4.0).collect()
    }

    /// Random blocks with small finite scales written into the f16 slots.
    fn random_q4k(blocks: usize, seed: u64) -> Vec<u8> {
        let mut v = bytes(blocks * Q4K_BYTES, seed);
        for b in v.as_chunks_mut::<Q4K_BYTES>().0 {
            b[..2].copy_from_slice(&f16::from_f32(0.01).to_le_bytes());
            b[2..4].copy_from_slice(&f16::from_f32(0.02).to_le_bytes());
        }
        v
    }

    fn random_q6k(blocks: usize, seed: u64) -> Vec<u8> {
        let mut v = bytes(blocks * Q6K_BYTES, seed);
        for b in v.as_chunks_mut::<Q6K_BYTES>().0 {
            b[Q6K_BYTES - 2..].copy_from_slice(&f16::from_f32(0.003).to_le_bytes());
        }
        v
    }

    fn candle_dequantize(dtype: candle_core::quantized::GgmlDType, raw: &[u8], n: usize) -> Vec<f32> {
        let dev = candle_core::Device::Cpu;
        let t = candle_core::quantized::ggml_file::qtensor_from_ggml(dtype, raw, vec![n], &dev).unwrap();
        t.dequantize(&dev).unwrap().to_vec1::<f32>().unwrap()
    }

    #[test]
    fn q4k_dequantize_matches_candle() {
        let raw = random_q4k(3, 1);
        let mut blocks = Vec::new();
        parse_q4k(&raw, &mut blocks).unwrap();
        let mut ours = Vec::new();
        dequantize_q4k(&blocks, &mut ours);
        assert_eq!(ours, candle_dequantize(candle_core::quantized::GgmlDType::Q4K, &raw, 3 * QK_K));
    }

    #[test]
    fn q6k_dequantize_matches_candle() {
        let raw = random_q6k(3, 2);
        let mut blocks = Vec::new();
        parse_q6k(&raw, &mut blocks).unwrap();
        let mut ours = Vec::new();
        dequantize_q6k(&blocks, &mut ours);
        assert_eq!(ours, candle_dequantize(candle_core::quantized::GgmlDType::Q6K, &raw, 3 * QK_K));
    }

    fn reference_dot(w: &[f32], x: &[BlockQ8K]) -> f64 {
        let xs: Vec<f64> = x.iter().flat_map(|b| b.qs.iter().map(move |&q| f64::from(q) * f64::from(b.d))).collect();
        w.iter().zip(&xs).map(|(&a, &b)| f64::from(a) * b).sum()
    }

    #[test]
    fn dots_match_the_dequantized_reference_at_every_level() {
        let (rows, cols) = (2 * 32 + 5, 3);
        let mut x = Vec::new();
        quantize_q8k(&values(cols * QK_K, 9), &mut x).unwrap();
        let (mut w4, mut w6) = (Vec::new(), Vec::new());
        parse_q4k(&random_q4k(rows * cols, 3), &mut w4).unwrap();
        parse_q6k(&random_q6k(rows * cols, 4), &mut w6).unwrap();
        let (mut d4, mut d6) = (Vec::new(), Vec::new());
        dequantize_q4k(&w4, &mut d4);
        dequantize_q6k(&w6, &mut d6);
        for level in [SimdLevel::Auto, SimdLevel::Baseline] {
            let (mut o4, mut o6) = (vec![0f32; rows], vec![0f32; rows]);
            matvec_at::<Q4K>(level, &w4, &x, &mut o4).unwrap();
            matvec_at::<Q6K>(level, &w6, &x, &mut o6).unwrap();
            for r in 0..rows {
                let span = r * cols * QK_K..(r + 1) * cols * QK_K;
                for (got, w) in [(o4[r], &d4[span.clone()]), (o6[r], &d6[span])] {
                    let want = reference_dot(w, &x);
                    assert!((f64::from(got) - want).abs() <= 1e-3 * want.abs().max(1.0), "row {r}: {got} vs {want}");
                }
            }
        }
    }

    #[test]
    fn matvecs_match_candles_qmatmul() {
        use candle_core::{Device, Module, Tensor, quantized};
        let dev = Device::Cpu;
        let (rows, cols) = (40, 4);
        let xs = values(cols * QK_K, 31);
        let mut x = Vec::new();
        quantize_q8k(&xs, &mut x).unwrap();
        let xt = Tensor::from_vec(xs.clone(), (1, cols * QK_K), &dev).unwrap();
        for (dtype, raw) in [(quantized::GgmlDType::Q4K, random_q4k(rows * cols, 32)), (quantized::GgmlDType::Q6K, random_q6k(rows * cols, 33))] {
            let qt = quantized::ggml_file::qtensor_from_ggml(dtype, &raw, vec![rows, cols * QK_K], &dev).unwrap();
            let want = quantized::QMatMul::from_qtensor(qt).unwrap().forward(&xt).unwrap().squeeze(0).unwrap().to_vec1::<f32>().unwrap();
            let mut base = vec![0f32; rows];
            let mut auto = vec![0f32; rows];
            if dtype == quantized::GgmlDType::Q4K {
                let mut w = Vec::new();
                parse_q4k(&raw, &mut w).unwrap();
                matvec_at::<Q4K>(SimdLevel::Baseline, &w, &x, &mut base).unwrap();
                matvec_at::<Q4K>(SimdLevel::Auto, &w, &x, &mut auto).unwrap();
            } else {
                let mut w = Vec::new();
                parse_q6k(&raw, &mut w).unwrap();
                matvec_at::<Q6K>(SimdLevel::Baseline, &w, &x, &mut base).unwrap();
                matvec_at::<Q6K>(SimdLevel::Auto, &w, &x, &mut auto).unwrap();
            }
            let scale = want.iter().fold(1f32, |m, v| m.max(v.abs()));
            let worst = |got: &[f32]| got.iter().zip(&want).fold(0f32, |m, (g, w)| m.max((g - w).abs()));
            eprintln!("{dtype:?}: max |ours − candle| = {:e} (baseline), {:e} (auto); scale {scale:e}", worst(&base), worst(&auto));
            // The portable loop is bit-identical to candle's scalar path (see the kernels' comment);
            // candle built with AVX2 (only when built with `-C target-cpu=x86-64-v3`) and our own AVX2 dots
            // (AI1.1g) each sum in their own order: equal to rounding.
            if cfg!(target_feature = "avx2") {
                assert!(worst(&base) <= 1e-5 * scale, "{dtype:?} baseline");
            } else {
                assert_eq!(worst(&base), 0.0, "{dtype:?} baseline");
            }
            assert!(worst(&auto) <= 1e-5 * scale, "{dtype:?} auto");
        }
    }

    #[test]
    fn q8k_quantize_matches_the_reference_rules() {
        let xs = values(QK_K, 5);
        let mut q = Vec::new();
        quantize_q8k(&xs, &mut q).unwrap();
        let b = &q[0];
        for (g, s) in b.bsums.iter().enumerate() {
            assert_eq!(*s, b.qs[g * 16..g * 16 + 16].iter().map(|&v| i16::from(v)).sum::<i16>());
        }
        for (&x, &v) in xs.iter().zip(&b.qs) {
            // Half a step, or one step where ggml clamps +128 to 127.
            assert!((x - f32::from(v) * b.d).abs() <= b.d.abs() * 1.01 + 1e-6);
        }
        quantize_q8k(&[0.0; QK_K], &mut q).unwrap();
        assert_eq!(q[0].d, 0.0);
    }

    #[test]
    fn hostile_inputs_are_errors_not_panics() {
        let mut b4 = Vec::new();
        let mut b6 = Vec::new();
        assert!(parse_q4k(&[0u8; Q4K_BYTES - 1], &mut b4).is_err());
        assert!(parse_q6k(&[0u8; Q6K_BYTES + 3], &mut b6).is_err());
        let mut nan4 = random_q4k(1, 7);
        nan4[2..4].copy_from_slice(&f16::INFINITY.to_le_bytes());
        assert!(parse_q4k(&nan4, &mut b4).is_err());
        let mut nan6 = random_q6k(1, 8);
        nan6[Q6K_BYTES - 2..].copy_from_slice(&f16::NAN.to_le_bytes());
        assert!(parse_q6k(&nan6, &mut b6).is_err());
        let mut q = Vec::new();
        assert!(quantize_q8k(&[1.0; 100], &mut q).is_err());
        let mut inf = vec![0.0; QK_K];
        inf[9] = f32::NEG_INFINITY;
        assert!(quantize_q8k(&inf, &mut q).is_err());
        parse_q4k(&random_q4k(2, 9), &mut b4).unwrap();
        quantize_q8k(&values(QK_K, 1), &mut q).unwrap();
        assert!(matvec_q4k(&b4, &q, &mut [0.0; 3]).is_err());
        assert!(matvec_q4k(&b4, &[], &mut []).is_err());
        assert!(matvec_q6k(&[], &q, &mut [0.0]).is_err());
    }

    /// Timing (run with `--release --ignored --nocapture`): 960 × 2560 Q4_K and Q6_K matvecs.
    #[test]
    #[ignore]
    fn time_matvec_levels() {
        let (rows, cols) = (2560, 1024 / QK_K);
        let mut x = Vec::new();
        quantize_q8k(&values(cols * QK_K, 10), &mut x).unwrap();
        let (mut w4, mut w6) = (Vec::new(), Vec::new());
        parse_q4k(&random_q4k(rows * cols, 11), &mut w4).unwrap();
        parse_q6k(&random_q6k(rows * cols, 12), &mut w6).unwrap();
        let mut out = vec![0f32; rows];
        for level in [SimdLevel::Auto, SimdLevel::Baseline] {
            let t = std::time::Instant::now();
            for _ in 0..200 {
                matvec_at::<Q4K>(level, &w4, &x, &mut out).unwrap();
            }
            let q4 = t.elapsed().as_secs_f64() * 1e6 / 200.0;
            let t = std::time::Instant::now();
            for _ in 0..200 {
                matvec_at::<Q6K>(level, &w6, &x, &mut out).unwrap();
            }
            eprintln!("{level:?}: Q4_K {q4:.1} µs, Q6_K {:.1} µs per matvec", t.elapsed().as_secs_f64() * 1e6 / 200.0);
        }
        use candle_core::{Device, Module, Tensor, quantized};
        let dev = Device::Cpu;
        let xt = Tensor::from_vec(values(cols * QK_K, 10), (1, cols * QK_K), &dev).unwrap();
        for (dtype, raw) in [(quantized::GgmlDType::Q4K, random_q4k(rows * cols, 11)), (quantized::GgmlDType::Q6K, random_q6k(rows * cols, 12))] {
            let qt = quantized::ggml_file::qtensor_from_ggml(dtype, &raw, vec![rows, cols * QK_K], &dev).unwrap();
            let mm = quantized::QMatMul::from_qtensor(qt).unwrap();
            let t = std::time::Instant::now();
            for _ in 0..200 {
                mm.forward(&xt).unwrap();
            }
            eprintln!("candle {dtype:?}: {:.1} µs per matvec", t.elapsed().as_secs_f64() * 1e6 / 200.0);
        }
    }
}
