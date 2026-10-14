//! candle f32 operations whose candle CPU kernels only vectorise when AVX is enabled at compile
//! time (`#[cfg(target_feature)]`), rewritten on candle's matmul, which `gemm` dispatches at run
//! time (AVX-512 / AVX2 / SSE / NEON). Measured in a default x86-64 build (synthetic, AI1.2):
//! transposed conv 45 → 8 ms, last-dim mean 0.93 → 0.09 ms. Results equal candle's up to
//! summation order. Time candle's other ops with `crates/speech/examples/candle_ops_bench.rs`
//! before relying on them in a new model.

use candle_core::{D, Error, Result, Tensor};

/// Transposed 1-D convolution (groups 1, dilation 1) as one matrix product plus a scatter-add.
/// candle's direct kernel only vectorises when AVX is enabled at compile time (6.7× slower in a
/// default x86-64 build, AI1.2); its matmul goes through `gemm`, which picks AVX2 / AVX-512 at run
/// time. `x` is `[batch, c_in, len]`, `w` is `[c_in, c_out, k]`. Same result up to summation order.
pub fn conv_transpose1d(x: &Tensor, w: &Tensor, pad: usize, out_pad: usize, stride: usize) -> Result<Tensor> {
    let (batch, c_in, len) = x.dims3()?;
    let (wc_in, c_out, k) = w.dims3()?;
    if wc_in != c_in || stride == 0 || k == 0 || len == 0 {
        return Err(Error::Msg(format!("conv_transpose1d: input {c_in}x{len}, weight {wc_in}x{c_out}x{k}, stride {stride}")));
    }
    let out_len = (len - 1)
        .checked_mul(stride)
        .and_then(|v| v.checked_add(k - 1 + out_pad + 1))
        .and_then(|v| v.checked_sub(2 * pad))
        .filter(|&v| v > 0)
        .ok_or_else(|| Error::Msg(format!("conv_transpose1d: empty output (len {len}, k {k}, stride {stride}, pad {pad})")))?;
    let out_size = batch
        .checked_mul(c_out)
        .and_then(|v| v.checked_mul(out_len))
        .filter(|&v| v <= 1 << 30)
        .ok_or_else(|| Error::Msg("conv_transpose1d: output too large".into()))?;
    // cols[b] = wᵀ · x[b]: [c_out · k, len]
    let wt = w.reshape((c_in, c_out * k))?.t()?;
    let cols = wt.broadcast_matmul(&x.contiguous()?)?;
    let cols = cols.flatten_all()?.to_vec1::<f32>()?;
    let mut out = vec![0f32; out_size];
    for (b, out_b) in out.chunks_exact_mut(c_out * out_len).enumerate() {
        for (co, out_row) in out_b.chunks_exact_mut(out_len).enumerate() {
            for kk in 0..k {
                let start = ((b * c_out + co) * k + kk) * len;
                let Some(row) = cols.get(start..start + len) else {
                    return Err(Error::Msg("conv_transpose1d: column buffer too short".into()));
                };
                for (l, v) in row.iter().enumerate() {
                    // output position l·stride + kk − pad
                    let Some(t) = (l * stride + kk).checked_sub(pad) else { continue };
                    if let Some(o) = out_row.get_mut(t) {
                        *o += v;
                    }
                }
            }
        }
    }
    Tensor::from_vec(out, (batch, c_out, out_len), x.device())
}

/// Mean over the last dimension, keeping it (size 1), as a product with a ones vector so it runs
/// through `gemm` (run-time SIMD); candle's reduction is scalar without compile-time AVX (AI1.2).
pub fn mean_last(x: &Tensor) -> Result<Tensor> {
    let n = x.dim(D::Minus1)?;
    if n == 0 {
        return Err(Error::Msg("mean over an empty dimension".into()));
    }
    let ones = Tensor::ones((n, 1), x.dtype(), x.device())?;
    x.contiguous()?.broadcast_matmul(&ones)? / n as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{D, Device};

    fn close(a: &Tensor, b: &Tensor) -> f32 {
        (a - b).unwrap().abs().unwrap().flatten_all().unwrap().max(0).unwrap().to_scalar::<f32>().unwrap()
    }

    #[test]
    fn conv_transpose_matches_candle() {
        let dev = Device::Cpu;
        // Kokoro's two upsamplers (k 20 s 10 p 5, k 12 s 6 p 3), plus out_pad, batch 2 and pad > k.
        for &(batch, c_in, c_out, len, k, stride, pad, out_pad) in
            &[(1, 8, 6, 17, 20, 10, 5, 0), (1, 6, 4, 33, 12, 6, 3, 0), (2, 5, 3, 9, 4, 2, 1, 1), (1, 3, 2, 12, 3, 1, 4, 0)]
        {
            let x = Tensor::randn(0f32, 1.0, (batch, c_in, len), &dev).unwrap();
            let w = Tensor::randn(0f32, 1.0, (c_in, c_out, k), &dev).unwrap();
            let ours = conv_transpose1d(&x, &w, pad, out_pad, stride).unwrap();
            let theirs = x.conv_transpose1d(&w, pad, out_pad, stride, 1, 1).unwrap();
            assert_eq!(ours.dims(), theirs.dims());
            assert!(close(&ours, &theirs) < 1e-4, "case k {k} s {stride} p {pad}");
        }
    }

    #[test]
    fn conv_transpose_refuses_bad_shapes() {
        let dev = Device::Cpu;
        let x = Tensor::zeros((1, 4, 3), candle_core::DType::F32, &dev).unwrap();
        let w = Tensor::zeros((5, 2, 3), candle_core::DType::F32, &dev).unwrap();
        assert!(conv_transpose1d(&x, &w, 0, 0, 1).is_err()); // channel mismatch
        let w = Tensor::zeros((4, 2, 3), candle_core::DType::F32, &dev).unwrap();
        assert!(conv_transpose1d(&x, &w, 0, 0, 0).is_err()); // stride 0
        assert!(conv_transpose1d(&x, &w, 9, 0, 1).is_err()); // padding eats the output
    }

    #[test]
    fn mean_last_matches_candle() {
        let dev = Device::Cpu;
        let x = Tensor::randn(0f32, 1.0, (1, 7, 1001), &dev).unwrap();
        let ours = mean_last(&x).unwrap();
        assert_eq!(ours.dims(), &[1, 7, 1]);
        assert!(close(&ours, &x.mean_keepdim(D::Minus1).unwrap()) < 1e-5);
    }

    /// `cargo test --release -p filmcraft-ml --features candle --lib op_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn op_timing() {
        let dev = Device::Cpu;
        let a = Tensor::randn(0f32, 1.0, (1, 512, 256), &dev).unwrap();
        let w = Tensor::randn(0f32, 0.1, (512, 256, 16), &dev).unwrap();
        let b = Tensor::randn(0f32, 1.0, (1, 256, 4096), &dev).unwrap();
        let time = |label: &str, f: &dyn Fn() -> Tensor| {
            f();
            let t = std::time::Instant::now();
            for _ in 0..10 {
                f();
            }
            println!("{label:<30} {:>8.2} ms", t.elapsed().as_secs_f64() * 100.0);
        };
        time("candle conv_transpose1d", &|| a.conv_transpose1d(&w, 4, 0, 8, 1, 1).unwrap());
        time("gemm conv_transpose1d", &|| conv_transpose1d(&a, &w, 4, 0, 8).unwrap());
        time("candle mean_keepdim", &|| b.mean_keepdim(D::Minus1).unwrap());
        time("gemm mean_last", &|| mean_last(&b).unwrap());
    }
}
