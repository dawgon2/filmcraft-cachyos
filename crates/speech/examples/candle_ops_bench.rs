//! Times the candle f32 operations Whisper (and Kokoro, which uses the same candle) spend their
//! time in, on whisper-tiny / Kokoro-like shapes with synthetic data. Used to check that building
//! without `-C target-cpu=x86-64-v3` keeps their speed (AI1.2): `gemm` dispatches at run time, the
//! element-wise ops do not.
//!
//! ```sh
//! cargo run --release -p filmcraft-speech --features whisper --example candle_ops_bench
//! ```

use candle_core::{D, Device, Tensor};

fn time(label: &str, reps: u32, mut f: impl FnMut() -> candle_core::Result<Tensor>) -> candle_core::Result<()> {
    f()?;
    let t = std::time::Instant::now();
    for _ in 0..reps {
        f()?;
    }
    println!("{label:<34} {:>9.2} ms", t.elapsed().as_secs_f64() * 1e3 / f64::from(reps));
    Ok(())
}

fn main() -> candle_core::Result<()> {
    let dev = Device::Cpu;
    let total = std::time::Instant::now();
    // Whisper-tiny encoder: 80 mel bins × 3000 frames, d = 384, 6 heads, 1500 positions, MLP 1536.
    let mel = Tensor::randn(0f32, 1.0, (1, 80, 3000), &dev)?;
    let w1 = Tensor::randn(0f32, 0.1, (384, 80, 3), &dev)?;
    let w2 = Tensor::randn(0f32, 0.1, (384, 384, 3), &dev)?;
    let x = Tensor::randn(0f32, 1.0, (1, 1500, 384), &dev)?;
    let fc1 = Tensor::randn(0f32, 0.1, (384, 1536), &dev)?;
    let q = Tensor::randn(0f32, 1.0, (1, 6, 1500, 64), &dev)?;
    let kt = Tensor::randn(0f32, 1.0, (1, 6, 64, 1500), &dev)?;
    let scores = q.matmul(&kt)?;
    let ln_w = Tensor::ones(384, candle_core::DType::F32, &dev)?;
    let ln_b = Tensor::zeros(384, candle_core::DType::F32, &dev)?;
    let h = x.matmul(&fc1.unsqueeze(0)?)?;
    time("conv1d 80->384 k3 x3000", 20, || mel.conv1d(&w1, 1, 1, 1, 1))?;
    time("conv1d 384->384 k3 s2 x3000", 10, || mel.conv1d(&w1, 1, 1, 1, 1)?.conv1d(&w2, 1, 2, 1, 1))?;
    time("matmul 1500x384 * 384x1536", 20, || x.matmul(&fc1.unsqueeze(0)?))?;
    time("attention q*k^T 6x1500x1500", 10, || q.matmul(&kt))?;
    time("softmax 6x1500x1500", 10, || candle_nn::ops::softmax_last_dim(&scores))?;
    time("gelu 1500x1536", 20, || h.gelu())?;
    time("layer_norm 1500x384", 50, || candle_nn::ops::layer_norm(&x, &ln_w, &ln_b, 1e-5))?;
    time("elementwise mul+add 1500x1536", 50, || (&h * 0.5)? + &h)?;
    time("sum/mean over d 1500x1536", 50, || h.mean_keepdim(D::Minus1))?;
    // Kokoro decoder-like: 512 channels, upsampling transposed conv, 256 -> 4096 frames.
    let a = Tensor::randn(0f32, 1.0, (1, 512, 256), &dev)?;
    let wt = Tensor::randn(0f32, 0.1, (512, 256, 16), &dev)?;
    let b = Tensor::randn(0f32, 1.0, (1, 256, 4096), &dev)?;
    let wc = Tensor::randn(0f32, 0.1, (256, 256, 7), &dev)?;
    time("conv_transpose1d 512->256 k16 s8", 5, || a.conv_transpose1d(&wt, 4, 0, 8, 1, 1))?;
    time("conv1d 256->256 k7 x4096", 10, || b.conv1d(&wc, 3, 1, 1, 1))?;
    time("sigmoid 256x4096", 50, || candle_nn::ops::sigmoid(&b))?;
    println!("{:<34} {:>9.2} s", "total", total.elapsed().as_secs_f64());
    Ok(())
}
