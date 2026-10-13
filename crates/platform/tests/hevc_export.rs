//! H.265 / HEVC export (Linux): VA-API in process, and through the system ffmpeg libx265
//! (B-frames) and `hevc_vaapi`. The
//! packets must decode with our own HEVC decoder from the encoder's sample entry, every frame in
//! order (pts from the composition offsets), with the requested bit depth; damaged settings must
//! give errors, never crash. Skips without ffmpeg or the encoder.
#![cfg(target_os = "linux")]

use filmcraft_export::{BitrateMode, ColorSignal, EncodedPacket, EncoderFrame, ExportSettings, Format, HevcEncoder, VideoEncoder};
use filmcraft_frame::PixelData;
use filmcraft_hevc::params::Sps;
use filmcraft_isobmff::{CodecConfig, ColorInfo};
use filmcraft_platform::hevc_export::{Backend, HevcEncoder as Encoder};
use filmcraft_time::FrameRate;

const W: u32 = 640;
const H: u32 = 360;

/// Frame `i`: flat grey whose level encodes `i` (so output order can be read back from luma).
fn frame(i: u64) -> Vec<u8> {
    let level = (16 + (i * 9) % 220) as u8;
    let mut v = vec![level; (W * H * 4) as usize];
    for px in v.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    v
}

fn settings(encoder: HevcEncoder, bits: u8, crf: Option<u8>) -> ExportSettings {
    ExportSettings {
        format: Format::Hevc,
        hevc_encoder: encoder,
        hevc_bit_depth: bits,
        crf: crf.map_or(filmcraft_export::DEFAULT_CRF, f32::from),
        bitrate_kbps: 4000,
        max_bitrate_kbps: Some(6000),
        bitrate_mode: if crf.is_some() { BitrateMode::Crf } else { BitrateMode::Vbr1Pass },
        keyframe_distance: Some(24),
        ..Default::default()
    }
}

/// Encode `n` frames; `None` when this encoder is not available here.
fn encode(encoder: HevcEncoder, bits: u8, crf: Option<u8>, n: u64) -> Option<(Encoder, Vec<EncodedPacket>)> {
    filmcraft_testkit::require_ffmpeg!(None);
    encode_with(&settings(encoder, bits, crf), None, n)
}

/// Encode `n` frames with `s`, on the first backend of `order` that starts.
fn encode_with(s: &ExportSettings, order: Option<&[Backend]>, n: u64) -> Option<(Encoder, Vec<EncodedPacket>)> {
    let started = match order {
        Some(o) => Encoder::with(W, H, FrameRate::new(24, 1), s, o),
        None => Encoder::new(W, H, FrameRate::new(24, 1), s),
    };
    let encoder = s.hevc_encoder;
    let mut enc = match started {
        Ok(e) => e,
        Err(e) => {
            eprintln!("SKIPPED: {encoder:?} HEVC encoder unavailable: {e}");
            return None;
        }
    };
    let mut packets = Vec::new();
    for i in 0..n {
        let rgba = frame(i);
        packets.extend(enc.encode(&EncoderFrame { width: W, height: H, rgba: &rgba, hdr: None, yuv: None, gpu: None, index: i }).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    Some((enc, packets))
}

/// Decode with FilmCraft's own HEVC decoder; pts are in frames (from the composition offsets).
fn decode(enc: &Encoder, packets: &[EncodedPacket]) -> Vec<(i64, f64, u32)> {
    let entry = enc.sample_entry();
    let mut d = filmcraft_codecs::software_video_decoder(&entry).unwrap();
    let den = packets.first().map_or(1, |p| p.duration as i64);
    let start = enc.media_start().unwrap_or(0);
    let mut out = Vec::new();
    for (k, p) in packets.iter().enumerate() {
        let pts = k as i64 * den + p.composition_offset as i64 - start;
        out.extend(d.decode(&p.data, pts).unwrap());
    }
    out.extend(d.flush());
    out.into_iter()
        .map(|f| {
            let (mean, bits) = match &f.frame.data {
                PixelData::Yuv8 { planes, .. } => (planes[0].iter().map(|&v| v as f64).sum::<f64>() / planes[0].len() as f64, 8),
                PixelData::Yuv16 { planes, bits, .. } => (planes[0].iter().map(|&v| v as f64).sum::<f64>() / planes[0].len() as f64 / 4.0, *bits),
                _ => panic!("planar YUV expected"),
            };
            (f.pts / den, mean, bits)
        })
        .collect()
}

/// Every frame, in order: pts 0..n and luma rising with the source level (wrapping every 25).
fn check(name: &str, out: &[(i64, f64, u32)], n: u64, bits: u32) {
    assert_eq!(out.len() as u64, n, "{name}: frame count");
    for (i, (pts, mean, b)) in out.iter().enumerate() {
        assert_eq!(*pts, i as i64, "{name}: pts order");
        assert_eq!(*b, bits, "{name}: bit depth");
        // limited range luma of grey level L: 16 + 219 * L / 255
        let level = (16 + (i as u64 * 9) % 220) as f64;
        let want = 16.0 + 219.0 * level / 255.0;
        assert!((mean - want).abs() < 3.0, "{name}: frame {i} luma {mean:.1}, want {want:.1} (wrong order?)");
    }
}

#[test]
fn x265_with_b_frames_round_trips_in_order() {
    for bits in [8u8, 10] {
        let Some((enc, packets)) = encode(HevcEncoder::X265, bits, None, 60) else { return };
        assert_eq!(enc.backend(), Backend::X265);
        assert!(packets.first().is_some_and(|p| p.key), "starts with a keyframe");
        assert!(enc.media_start().is_some(), "B-frames: an edit list skips the reorder delay");
        assert!(packets.iter().any(|p| p.composition_offset > 0), "reordered pictures");
        check(&format!("x265 {bits}-bit"), &decode(&enc, &packets), 60, bits as u32);
    }
}

#[test]
fn x265_constant_quality() {
    let Some((enc, packets)) = encode(HevcEncoder::X265, 8, Some(30), 30) else { return };
    check("x265 crf", &decode(&enc, &packets), 30, 8);
}

#[test]
fn vaapi_round_trips_in_order() {
    for (bits, crf) in [(8u8, None), (10, None), (8, Some(28))] {
        let Some((enc, packets)) = encode(HevcEncoder::Vaapi, bits, crf, 60) else { return };
        assert!(matches!(enc.backend(), Backend::VaNative | Backend::Vaapi), "{:?}", enc.backend());
        let keys = packets.iter().filter(|p| p.key).count();
        assert!(keys >= 2, "a keyframe every 24 frames ({keys})");
        check(&format!("VA-API {bits}-bit {crf:?}"), &decode(&enc, &packets), 60, bits as u32);
    }
}

/// ffmpeg's `hevc_vaapi` (the in-process encoder's fallback), forced.
#[test]
fn ffmpeg_vaapi_round_trips_in_order() {
    filmcraft_testkit::require_ffmpeg!();
    let Some((enc, packets)) = encode_with(&settings(HevcEncoder::Vaapi, 10, None), Some(&[Backend::Vaapi]), 30) else { return };
    assert_eq!(enc.backend(), Backend::Vaapi);
    check("hevc_vaapi 10-bit", &decode(&enc, &packets), 30, 10);
}

/// The in-process encoder (fork Phase 5.2b): every rate control, keyframes where asked, no
/// reordering, and the colour description in the stream and the sample entry.
#[test]
fn in_process_vaapi_round_trips_in_order() {
    for (bits, crf, mode) in [(10u8, None, BitrateMode::Vbr1Pass), (8, None, BitrateMode::Cbr), (10, Some(24), BitrateMode::Crf)] {
        let mut s = settings(HevcEncoder::Vaapi, bits, crf);
        s.bitrate_mode = mode;
        s.signal = ColorSignal::HLG;
        let Some((enc, packets)) = encode_with(&s, Some(&[Backend::VaNative]), 50) else { return };
        assert_eq!(enc.backend(), Backend::VaNative);
        let keys: Vec<usize> = packets.iter().enumerate().filter(|(_, p)| p.key).map(|(i, _)| i).collect();
        assert_eq!(keys, vec![0, 24, 48], "an IDR every 24 frames");
        assert!(packets.iter().all(|p| p.composition_offset == 0) && enc.media_start().is_none(), "no reordering");
        let entry = enc.sample_entry();
        let colr = entry.video.as_ref().and_then(|v| v.color.clone());
        assert!(matches!(colr, Some(ColorInfo::Nclx { primaries: 9, transfer: 18, matrix: 9, full_range: false })), "colr box: {colr:?}");
        let CodecConfig::Hevc(cfg) = &entry.codec else { panic!("an hvcC sample entry") };
        let sps = cfg.arrays.iter().find(|a| a.nal_type == 33).and_then(|a| a.nalus.first()).expect("SPS in hvcC");
        let sps = Sps::parse(&filmcraft_bitstream::unescape_rbsp(&sps[2..])).expect("SPS parses");
        let vui = sps.vui.expect("VUI");
        assert_eq!((vui.colour_primaries, vui.transfer_characteristics, vui.matrix_coefficients, vui.full_range), (9, 18, 9, false), "VUI colour");
        assert_eq!(sps.bit_depth_luma, bits as u32);
        check(&format!("in-process {bits}-bit {crf:?} {mode:?}"), &decode(&enc, &packets), 50, bits as u32);
    }
}

/// The "HEVC 4:2:2 10-bit master" (fork 6E.4) on each backend that can write it: Main 4:2:2 10
/// in the hvcC and the SPS, 4:2:2 planes out of our own decoder, every frame in order. The
/// VA-API backends run on a device that encodes 4:2:2 (Intel's here, chosen automatically) and
/// are skipped where none does.
#[test]
fn hevc_422_master_round_trips() {
    filmcraft_testkit::require_ffmpeg!();
    for backend in [Backend::VaNative, Backend::Vaapi, Backend::X265] {
        // asks for 8-bit: 4:2:2 is always written as 10-bit
        let mut s = settings(if backend == Backend::X265 { HevcEncoder::X265 } else { HevcEncoder::Vaapi }, 8, Some(24));
        s.hevc_chroma_422 = true;
        let Some((enc, packets)) = encode_with(&s, Some(&[backend]), 30) else { continue };
        assert_eq!(enc.backend(), backend);
        let entry = enc.sample_entry();
        let CodecConfig::Hevc(cfg) = &entry.codec else { panic!("an hvcC sample entry") };
        assert_eq!((cfg.chroma_format_idc, cfg.bit_depth_luma), (2, 10), "{backend:?}: hvcC");
        let sps = cfg.arrays.iter().find(|a| a.nal_type == 33).and_then(|a| a.nalus.first()).expect("SPS in hvcC");
        let sps = Sps::parse(&filmcraft_bitstream::unescape_rbsp(&sps[2..])).expect("SPS parses");
        assert_eq!((sps.chroma_format_idc, sps.ptl.profile_idc), (2, 4), "{backend:?}: Main 4:2:2 10");
        // 4:2:2 planes: half width, full height
        let mut d = filmcraft_codecs::software_video_decoder(&entry).unwrap();
        let first = packets.first().expect("packets");
        let frames = d.decode(&first.data, 0).unwrap().into_iter().chain(d.flush()).collect::<Vec<_>>();
        let f = frames.first().expect("a decoded frame");
        let PixelData::Yuv16 { planes, .. } = &f.frame.data else { panic!("16-bit planar") };
        assert_eq!((planes[0].len(), planes[1].len()), ((W * H) as usize, (W / 2 * H) as usize), "{backend:?}: 4:2:2 planes");
        check(&format!("4:2:2 {backend:?}"), &decode(&enc, &packets), 30, 10);
    }
}

/// Top half: rows alternating saturated red / blue (only full vertical chroma resolution keeps
/// them apart); bottom half: flat orange (chroma level and plane order).
fn chroma_frame() -> Vec<u8> {
    let mut v = vec![0u8; (W * H * 4) as usize];
    for (i, px) in v.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let row = i as u32 / W;
        *px = if row >= H / 2 {
            [255, 128, 0, 255]
        } else if row.is_multiple_of(2) {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        };
    }
    v
}

/// BT.709 limited-range 10-bit (Cb, Cr) of an RGB8 colour (independent of the exporter's code).
fn cbcr_709(r: u8, g: u8, b: u8) -> (f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    (512.0 + 896.0 * (b - y) / 1.8556, 512.0 + 896.0 * (r - y) / 1.5748)
}

/// Fork 6E.4 (owed since Phase 6): the 4:2:2 master really carries 4:2:2 chroma. Every row of a
/// one-row red / blue alternation keeps its own Cb / Cr (4:2:0 would average row pairs to the
/// same value), and a flat colour lands at its BT.709 Cb / Cr in the right planes.
#[test]
fn hevc_422_master_keeps_per_row_chroma() {
    filmcraft_testkit::require_ffmpeg!();
    let src = chroma_frame();
    for backend in [Backend::VaNative, Backend::Vaapi, Backend::X265] {
        let mut s = settings(if backend == Backend::X265 { HevcEncoder::X265 } else { HevcEncoder::Vaapi }, 10, Some(4));
        s.hevc_chroma_422 = true;
        let mut enc = match Encoder::with(W, H, FrameRate::new(24, 1), &s, &[backend]) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("SKIPPED: {backend:?} 4:2:2 encoder unavailable: {e}");
                continue;
            }
        };
        assert_eq!(enc.backend(), backend);
        let mut packets = Vec::new();
        for i in 0..3 {
            packets.extend(enc.encode(&EncoderFrame { width: W, height: H, rgba: &src, hdr: None, yuv: None, gpu: None, index: i }).unwrap());
        }
        packets.extend(enc.flush().unwrap());
        let mut d = filmcraft_codecs::software_video_decoder(&enc.sample_entry()).unwrap();
        let first = packets.first().expect("packets");
        let frames = d.decode(&first.data, 0).unwrap().into_iter().chain(d.flush()).collect::<Vec<_>>();
        let f = frames.first().expect("a decoded frame");
        let PixelData::Yuv16 { planes, .. } = &f.frame.data else { panic!("16-bit planar") };
        let cw = (W / 2) as usize;
        assert_eq!(planes[1].len(), cw * H as usize, "{backend:?}: 4:2:2 chroma plane");
        // mean of a chroma plane over rows `rows` (every row whole except 8 columns at each edge)
        let mean = |p: usize, rows: &mut dyn Iterator<Item = usize>| {
            let (mut sum, mut n) = (0f64, 0f64);
            for r in rows {
                for &v in &planes[p][r * cw + 4..(r + 1) * cw - 4] {
                    sum += v as f64;
                    n += 1.0;
                }
            }
            sum / n
        };
        let half = (H / 2) as usize;
        // stripes (skipping 8 rows at the top and the band edge)
        let (red, blue) = (cbcr_709(255, 0, 0), cbcr_709(0, 0, 255));
        let red_cr = mean(2, &mut (8..half - 8).step_by(2));
        let blue_cr = mean(2, &mut (9..half - 8).step_by(2));
        let red_cb = mean(1, &mut (8..half - 8).step_by(2));
        let blue_cb = mean(1, &mut (9..half - 8).step_by(2));
        // a lossy encoder may soften the alternation; demand most of it survives
        let (want_cr, want_cb) = (red.1 - blue.1, blue.0 - red.0);
        assert!(red_cr - blue_cr > 0.75 * want_cr, "{backend:?}: Cr red {red_cr:.0} vs blue {blue_cr:.0}, want a gap near {want_cr:.0}");
        assert!(blue_cb - red_cb > 0.75 * want_cb, "{backend:?}: Cb blue {blue_cb:.0} vs red {red_cb:.0}, want a gap near {want_cb:.0}");
        // flat orange: level and plane order (Cb below neutral, Cr above)
        let (cb, cr) = cbcr_709(255, 128, 0);
        let got_cb = mean(1, &mut (half + 8..H as usize - 8));
        let got_cr = mean(2, &mut (half + 8..H as usize - 8));
        assert!((got_cb - cb).abs() < 12.0 && (got_cr - cr).abs() < 12.0, "{backend:?}: orange Cb/Cr {got_cb:.0}/{got_cr:.0}, want {cb:.0}/{cr:.0}");
    }
}

#[test]
fn sample_entry_signals_hdr_colour() {
    let mut s = settings(HevcEncoder::Auto, 10, None);
    s.signal = ColorSignal::PQ;
    filmcraft_testkit::require_ffmpeg!();
    let Ok(enc) = Encoder::new(W, H, FrameRate::new(24, 1), &s) else { return };
    let e = enc.sample_entry();
    let v = e.video.as_ref().unwrap();
    assert!(v.color.is_some() && v.mastering_display.is_some(), "colr / mdcv for PQ");
}

#[test]
fn hostile_settings_are_errors() {
    filmcraft_testkit::require_ffmpeg!();
    let s = settings(HevcEncoder::Auto, 8, None);
    for (w, h, rate) in [
        (0, 360, FrameRate::new(24, 1)),
        (641, 360, FrameRate::new(24, 1)),
        (100_000, 360, FrameRate::new(24, 1)),
        (640, 360, FrameRate::new(0, 1)),
        (640, 360, FrameRate::new(24, -1)),
    ] {
        assert!(Encoder::new(w, h, rate, &s).is_err(), "{w}x{h} @ {rate:?}");
    }
    // a frame of the wrong size is an error, not a crash
    if let Ok(mut enc) = Encoder::new(W, H, FrameRate::new(24, 1), &s) {
        let small = vec![0u8; 16];
        assert!(enc.encode(&EncoderFrame { width: 2, height: 2, rgba: &small, hdr: None, yuv: None, gpu: None, index: 0 }).is_err());
    }
}

#[test]
fn registration_marks_the_format_available() {
    filmcraft_testkit::require_ffmpeg!();
    let registered = filmcraft_platform::hevc_export::register();
    assert_eq!(filmcraft_export::available(Format::Hevc), registered);
    assert_eq!(filmcraft_platform::hevc_export::register(), registered, "registering twice is harmless");
}
