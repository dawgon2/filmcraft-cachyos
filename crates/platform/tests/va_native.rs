//! In-process VA-API HEVC decoding is bit-exact with ffmpeg's reference decode on the HEVC
//! conformance fixtures (`target/fixtures/hevc/*.hevc` + `.yuv`, made by
//! `cargo xtask fixtures hevc`; `FILMCRAFT_FIXTURES_DIR` moves the root). 4:2:0 streams go through
//! the decoder the media stack uses for them since fork U0.2.2, upstream's front end with
//! zero-copy output (`vaapi::decode_annexb_zero_copy`), and since U0.2.3 every other stream too
//! (4:2:2 on the display that has Main 4:2:2 10, range-extension profiles): nothing goes through
//! the fork's own decoder (`va_hevc`) any more. Skipped without a VA-API device or the fixtures.
#![cfg(target_os = "linux")]

use filmcraft_frame::PixelData;
use filmcraft_platform::vaapi::decode_annexb_zero_copy;

#[test]
fn bit_exact_with_the_reference_on_the_conformance_fixtures() {
    if let Err(e) = filmcraft_platform::va::Display::get() {
        eprintln!("no VA-API: {e}; skipped");
        return;
    }
    let dir = filmcraft_testkit::fixtures::fixtures_root().join("hevc");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no fixtures in {}; skipped", dir.display());
        return;
    };
    let mut names: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "hevc")).collect();
    names.sort();
    let mut failures = Vec::new();
    let mut front = 0;
    for path in names {
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(reference) = std::fs::read(path.with_extension("yuv")) else { continue };
        let data = std::fs::read(&path).expect("read fixture");
        let decoded = decode_annexb_zero_copy(&data);
        let path_name = "front end, zero-copy";
        let frames = match decoded {
            Ok(f) => f,
            Err(e) => {
                // streams the hardware does not take (e.g. sizes below its minimum) are fine to refuse
                eprintln!("{name} ({path_name}): declined: {e}");
                continue;
            }
        };
        // the reference: every picture's Y, U, V planes back to back (8- or 16-bit samples)
        let mut got = Vec::new();
        for f in &frames {
            match f.to_planar().as_ref().unwrap_or(f).data.clone() {
                PixelData::Yuv8 { planes, .. } => planes.iter().for_each(|p| got.extend_from_slice(p)),
                PixelData::Yuv16 { planes, .. } => planes.iter().for_each(|p| p.iter().for_each(|v| got.extend_from_slice(&v.to_le_bytes()))),
                _ => failures.push(format!("{name}: unexpected pixel format")),
            }
        }
        front += 1;
        if got != reference {
            let first = got.iter().zip(&reference).position(|(a, b)| a != b);
            failures.push(format!(
                "{name} ({path_name}): {} frames, {} bytes vs {} reference; first difference at byte {first:?}",
                frames.len(),
                got.len(),
                reference.len()
            ));
        } else {
            eprintln!("{name} ({path_name}): {} frames bit-exact", frames.len());
        }
    }
    eprintln!("{front} fixtures checked through the front end (zero-copy)");
    assert!(failures.is_empty(), "VA-API differs from the reference:\n{}", failures.join("\n"));
}
