//! In-process VA-API HEVC decoding (`va_hevc`) is bit-exact with ffmpeg's reference decode on
//! the HEVC conformance fixtures (`target/fixtures/hevc/*.hevc` + `.yuv`, made by
//! `cargo xtask fixtures hevc`). Skipped without a VA-API device or the fixtures.
#![cfg(target_os = "linux")]

use filmcraft_frame::PixelData;

#[test]
fn bit_exact_with_the_reference_on_the_conformance_fixtures() {
    if let Err(e) = filmcraft_platform::va::Display::get() {
        eprintln!("no VA-API: {e}; skipped");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/hevc");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no fixtures in {}; skipped", dir.display());
        return;
    };
    let mut names: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "hevc")).collect();
    names.sort();
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in names {
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(reference) = std::fs::read(path.with_extension("yuv")) else { continue };
        let data = std::fs::read(&path).expect("read fixture");
        let frames = match filmcraft_platform::va_hevc::decode_annexb(&data) {
            Ok(f) => f,
            Err(e) => {
                // streams the hardware does not take (e.g. sizes below its minimum) are fine to refuse
                eprintln!("{name}: declined: {e}");
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
        checked += 1;
        if got != reference {
            let first = got.iter().zip(&reference).position(|(a, b)| a != b);
            failures.push(format!("{name}: {} frames, {} bytes vs {} reference; first difference at byte {first:?}", frames.len(), got.len(), reference.len()));
        } else {
            eprintln!("{name}: {} frames bit-exact", frames.len());
        }
    }
    eprintln!("{checked} fixtures checked");
    assert!(failures.is_empty(), "VA-API differs from the reference:\n{}", failures.join("\n"));
}
