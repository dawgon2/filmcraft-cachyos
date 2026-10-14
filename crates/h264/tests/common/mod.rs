//! Test fixture generation (ffmpeg/libx264 as an external oracle) and comparison helpers.
#![allow(dead_code)]

use filmcraft_h264::{Decoder, Picture};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One encoder configuration of the fixture matrix.
pub struct Fixture {
    pub name: &'static str,
    /// lavfi source (without size / rate).
    pub source: &'static str,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Extra video filter appended after the source (e.g. noise, fade).
    pub filter: &'static str,
    /// Encoder arguments.
    pub args: &'static [&'static str],
}

const NOISE: &str = "noise=alls=12:allf=t+u";

pub const FIXTURES: &[Fixture] = &[
    // Apple VideoToolbox (hardware) encoder streams: different encoder design, generated on macOS only
    Fixture {
        name: "vt_high",
        source: "testsrc2",
        width: 640,
        height: 360,
        frames: 30,
        filter: NOISE,
        args: &["-c:v", "h264_videotoolbox", "-profile:v", "high", "-b:v", "3M"],
    },
    Fixture {
        name: "vt_main",
        source: "mandelbrot",
        width: 640,
        height: 360,
        frames: 30,
        filter: NOISE,
        args: &["-c:v", "h264_videotoolbox", "-profile:v", "main", "-b:v", "2M"],
    },
    Fixture {
        name: "vt_baseline",
        source: "testsrc2",
        width: 320,
        height: 240,
        frames: 30,
        filter: NOISE,
        args: &["-c:v", "h264_videotoolbox", "-profile:v", "baseline", "-b:v", "1M"],
    },
    Fixture {
        name: "vt_high_1080p",
        source: "testsrc2",
        width: 1920,
        height: 1080,
        frames: 10,
        filter: "",
        args: &["-c:v", "h264_videotoolbox", "-profile:v", "high", "-b:v", "8M"],
    },
    // performance reference stream (see tests/perf.rs)
    Fixture {
        name: "bench_1080p",
        source: "testsrc2",
        width: 1920,
        height: 1080,
        frames: 120,
        filter: "noise=alls=3:allf=t",
        args: &["-preset", "medium", "-crf", "20"],
    },
    Fixture {
        name: "intra_cavlc",
        source: "testsrc2",
        width: 176,
        height: 144,
        frames: 3,
        filter: "",
        args: &["-profile:v", "baseline", "-x264-params", "keyint=1:no-deblock=1"],
    },
    Fixture {
        name: "intra_cavlc_noise",
        source: "mandelbrot",
        width: 352,
        height: 288,
        frames: 3,
        filter: NOISE,
        args: &["-profile:v", "baseline", "-x264-params", "keyint=1:no-deblock=1"],
    },
    Fixture {
        name: "intra_cavlc_8x8",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 3,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "keyint=1:no-deblock=1:cabac=0"],
    },
    Fixture {
        name: "intra_cavlc_cqm",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 3,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "keyint=1:no-deblock=1:cabac=0:cqm=jvt"],
    },
    Fixture {
        name: "p_cavlc_nodeblock",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "baseline", "-x264-params", "no-deblock=1:ref=3"],
    },
    Fixture {
        name: "baseline_qcif",
        source: "testsrc2",
        width: 176,
        height: 144,
        frames: 30,
        filter: "",
        args: &["-profile:v", "baseline", "-x264-params", "keyint=15"],
    },
    Fixture { name: "baseline_cif_noise", source: "mandelbrot", width: 352, height: 288, frames: 25, filter: NOISE, args: &["-profile:v", "baseline"] },
    Fixture {
        name: "cavlc_b",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "cabac=0:bframes=3"],
    },
    Fixture {
        name: "cavlc_b_temporal",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "main", "-x264-params", "cabac=0:bframes=3:direct=temporal:weightb=1"],
    },
    Fixture {
        name: "cavlc_weightp",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: "fade=in:0:25",
        args: &["-profile:v", "main", "-x264-params", "cabac=0:weightp=2:bframes=2:weightb=1"],
    },
    Fixture { name: "main_cabac_b", source: "testsrc2", width: 352, height: 288, frames: 30, filter: NOISE, args: &["-profile:v", "main"] },
    Fixture { name: "high_720p", source: "smptehdbars", width: 1280, height: 720, frames: 10, filter: NOISE, args: &["-profile:v", "high"] },
    Fixture { name: "high_1080p", source: "testsrc2", width: 1920, height: 1080, frames: 6, filter: "", args: &["-profile:v", "high"] },
    Fixture { name: "crop_1918x1078", source: "testsrc2", width: 1918, height: 1078, frames: 5, filter: NOISE, args: &["-profile:v", "high"] },
    Fixture {
        name: "bpyramid",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "bframes=3:b-pyramid=normal"],
    },
    Fixture {
        name: "weightp2",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: "fade=in:0:25",
        args: &["-profile:v", "high", "-x264-params", "weightp=2"],
    },
    Fixture {
        name: "weightb",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: "fade=in:0:25",
        args: &["-profile:v", "high", "-x264-params", "weightb=1:bframes=3"],
    },
    Fixture {
        name: "direct_temporal",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "direct=temporal:bframes=3"],
    },
    Fixture {
        name: "direct_spatial",
        source: "mandelbrot",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "direct=spatial:bframes=3"],
    },
    Fixture {
        name: "ref4",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "ref=4:bframes=2"],
    },
    Fixture {
        name: "no_deblock",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "no-deblock=1"],
    },
    Fixture {
        name: "deblock_m2_2",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "deblock=-2,2"],
    },
    Fixture {
        name: "cqm_jvt",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "cqm=jvt"],
    },
    Fixture {
        name: "slices4",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "slices=4"],
    },
    Fixture {
        name: "slices4_cavlc",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "baseline", "-x264-params", "slices=4"],
    },
    Fixture {
        name: "keyint10",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 30,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "keyint=10"],
    },
    Fixture {
        name: "open_gop",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 40,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "open-gop=1:keyint=12:bframes=3"],
    },
    Fixture {
        name: "constrained_intra",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "constrained-intra=1"],
    },
    Fixture {
        name: "no8x8dct",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high", "-x264-params", "8x8dct=0"],
    },
    Fixture { name: "qp50", source: "testsrc2", width: 352, height: 288, frames: 20, filter: NOISE, args: &["-profile:v", "high", "-qp", "50"] },
    Fixture { name: "qp1", source: "mandelbrot", width: 352, height: 288, frames: 10, filter: NOISE, args: &["-profile:v", "high", "-qp", "1"] },
    Fixture {
        name: "qp1_cavlc",
        source: "mandelbrot",
        width: 176,
        height: 144,
        frames: 10,
        filter: NOISE,
        args: &["-profile:v", "main", "-qp", "1", "-x264-params", "cabac=0"],
    },
    // High 10 (fork H2): x264 10-bit streams, compared with ffmpeg's yuv420p10le decode
    Fixture {
        name: "high10_intra",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 6,
        filter: NOISE,
        args: &["-profile:v", "high10", "-pix_fmt", "yuv420p10le", "-x264-params", "keyint=1"],
    },
    Fixture {
        name: "high10_cavlc_b",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high10", "-pix_fmt", "yuv420p10le", "-x264-params", "cabac=0:bframes=3"],
    },
    Fixture {
        name: "high10_weightp_fade",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 25,
        filter: "fade=in:0:20,noise=alls=12:allf=t+u",
        args: &["-profile:v", "high10", "-pix_fmt", "yuv420p10le", "-x264-params", "weightp=2:bframes=3:b-pyramid=normal:ref=4:weightb=1"],
    },
    Fixture {
        name: "high10_qp1",
        source: "mandelbrot",
        width: 352,
        height: 288,
        frames: 8,
        filter: NOISE,
        args: &["-profile:v", "high10", "-pix_fmt", "yuv420p10le", "-qp", "1"],
    },
    Fixture {
        name: "high10_slices_deblock",
        source: "testsrc2",
        width: 640,
        height: 360,
        frames: 15,
        filter: NOISE,
        args: &["-profile:v", "high10", "-pix_fmt", "yuv420p10le", "-x264-params", "slices=4:deblock=-2,-2:bframes=2"],
    },
    Fixture {
        name: "high10_crop",
        source: "testsrc2",
        width: 638,
        height: 358,
        frames: 10,
        filter: NOISE,
        args: &["-profile:v", "high10", "-pix_fmt", "yuv420p10le"],
    },
    // High 4:2:2 intra (fork H2.3): every picture an IDR (keyint=1), 8- and 10-bit, CABAC and CAVLC
    Fixture {
        name: "i422_cabac",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 6,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p", "-x264-params", "keyint=1"],
    },
    Fixture {
        name: "i422_cavlc",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 6,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p", "-x264-params", "keyint=1:cabac=0"],
    },
    Fixture {
        name: "i422_10_cabac",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 6,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-x264-params", "keyint=1"],
    },
    Fixture {
        name: "i422_10_cavlc_no8x8",
        source: "mandelbrot",
        width: 352,
        height: 288,
        frames: 6,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-x264-params", "keyint=1:cabac=0:8x8dct=0"],
    },
    Fixture {
        name: "i422_10_qp1",
        source: "mandelbrot",
        width: 352,
        height: 288,
        frames: 4,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-qp", "1", "-x264-params", "keyint=1"],
    },
    Fixture {
        name: "i422_slices_deblock_crop",
        source: "testsrc2",
        width: 638,
        height: 358,
        frames: 6,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p", "-x264-params", "keyint=1:slices=4:deblock=-2,-2"],
    },
    // High 4:2:2 Long-GOP (fork H2.4): chroma motion compensation at full vertical resolution
    Fixture {
        name: "p422_cabac_bpyramid",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 25,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p", "-x264-params", "bframes=3:b-pyramid=normal:ref=4"],
    },
    Fixture {
        name: "p422_10_cavlc",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-x264-params", "cabac=0:bframes=2"],
    },
    Fixture {
        name: "p422_10_weightp_fade",
        source: "testsrc2",
        width: 352,
        height: 288,
        frames: 25,
        filter: "fade=in:0:20,noise=alls=12:allf=t+u",
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-x264-params", "weightp=2:bframes=3:weightb=1:ref=3"],
    },
    Fixture {
        name: "p422_direct_temporal_crop",
        source: "testsrc2",
        width: 638,
        height: 358,
        frames: 20,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p", "-x264-params", "bframes=3:direct=temporal"],
    },
    Fixture {
        name: "p422_10_qp1",
        source: "mandelbrot",
        width: 352,
        height: 288,
        frames: 8,
        filter: NOISE,
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-qp", "1", "-x264-params", "bframes=2"],
    },
    // 4K 10-bit 4:2:2 benchmarks (fork H2.7; XAVC S / S-I-like rates; stream only, no reference)
    Fixture {
        name: "bench_4k_422_10",
        source: "testsrc2",
        width: 3840,
        height: 2160,
        frames: 30,
        filter: "noise=alls=3:allf=t",
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-preset", "veryfast", "-crf", "16", "-x264-params", "keyint=24:bframes=2"],
    },
    Fixture {
        name: "bench_4k_422_10_intra",
        source: "testsrc2",
        width: 3840,
        height: 2160,
        frames: 30,
        filter: "noise=alls=3:allf=t",
        args: &["-profile:v", "high422", "-pix_fmt", "yuv422p10le", "-preset", "veryfast", "-crf", "12", "-x264-params", "keyint=1"],
    },
];

/// Benchmark fixtures generated without a raw reference (4K 10-bit 4:2:2: ~1 GB of yuv).
pub fn stream_only(f: &Fixture) -> bool {
    f.name.starts_with("bench_4k")
}

pub fn fixture(name: &str) -> &'static Fixture {
    FIXTURES.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("unknown fixture {name}"))
}

/// ffmpeg (see `filmcraft_testkit::oracle`).
pub fn ffmpeg() -> Option<PathBuf> {
    filmcraft_testkit::ffmpeg()
}

pub fn fixtures_dir() -> PathBuf {
    filmcraft_testkit::fixtures_dir("h264")
}

fn run(cmd: &mut Command) -> bool {
    match cmd.output() {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            eprintln!("command failed: {:?}\n{}", cmd, String::from_utf8_lossy(&o.stderr));
            false
        }
        Err(e) => {
            eprintln!("failed to run {:?}: {e}", cmd);
            false
        }
    }
}

/// The fixture's pixel format: its `-pix_fmt` argument, `yuv420p` without one.
pub fn pix_fmt(f: &Fixture) -> &'static str {
    f.args.iter().position(|a| *a == "-pix_fmt").and_then(|i| f.args.get(i + 1)).copied().unwrap_or("yuv420p")
}

/// Bytes per sample of the fixture's reference decode (2 for 10-bit `yuv420p10le`).
pub fn sample_bytes(f: &Fixture) -> usize {
    if pix_fmt(f).contains("10") { 2 } else { 1 }
}

/// Generate (if needed) only the fixture stream (benchmarks whose raw reference would be huge).
/// Returns None (with a message) when ffmpeg or the encoder is unavailable.
pub fn ensure_stream(f: &Fixture) -> Option<PathBuf> {
    let ff = filmcraft_testkit::ffmpeg_or_skip(f.name)?;
    let h264 = fixtures_dir().join(format!("{}.h264", f.name));
    if !h264.exists() {
        let tmp = filmcraft_testkit::temp_path(&h264);
        let mut vf = format!("{}=size={}x{}:rate=25,format={}", f.source, f.width, f.height, pix_fmt(f));
        if !f.filter.is_empty() {
            vf.push(',');
            vf.push_str(f.filter);
        }
        let mut c = Command::new(&ff);
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &vf]);
        c.args(["-frames:v", &f.frames.to_string()]);
        if !f.args.contains(&"-c:v") {
            c.args(["-c:v", "libx264"]);
        }
        c.args(f.args);
        c.args(["-f", "h264"]).arg(&tmp);
        if !run(&mut c) {
            // Platform encoders (VideoToolbox) are optional; libx264 fixtures must generate.
            assert!(f.args.contains(&"-c:v"), "fixture generation failed for {}", f.name);
            eprintln!("SKIPPED ({}): encoder unavailable", f.name);
            return None;
        }
        std::fs::rename(&tmp, &h264).unwrap();
    }
    Some(h264)
}

/// Generate (if needed) the fixture stream and its ffmpeg reference decode.
/// Returns None (with a message) when ffmpeg is unavailable.
pub fn ensure(f: &Fixture) -> Option<(PathBuf, PathBuf)> {
    let h264 = ensure_stream(f)?;
    let ff = filmcraft_testkit::ffmpeg_or_skip(f.name)?;
    let yuv = fixtures_dir().join(format!("{}.yuv", f.name));
    if !yuv.exists() {
        let tmp = filmcraft_testkit::temp_path(&yuv);
        let mut c = Command::new(&ff);
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&h264);
        c.args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", pix_fmt(f)]).arg(&tmp);
        assert!(run(&mut c), "reference decode failed for {}", f.name);
        std::fs::rename(&tmp, &yuv).unwrap();
    }
    Some((h264, yuv))
}

/// Split an Annex-B stream into access units (new AU at AUD/SPS/PPS/SEI after a slice, or at a slice with
/// first_mb_in_slice == 0).
pub fn split_access_units(data: &[u8]) -> Vec<&[u8]> {
    // find start code positions
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let s = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            starts.push((s, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut aus = Vec::new();
    let mut au_start = 0usize;
    let mut seen_slice = false;
    for &(sc, payload) in &starts {
        if payload >= data.len() {
            continue;
        }
        let t = data[payload] & 0x1f;
        let is_slice = t == 1 || t == 5;
        let first_mb_zero = is_slice && payload + 1 < data.len() && data[payload + 1] & 0x80 != 0;
        let boundary = seen_slice && (matches!(t, 6..=9) || first_mb_zero);
        if boundary {
            aus.push(&data[au_start..sc]);
            au_start = sc;
            seen_slice = false;
        }
        if is_slice {
            seen_slice = true;
        }
    }
    if au_start < data.len() {
        aus.push(&data[au_start..]);
    }
    aus
}

/// Decode a whole Annex-B file access unit by access unit (pts = AU index).
/// Failed decode: (access unit index, error, pictures output before the error).
pub type DecodeFailure = (usize, filmcraft_h264::Error, Vec<Picture>);

pub fn decode_file(path: &Path) -> Result<Vec<Picture>, DecodeFailure> {
    decode_file_threads(path, 0)
}

/// Decode with `threads` worker threads (0 = decoder default).
pub fn decode_file_threads(path: &Path, threads: usize) -> Result<Vec<Picture>, DecodeFailure> {
    decode_file_opts(path, threads, false)
}

/// [`decode_file_threads`] with draft mode ([`Decoder::set_draft`]) on or off.
pub fn decode_file_opts(path: &Path, threads: usize, draft: bool) -> Result<Vec<Picture>, DecodeFailure> {
    decode_file_with(path, threads, draft, false)
}

/// [`decode_file_opts`], optionally through the 16-bit sample path (`Decoder::set_wide_samples`).
pub fn decode_file_with(path: &Path, threads: usize, draft: bool, wide: bool) -> Result<Vec<Picture>, DecodeFailure> {
    let data = std::fs::read(path).unwrap();
    let mut dec = if threads == 0 { Decoder::new() } else { Decoder::with_threads(threads) };
    assert!(!dec.draft(), "draft mode is off by default");
    dec.set_draft(draft);
    dec.set_wide_samples(wide);
    let mut out = Vec::new();
    for (i, au) in split_access_units(&data).into_iter().enumerate() {
        match dec.decode(au, i as i64) {
            Ok(p) => out.extend(p),
            Err(e) => return Err((i, e, out)),
        }
    }
    out.extend(dec.flush());
    if let Some(e) = dec.take_error() {
        return Err((usize::MAX, e, out));
    }
    Ok(out)
}

/// Compare decoded pictures with a raw yuv420p reference; returns a diagnostic on the first mismatch.
pub fn compare(pics: &[Picture], reference: &[u8], w: usize, h: usize) -> Result<(), String> {
    compare_bytes(pics, reference, w, h, 1)
}

/// [`compare`] with `bytes` per reference sample (2: little-endian 16-bit, compared with
/// [`Picture::high`]).
pub fn compare_bytes(pics: &[Picture], reference: &[u8], w: usize, h: usize, bytes: usize) -> Result<(), String> {
    compare_format(pics, reference, w, h, bytes, false)
}

/// [`compare_bytes`] for 4:2:0 or (`c422`) 4:2:2 references.
pub fn compare_format(pics: &[Picture], reference: &[u8], w: usize, h: usize, bytes: usize, c422: bool) -> Result<(), String> {
    let (cw, ch) = (w.div_ceil(2), if c422 { h } else { h.div_ceil(2) });
    let fsize = (w * h + 2 * cw * ch) * bytes;
    let samples = |b: &[u8]| -> Vec<u16> {
        if bytes == 2 { b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect() } else { b.iter().map(|&v| v as u16).collect() }
    };
    let nref = reference.len() / fsize;
    for (i, p) in pics.iter().enumerate() {
        if i >= nref {
            return Err(format!("decoder produced {} frames, reference has {}", pics.len(), nref));
        }
        if p.width as usize != w || p.height as usize != h {
            return Err(format!("frame {i}: size {}x{} != {}x{}", p.width, p.height, w, h));
        }
        let f = samples(&reference[i * fsize..(i + 1) * fsize]);
        let got: [Vec<u16>; 3] = match (&p.high, bytes) {
            (Some(hp), 2) => hp.clone(),
            (None, 1) => [&p.y, &p.u, &p.v].map(|v| v.iter().map(|&s| s as u16).collect()),
            _ => return Err(format!("frame {i}: bit depth {} does not match the {bytes}-byte reference", p.bit_depth)),
        };
        let planes = [
            ("Y", &got[0][..], &f[..w * h], w, h, 16),
            ("U", &got[1][..], &f[w * h..w * h + cw * ch], cw, ch, 8),
            ("V", &got[2][..], &f[w * h + cw * ch..], cw, ch, 8),
        ];
        for (name, got, exp, pw, ph, mbs) in planes {
            if got != exp {
                let mut first = None;
                let mut count = 0usize;
                let mut maxd = 0i32;
                for y in 0..ph {
                    for x in 0..pw {
                        let (a, b) = (got[y * pw + x], exp[y * pw + x]);
                        if a != b {
                            count += 1;
                            maxd = maxd.max((a as i32 - b as i32).abs());
                            if first.is_none() {
                                first = Some((x, y, a, b));
                            }
                        }
                    }
                }
                let (x, y, a, b) = first.unwrap();
                let mb_w = pw.div_ceil(mbs);
                return Err(format!(
                    "frame {i} (poc {}, pts {}) plane {name}: first mismatch at ({x},{y}) MB {} (mb_x {}, mb_y {}): got {a} want {b}; {count} samples differ, max diff {maxd}",
                    p.poc,
                    p.pts,
                    (y / mbs) * mb_w + x / mbs,
                    x / mbs,
                    y / mbs
                ));
            }
        }
    }
    if pics.len() != nref {
        return Err(format!("decoder produced {} frames, reference has {}", pics.len(), nref));
    }
    Ok(())
}

/// Full check of one fixture: generate, decode, compare. Returns Ok(false) when skipped.
pub fn check_fixture(name: &str) -> Result<bool, String> {
    let f = fixture(name);
    let Some((h264, yuv)) = ensure(f) else { return Ok(false) };
    let reference = std::fs::read(&yuv).unwrap();
    let bytes = sample_bytes(f);
    let c422 = pix_fmt(f).contains("422");
    // single-threaded and frame-threaded decoding (an odd thread count and every core) must all
    // be bit-exact, and so must the 16-bit sample path (fork H2)
    for (threads, wide) in [(1, false), (3, false), (0, false), (3, true)] {
        let pics = match decode_file_with(&h264, threads, false, wide) {
            Ok(p) => p,
            Err((au, e, partial)) => {
                // still report comparison of what was decoded
                let cmp = compare_format(&partial, &reference, f.width as usize, f.height as usize, bytes, c422).err().unwrap_or_default();
                return Err(format!("{name} (threads {threads}, wide {wide}): decode error at access unit {au}: {e} (after {} pictures) {cmp}", partial.len()));
            }
        };
        compare_format(&pics, &reference, f.width as usize, f.height as usize, bytes, c422)
            .map_err(|e| format!("{name} (threads {threads}, wide {wide}): {e}"))?;
        if pics.iter().any(|p| p.draft) {
            return Err(format!("{name} (threads {threads}): draft picture without draft mode"));
        }
        check_pts(&pics).map_err(|e| format!("{name} (threads {threads}): {e}"))?;
    }
    Ok(true)
}

/// Output pts values (= access unit index) must be a permutation of 0..n, and follow POC order
/// between IDR pictures.
pub fn check_pts(pics: &[Picture]) -> Result<(), String> {
    let mut seen = vec![false; pics.len()];
    for p in pics {
        let i = p.pts as usize;
        if i >= seen.len() || seen[i] {
            return Err(format!("unexpected pts {} in output", p.pts));
        }
        seen[i] = true;
    }
    for w in pics.windows(2) {
        if !w[1].key && w[1].poc <= w[0].poc {
            return Err(format!("output POC order violated: {} then {}", w[0].poc, w[1].poc));
        }
    }
    Ok(())
}
