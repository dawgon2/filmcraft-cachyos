//! Stream-copy trim tests: cut ffmpeg-made files with `trim_mp4`, then check the result with our
//! demuxer (sample bytes identical to the source) and with ffprobe / ffmpeg as oracles.

mod common;

use common::*;
use filmcraft_isobmff::*;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

const TICKS: u64 = 254_016_000_000;

fn ms(v: i64) -> PresentationTime {
    PresentationTime::from_millis(v)
}

/// Generate a trim fixture into `target/fixtures/isobmff/` (None + note when ffmpeg or the encoder is missing).
fn fixture_gen(name: &str, args: &[&str]) -> Option<PathBuf> {
    let ff = ffmpeg()?;
    let path = fixture_dir().join(name);
    if path.exists() {
        return Some(path);
    }
    let tmp = filmcraft_testkit::temp_path(&path);
    let o = Command::new("nice").args(["-n", "19", "ionice", "-c", "3"]).arg(&ff).args(["-v", "error", "-y", "-nostdin"]).args(args).arg(&tmp).output().ok()?;
    if !o.status.success() {
        eprintln!("SKIPPED {name}: ffmpeg failed: {}", String::from_utf8_lossy(&o.stderr));
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

const V: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=25"];
const A: &[&str] = &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"];

fn h264_aac() -> Option<PathBuf> {
    fixture_gen("trim_h264_aac.mp4", &[V, A, &["-t", "4", "-c:v", "libx264", "-bf", "2", "-g", "24", "-pix_fmt", "yuv420p", "-c:a", "aac"]].concat())
}

fn h264_pcm() -> Option<PathBuf> {
    fixture_gen("trim_h264_pcm.mov", &[V, A, &["-t", "4", "-c:v", "libx264", "-bf", "2", "-g", "24", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"]].concat())
}

fn hevc_aac() -> Option<PathBuf> {
    fixture_gen(
        "trim_hevc_aac.mp4",
        &[V, A, &["-t", "4", "-c:v", "libx265", "-tag:v", "hvc1", "-x265-params", "log-level=error:bframes=3:keyint=24", "-pix_fmt", "yuv420p", "-c:a", "aac"]]
            .concat(),
    )
}

fn timecode_mov() -> Option<PathBuf> {
    fixture_gen("trim_tmcd.mov", &[V, &["-t", "4", "-c:v", "libx264", "-bf", "2", "-g", "24", "-pix_fmt", "yuv420p", "-timecode", "01:00:00:00"]].concat())
}

fn h264_open_gop() -> Option<PathBuf> {
    fixture_gen(
        "trim_h264_opengop.mp4",
        &[V, &["-t", "4", "-c:v", "libx264", "-bf", "2", "-x264-params", "open_gop=1:keyint=24:min-keyint=24:scenecut=0", "-pix_fmt", "yuv420p"]].concat(),
    )
}

fn hevc_open_gop() -> Option<PathBuf> {
    fixture_gen(
        "trim_hevc_cra.mp4",
        &[
            V,
            &[
                "-t",
                "4",
                "-c:v",
                "libx265",
                "-tag:v",
                "hvc1",
                "-x265-params",
                "log-level=error:bframes=3:keyint=24:min-keyint=24:scenecut=0:open-gop=1",
                "-pix_fmt",
                "yuv420p",
            ],
        ]
        .concat(),
    )
}

/// Trim and check, or accept a refusal that asks the caller to copy the whole file.
fn check_or_refused(p: &Path, s: i64, e: i64, name: &str) -> Option<TrimReport> {
    let data = std::fs::read(p).unwrap();
    match trim_mp4(data.as_slice(), ms(s), ms(e), Cursor::new(Vec::new())) {
        Err(err) => {
            assert!(err.copy_whole_file_instead(), "{name}: {err}");
            eprintln!("{name}: refused ({err})");
            None
        }
        Ok(_) => Some(check_trim(p, s, e, name).3),
    }
}

fn fragmented() -> Option<PathBuf> {
    fixture_gen(
        "trim_frag.mp4",
        &[V, A, &["-t", "3", "-c:v", "libx264", "-bf", "2", "-g", "24", "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "frag_keyframe+empty_moov"]]
            .concat(),
    )
}

fn format_duration(path: &Path) -> Option<f64> {
    let v = probe_json(path, &["-show_format"])?;
    v["format"]["duration"].as_str()?.parse().ok()
}

/// Per-frame MD5s of the decoded first video stream (ffmpeg oracle).
fn video_md5s(path: &Path) -> Option<Vec<String>> {
    let ff = ffmpeg()?;
    let o = Command::new(ff).args(["-v", "error", "-nostdin", "-i"]).arg(path).args(["-map", "0:v:0", "-f", "framemd5", "-"]).output().ok()?;
    if !o.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    Some(text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.rsplit(',').next()).map(|s| s.trim().to_string()).collect())
}

fn pcm_bytes(f: &Mp4File, data: &[u8], t: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..f.tracks[t].samples.len() {
        v.extend(f.read_sample(data, t, i).unwrap());
    }
    v
}

/// Trim `[start_ms, end_ms)` and check everything the task asks for. Returns (source, output).
fn check_trim(src_path: &Path, start_ms: i64, end_ms: i64, out_name: &str) -> (Mp4File, Mp4File, Vec<u8>, TrimReport) {
    let data = std::fs::read(src_path).unwrap();
    let src = open(data.as_slice()).unwrap();
    let mut out = Cursor::new(Vec::new());
    let report = trim_mp4(data.as_slice(), ms(start_ms), ms(end_ms), &mut out).unwrap();
    let out = out.into_inner();
    assert_eq!(report.bytes_written, out.len() as u64);
    let path = out_dir().join(out_name);
    std::fs::write(&path, &out).unwrap();
    let dst = open(out.as_slice()).unwrap();
    let want = (end_ms - start_ms) as f64 / 1000.0;
    // The start snaps back to the video frame on screen (at most one frame).
    assert!(report.duration_seconds >= want - 1e-9 && report.duration_seconds <= want + 0.04 + 1e-9, "{out_name}: report duration {}", report.duration_seconds);
    let origin = report.origin.value as f64 / report.origin.timescale as f64;
    assert!((end_ms as f64 / 1000.0 - report.duration_seconds - origin).abs() < 0.002, "{out_name}: origin {origin} vs duration");
    assert!(origin <= start_ms as f64 / 1000.0 + 1e-9 && origin > start_ms as f64 / 1000.0 - 0.04, "{out_name}: origin {origin}");

    for (k, rt) in report.tracks.iter().enumerate() {
        let si = src.tracks.iter().position(|t| t.id == rt.source_track_id).unwrap();
        let (s, d) = (&src.tracks[si], &dst.tracks[k]);
        assert_eq!(s.kind, d.kind);
        assert!(d.samples.len() < s.samples.len() || s.pcm_chunked, "{out_name}: track {k} not smaller");
        // Presented duration of the edit list ≈ end − start (within one frame).
        let presented: u64 = d.edits.iter().map(|e| e.segment_duration).sum();
        let presented = presented as f64 / dst.timescale as f64;
        let frame = if d.kind == TrackKind::Video { 1.0 / 25.0 } else { 1024.0 / 48000.0 };
        assert!((presented - report.duration_seconds).abs() <= frame + 1e-6, "{out_name}: track {k} presents {presented}s, want {want}s");
        assert!(rt.kept_start_seconds <= origin + 1e-9 && rt.kept_end_seconds + frame >= end_ms as f64 / 1000.0, "{out_name}: kept {rt:?}");
        if s.pcm_chunked {
            let a = pcm_bytes(&src, &data, si);
            let b = pcm_bytes(&dst, &out, k);
            assert!(b.len() < a.len());
            let fb = (s.samples[0].size / s.samples[0].duration) as usize;
            let skip = (origin * 48000.0).round() as usize * fb;
            assert_eq!(&a[skip..skip + b.len()], &b[..], "{out_name}: PCM bytes");
            assert_eq!(b.len(), (report.duration_seconds * 48000.0).round() as usize * fb, "{out_name}: PCM exact length");
            continue;
        }
        if d.kind == TrackKind::Video {
            assert!(d.samples[0].is_sync, "{out_name}: first video sample must be sync");
        }
        // Find where the copied run starts in the source and prove every sample is a byte copy.
        let first = dst.read_sample(out.as_slice(), k, 0).unwrap();
        let a = (0..s.samples.len()).find(|&i| s.samples[i].size == d.samples[0].size && src.read_sample(data.as_slice(), si, i).unwrap() == first).unwrap();
        for j in 0..d.samples.len() {
            assert_eq!(src.read_sample(data.as_slice(), si, a + j).unwrap(), dst.read_sample(out.as_slice(), k, j).unwrap(), "{out_name}: sample {j}");
            let (x, y) = (s.samples[a + j], d.samples[j]);
            assert_eq!((x.duration, x.pts - x.dts), (y.duration, y.pts - y.dts), "{out_name}: sample {j} timing");
            // Sync flags are kept, except on sync samples that are not clean entry points.
            assert!(!y.is_sync || x.is_sync, "{out_name}: sample {j} became sync");
        }
    }

    // Oracles.
    if let Some(dur) = format_duration(&path) {
        assert!((dur - want).abs() <= 0.045, "{out_name}: ffprobe duration {dur}, want {want}");
    }
    if let Some(errs) = ffmpeg_decode_errors(&path) {
        assert!(errs.trim().is_empty(), "{out_name}: ffmpeg decode errors: {errs}");
    }
    // Decoded pictures: exactly the source's frames from the origin frame up to `end` (25 fps).
    if let (Some(a), Some(b)) = (video_md5s(src_path), video_md5s(&path)) {
        let first = (origin * 25.0).round() as usize;
        let last = ((end_ms as f64 / 1000.0) * 25.0).ceil() as usize;
        assert_eq!(&b[..], &a[first..last.min(a.len())], "{out_name}: decoded frames");
    }
    (src, dst, out, report)
}

#[test]
fn h264_bframes_aac_mp4() {
    let Some(p) = h264_aac() else { return };
    let (_, dst, _, report) = check_trim(&p, 1300, 2900, "trim_h264_aac_out.mp4");
    assert_eq!(report.tracks.len(), 2);
    assert!(report.dropped.is_empty());
    assert!(!dst.is_quicktime);
    // A cut on a GOP boundary and one ending at the file end.
    check_trim(&p, 960, 1920, "trim_h264_aac_gop.mp4");
    check_trim(&p, 2500, 4000, "trim_h264_aac_tail.mp4");
}

#[test]
fn h264_pcm_mov() {
    let Some(p) = h264_pcm() else { return };
    let (_, dst, _, report) = check_trim(&p, 700, 3100, "trim_h264_pcm_out.mov");
    assert!(dst.is_quicktime);
    assert_eq!(report.tracks.len(), 2);
}

#[test]
fn hevc_bframes() {
    let Some(p) = hevc_aac() else { return };
    let src = open(std::fs::read(&p).unwrap().as_slice()).unwrap();
    if !matches!(src.tracks.iter().find(|t| t.kind == TrackKind::Video).and_then(|t| t.codec()), Some(CodecConfig::Hevc(_))) {
        eprintln!("SKIPPED hevc trim: fixture is not HEVC");
        return;
    }
    check_trim(&p, 1100, 2700, "trim_hevc_out.mp4");
}

#[test]
fn timecode_shifted() {
    let Some(p) = timecode_mov() else { return };
    let (src, dst, _, _) = check_trim(&p, 1000, 3000, "trim_tmcd_out.mov");
    let tc_of = |f: &Mp4File| {
        f.tracks.iter().find(|t| t.kind == TrackKind::Timecode).and_then(|t| match t.codec() {
            Some(CodecConfig::Timecode(tc)) => tc.start_frame,
            _ => None,
        })
    };
    let (a, b) = (tc_of(&src).unwrap(), tc_of(&dst).unwrap());
    assert_eq!(b, a + 25, "timecode advanced by the 25 frames cut");
}

#[test]
fn tick_timescale_matches_millis() {
    let Some(p) = h264_aac() else { return };
    let data = std::fs::read(&p).unwrap();
    let mut a = Cursor::new(Vec::new());
    let mut b = Cursor::new(Vec::new());
    trim_mp4(data.as_slice(), ms(1300), ms(2900), &mut a).unwrap();
    let t = |s: i64| PresentationTime::new(s * (TICKS as i64) / 1000, TICKS);
    trim_mp4(data.as_slice(), t(1300), t(2900), &mut b).unwrap();
    assert_eq!(a.into_inner(), b.into_inner());
}

#[test]
fn fragmented_is_refused() {
    let Some(p) = fragmented() else { return };
    let data = std::fs::read(&p).unwrap();
    let e = trim_mp4(data.as_slice(), ms(500), ms(1500), Cursor::new(Vec::new())).unwrap_err();
    assert!(e.copy_whole_file_instead(), "{e}");
}

#[test]
fn hostile_inputs_fail_cleanly() {
    let Some(p) = h264_aac() else { return };
    let data = std::fs::read(&p).unwrap();
    let run = |d: &[u8], s: i64, e: i64| trim_mp4(d, ms(s), ms(e), Cursor::new(Vec::new()));
    assert!(matches!(run(&data, 2000, 2000), Err(TrimError::BadRange(_))));
    assert!(matches!(run(&data, 3000, 1000), Err(TrimError::BadRange(_))));
    assert!(matches!(run(&data, 5000, 6000), Err(TrimError::BadRange(_))));
    // Truncations: moov is at the end (no faststart), so most cuts lose it; any that parse must
    // fail on missing sample data, never panic.
    for n in [0usize, 7, 8, 100, data.len() / 3, data.len() / 2, data.len() - 1000, data.len() - 1] {
        let r = std::panic::catch_unwind(|| run(&data[..n], 1000, 2000).is_err());
        assert!(r.unwrap(), "truncated at {n} should fail");
    }
    // Random bytes.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = Vec::with_capacity(4096);
    for _ in 0..4096 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        rnd.push(seed as u8);
    }
    assert!(run(&rnd, 0, 1000).is_err());
    // Bit flips in the moov (at the end of the file) must never panic.
    let len = data.len();
    let span = len.min(3000);
    for k in 0..200usize {
        let mut d = data.clone();
        let i = len - 1 - (k * 7919 + (k * k) * 131) % span;
        d[i] ^= 1 << (k % 8);
        let r = std::panic::catch_unwind(|| {
            let _ = run(&d, 1000, 2500);
        });
        assert!(r.is_ok(), "panic with bit flip at {i}");
    }
}

#[test]
fn h264_open_gop_starts_at_idr_or_refuses() {
    let Some(p) = h264_open_gop() else { return };
    let data = std::fs::read(&p).unwrap();
    let f = open(data.as_slice()).unwrap();
    let v = f.track_of_kind(TrackKind::Video).unwrap();
    eprintln!("open-GOP sync samples: {:?}", f.tracks[v].sync_samples());
    // Just after the second and third sync samples (non-IDR I pictures).
    check_or_refused(&p, 1000, 2500, "trim_h264_opengop_1.mp4");
    check_or_refused(&p, 2000, 3500, "trim_h264_opengop_2.mp4");
}

#[test]
fn hevc_start_before_cra_keeps_previous_gop() {
    let Some(p) = hevc_open_gop() else { return };
    let data = std::fs::read(&p).unwrap();
    let f = open(data.as_slice()).unwrap();
    let Some(v) = f.track_of_kind(TrackKind::Video) else { return };
    if !matches!(f.tracks[v].codec(), Some(CodecConfig::Hevc(_))) {
        eprintln!("SKIPPED hevc CRA trim: fixture is not HEVC");
        return;
    }
    // CRA at frame 48 (1.92 s): start two frames before it, on its leading (RASL) pictures.
    let r = check_or_refused(&p, 1840, 3000, "trim_hevc_cra_out.mp4");
    if let Some(r) = r {
        // The run had to start at the previous GOP (≥ 1 GOP of pre-roll).
        let vt = r.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap();
        assert!(vt.kept_start_seconds <= 0.96 + 1e-9, "kept from {}", vt.kept_start_seconds);
    }
}

/// A hostile file: many all-sync samples with reversed presentation order, so every sync sample
/// has "shown leading pictures". The step-back must give up quickly (it used to scan once per
/// sync sample).
#[test]
fn many_sync_reversed_pts_finishes_fast() {
    let n: i32 = 200_000;
    let entry = SampleEntry {
        format: FourCc(*b"jpeg"),
        data_reference_index: 1,
        codec: CodecConfig::Jpeg { fourcc: FourCc(*b"jpeg") },
        video: Some(VideoParams { width: 16, height: 16, ..Default::default() }),
        audio: None,
        bitrate: None,
    };
    let mut opts = WriterOptions::new(Brand::Mov);
    opts.movie_timescale = 1000;
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), opts).unwrap();
    let t = w.add_track(TrackConfig::new(entry, 1000)).unwrap();
    for i in 0..n {
        w.write_sample(t, WriteSample { data: &[0], duration: 1, composition_offset: n - 2 * i, is_sync: true }).unwrap();
    }
    let data = w.finish().unwrap().into_inner();
    let t0 = std::time::Instant::now();
    let r = trim_mp4(data.as_slice(), ms(50_000), ms(150_000), Cursor::new(Vec::new()));
    assert!(t0.elapsed().as_secs() < 20, "took {:?}", t0.elapsed());
    let e = r.unwrap_err();
    assert!(e.copy_whole_file_instead(), "{e}");
}
