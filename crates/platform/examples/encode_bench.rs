//! HEVC export encoder throughput on synthetic frames (no timeline rendering): RGBA → YUV
//! conversion, ffmpeg (VA-API or libx265) and access-unit parsing, as an export runs them.
//!
//! `cargo run --release -p filmcraft-platform --example encode_bench -- [vaapi|x265] [8|10] [frames] [WxH] [fps]`

use std::time::Instant;

use filmcraft_export::{EncoderFrame, ExportSettings, Format, HevcEncoder, VideoEncoder};
use filmcraft_time::FrameRate;

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let encoder = HevcEncoder::from_name(a.first().map_or("vaapi", String::as_str)).ok_or("encoder: auto | vaapi | x265")?;
    let bits: u8 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(8);
    let frames: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let (w, h) = a.get(3).and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).unwrap_or((3840u32, 2160u32));
    let fps: i64 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(60);
    let s = ExportSettings {
        format: Format::Hevc,
        hevc_encoder: encoder,
        hevc_bit_depth: bits,
        bitrate_kbps: 40_000,
        max_bitrate_kbps: Some(50_000),
        keyframe_distance: Some(fps as u32),
        ..Default::default()
    };
    // a few distinct moving pictures, reused
    let pics: Vec<Vec<u8>> = (0..8u32)
        .map(|k| {
            let mut v = vec![0u8; (w * h * 4) as usize];
            for (i, px) in v.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i as u32 % w, i as u32 / w);
                *px = [((x + k * 16) % 256) as u8, ((y + k * 8) % 256) as u8, ((x ^ y) % 256) as u8, 255];
            }
            v
        })
        .collect();
    let t0 = Instant::now();
    let mut enc = filmcraft_platform::hevc_export::HevcEncoder::new(w, h, FrameRate::new(fps, 1), &s).map_err(|e| e.to_string())?;
    let setup = t0.elapsed().as_secs_f64();
    let t1 = Instant::now();
    let mut bytes = 0usize;
    let mut packets = 0usize;
    for i in 0..frames {
        let rgba = pics.get(i as usize % pics.len()).ok_or("no picture")?;
        for p in enc.encode(&EncoderFrame { width: w, height: h, rgba, hdr: None, yuv: None, gpu: None, index: i }).map_err(|e| e.to_string())? {
            bytes += p.data.len();
            packets += 1;
        }
    }
    for p in enc.flush().map_err(|e| e.to_string())? {
        bytes += p.data.len();
        packets += 1;
    }
    let secs = t1.elapsed().as_secs_f64();
    println!(
        "{:?} {w}x{h} {bits}-bit: {packets} packets, {:.1} MB, setup {setup:.2} s, {frames} frames in {secs:.2} s = {:.1} fps",
        enc.backend(),
        bytes as f64 / 1e6,
        frames as f64 / secs.max(1e-9)
    );
    Ok(())
}
