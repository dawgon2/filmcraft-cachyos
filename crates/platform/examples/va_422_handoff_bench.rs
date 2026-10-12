//! HEVC 4:2:2 decode on Intel's media engine and the hand-off to the compositing GPU (fork 6.1b,
//! owner §1a.30), timed per picture. Synthetic, one run each; idle the machine for numbers.
//!
//! 1. `read-back`: decode + sync + read back to memory (the baseline, no GPU).
//! 2. `hand-off decode`: decode + linear copy + sync + export (no GPU).
//! 3. `read-back → GPU`: 1 + upload to the compositing GPU (drawn into a 64×64 canvas, waited).
//! 4. `hand-off → GPU`: 2 + DMA-BUF import + unpack into VRAM (drawn the same way, waited).
//!
//! `cargo run --release -p filmcraft-platform --example va_422_handoff_bench -- <4:2:2 MOV> [pictures]`

use std::sync::Arc;
use std::time::Instant;

use filmcraft_codecs::VideoDecoder;
use filmcraft_frame::PixelData;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: va_422_handoff_bench <file> [pictures]")?;
    let limit: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(90);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let file = filmcraft_isobmff::open(bytes.clone()).map_err(|e| e.to_string())?;
    let t = file.track_of_kind(filmcraft_isobmff::TrackKind::Video).ok_or("no video track")?;
    let track = file.tracks.get(t).ok_or("no video track")?;
    let entry = track.entries.first().ok_or("no sample entry")?.clone();
    let samples: Vec<(Vec<u8>, i64)> = (0..track.samples.len().min(limit))
        .map(|i| Ok((file.read_sample(&bytes, t, i).map_err(|e| e.to_string())?, track.samples.get(i).map_or(0, |s| s.pts))))
        .collect::<Result<_, String>>()?;
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(&entry).ok_or("not an hvcC stream")?.map_err(|e| e.to_string())?;
    let (dev, q) = filmcraft_gpu::GpuCompositor::headless_device().ok_or("no GPU")?;
    println!(
        "decode: {}; compositing GPU: DMA-BUF import {}",
        filmcraft_platform::va::Display::for_hevc_422()?.vendor,
        filmcraft_gpu::interop::can_import(&dev)
    );
    let mut comp = filmcraft_gpu::GpuCompositor::new(&dev, &q);

    for (label, handoff, to_gpu) in
        [("read-back", false, false), ("hand-off decode", true, false), ("read-back → GPU", false, true), ("hand-off → GPU", true, true)]
    {
        let mut dec = filmcraft_platform::va_hevc::NativeVaDecoder::new(info.clone())?;
        dec.set_handoff(handoff);
        let (in_place, read_back) = (comp.gpu_frames_in_place, comp.gpu_frames_read_back);
        let mut per = Vec::new();
        let mut n = 0usize;
        let mut kind = String::new();
        let t0 = Instant::now();
        let draw = |f: &filmcraft_frame::VideoFrame, comp: &mut filmcraft_gpu::GpuCompositor| {
            let layer = filmcraft_render::plan::PlanLayer::new(Arc::new(f.clone()), filmcraft_geom::Affine::IDENTITY, 1.0, filmcraft_render::Blend::Normal);
            let plan = filmcraft_render::plan::FramePlan::Layers { width: 64, height: 64, layers: vec![layer], output: None };
            comp.composite(&plan);
            let _ = dev.poll(wgpu::PollType::wait_indefinitely());
        };
        let mut feed = |out: Vec<filmcraft_codecs::DecodedFrame>, per: &mut Vec<f64>, last: &mut Instant, comp: &mut filmcraft_gpu::GpuCompositor| {
            for f in out {
                if kind.is_empty() {
                    kind = match &f.frame.data {
                        PixelData::Gpu(s) => format!("{:?} GPU frame", s.format()),
                        d => format!("{} in memory", if matches!(d, PixelData::Yuv16Bi { .. }) { "Yuv16Bi" } else { "other" }),
                    };
                }
                if to_gpu {
                    draw(&f.frame, comp);
                }
                n += 1;
                per.push(last.elapsed().as_secs_f64() * 1e3);
                *last = Instant::now();
            }
        };
        let mut last = Instant::now();
        for (s, p) in &samples {
            let out = dec.decode(s, *p).map_err(|e| e.to_string())?;
            feed(out, &mut per, &mut last, &mut comp);
        }
        let out = dec.flush();
        feed(out, &mut per, &mut last, &mut comp);
        let secs = t0.elapsed().as_secs_f64();
        // the first pictures include the reorder delay and warm-up: the median of the rest
        let mut steady: Vec<f64> = per.iter().skip(4).copied().collect();
        steady.sort_by(f64::total_cmp);
        let median = steady.get(steady.len() / 2).copied().unwrap_or(0.0);
        let p90 = steady.get(steady.len() * 9 / 10).copied().unwrap_or(0.0);
        println!(
            "{label:>16}: {n} pictures in {secs:.2} s = {:.1} ms/picture ({:.1} fps), steady median {median:.1} ms, p90 {p90:.1} ms; {kind}; GPU in place {} / read back {}",
            secs * 1e3 / n.max(1) as f64,
            n as f64 / secs.max(1e-9),
            comp.gpu_frames_in_place - in_place,
            comp.gpu_frames_read_back - read_back,
        );
    }
    Ok(())
}
