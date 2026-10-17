//! Fork H3.3 (H264-PLAN H3): H.264 and HEVC decode with ffmpeg absent, through the product's own
//! decoder chain (`filmcraft_platform::register` + `filmcraft_codecs::make_video_decoder`), with
//! the hardware present, with Hardware decoding Off, and with the VA-API driver pulled. ffmpeg
//! makes the fixtures first (a generator only); then each setup runs in a child process (the
//! ffmpeg probe and the VA display are per process) with `FILMCRAFT_FFMPEG` pointing nowhere and
//! every directory holding an `ffmpeg` removed from `PATH`. Every picture must match FilmCraft's
//! software decoder, and no stream may be decoded by the ffmpeg pipe. Ignored (spawns processes,
//! wants a VA-API machine for the hardware setup): `cargo test --release -p filmcraft-platform
//! --test ffmpeg_free -- --ignored --nocapture`.
#![cfg(target_os = "linux")]

mod common;

use std::path::PathBuf;

use common::*;

/// Set in the child: which setup it runs.
const CHILD: &str = "FC_H3_CHILD";
/// The fixture files for the child, one per line.
const FILES: &str = "FC_H3_FIXTURES";

/// The fixture set: (file, ffmpeg arguments, whether VA-API takes it on AMD / Intel). Progressive
/// 8-bit, High 10, High 4:2:2 10-bit Long-GOP, MBAFF (8-bit and 4:2:2 10-bit), HEVC Main and
/// Main 10 come from `common::FIXTURES`.
fn fixture_set(ff: &std::path::Path) -> Vec<(PathBuf, bool)> {
    let src = "testsrc2=s=640x360:r=25:d=2,noise=alls=12:allf=t";
    let x264 = |name: &str, profile: &str, pix: &str, params: &str| {
        let args = ["-f", "lavfi", "-i", src, "-c:v", "libx264", "-profile:v", profile, "-preset", "fast", "-x264-params", params, "-pix_fmt", pix];
        fixture(ff, name, &args)
    };
    let interlaced = "interlaced=1:tff=1:keyint=12:bframes=3:b-pyramid=normal:ref=3";
    let made = [
        (named(ff, "h264_high.mp4"), true),
        (x264("h3_h264_high10.mp4", "high10", "yuv420p10le", "keyint=12:bframes=2"), false),
        (x264("h3_h264_422_10.mp4", "high422", "yuv422p10le", "keyint=12:bframes=2:weightp=2"), false),
        (x264("h3_h264_mbaff.mp4", "high", "yuv420p", interlaced), true),
        (x264("h3_h264_mbaff_422_10.mp4", "high422", "yuv422p10le", interlaced), false),
        (named(ff, "hevc_main.mp4"), true),
        (named(ff, "hevc_main10.mp4"), true),
    ];
    made.into_iter().filter_map(|(p, hw)| p.map(|p| (p, hw))).collect()
}

/// `PATH` without the directories that hold an `ffmpeg`.
fn path_without_ffmpeg() -> std::ffi::OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).filter(|d| !d.join("ffmpeg").exists()).collect();
    std::env::join_paths(dirs).unwrap()
}

#[test]
#[ignore]
fn h264_and_hevc_decode_without_ffmpeg() {
    if std::env::var_os(CHILD).is_some() {
        return;
    }
    let ff = filmcraft_testkit::require_ffmpeg!();
    let set = fixture_set(&ff);
    assert_eq!(set.len(), 7, "every fixture generated");
    let files: Vec<String> = set.iter().map(|(p, hw)| format!("{}\t{hw}", p.display())).collect();
    let exe = std::env::current_exe().unwrap();
    for setup in ["hardware", "hardware_off", "no_driver"] {
        let mut c = std::process::Command::new(&exe);
        c.args(["--ignored", "--exact", "child_decodes_the_set", "--nocapture", "--test-threads=1"])
            .env(CHILD, setup)
            .env(FILES, files.join("\n"))
            .env("FILMCRAFT_FFMPEG", "/nonexistent/ffmpeg")
            .env("PATH", path_without_ffmpeg())
            .stdin(std::process::Stdio::null());
        if setup == "no_driver" {
            // libva finds no driver: the display never opens
            c.env("LIBVA_DRIVER_NAME", "fc_h3_no_such_driver").env("LIBVA_DRIVERS_PATH", "/nonexistent");
        }
        let out = c.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        // (libtest's "test child_decodes_the_set ... " shares a line with the first report)
        for line in stdout.lines().filter_map(|l| l.find("H3.3").and_then(|i| l.get(i..))) {
            println!("{line}");
        }
        assert!(
            out.status.success() && stdout.contains("H3.3 done"),
            "{setup}: child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr).chars().rev().take(4000).collect::<String>().chars().rev().collect::<String>()
        );
    }
}

/// The child: decodes the set through the product's chain in the setup `FC_H3_CHILD` names.
#[test]
#[ignore]
fn child_decodes_the_set() {
    let Ok(setup) = std::env::var(CHILD) else { return };
    assert!(filmcraft_platform::ffmpeg::ffmpeg().is_none(), "ffmpeg must be hidden from the child");
    let avail = filmcraft_platform::register();
    filmcraft_codecs::hw::set_hardware_decoding(setup != "hardware_off");
    let hardware = setup == "hardware" && matches!(avail, filmcraft_platform::Availability::Available(_));
    if setup == "hardware" && !hardware {
        println!("H3.3 {setup}: no hardware decoder on this machine ({avail:?}); checking software only");
    }
    let files = std::env::var(FILES).unwrap();
    for line in files.lines() {
        let (path, hw) = line.split_once('\t').unwrap();
        let file = std::path::Path::new(path).file_name().unwrap().to_string_lossy().to_string();
        let s = read_stream(std::path::Path::new(path));
        let mut d = filmcraft_codecs::make_video_decoder(&s.entry).unwrap_or_else(|e| panic!("{setup}: {file}: {e}"));
        let name = d.name().to_string();
        assert!(name != "VA-API H.264" && name != "VA-API HEVC", "{setup}: {file} went through the ffmpeg pipe ({name})");
        if hardware && hw == "true" {
            assert!(name.starts_with("VA-API"), "{setup}: {file}: expected the hardware, got {name}");
        } else if !hardware {
            assert!(name.starts_with("Revolution"), "{setup}: {file}: expected Revolution's decoder, got {name}");
        }
        let got = decode_all(d.as_mut(), &s.samples);
        assert_eq!(got.len(), s.samples.len(), "{setup}: {file}: every picture");
        let mut sw = filmcraft_codecs::software_video_decoder(&s.entry).unwrap();
        let want = decode_all(sw.as_mut(), &s.samples);
        assert_same(&format!("{setup}: {file}"), &got, &want);
        println!("H3.3 {setup}: {file}: {} pictures by {name}, identical to the software decoder", got.len());
    }
    println!("H3.3 done ({setup})");
}
