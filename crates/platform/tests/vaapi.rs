//! VA-API decoding (Linux, through the system ffmpeg) against our software decoders: every
//! picture of H.264 High, HEVC Main and HEVC Main 10 fixtures with B-frames must be identical
//! (bit-exact planes, colour, pixel aspect, pts and presentation order), also after `reset` +
//! reseek and a mid-stream `flush`; a forced mid-stream failure must continue in software with
//! the software decoder's exact output; damaged samples and parameter sets must give errors or
//! fall back, never crash or hang. Skips without ffmpeg or without a VA-API device.
#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use common::*;
use filmcraft_codecs::VideoDecoder;
use filmcraft_codecs::hw::NalStreamInfo;
use filmcraft_platform::HybridDecoder;
use filmcraft_platform::vaapi::VaapiDecoder;

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
    assert!(filmcraft_platform::vaapi_factory(&s.entry).is_none(), "4:2:2 declined");
}

/// Regression: picture conversion used rayon's global pool, so while every global worker was a
/// render task waiting on the GOP cache being filled (an export), the decoder could not convert a
/// picture and timed out into the software fallback after 10 s. Decoding must stay in hardware
/// and fast with the global pool saturated by blocked tasks.
#[test]
fn decodes_while_the_global_rayon_pool_is_blocked() {
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
