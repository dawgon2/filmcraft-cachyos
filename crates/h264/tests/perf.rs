//! Decode speed on a 1080p High-profile stream (ignored by default; run with
//! `cargo test --release -p filmcraft-h264 --test perf -- --ignored --nocapture`).

mod common;

use filmcraft_h264::Decoder;
use std::time::Instant;

fn bench(data: &[u8], threads: usize, iters: usize) -> (usize, f64) {
    let mut best = f64::MAX;
    let mut frames = 0;
    for _ in 0..iters {
        let mut dec = Decoder::with_threads(threads);
        let t0 = Instant::now();
        frames = dec.decode(data, 0).expect("decode").len() + dec.flush().len();
        best = best.min(t0.elapsed().as_secs_f64());
    }
    (frames, best)
}

#[test]
#[ignore]
fn perf_1080p() {
    let f = common::fixture("bench_1080p");
    let Some((h264, _)) = common::ensure(f) else { return };
    // correctness first
    common::check_fixture("bench_1080p").unwrap();
    let data = std::fs::read(h264).unwrap();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    for threads in [1, cores.min(16)] {
        let (frames, secs) = bench(&data, threads, 5);
        println!("1080p High CRF20 ({} KB): {threads} thread(s): {frames} frames in {secs:.3}s = {:.1} fps", data.len() / 1024, frames as f64 / secs);
    }
}

/// 4K 10-bit 4:2:2 Long-GOP and Intra (fork H2.7; XAVC S / S-I-like): frame-threaded decode speed
/// (`cargo test --release -p filmcraft-h264 --test perf perf_4k_422_10 -- --ignored --nocapture`).
#[test]
#[ignore]
fn perf_4k_422_10() {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    for name in ["bench_4k_422_10", "bench_4k_422_10_intra"] {
        let f = common::fixture(name);
        let Some(h264) = common::ensure_stream(f) else { return };
        let data = std::fs::read(h264).unwrap();
        let mbps = data.len() as f64 * 8.0 / (f.frames as f64 / 25.0) / 1e6;
        for (threads, iters) in [(1, 1), (cores.min(16), 2)] {
            let (frames, secs) = bench(&data, threads, iters);
            println!("{name} ({mbps:.0} Mbit/s at 25 fps): {threads} thread(s): {frames} frames in {secs:.2}s = {:.1} fps", frames as f64 / secs);
        }
    }
}
