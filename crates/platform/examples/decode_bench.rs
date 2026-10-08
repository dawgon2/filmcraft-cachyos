//! Decode throughput of an MP4's video track: FilmCraft's software decoder vs the registered
//! hardware decoder (VA-API on Linux, VideoToolbox on macOS).
//!
//! `cargo run --release -p filmcraft-platform --example decode_bench -- <file.mp4> [frames]`

use std::time::Instant;

use filmcraft_codecs::VideoDecoder;

fn run(label: &str, d: &mut dyn VideoDecoder, samples: &[(Vec<u8>, i64)]) -> Result<(), String> {
    let t0 = Instant::now();
    let mut n = 0usize;
    for (s, p) in samples {
        n += d.decode(s, *p).map_err(|e| e.to_string())?.len();
    }
    n += d.flush().len();
    let secs = t0.elapsed().as_secs_f64();
    println!("{label:<10} {:<22} {n:>5} frames in {secs:>6.2} s = {:>7.1} fps", d.name(), n as f64 / secs.max(1e-9));
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: decode_bench <file.mp4> [frames]")?;
    let limit: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(usize::MAX);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let file = filmcraft_isobmff::open(bytes.clone()).map_err(|e| e.to_string())?;
    let t = file.track_of_kind(filmcraft_isobmff::TrackKind::Video).ok_or("no video track")?;
    let track = file.tracks.get(t).ok_or("no video track")?;
    let entry = track.entries.first().ok_or("no sample entry")?.clone();
    let samples: Vec<(Vec<u8>, i64)> = (0..track.samples.len().min(limit))
        .map(|i| Ok((file.read_sample(&bytes, t, i).map_err(|e| e.to_string())?, track.samples.get(i).map_or(0, |s| s.pts))))
        .collect::<Result<_, String>>()?;
    println!("{path}: {} samples", samples.len());

    let mut sw = filmcraft_codecs::software_video_decoder(&entry).map_err(|e| e.to_string())?;
    run("software", sw.as_mut(), &samples)?;

    println!("hardware: {:?}", filmcraft_platform::register());
    let mut hw = filmcraft_codecs::make_video_decoder(&entry).map_err(|e| e.to_string())?;
    run("hardware", hw.as_mut(), &samples)?;
    Ok(())
}
