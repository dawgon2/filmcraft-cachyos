//! In-process VA-API HEVC decode of an MP4 / MOV video track: throughput (pictures read back to
//! memory) and bit-exactness against FilmCraft's software decoder on the first pictures.
//!
//! `cargo run --release -p filmcraft-platform --example va_native_bench -- <file> [frames] [compare]`

use std::time::Instant;

use filmcraft_codecs::VideoDecoder;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: va_native_bench <file> [frames] [compare]")?;
    let limit: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(usize::MAX);
    let compare: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let file = filmcraft_isobmff::open(bytes.clone()).map_err(|e| e.to_string())?;
    let t = file.track_of_kind(filmcraft_isobmff::TrackKind::Video).ok_or("no video track")?;
    let track = file.tracks.get(t).ok_or("no video track")?;
    let entry = track.entries.first().ok_or("no sample entry")?.clone();
    let samples: Vec<(Vec<u8>, i64)> = (0..track.samples.len().min(limit))
        .map(|i| Ok((file.read_sample(&bytes, t, i).map_err(|e| e.to_string())?, track.samples.get(i).map_or(0, |s| s.pts))))
        .collect::<Result<_, String>>()?;
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(&entry).ok_or("not an avcC / hvcC stream")?.map_err(|e| e.to_string())?;
    let mut hw = filmcraft_platform::va_hevc::NativeVaDecoder::new(info)?;
    let t0 = Instant::now();
    let mut out = Vec::new();
    for (s, p) in &samples {
        let f = hw.decode(s, *p).map_err(|e| e.to_string())?;
        if out.len() < compare {
            out.extend(f);
        } else {
            out.extend(f.into_iter().take(0));
        }
    }
    let n_flush = hw.flush().len();
    let secs = t0.elapsed().as_secs_f64();
    let n = samples.len();
    println!("{path}: {n} samples ({n_flush} at flush) in {secs:.2} s = {:.1} fps (in-process VA-API, read back)", n as f64 / secs.max(1e-9));
    if compare > 0 {
        let mut sw = filmcraft_codecs::software_video_decoder(&entry).map_err(|e| e.to_string())?;
        let mut want = Vec::new();
        for (s, p) in &samples {
            want.extend(sw.decode(s, *p).map_err(|e| e.to_string())?);
            if want.len() >= compare {
                break;
            }
        }
        let mut same = 0;
        for (a, b) in out.iter().zip(&want).take(compare) {
            let (pa, pb) = (a.frame.to_planar(), b.frame.to_planar());
            let (fa, fb) = (pa.as_ref().unwrap_or(&a.frame), pb.as_ref().unwrap_or(&b.frame));
            let ok = a.pts == b.pts
                && match (&fa.data, &fb.data) {
                    (filmcraft_frame::PixelData::Yuv16 { planes: x, .. }, filmcraft_frame::PixelData::Yuv16 { planes: y, .. }) => x == y,
                    (filmcraft_frame::PixelData::Yuv8 { planes: x, .. }, filmcraft_frame::PixelData::Yuv8 { planes: y, .. }) => x == y,
                    _ => false,
                };
            same += ok as usize;
        }
        println!("bit-exact with the software decoder: {same} of {}", out.len().min(want.len()).min(compare));
    }
    Ok(())
}
