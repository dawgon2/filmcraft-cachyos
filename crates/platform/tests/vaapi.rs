//! VA-API decoding against our software decoders (Linux): every picture of H.264 and HEVC fixtures
//! covering the stream features the hardware path takes (H.264: CAVLC and CABAC, B-pyramids,
//! several references, weighted prediction, temporal direct, custom scaling matrices, several
//! slices per picture; HEVC Main and Main 10: open GOP with RASL pictures, scaling lists, several
//! slices with wavefront parallel processing, weighted prediction, transform skip and AMP; 1080p /
//! 2160p) must be identical (bit-exact planes, colour, pixel aspect, pts and presentation
//! order), also after `reset` + reseek and a mid-stream `flush`; a forced mid-stream failure must
//! continue in software with the software decoder's exact output; what the hardware path does not
//! take (interlaced and 10-bit H.264, 4:2:2 HEVC without a 4:2:2 display) is declined; damaged samples and parameter sets give
//! errors or fall back, never crash or hang. Skips without ffmpeg (fixture generator) or without a
//! VA-API driver.
#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use common::*;
use filmcraft_codecs::VideoDecoder;
use filmcraft_codecs::hw::NalStreamInfo;
use filmcraft_platform::HybridDecoder;
use filmcraft_platform::vaapi::VaDecoder;

/// The hardware decoder for a stream, or `None` (skip) when this machine has none for it.
fn hardware(s: &Stream) -> Option<Box<dyn VideoDecoder>> {
    match filmcraft_platform::vaapi_factory(&s.entry) {
        Some(Ok(d)) => {
            assert!(d.name().starts_with("VA-API"), "{}", d.name());
            Some(d)
        }
        Some(Err(e)) => panic!("factory error: {e}"),
        None => {
            eprintln!("SKIPPED: no VA-API hardware decoder for {}", s.entry.codec.name());
            None
        }
    }
}

fn software(s: &Stream) -> Box<dyn VideoDecoder> {
    filmcraft_codecs::software_video_decoder(&s.entry).unwrap()
}

/// H.264 fixtures for the stream features the hardware path takes: (file, x264 profile, x264
/// parameters). All 640x360 (coded 368: cropping), three seconds at 24 fps, keyframes every second.
const H264: &[(&str, &str, &str)] = &[
    ("va_h264_cbp_slices.mp4", "baseline", "keyint=24:min-keyint=24:scenecut=0:slices=3:ref=3"),
    ("va_h264_main_weighted.mp4", "main", "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0:ref=4:weightp=2:weightb=1"),
    ("va_h264_temporal_direct.mp4", "high", "bframes=2:b-pyramid=strict:direct=temporal:keyint=24:min-keyint=24:scenecut=0:ref=3:weightb=0"),
    ("va_h264_cqm_jvt.mp4", "high", "bframes=3:keyint=24:min-keyint=24:scenecut=0:cqm=jvt:8x8dct=1:slices=2"),
    ("va_h264_open_gop.mp4", "high", "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0:open-gop=1"),
];

fn h264_fixture(ff: &std::path::Path, name: &str, profile: &str, params: &str) -> Option<std::path::PathBuf> {
    let src = "testsrc2=s=640x360:r=24:d=3,noise=alls=12:allf=t";
    fixture(ff, name, &["-f", "lavfi", "-i", src, "-c:v", "libx264", "-profile:v", profile, "-preset", "fast", "-x264-params", params, "-pix_fmt", "yuv420p"])
}

/// HEVC fixtures for the stream features the hardware path takes: (file, pixel format, x265
/// parameters). 640x360, three seconds at 24 fps, keyframes every second.
const HEVC: &[(&str, &str, &str)] = &[
    ("va_hevc_scaling_lists.mp4", "yuv420p", "log-level=error:bframes=3:keyint=24:min-keyint=24:scenecut=0:scaling-list=default"),
    ("va_hevc_slices_wpp.mp4", "yuv420p10le", "log-level=error:bframes=3:keyint=24:min-keyint=24:scenecut=0:slices=3:wpp=1"),
    ("va_hevc_weighted.mp4", "yuv420p", "log-level=error:bframes=3:keyint=24:min-keyint=24:scenecut=0:weightp=1:weightb=1:ref=4"),
    ("va_hevc_tskip_amp.mp4", "yuv420p10le", "log-level=error:bframes=4:b-pyramid=1:keyint=24:min-keyint=24:scenecut=0:tskip=1:amp=1:rect=1:open-gop=1"),
];

fn hevc_fixture(ff: &std::path::Path, name: &str, pix_fmt: &str, params: &str) -> Option<std::path::PathBuf> {
    let src = "testsrc2=s=640x360:r=24:d=3,noise=alls=12:allf=t";
    let profile = if pix_fmt.contains("10") { "main10" } else { "main" };
    fixture(
        ff,
        name,
        &["-f", "lavfi", "-i", src, "-c:v", "libx265", "-preset", "fast", "-profile:v", profile, "-tag:v", "hvc1", "-x265-params", params, "-pix_fmt", pix_fmt],
    )
}

/// The small fixtures: the shared H.264 High, HEVC Main (open GOP) and Main 10 ones and the
/// feature fixtures above.
fn small_streams(ff: &std::path::Path) -> Vec<(String, Stream)> {
    let mut out = Vec::new();
    for name in ["h264_high.mp4", "hevc_main.mp4", "hevc_main10.mp4"] {
        if let Some(path) = named(ff, name) {
            out.push((name.to_string(), read_stream(&path)));
        }
    }
    for (name, profile, params) in H264 {
        if let Some(path) = h264_fixture(ff, name, profile, params) {
            out.push((name.to_string(), read_stream(&path)));
        }
    }
    for (name, pix_fmt, params) in HEVC {
        if let Some(path) = hevc_fixture(ff, name, pix_fmt, params) {
            out.push((name.to_string(), read_stream(&path)));
        }
    }
    out
}

/// Every picture of `s` from the hardware decoder equals the software decoder's: whole stream,
/// after `reset` + reseek to each later sync sample, after a mid-stream `flush`, and a full pass
/// after all the resets.
fn parity(name: &str, s: &Stream) {
    let Some(mut hw) = hardware(s) else { return };
    let mut sw = software(s);
    let a = decode_all(hw.as_mut(), &s.samples);
    let b = decode_all(sw.as_mut(), &s.samples);
    assert!(a.len() >= 40, "{name}: {} pictures", a.len());
    assert_same(name, &a, &b);
    assert!(hw.name().starts_with("VA-API"), "{name}: stayed in hardware ({})", hw.name());

    // reset + reseek to each later sync sample, decode a stretch. After a seek to a non-IDR I picture
    // (open GOP), the pictures shown before it are predicted from frames that were never decoded:
    // both decoders conceal them (the software decoder from mid-gray stand-ins, the hardware also
    // from motion data those stand-ins do not have), so only the pictures from the seek point on
    // must match, as they must.
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    let syncs: Vec<usize> = (1..s.samples.len()).filter(|&i| s.sync[i]).collect();
    assert!(!syncs.is_empty(), "{name}: more than one GOP");
    for &k in syncs.iter().rev() {
        let end = (k + 17).min(s.samples.len());
        for d in [&mut hw, &mut sw] {
            d.reset();
        }
        let mut a = decode_all(hw.as_mut(), &s.samples[k..end]);
        let mut b = decode_all(sw.as_mut(), &s.samples[k..end]);
        assert_eq!(a.len(), b.len(), "{name} from sample {k}: pictures");
        if info.codec == filmcraft_codecs::hw::NalCodec::H264 && !info.nal_types(&s.samples[k].0).contains(&5) {
            let start = s.samples[k].1;
            a.retain(|f| f.pts >= start);
            b.retain(|f| f.pts >= start);
        }
        assert!(!a.is_empty(), "{name}: pictures after seeking to {k}");
        assert_same(&format!("{name} from sample {k}"), &a, &b);
    }
    // a flush in the middle of a GOP, then carrying on: H.264 only, where references survive a
    // flush (an HEVC flush ends the stream; the GOP cache always seeks after one)
    if info.codec == filmcraft_codecs::hw::NalCodec::H264 {
        for d in [&mut hw, &mut sw] {
            d.reset();
        }
        let mid = s.samples.len() / 2;
        let mut a = Vec::new();
        let mut b = Vec::new();
        for (d, out) in [(&mut hw, &mut a), (&mut sw, &mut b)] {
            for (smp, pts) in &s.samples[..mid] {
                out.extend(d.decode(smp, *pts).unwrap());
            }
            out.extend(d.flush());
            out.extend(decode_all(d.as_mut(), &s.samples[mid..]));
        }
        assert_same(&format!("{name} with a flush at {mid}"), &a, &b);
    }
    // back to the start after seeking around: the whole stream again
    hw.reset();
    sw.reset();
    assert_same(&format!("{name} after resets"), &decode_all(hw.as_mut(), &s.samples), &decode_all(sw.as_mut(), &s.samples));
    assert!(hw.name().starts_with("VA-API"), "{name}: stayed in hardware ({})", hw.name());
    // random-access / disposable answers match the software decoder's
    for (smp, _) in &s.samples {
        assert_eq!(hw.is_random_access(smp), sw.is_random_access(smp), "{name}");
        assert_eq!(hw.is_disposable(smp), sw.is_disposable(smp), "{name}");
    }
}

/// [`parity`] for large streams, with little memory: the two decoders run side by side and each
/// picture is compared and dropped as soon as both have output it (a 4K 10-bit stream held whole
/// takes gigabytes). The whole stream, then from the last sync sample after a reset.
fn parity_streaming(name: &str, s: &Stream) {
    let Some(mut hw) = hardware(s) else { return };
    let mut sw = software(s);
    let last_sync = (1..s.samples.len()).rev().find(|&i| s.sync[i]).unwrap_or(0);
    for start in [0, last_sync] {
        hw.reset();
        sw.reset();
        let (mut a, mut b) = (std::collections::VecDeque::new(), std::collections::VecDeque::new());
        let mut compared = 0usize;
        let mut compare = |a: &mut std::collections::VecDeque<filmcraft_codecs::DecodedFrame>,
                           b: &mut std::collections::VecDeque<filmcraft_codecs::DecodedFrame>| {
            while !a.is_empty() && !b.is_empty() {
                let (Some(x), Some(y)) = (a.pop_front(), b.pop_front()) else { break };
                assert_same(&format!("{name} from sample {start}"), &[x], &[y]);
                compared += 1;
            }
        };
        for (smp, pts) in &s.samples[start..] {
            a.extend(hw.decode(smp, *pts).unwrap());
            b.extend(sw.decode(smp, *pts).unwrap());
            compare(&mut a, &mut b);
        }
        a.extend(hw.flush());
        b.extend(sw.flush());
        compare(&mut a, &mut b);
        assert!(a.is_empty() && b.is_empty(), "{name} from sample {start}: picture counts differ");
        assert!(compared >= s.samples.len().saturating_sub(start) / 2, "{name} from sample {start}: {compared} pictures");
        assert!(hw.name().starts_with("VA-API"), "{name}: stayed in hardware ({})", hw.name());
    }
}

#[test]
fn bit_exact_with_the_software_decoder() {
    let _engine = engine_shared();
    let ff = filmcraft_testkit::require_ffmpeg!();
    let streams = small_streams(&ff);
    assert!(streams.len() > H264.len(), "fixtures");
    for (name, s) in streams {
        parity(&name, &s);
    }
}

fn large(size: &str) {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let (_, h) = size.split_once('x').unwrap();
    let src = format!("testsrc2=s={size}:r=24:d=2,noise=alls=12:allf=t");
    let args = [
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:v",
        "libx264",
        "-profile:v",
        "high",
        "-preset",
        "veryfast",
        "-x264-params",
        "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0",
        "-pix_fmt",
        "yuv420p",
    ];
    let Some(path) = fixture(&ff, &format!("h264_{h}.mp4"), &args) else { return };
    parity_streaming(&format!("h264 {size}"), &read_stream(&path));
}

#[test]
fn h264_1080p_and_2160p_are_bit_exact() {
    let _engine = engine_shared();
    large("1920x1080");
    large("3840x2160");
}

fn large_hevc(size: &str, pix_fmt: &str) {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let (_, h) = size.split_once('x').unwrap();
    let src = format!("testsrc2=s={size}:r=24:d=2,noise=alls=12:allf=t");
    let ten = pix_fmt.contains("10");
    let args = [
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:v",
        "libx265",
        "-preset",
        "ultrafast",
        "-profile:v",
        if ten { "main10" } else { "main" },
        "-tag:v",
        "hvc1",
        "-x265-params",
        "log-level=error:bframes=4:keyint=24:min-keyint=24:scenecut=0:open-gop=1",
        "-pix_fmt",
        pix_fmt,
    ];
    let name = format!("va_hevc_{}_{h}.mp4", if ten { "main10" } else { "main" });
    let Some(path) = fixture(&ff, &name, &args) else { return };
    parity_streaming(&name, &read_stream(&path));
}

#[test]
fn hevc_main_and_main10_1080p_and_2160p_are_bit_exact() {
    let _engine = engine_shared();
    for size in ["1920x1080", "3840x2160"] {
        large_hevc(size, "yuv420p");
        large_hevc(size, "yuv420p10le");
    }
}

#[test]
fn mid_stream_failure_continues_in_software() {
    let _engine = engine_shared();
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (name, s) in small_streams(&ff) {
        let name = name.as_str();
        let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
        let Ok(_) = VaDecoder::new(info.clone()) else {
            eprintln!("SKIPPED: no VA-API hardware decoder for {name}");
            continue;
        };
        let reference = decode_all(software(&s).as_mut(), &s.samples);
        assert!(!reference.is_empty(), "{name}");
        // fail at the first sample, inside the first GOP, at a sync sample and just after one
        let k = (1..s.samples.len()).find(|&i| s.sync[i]).unwrap();
        for fail_at in [0, 5, k, k + 1, k + 9] {
            let mut va = VaDecoder::new(info.clone()).unwrap();
            va.fail_after(fail_at as u64);
            let mut d = HybridDecoder::new(Box::new(va), s.entry.clone(), info.clone());
            let before = filmcraft_codecs::hw::hw_stats().fallbacks;
            let out = decode_all(&mut d, &s.samples);
            assert!(!d.is_hardware(), "{name}: switched to software");
            assert!(filmcraft_codecs::hw::hw_stats().fallbacks > before, "{name}: fallback counted");
            assert_same(&format!("{name} failing at sample {fail_at}"), &out, &reference);
            d.reset();
            assert_same(&format!("{name} after the fallback and a reset"), &decode_all(&mut d, &s.samples), &reference);
        }
    }
}

/// What the hardware path does not take is declined (the software decoders keep it): interlaced
/// and 10-bit H.264, and HEVC 4:2:2 without a display that has Main 4:2:2 10.
#[test]
fn declines_what_the_hardware_path_does_not_take() {
    let _engine = engine_shared();
    let ff = filmcraft_testkit::require_ffmpeg!();
    let src = "testsrc2=s=640x360:r=24:d=1";
    let cases: [(&str, Vec<&str>); 2] = [
        ("va_h264_interlaced.mp4", vec!["-f", "lavfi", "-i", src, "-c:v", "libx264", "-x264-params", "interlaced=1:keyint=12", "-pix_fmt", "yuv420p"]),
        ("va_h264_10bit.mp4", vec!["-f", "lavfi", "-i", src, "-c:v", "libx264", "-profile:v", "high10", "-pix_fmt", "yuv420p10le"]),
    ];
    for (name, args) in cases {
        // an ffmpeg whose x264 lacks 10-bit leaves the fixture out
        let Some(path) = fixture(&ff, name, &args) else { continue };
        let s = read_stream(&path);
        assert!(filmcraft_platform::vaapi_factory(&s.entry).is_none(), "{name}: declined");
    }
    let args = ["-f", "lavfi", "-i", src, "-c:v", "libx265", "-tag:v", "hvc1", "-x265-params", "log-level=error:keyint=12", "-pix_fmt", "yuv422p10le"];
    if let Some(path) = fixture(&ff, "va_hevc_422.mp4", &args) {
        let s = read_stream(&path);
        // taken only where a display has HEVC Main 4:2:2 10 (fork 6.1b, tests/va_hevc_422.rs)
        let taken = filmcraft_platform::vaapi_factory(&s.entry).and_then(Result::ok).map(|d| d.name().to_string());
        let want = filmcraft_platform::hevc_422_in_hardware().then(|| filmcraft_platform::va_hevc::NAME_422.to_string());
        assert_eq!(taken, want, "HEVC 4:2:2: declined without a 4:2:2 display");
    }
    // Settings ▸ Playback ▸ Hardware decoding Off
    let Some(path) = named(&ff, "h264_high.mp4") else { return };
    let s = read_stream(&path);
    filmcraft_codecs::hw::set_hardware_decoding(false);
    let off = filmcraft_platform::vaapi_factory(&s.entry).is_none();
    filmcraft_codecs::hw::set_hardware_decoding(true);
    assert!(off, "declined while hardware decoding is Off");
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

#[test]
fn damaged_samples_and_parameter_sets_never_crash_or_hang() {
    let _engine = engine_exclusive();
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (name, s) in small_streams(&ff) {
        let name = name.as_str();
        if hardware(&s).is_none() {
            continue;
        }
        // damaged samples (bit flips, truncation, corrupt length prefixes): through the hybrid, and
        // straight into the hardware decoder (no software to fall back to)
        for seed in 1..=8u64 {
            let samples = s.samples.clone();
            let entry = s.entry.clone();
            bounded(&format!("{name} seed {seed}"), Duration::from_secs(60), move || {
                let mut seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
                let mut d = filmcraft_platform::vaapi_factory(&entry).unwrap().unwrap();
                let info = NalStreamInfo::from_entry(&entry).unwrap().unwrap();
                let mut raw = VaDecoder::new(info).unwrap();
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
                    let _ = d.decode(&m, *pts);
                    let _ = raw.decode(&m, *pts);
                    if i % 13 == 12 {
                        d.reset();
                        raw.reset();
                    }
                }
                let _ = d.flush();
                let _ = raw.flush();
            });
        }
        // damaged parameter sets: the decoder declines, errors or decodes; never crashes
        let filmcraft_isobmff::CodecConfig::Avc(a) = &s.entry.codec else { continue };
        let rec = a.to_bytes();
        let mut seed = 0x1234_5678u64;
        for round in 0..24 {
            let mut r = rec.clone();
            if round % 3 == 2 {
                r.truncate(r.len() * (round + 1) / 30);
            } else {
                for _ in 0..1 + round % 4 {
                    let at = 6 + (xorshift(&mut seed) as usize) % r.len().saturating_sub(6).max(1);
                    if let Some(b) = r.get_mut(at) {
                        *b ^= 1 << (xorshift(&mut seed) % 8);
                    }
                }
            }
            let samples: Vec<_> = s.samples.iter().take(12).cloned().collect();
            bounded(&format!("{name} record round {round}"), Duration::from_secs(60), move || {
                let Ok(info) = NalStreamInfo::from_avcc(&r) else { return };
                let Ok(mut va) = VaDecoder::new(info) else { return };
                for (smp, pts) in &samples {
                    if va.decode(smp, *pts).is_err() {
                        break;
                    }
                }
                let _ = va.flush();
            });
        }
    }
}

/// Creating and dropping decoders again and again (every clip on a timeline makes one) neither
/// fails nor leaks VA-API objects into the next one.
#[test]
fn repeated_initialisation() {
    let _engine = engine_shared();
    let ff = filmcraft_testkit::require_ffmpeg!();
    let Some(path) = named(&ff, "h264_high.mp4") else { return };
    let s = read_stream(&path);
    let reference = decode_all(software(&s).as_mut(), &s.samples[..13]);
    for round in 0..40 {
        let Some(mut hw) = hardware(&s) else { return };
        if round % 8 == 0 {
            assert_same(&format!("round {round}"), &decode_all(hw.as_mut(), &s.samples[..13]), &reference);
        }
    }
}

/// Several decoders at once on different threads (the frame workers) give the software decoder's
/// pictures.
#[test]
fn concurrent_decoders() {
    let _engine = engine_shared();
    let ff = filmcraft_testkit::require_ffmpeg!();
    let mut paths: Vec<_> = named(&ff, "h264_high.mp4").into_iter().collect();
    for (name, profile, params) in H264.iter().take(3) {
        paths.extend(h264_fixture(&ff, name, profile, params));
    }
    let jobs: Vec<_> = paths
        .into_iter()
        .chain(named(&ff, "h264_high.mp4"))
        .map(|path| {
            std::thread::spawn(move || {
                let s = read_stream(&path);
                let Some(mut hw) = hardware(&s) else { return };
                let n = s.samples.len().min(40);
                let a = decode_all(hw.as_mut(), &s.samples[..n]);
                hw.reset();
                let b = decode_all(hw.as_mut(), &s.samples[..n]);
                let c = decode_all(software(&s).as_mut(), &s.samples[..n]);
                assert_same("concurrent", &a, &c);
                assert_same("concurrent, after a reset", &b, &c);
            })
        })
        .collect();
    for j in jobs {
        j.join().unwrap();
    }
}

/// A decoder used and moved between threads (decoders migrate between frame workers).
#[test]
fn decoder_moves_between_threads() {
    let _engine = engine_shared();
    let ff = filmcraft_testkit::require_ffmpeg!();
    let Some(path) = named(&ff, "h264_high.mp4") else { return };
    let s = read_stream(&path);
    let Some(mut hw) = hardware(&s) else { return };
    let reference = decode_all(software(&s).as_mut(), &s.samples[..30]);
    let samples = s.samples.clone();
    let (hw, out) = std::thread::spawn(move || {
        let out = decode_all(hw.as_mut(), &samples[..30]);
        (hw, out)
    })
    .join()
    .unwrap();
    assert_same("first thread", &out, &reference);
    let mut hw = hw;
    hw.reset();
    let samples = s.samples.clone();
    let out = std::thread::spawn(move || decode_all(hw.as_mut(), &samples[..30])).join().unwrap();
    assert_same("second thread", &out, &reference);
}

/// The fork's suite for the ffmpeg-pipe decoder (`filmcraft_platform::vaapi_pipe`, VA-API through
/// the system ffmpeg as a child process) and the in-process → pipe → software fallback chain:
/// the same parity, failover and robustness checks on the shared fixtures. Skips without ffmpeg
/// or without a VA-API device.
mod pipe {
    use std::time::Duration;

    use super::common::*;
    use filmcraft_codecs::VideoDecoder;
    use filmcraft_codecs::hw::NalStreamInfo;
    use filmcraft_platform::HybridDecoder;
    use filmcraft_platform::vaapi_pipe::VaapiDecoder;

    /// The hardware decoder for a stream, or `None` (skip) when this machine has none for it.
    fn hardware(s: &Stream) -> Option<Box<dyn VideoDecoder>> {
        match filmcraft_platform::vaapi_factory(&s.entry) {
            Some(Ok(d)) => {
                assert!(d.name().starts_with("VA-API"), "{}", d.name());
                Some(d)
            }
            Some(Err(e)) => panic!("factory error: {e}"),
            None => {
                eprintln!("SKIPPED: no VA-API hardware decoder for {}", s.entry.codec.name());
                None
            }
        }
    }

    fn software(s: &Stream) -> Box<dyn VideoDecoder> {
        filmcraft_codecs::software_video_decoder(&s.entry).unwrap()
    }

    #[test]
    fn bit_exact_with_the_software_decoders() {
        let _engine = engine_shared();
        let ff = filmcraft_testkit::require_ffmpeg!();
        for (name, _) in FIXTURES {
            let Some(path) = named(&ff, name) else { continue };
            let s = read_stream(&path);
            let Some(mut hw) = hardware(&s) else { continue };
            let mut sw = software(&s);
            let a = decode_all(hw.as_mut(), &s.samples);
            let b = decode_all(sw.as_mut(), &s.samples);
            assert!(a.len() >= 60, "{name}: {} pictures", a.len());
            assert_same(name, &a, &b);
            assert!(hw.name().starts_with("VA-API"), "{name}: stayed in hardware ({})", hw.name());

            // reset + reseek to each later sync sample, decode a stretch, flush mid-stream
            let syncs: Vec<usize> = (1..s.samples.len()).filter(|&i| s.sync[i]).collect();
            assert!(!syncs.is_empty(), "{name}: more than one GOP");
            for &k in syncs.iter().rev() {
                let end = (k + 17).min(s.samples.len());
                for d in [&mut hw, &mut sw] {
                    d.reset();
                }
                let a = decode_all(hw.as_mut(), &s.samples[k..end]);
                let b = decode_all(sw.as_mut(), &s.samples[k..end]);
                assert!(!a.is_empty(), "{name}: pictures after seeking to {k}");
                assert_same(&format!("{name} from sample {k}"), &a, &b);
            }
            // back to the start after seeking around: the whole stream again
            hw.reset();
            sw.reset();
            assert_same(&format!("{name} after resets"), &decode_all(hw.as_mut(), &s.samples), &decode_all(sw.as_mut(), &s.samples));
            // random-access / disposable answers match the software decoder's
            for (smp, _) in &s.samples {
                assert_eq!(hw.is_random_access(smp), sw.is_random_access(smp), "{name}");
                assert_eq!(hw.is_disposable(smp), sw.is_disposable(smp), "{name}");
            }
        }
    }

    #[test]
    fn mid_stream_failure_continues_in_software() {
        let _engine = engine_shared();
        let ff = filmcraft_testkit::require_ffmpeg!();
        for (name, _) in FIXTURES {
            let Some(path) = named(&ff, name) else { continue };
            let s = read_stream(&path);
            let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
            let Ok(_) = VaapiDecoder::new(info.clone()) else {
                eprintln!("SKIPPED: no VA-API hardware decoder for {name}");
                continue;
            };
            let reference = decode_all(software(&s).as_mut(), &s.samples);
            // fail at the first sample, inside the first GOP, at a sync sample and just after one
            let k = (1..s.samples.len()).find(|&i| s.sync[i]).unwrap();
            for fail_at in [0, 5, k, k + 1, k + 9] {
                let mut hwd = VaapiDecoder::new(info.clone()).unwrap();
                hwd.fail_after(fail_at as u64);
                let mut d = HybridDecoder::new(Box::new(hwd), s.entry.clone(), info.clone());
                let before = filmcraft_codecs::hw::hw_stats().fallbacks;
                let out = decode_all(&mut d, &s.samples);
                assert!(!d.is_hardware(), "{name}: switched to software");
                assert!(filmcraft_codecs::hw::hw_stats().fallbacks > before, "{name}: fallback counted");
                assert_same(&format!("{name} failing at sample {fail_at}"), &out, &reference);
                // the instance stays in software across seeks
                d.reset();
                assert_same(&format!("{name} after the fallback and a reset"), &decode_all(&mut d, &s.samples), &reference);
            }
        }
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

    #[test]
    fn damaged_samples_and_parameter_sets_never_crash_or_hang() {
        let _engine = engine_exclusive();
        let ff = filmcraft_testkit::require_ffmpeg!();
        for (name, _) in FIXTURES {
            let Some(path) = named(&ff, name) else { continue };
            let s = read_stream(&path);
            if hardware(&s).is_none() {
                continue;
            }
            // damaged samples (bit flips, truncation, corrupt length prefixes) through the hybrid
            for seed in 1..=6u64 {
                let samples = s.samples.clone();
                let entry = s.entry.clone();
                bounded(&format!("{name} seed {seed}"), Duration::from_secs(60), move || {
                    let mut seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
                    let mut d = filmcraft_platform::vaapi_factory(&entry).unwrap().unwrap();
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
                        let _ = d.decode(&m, *pts);
                        if i % 13 == 12 {
                            d.reset();
                        }
                    }
                    let _ = d.flush();
                });
            }
            // damaged parameter sets: the factory declines or the decoder errors / falls back
            let rec = match &s.entry.codec {
                filmcraft_isobmff::CodecConfig::Avc(a) => a.to_bytes(),
                filmcraft_isobmff::CodecConfig::Hevc(c) => c.to_bytes(),
                _ => continue,
            };
            let hevc = matches!(s.entry.codec, filmcraft_isobmff::CodecConfig::Hevc(_));
            let mut seed = 0x1234_5678u64;
            for round in 0..24 {
                let mut r = rec.clone();
                if round % 3 == 2 {
                    r.truncate(r.len() * (round + 1) / 30);
                } else {
                    for _ in 0..1 + round % 4 {
                        let at = 6 + (xorshift(&mut seed) as usize) % r.len().saturating_sub(6).max(1);
                        if let Some(b) = r.get_mut(at) {
                            *b ^= 1 << (xorshift(&mut seed) % 8);
                        }
                    }
                }
                let samples: Vec<_> = s.samples.iter().take(12).cloned().collect();
                bounded(&format!("{name} record round {round}"), Duration::from_secs(60), move || {
                    let Some(info) = (if hevc { NalStreamInfo::from_hvcc(&r) } else { NalStreamInfo::from_avcc(&r) }).ok() else { return };
                    let Ok(mut hwd) = VaapiDecoder::new(info) else { return };
                    for (smp, pts) in &samples {
                        if hwd.decode(smp, *pts).is_err() {
                            break;
                        }
                    }
                    let _ = hwd.flush();
                });
            }
        }
    }

    /// 4:2:2 (which VA-API on most GPUs does not decode): the factory declines, so the software path
    /// (or an error from it) takes the stream; never a hardware decoder that then fails.
    #[test]
    fn hevc_422_is_declined() {
        let _engine = engine_shared();
        let ff = filmcraft_testkit::require_ffmpeg!();
        let args = [
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=24:d=1",
            "-c:v",
            "libx265",
            "-preset",
            "fast",
            "-tag:v",
            "hvc1",
            "-x265-params",
            "log-level=error",
            "-pix_fmt",
            "yuv422p10le",
        ];
        let Some(path) = fixture(&ff, "hevc_422_10.mp4", &args) else { return };
        let s = read_stream(&path);
        if filmcraft_platform::hevc_422_in_hardware() {
            // a display with HEVC Main 4:2:2 10 (fork 6.1b, tests/va_hevc_422.rs): the in-process decoder takes it
            let d = filmcraft_platform::vaapi_factory(&s.entry).expect("taken").expect("decoder");
            assert_eq!(d.name(), filmcraft_platform::va_hevc::NAME_422);
        } else {
            assert!(filmcraft_platform::vaapi_factory(&s.entry).is_none(), "4:2:2 declined");
        }
    }

    /// Regression: picture conversion used rayon's global pool, so while every global worker was a
    /// render task waiting on the GOP cache being filled (an export), the decoder could not convert a
    /// picture and timed out into the software fallback after 10 s. Decoding must stay in hardware
    /// and fast with the global pool saturated by blocked tasks.
    #[test]
    fn decodes_while_the_global_rayon_pool_is_blocked() {
        let _engine = engine_shared();
        let ff = filmcraft_testkit::require_ffmpeg!();
        let Some(path) = named(&ff, "hevc_main.mp4") else { return };
        let s = read_stream(&path);
        let Some(mut hw) = hardware(&s) else { return };
        let gate = std::sync::Arc::new(std::sync::Mutex::new(()));
        let held = gate.lock().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        for _ in 0..rayon::current_num_threads() {
            let (gate, tx) = (gate.clone(), started_tx.clone());
            rayon::spawn(move || {
                let _ = tx.send(());
                drop(gate.lock());
            });
        }
        for _ in 0..rayon::current_num_threads() {
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        let t0 = std::time::Instant::now();
        let out = decode_all(hw.as_mut(), &s.samples);
        let secs = t0.elapsed().as_secs_f64();
        drop(held);
        assert_eq!(out.len(), s.samples.len(), "every picture");
        assert!(hw.name().starts_with("VA-API"), "stayed in hardware ({})", hw.name());
        assert!(secs < 8.0, "decoding took {secs:.1} s with the global pool blocked");
    }

    /// The fallback chain (ADR 0002): the in-process VA-API decoder failing mid-stream hands the
    /// stream to the ffmpeg pipe (still hardware), and when that fails too, to software; the output
    /// is the software decoder's sequence either way.
    #[test]
    fn in_process_failure_continues_on_the_pipe_then_in_software() {
        let _engine = engine_shared();
        let ff = filmcraft_testkit::require_ffmpeg!();
        for (name, _) in FIXTURES.iter().filter(|f| f.0.starts_with("hevc")) {
            let Some(path) = named(&ff, name) else { continue };
            let s = read_stream(&path);
            let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
            if filmcraft_platform::va_hevc::NativeVaDecoder::new(info.clone()).is_err() || VaapiDecoder::new(info.clone()).is_err() {
                eprintln!("SKIPPED: {name} needs both VA-API paths");
                continue;
            }
            let reference = decode_all(software(&s).as_mut(), &s.samples);
            let k = (1..s.samples.len()).find(|&i| s.sync[i]).unwrap();
            for (fail_native, fail_pipe) in [(5u64, None), (k as u64 + 2, None), (3, Some(4u64))] {
                let mut native = filmcraft_platform::va_hevc::NativeVaDecoder::new(info.clone()).unwrap();
                native.fail_after(fail_native);
                let pipe_info = info.clone();
                let mut d = HybridDecoder::new(Box::new(native), s.entry.clone(), info.clone()).with_fallback(Box::new(move || {
                    let mut p = VaapiDecoder::new(pipe_info)?;
                    if let Some(n) = fail_pipe {
                        p.fail_after(n);
                    }
                    Ok(Box::new(p) as Box<dyn VideoDecoder>)
                }));
                let out = decode_all(&mut d, &s.samples);
                assert_same(&format!("{name}: native failing at {fail_native}, pipe at {fail_pipe:?}"), &out, &reference);
                match fail_pipe {
                    None => assert!(d.is_hardware() && d.name() == "VA-API HEVC", "{name}: continued on the pipe ({})", d.name()),
                    Some(_) => assert!(!d.is_hardware(), "{name}: ended in software ({})", d.name()),
                }
                let (backends, why) = filmcraft_codecs::hw::backend_stats();
                assert!(backends.iter().any(|(b, _)| b == filmcraft_platform::va_hevc::NAME), "{backends:?}");
                assert!(why.is_some_and(|w| w.contains("failed")), "the fallback reason is reported");
            }
        }
    }
}
