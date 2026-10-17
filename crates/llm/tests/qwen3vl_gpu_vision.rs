//! Qwen3-VL 4B's vision encoder on the GPU (AI4.2) against the CPU encoder (itself checked
//! against candle in `qwen3vl_vision.rs`). Runs only with `FILMCRAFT_QWEN3VL_DIR` set to the
//! downloaded model folder and a GPU adapter; skips otherwise. Build with `--features gpu`
//! (`--release`: the CPU encode of the larger pictures is slow in debug). Reports
//! max |gpu − cpu| / max |cpu| for the tokens and every DeepStack tap. With
//! `FILMCRAFT_QWEN3VL_TIME=1` it also times a 2^18-pixel picture and a 1024 × 768 one on both.
//! Pick the adapter with `MESA_VK_DEVICE_SELECT=<vendor>:<device>` (e.g. `1002:73df`, `8086:4c8a`).

#![cfg(feature = "gpu")]

use filmcraft_llm::gpu_vision::GpuVisionEncoder;
use filmcraft_llm::vision::VisionEncoder;
use filmcraft_llm::{vision_config, vision_prep};

fn gradient(w: usize, h: usize) -> Vec<f32> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            [x, y, 1.0 - x * y]
        })
        .collect()
}

/// max |a − b| / max |b|.
fn gap(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let scale = b.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs())) / scale
}

fn report(what: &str, got: &filmcraft_llm::vision::VisionOutput, want: &filmcraft_llm::vision::VisionOutput) {
    assert_eq!((got.count, got.rows, got.cols), (want.count, want.rows, want.cols));
    assert_eq!(got.deepstack.len(), want.deepstack.len());
    let gaps: Vec<f32> = std::iter::once(gap(&got.tokens, &want.tokens)).chain(got.deepstack.iter().zip(&want.deepstack).map(|(a, b)| gap(a, b))).collect();
    eprintln!("{what}: {} tokens, GPU vs CPU max |diff| / max |cpu|: tokens then taps {gaps:?}", got.count);
    assert!(gaps.iter().all(|g| *g < 1e-3), "{gaps:?}");
    assert!(got.tokens.iter().all(|v| v.is_finite()));
}

#[test]
fn gpu_vision_encoder_matches_the_cpu() {
    let Some(dir) = std::env::var_os("FILMCRAFT_QWEN3VL_DIR").map(std::path::PathBuf::from) else { return };
    let cfg = vision_config::parse(&std::fs::read(dir.join("config.json")).unwrap(), "config.json").unwrap();
    let cpu = VisionEncoder::load(&dir, &cfg).unwrap();
    let t = std::time::Instant::now();
    let gpu = match GpuVisionEncoder::new(&cpu) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("SKIPPED: {e}");
            return;
        }
    };
    eprintln!("{}: {:.2} GB of weights uploaded in {:.2} s", gpu.adapter(), gpu.bytes() as f64 / 1e9, t.elapsed().as_secs_f32());

    // 64 × 64: 4 × 4 patches → 2 × 2 merged tokens (the oracle test's picture)
    let (w, h) = (64usize, 64usize);
    let rgb = gradient(w, h);
    let ((gh, gw), patches) = vision_prep::patches(&rgb, w, h).unwrap();
    let want = cpu.encode(&patches, (gh, gw)).unwrap();
    let got = gpu.encode(&patches, (gh, gw)).unwrap();
    report("64 × 64", &got, &want);

    // 2^18 pixels: 512 × 512 → 32 × 32 patches = 1024 patches, 256 tokens (many key blocks)
    let (w, h) = (512usize, 512usize);
    let rgb = gradient(w, h);
    let t = std::time::Instant::now();
    let want = cpu.encode_image(&rgb, w, h).unwrap();
    let cpu_s = t.elapsed().as_secs_f32();
    let t = std::time::Instant::now();
    let got = gpu.encode_image(&rgb, w, h).unwrap();
    let gpu_s = t.elapsed().as_secs_f32();
    report("512 × 512", &got, &want);
    eprintln!("512 × 512 (2^18 pixels): CPU {cpu_s:.2} s, GPU {gpu_s:.2} s (first run)");

    if std::env::var_os("FILMCRAFT_QWEN3VL_TIME").is_some() {
        let time = |w: usize, h: usize, runs: u32| {
            let rgb = gradient(w, h);
            let _ = gpu.encode_image(&rgb, w, h).unwrap();
            let t = std::time::Instant::now();
            let mut out = None;
            for _ in 0..runs {
                out = Some(gpu.encode_image(&rgb, w, h).unwrap());
            }
            let s = t.elapsed().as_secs_f32() / runs as f32;
            let out = out.unwrap();
            eprintln!("{w} × {h}: {} tokens in {s:.3} s on {} (mean of {runs} after a warm-up)", out.count, gpu.adapter());
            (rgb, out)
        };
        time(512, 512, 5);
        let (rgb, got) = time(1024, 768, 3);
        let t = std::time::Instant::now();
        let want = cpu.encode_image(&rgb, 1024, 768).unwrap();
        eprintln!("1024 × 768: CPU {:.1} s", t.elapsed().as_secs_f32());
        report("1024 × 768", &got, &want);
    }
}
