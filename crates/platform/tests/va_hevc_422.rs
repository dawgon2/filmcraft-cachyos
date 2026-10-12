//! HEVC Main 4:2:2 10 through FilmCraft's own VA-API decoder (`va_hevc`, fork Phase 6.1b) on the
//! display whose driver has `VAProfileHEVCMain422_10` (Intel's media engine): bit-exact with the
//! reference on the 4:2:2 conformance fixtures (`target/fixtures/hevc/*_422.hevc` + `.yuv`, the
//! ffmpeg oracle the software decoder already matches) and with the built-in software decoder on
//! the first pictures of the 6.2K xm5 4:2:2 clip; a mid-stream hardware failure continues in
//! software with the software decoder's exact output; damaged samples and parameter sets and
//! formats the profile does not cover give errors, never a crash or a hang. Skips (with SKIPPED)
//! when no display has the profile or the fixtures are missing.
#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::*;
use filmcraft_codecs::VideoDecoder;
use filmcraft_codecs::hw::NalStreamInfo;
use filmcraft_frame::PixelData;
use filmcraft_platform::HybridDecoder;
use filmcraft_platform::va_hevc::{NAME_422, NativeVaDecoder, decode_annexb};

/// The tests take turns on the media engine: a damaged stream can make Intel's engine reset
/// (i915 "GPU HANG", recovered in 1–7 s), which corrupts pictures other contexts decode at that
/// moment, so a parity test must not run beside the damaged-stream test.
static ENGINE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn engine() -> std::sync::MutexGuard<'static, ()> {
    ENGINE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The 4:2:2 display, or `None` (skip) when no render node has the profile.
fn display_422() -> Option<std::sync::Arc<filmcraft_platform::va::Display>> {
    match filmcraft_platform::va::Display::for_hevc_422() {
        Ok(d) => {
            eprintln!("HEVC 4:2:2 on {} ({})", d.node, d.vendor);
            Some(d)
        }
        Err(e) => {
            eprintln!("SKIPPED: no VA-API display with HEVC Main 4:2:2 10: {e}");
            None
        }
    }
}

fn hevc_fixtures() -> std::path::PathBuf {
    filmcraft_testkit::fixtures::fixtures_root().join("hevc")
}

/// Every picture's Y, U, V planes back to back as 16-bit little-endian codes (the `.yuv` layout).
fn planar_bytes(frames: &[filmcraft_frame::VideoFrame]) -> Result<Vec<u8>, String> {
    let mut got = Vec::new();
    for f in frames {
        let p = f.to_planar().ok_or("not biplanar")?;
        match &p.data {
            PixelData::Yuv16 { planes, chroma, bits, .. } => {
                if *chroma != filmcraft_frame::Chroma::C422 || *bits != 10 {
                    return Err(format!("{chroma:?} {bits}-bit"));
                }
                planes.iter().for_each(|p| p.iter().for_each(|v| got.extend_from_slice(&v.to_le_bytes())));
            }
            _ => return Err("unexpected pixel format".into()),
        }
    }
    Ok(got)
}

#[test]
fn bit_exact_with_the_reference_on_the_422_fixtures() {
    let _engine = engine();
    if display_422().is_none() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(hevc_fixtures()) else {
        eprintln!("SKIPPED: no fixtures in {} (cargo xtask fixtures hevc)", hevc_fixtures().display());
        return;
    };
    let mut names: Vec<_> =
        entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "hevc")).filter(|p| p.to_string_lossy().contains("_422")).collect();
    names.sort();
    if names.is_empty() {
        eprintln!("SKIPPED: no 4:2:2 fixtures");
        return;
    }
    let (mut failures, mut checked) = (Vec::new(), 0);
    for path in names {
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(reference) = std::fs::read(path.with_extension("yuv")) else { continue };
        let data = std::fs::read(&path).expect("read fixture");
        let t0 = Instant::now();
        let frames = match decode_annexb(&data) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let secs = t0.elapsed().as_secs_f64();
        checked += 1;
        match planar_bytes(&frames) {
            Ok(got) if got == reference => {
                eprintln!("{name}: {} frames bit-exact ({:.0} fps incl. read-back)", frames.len(), frames.len() as f64 / secs.max(1e-9))
            }
            Ok(got) => {
                let first = got.iter().zip(&reference).position(|(a, b)| a != b);
                failures.push(format!(
                    "{name}: {} frames, {} bytes vs {} reference; first difference at byte {first:?}",
                    frames.len(),
                    got.len(),
                    reference.len()
                ));
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    eprintln!("{checked} 4:2:2 fixtures checked");
    assert!(failures.is_empty(), "VA-API 4:2:2 differs from the reference:\n{}", failures.join("\n"));
}

/// The first `n` samples (decode order) of the video track of a large MOV, without reading the
/// samples after them.
fn first_samples(path: &std::path::Path, n: usize) -> Option<Stream> {
    let bytes = std::fs::read(path).ok()?;
    let file = filmcraft_isobmff::open(bytes.clone()).ok()?;
    let t = file.track_of_kind(filmcraft_isobmff::TrackKind::Video)?;
    let track = file.tracks.get(t)?;
    let n = n.min(track.samples.len());
    let samples = (0..n).map(|i| Some((file.read_sample(&bytes, t, i).ok()?, track.samples[i].pts))).collect::<Option<Vec<_>>>()?;
    Some(Stream { entry: track.entries.first()?.clone(), samples, sync: track.samples.iter().take(n).map(|s| s.is_sync).collect() })
}

fn xm5_422() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("FILMCRAFT_XM5_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| filmcraft_testkit::fixtures::fixtures_root().join("xm5"));
    let p = dir.join("xm5_422_2997_20s.mov");
    if p.exists() {
        Some(p)
    } else {
        eprintln!("SKIPPED: no {}", p.display());
        None
    }
}

#[test]
fn bit_exact_with_the_software_decoder_on_the_xm5_422_clip() {
    let _engine = engine();
    if display_422().is_none() {
        return;
    }
    let Some(path) = xm5_422() else { return };
    let Some(s) = first_samples(&path, 10) else { panic!("unreadable {}", path.display()) };
    let info = NalStreamInfo::from_entry(&s.entry).expect("HEVC").expect("hvcC");
    assert_eq!((info.chroma_format_idc, info.bit_depth_luma), (2, 10), "a Main 4:2:2 10 clip");
    let mut hw = NativeVaDecoder::new(info).expect("taken in hardware");
    assert_eq!(hw.name(), NAME_422);
    let t0 = Instant::now();
    let got = decode_all(&mut hw, &s.samples);
    let hw_secs = t0.elapsed().as_secs_f64();
    let t0 = Instant::now();
    let reference = decode_all(filmcraft_codecs::software_video_decoder(&s.entry).expect("software").as_mut(), &s.samples);
    let sw_secs = t0.elapsed().as_secs_f64();
    assert_eq!(got.len(), s.samples.len(), "every picture");
    assert_same("xm5 4:2:2", &got, &reference);
    let (w, h) = (got[0].frame.width, got[0].frame.height);
    eprintln!(
        "xm5 4:2:2 {w}x{h}: {} pictures bit-exact; single run (synthetic, debug test build): hardware + read-back {:.1} fps, software {:.1} fps",
        got.len(),
        got.len() as f64 / hw_secs.max(1e-9),
        got.len() as f64 / sw_secs.max(1e-9)
    );
}

/// A small 4:2:2 MP4 (ffmpeg-made, cached).
fn mp4_422(ff: &std::path::Path, name: &str, pix_fmt: &str, profile: &str) -> Option<std::path::PathBuf> {
    let params = "log-level=error:bframes=3:keyint=24:min-keyint=24:scenecut=0:weightp=1";
    let args = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=640x360:r=24:d=2,noise=alls=12:allf=t,fade=in:0:20",
        "-c:v",
        "libx265",
        "-preset",
        "fast",
        "-profile:v",
        profile,
        "-tag:v",
        "hvc1",
        "-x265-params",
        params,
        "-pix_fmt",
        pix_fmt,
    ];
    fixture(ff, name, &args)
}

/// The factory takes 4:2:2 in hardware and matches software; a forced mid-stream failure goes on
/// in software with the software decoder's exact output, and the reason is reported.
#[test]
fn factory_parity_and_fallback_to_software() {
    let _engine = engine();
    let ff = filmcraft_testkit::require_ffmpeg!();
    if display_422().is_none() {
        return;
    }
    let Some(path) = mp4_422(&ff, "hevc_422_10_wp.mp4", "yuv422p10le", "main422-10") else { return };
    let s = read_stream(&path);
    let reference = decode_all(filmcraft_codecs::software_video_decoder(&s.entry).unwrap().as_mut(), &s.samples);
    let mut d = filmcraft_platform::vaapi_factory(&s.entry).expect("taken").expect("decoder");
    assert_eq!(d.name(), NAME_422);
    let out = decode_all(d.as_mut(), &s.samples);
    assert_same("4:2:2 through the factory", &out, &reference);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    let k = (1..s.samples.len()).find(|&i| s.sync[i]).unwrap();
    for fail in [3u64, k as u64 + 2] {
        let mut native = NativeVaDecoder::new(info.clone()).unwrap();
        native.fail_after(fail);
        let mut d = HybridDecoder::new(Box::new(native), s.entry.clone(), info.clone());
        let out = decode_all(&mut d, &s.samples);
        assert_same(&format!("4:2:2 failing at {fail}"), &out, &reference);
        assert!(!d.is_hardware(), "ended in software ({})", d.name());
        let (backends, why) = filmcraft_codecs::hw::backend_stats();
        assert!(backends.iter().any(|(b, _)| b == NAME_422), "{backends:?}");
        assert!(why.is_some_and(|w| w.contains("failed")), "the fallback reason is reported");
    }
    // after a hardware failure 4:2:2 clips say they decode in software again
    assert!(!filmcraft_platform::hevc_422_in_hardware());
}

/// Run `f` on a thread; fail if it panics or takes longer than `limit`.
fn bounded(what: &str, limit: Duration, f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(r.is_ok());
    });
    match rx.recv_timeout(limit) {
        Ok(true) => {}
        Ok(false) => panic!("{what}: panicked"),
        Err(_) => panic!("{what}: hung"),
    }
}

/// Damaged input gives errors (or the hybrid's software fallback), never a crash or a hang. Some
/// damaged slices still make Intel's engine reset (kernel-recovered, the decode returns an
/// error after 1–7 s); feeding on after a failed picture made it reset on most seeds, so the
/// decoder stops a 4:2:2 stream at its first failure until the next seek, and truncated slices
/// whose entry points run past their end are refused before the engine sees them.
#[test]
fn damaged_422_samples_and_parameter_sets_never_crash_or_hang() {
    let _engine = engine();
    let ff = filmcraft_testkit::require_ffmpeg!();
    if display_422().is_none() {
        return;
    }
    let Some(path) = mp4_422(&ff, "hevc_422_10_wp.mp4", "yuv422p10le", "main422-10") else { return };
    let s = read_stream(&path);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    // damaged samples (bit flips, truncation, corrupt length prefixes): straight into the hardware
    // decoder (errors only) and through the hybrid
    for seed in 1..=8u64 {
        let (samples, entry, info) = (s.samples.clone(), s.entry.clone(), info.clone());
        bounded(&format!("seed {seed}"), Duration::from_secs(60), move || {
            let mut seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut native = NativeVaDecoder::new(info).unwrap();
            let mut hybrid = filmcraft_platform::vaapi_factory(&entry).unwrap().unwrap();
            for (i, (smp, pts)) in samples.iter().enumerate().take(40) {
                let mut m = smp.clone();
                match xorshift(&mut seed) % 4 {
                    0 => {
                        for _ in 0..1 + xorshift(&mut seed) % 8 {
                            let at = (xorshift(&mut seed) as usize) % m.len().max(1);
                            if let Some(b) = m.get_mut(at) {
                                *b ^= 1 << (xorshift(&mut seed) % 8);
                            }
                        }
                    }
                    1 => m.truncate((xorshift(&mut seed) as usize) % m.len().max(1)),
                    2 if m.len() > 4 => m[..4].copy_from_slice(&(xorshift(&mut seed) as u32).to_be_bytes()),
                    _ => {}
                }
                let _ = native.decode(&m, *pts);
                let _ = hybrid.decode(&m, *pts);
                if i % 13 == 12 {
                    native.reset();
                    hybrid.reset();
                }
            }
            let _ = native.flush();
            let _ = hybrid.flush();
        });
    }
    // damaged parameter sets (absurd sizes, depths, formats among them): declined or errors
    let filmcraft_isobmff::CodecConfig::Hevc(c) = &s.entry.codec else { panic!("HEVC") };
    let rec = c.to_bytes();
    let mut seed = 0x5151_4242u64;
    for round in 0..32 {
        let mut r = rec.clone();
        if round % 3 == 2 {
            r.truncate(r.len() * (round + 1) / 36);
        } else {
            for _ in 0..1 + round % 4 {
                let at = 6 + (xorshift(&mut seed) as usize) % r.len().saturating_sub(6).max(1);
                if let Some(b) = r.get_mut(at) {
                    *b ^= 1 << (xorshift(&mut seed) % 8);
                }
            }
        }
        let samples: Vec<_> = s.samples.iter().take(12).cloned().collect();
        bounded(&format!("record round {round}"), Duration::from_secs(60), move || {
            let Ok(info) = NalStreamInfo::from_hvcc(&r) else { return };
            let Ok(mut d) = NativeVaDecoder::new(info) else { return };
            for (smp, pts) in &samples {
                if d.decode(smp, *pts).is_err() {
                    break;
                }
            }
            let _ = d.flush();
        });
    }
}

/// Formats Main 4:2:2 10 does not cover are declined up front: 8-bit and 12-bit 4:2:2, and
/// stream descriptions with absurd or mismatched depths.
#[test]
fn other_422_depths_are_declined() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    if let Some(path) = mp4_422(&ff, "hevc_422_8.mp4", "yuv422p", "main422-10") {
        let s = read_stream(&path);
        let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
        assert_eq!((info.chroma_format_idc, info.bit_depth_luma), (2, 8));
        let e = NativeVaDecoder::new(info).err().expect("8-bit 4:2:2 declined");
        assert!(e.contains("chroma format 2"), "{e}");
    }
    if let Some(path) = mp4_422(&ff, "hevc_422_12.mp4", "yuv422p12le", "main422-12") {
        let s = read_stream(&path);
        let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
        assert!(NativeVaDecoder::new(info).is_err(), "12-bit 4:2:2 declined");
    }
    let Some(path) = mp4_422(&ff, "hevc_422_10_wp.mp4", "yuv422p10le", "main422-10") else { return };
    let s = read_stream(&path);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    for (luma, chroma) in [(10, 8), (8, 10), (0, 0), (16, 16), (u32::MAX, u32::MAX)] {
        let mut i = info.clone();
        (i.bit_depth_luma, i.bit_depth_chroma) = (luma, chroma);
        assert!(NativeVaDecoder::new(i).is_err(), "{luma}/{chroma}-bit declined");
    }
    let mut i = info.clone();
    i.chroma_format_idc = 3;
    assert!(NativeVaDecoder::new(i).is_err(), "4:4:4 declined");
}
