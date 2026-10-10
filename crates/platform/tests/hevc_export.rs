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
        crf,
        bitrate_kbps: 4000,
        max_bitrate_kbps: Some(6000),
        bitrate_mode: BitrateMode::Vbr1Pass,
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
    for (bits, crf, mode) in [(10u8, None, BitrateMode::Vbr1Pass), (8, None, BitrateMode::Cbr), (10, Some(24), BitrateMode::Vbr1Pass)] {
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
