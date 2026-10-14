//! ITU-T H.264.1 conformance bitstreams (fork H2.8): each stream is decoded and compared with
//! the reference decode shipped with it. The streams are not in the repository (downloaded once,
//! kept outside git): point `FILMCRAFT_H264_CONFORMANCE_DIR` at a directory holding one folder per
//! extracted archive and run
//! `cargo test --release -p filmcraft-h264 --test conformance_vectors -- --ignored --nocapture`.
//!
//! Every stream is reported as `PASS`, `MISMATCH`, `UNSUPPORTED` (a feature the decoder refuses,
//! e.g. field pictures before H2.5) or `SKIP` (no single bitstream in the folder). Archives without
//! a reference decode (some FRExt ones ship encoder statistics only) are compared with ffmpeg's
//! decode instead (`PASS (… ffmpeg)`; unsupported when ffmpeg is absent). The test fails on a
//! mismatch, on a decode error that is not "unsupported", or on a panic.

mod common;

use filmcraft_h264::params::Sps;
use filmcraft_h264::{Decoder, Error, Picture};
use std::path::{Path, PathBuf};

const STREAM_EXT: &[&str] = &["264", "h264", "jsv", "26l", "avc", "jvt", "bit", "bits", "bin"];
const NOT_REFERENCE_EXT: &[&str] = &["txt", "doc", "docx", "pdf", "html", "htm", "xls", "xlsx", "md5", "zip", "log", "cfg", "rtf", "trc", "dat"];

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// The bitstream and (when the archive has one) its reference decode in one extracted archive.
fn pair(dir: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let mut all = Vec::new();
    files(dir, &mut all);
    let streams: Vec<&PathBuf> = all.iter().filter(|p| STREAM_EXT.contains(&ext(p).as_str())).collect();
    let [stream] = streams.as_slice() else { return None };
    let size = |p: &PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let lower = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let reference =
        all.iter().filter(|p| *p != *stream && !NOT_REFERENCE_EXT.contains(&ext(p).as_str()) && !lower(p).contains("trace")).max_by_key(|p| size(p));
    Some(((*stream).clone(), reference.cloned()))
}

enum Outcome {
    Pass(usize, &'static str),
    Mismatch(String),
    Unsupported(String),
    Error(String),
}

/// Bytes of one reference picture for `p`'s format.
fn frame_bytes(p: &Picture) -> usize {
    let (w, h) = (p.width as usize, p.height as usize);
    let (cw, ch) = (p.chroma_width as usize, p.chroma_height as usize);
    (w * h + 2 * cw * ch) * if p.bit_depth > 8 { 2 } else { 1 }
}

fn picture_bytes(p: &Picture) -> Vec<u8> {
    match &p.high {
        Some(planes) => planes.iter().flat_map(|pl| pl.iter().flat_map(|s| s.to_le_bytes())).collect(),
        None => [&p.y, &p.u, &p.v].into_iter().flat_map(|pl| pl.iter().copied()).collect(),
    }
}

/// ffmpeg's raw decode of `stream` in `p`'s format (the oracle for archives without a reference).
fn ffmpeg_reference(stream: &Path, p: &Picture) -> Option<Vec<u8>> {
    let ff = filmcraft_testkit::ffmpeg()?;
    let chroma = if p.chroma_format_idc == 2 { "422" } else { "420" };
    let fmt = match p.bit_depth {
        8 => format!("yuv{chroma}p"),
        b => format!("yuv{chroma}p{b}le"),
    };
    let out = common::fixtures_dir().join(format!("conformance-{}.yuv", stream.file_stem()?.to_str()?));
    let ok = std::process::Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(stream)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", &fmt])
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    let data = if ok { std::fs::read(&out).ok() } else { None };
    let _ = std::fs::remove_file(&out);
    data
}

fn run(stream: &Path, reference: Option<&Path>) -> Outcome {
    let data = std::fs::read(stream).unwrap_or_default();
    let mut dec = Decoder::with_threads(4);
    let mut pics = Vec::new();
    for au in common::split_access_units(&data) {
        match dec.decode(au, 0) {
            Ok(p) => pics.extend(p),
            Err(Error::Unsupported(what)) => return Outcome::Unsupported(what),
            Err(e) => return Outcome::Error(format!("after {} pictures: {e}", pics.len())),
        }
    }
    pics.extend(dec.flush());
    if let Some(e) = dec.take_error() {
        return match e {
            Error::Unsupported(what) => Outcome::Unsupported(what),
            e => Outcome::Error(format!("after {} pictures: {e}", pics.len())),
        };
    }
    let Some(first) = pics.first() else { return Outcome::Error("no pictures".into()) };
    let (reference, oracle) = match reference {
        Some(r) => (std::fs::read(r).unwrap_or_default(), "reference"),
        None => match ffmpeg_reference(stream, first) {
            Some(r) => (r, "ffmpeg"),
            None => return Outcome::Unsupported("no reference decode and no ffmpeg".into()),
        },
    };
    let fb = frame_bytes(first);
    // some references are stored uncropped (CVFC1_Sony_C): crop them like the decoder does
    let (reference, oracle) = match uncropped_reference(&data, first, &reference) {
        Some(cropped) if fb != 0 && !reference.len().is_multiple_of(fb) => (cropped, "uncropped reference"),
        _ => (reference, oracle),
    };
    if fb == 0 || !reference.len().is_multiple_of(fb) {
        return Outcome::Mismatch(format!("reference is {} bytes, not a multiple of {fb} ({}x{})", reference.len(), first.width, first.height));
    }
    let nref = reference.len() / fb;
    if nref > pics.len() {
        // the reference shows pictures as often as the pic_struct SEI says (frame doubling /
        // tripling, 3:2 pulldown with frames mixing two pictures' fields; CVSE2 / CVSE3 /
        // CVSEFDFT3): both fields of every decoded picture must appear in it, in order
        let fields = |b: &[u8], parity: usize| field_lines(b, first, parity);
        let ref_fields: Vec<[Vec<u8>; 2]> = (0..nref).map(|j| [0, 1].map(|par| fields(&reference[j * fb..(j + 1) * fb], par))).collect();
        let mut next = 0;
        for p in &pics {
            let want = picture_bytes(p);
            let mut at = [0usize; 2];
            for par in 0..2 {
                let f = fields(&want, par);
                let Some(j) = (next..nref).find(|&j| ref_fields[j][par] == f) else {
                    return Outcome::Mismatch(format!("picture (poc {}) field {par} not in the reference after its picture {next}", p.poc));
                };
                at[par] = j;
            }
            next = at[0].max(at[1]);
        }
        return Outcome::Pass(pics.len(), "reference with pic_struct repeats");
    }
    if nref != pics.len() {
        return Outcome::Mismatch(format!("{} pictures decoded, reference has {nref}", pics.len()));
    }
    for (i, p) in pics.iter().enumerate() {
        if picture_bytes(p) != reference[i * fb..(i + 1) * fb] {
            return Outcome::Mismatch(format!("picture {i} (poc {}) differs", p.poc));
        }
    }
    Outcome::Pass(pics.len(), oracle)
}

/// The lines of parity `parity` (0 top, 1 bottom) of every plane of a picture in `p`'s format.
fn field_lines(pic: &[u8], p: &Picture, parity: usize) -> Vec<u8> {
    let bytes = if p.bit_depth > 8 { 2 } else { 1 };
    let (w, h) = (p.width as usize * bytes, p.height as usize);
    let (cw, ch) = (p.chroma_width as usize * bytes, p.chroma_height as usize);
    let mut out = Vec::new();
    let mut at = 0;
    for (pw, ph) in [(w, h), (cw, ch), (cw, ch)] {
        for y in (parity..ph).step_by(2) {
            out.extend_from_slice(pic.get(at + y * pw..at + (y + 1) * pw).unwrap_or_default());
        }
        at += pw * ph;
    }
    out
}

/// `reference` cropped by the stream's first SPS cropping rectangle, when it holds whole
/// uncropped pictures in `p`'s format.
fn uncropped_reference(stream: &[u8], p: &Picture, reference: &[u8]) -> Option<Vec<u8>> {
    let nal = filmcraft_bitstream::annexb_nals(stream).into_iter().find(|n| n.first().is_some_and(|b| b & 0x1f == 7))?;
    let sps = Sps::parse(&filmcraft_bitstream::unescape_rbsp(nal.get(1..)?)).ok()?;
    let (w, h) = (sps.width() as usize, sps.height() as usize);
    let (cx, cy, cw, ch) = sps.crop_rect();
    let (cx, cy, cw, ch) = (cx as usize, cy as usize, cw as usize, ch as usize);
    if (cw, ch) != (p.width as usize, p.height as usize) || (cw, ch) == (w, h) {
        return None;
    }
    let bytes = if p.bit_depth > 8 { 2 } else { 1 };
    let (sx, sy) = (2, if p.chroma_format_idc == 2 { 1 } else { 2 });
    let full = (w * h + 2 * (w / sx) * (h / sy)) * bytes;
    if full == 0 || !reference.len().is_multiple_of(full) {
        return None;
    }
    let mut out = Vec::new();
    for pic in reference.chunks(full) {
        let mut plane_at = 0;
        for (pw, ph, x0, y0, rw, rh) in [
            (w, h, cx, cy, cw, ch),
            (w / sx, h / sy, cx / sx, cy / sy, cw.div_ceil(sx), ch.div_ceil(sy)),
            (w / sx, h / sy, cx / sx, cy / sy, cw.div_ceil(sx), ch.div_ceil(sy)),
        ] {
            for y in y0..y0 + rh {
                let row = plane_at + (y * pw + x0) * bytes;
                out.extend_from_slice(pic.get(row..row + rw * bytes)?);
            }
            plane_at += pw * ph * bytes;
        }
    }
    Some(out)
}

#[test]
#[ignore]
fn itu_conformance_streams() {
    let Some(root) = std::env::var_os("FILMCRAFT_H264_CONFORMANCE_DIR").map(PathBuf::from) else {
        eprintln!("skipped: FILMCRAFT_H264_CONFORMANCE_DIR not set");
        return;
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    dirs.sort();
    let (mut pass, mut unsupported, mut skipped) = (0, 0, 0);
    let mut failures = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let Some((stream, reference)) = pair(dir) else {
            println!("SKIP        {name}: no single bitstream");
            skipped += 1;
            continue;
        };
        let outcome = std::panic::catch_unwind(|| run(&stream, reference.as_deref())).unwrap_or_else(|_| Outcome::Error("panicked".into()));
        match outcome {
            Outcome::Pass(n, oracle) => {
                println!("PASS        {name} ({n} pictures, {oracle})");
                pass += 1;
            }
            Outcome::Unsupported(what) => {
                println!("UNSUPPORTED {name}: {what}");
                unsupported += 1;
            }
            Outcome::Mismatch(m) => {
                println!("MISMATCH    {name}: {m}");
                failures.push(format!("{name}: {m}"));
            }
            Outcome::Error(e) => {
                println!("ERROR       {name}: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    println!("conformance: {pass} pass, {unsupported} unsupported, {skipped} skipped, {} failed", failures.len());
    assert!(failures.is_empty(), "{failures:#?}");
}
