//! The automatic proxy preset (fork 6.2, `hevc_half`) and the settings transcodes build.

use filmcraft_export::{BitrateMode, Format, GpuRendering, HevcEncoder, Multiplexer};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{AudioStreamInfo, Generator, MediaKind, MediaSource};
use filmcraft_project::{MediaClip, MediaRef};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};

use crate::proxies::{FALLBACK_PROXY_PRESET, HEVC_HALF, HEVC_HALF_MIN_KBPS, PRESETS, auto_proxy_preset, hevc_half_kbps, preset, transcode_settings};

/// A 6.2K 4:2:2 movie with audio, `secs` long.
fn clip(w: u32, h: u32, rate: FrameRate, secs: i64, bitrate: Option<u64>, file_size: Option<u64>) -> MediaClip {
    let g = GeneratorSource::new(Generator::ColorMatte { color: [0.5, 0.5, 0.5, 1.0] }, w, h, rate, Tick(secs * TICKS_PER_SECOND));
    let mut info = g.info().clone();
    info.kind = MediaKind::Movie;
    info.file_size = file_size;
    if let Some(v) = info.video.as_mut() {
        v.bitrate = bitrate;
        v.codec = "hevc".into();
        v.pixel_format = "yuv422p10le".into();
    }
    info.audio_streams = vec![AudioStreamInfo { sample_rate: 48_000, channels: 2, codec: "pcm_s24le".into(), bits_per_sample: Some(24) }];
    MediaClip {
        media: MediaRef::File { path: "/footage/A001.mov".into() },
        info,
        interpret: Default::default(),
        mark_in: None,
        mark_out: None,
        markers: vec![],
        offline: false,
        proxy: None,
        identity: None,
    }
}

#[test]
fn hevc_half_settings() {
    // 6.2K 4:2:2 at ~200 Mbps: an eighth is 25 Mbps
    let c = clip(6240, 4160, FrameRate::FPS_29_97, 10, Some(200_000_000), None);
    let s = transcode_settings(&c, &HEVC_HALF, "/footage/Proxies/A001_Proxy.0123456789abcdef.mov");
    assert_eq!(s.format, Format::Hevc);
    assert_eq!(s.scale, 0.5, "half size");
    assert_eq!(s.hevc_bit_depth, 10, "Main 10");
    assert_eq!(s.hevc_encoder, HevcEncoder::Vaapi, "the VA-API encoder");
    assert_eq!(s.multiplexer, Multiplexer::Mov, "QuickTime (hvc1)");
    assert_eq!(s.extension(), "mov");
    assert_eq!(s.bitrate_mode, BitrateMode::Vbr1Pass);
    assert_eq!(s.bitrate_kbps, 25_000);
    assert_eq!(s.max_bitrate_kbps, Some(37_500));
    assert_eq!(s.keyframe_distance, Some(30), "a keyframe every second at 29.97");
    assert_eq!(s.gpu_rendering, GpuRendering::Auto, "composited on the GPU");
    assert!(!s.include_audio, "no audio: playback takes it from the original");
    assert!(s.part_of_batch);
    assert!(s.validate().is_ok());
    assert_eq!(s.path, "/footage/Proxies/A001_Proxy.0123456789abcdef.mov");
}

#[test]
fn hevc_half_bitrate_floor_and_fallbacks() {
    // a low-bitrate source: the 8 Mbps floor
    let c = clip(1920, 1080, FrameRate::FPS_24, 10, Some(20_000_000), None);
    assert_eq!(hevc_half_kbps(&c), HEVC_HALF_MIN_KBPS);
    let s = transcode_settings(&c, &HEVC_HALF, "/p.mov");
    assert_eq!(s.bitrate_kbps, 8_000);
    assert_eq!(s.keyframe_distance, Some(24));
    // no stream bitrate: file size / duration (1 GB over 10 s = 800 Mbps, an eighth 100 Mbps)
    let c = clip(6240, 4160, FrameRate::FPS_25, 10, None, Some(1_000_000_000));
    assert_eq!(hevc_half_kbps(&c), 100_000);
    // nothing known: the floor; hostile values don't overflow
    assert_eq!(hevc_half_kbps(&clip(640, 480, FrameRate::FPS_25, 10, None, None)), HEVC_HALF_MIN_KBPS);
    assert_eq!(hevc_half_kbps(&clip(640, 480, FrameRate::FPS_25, 10, Some(u64::MAX), None)), u32::MAX);
    assert_eq!(hevc_half_kbps(&clip(640, 480, FrameRate::FPS_25, 0, None, Some(u64::MAX))), HEVC_HALF_MIN_KBPS);
}

#[test]
fn auto_preset_falls_back_without_va_encode() {
    assert_eq!(auto_proxy_preset(true).id, "hevc_half");
    let fb = auto_proxy_preset(false);
    assert_eq!(fb.id, FALLBACK_PROXY_PRESET);
    assert_eq!((fb.format, fb.scale, fb.proxy), (Format::ProRes, 0.5, true));
    assert_eq!(preset("hevc_half").map(|p| p.id), Some("hevc_half"));
    assert!(PRESETS.iter().all(|p| p.id != "hevc_half"), "not offered in the Create Proxies dialog yet");
    // the fallback is a proxy: GPU compositing, audio kept like every existing preset
    let c = clip(6240, 4160, FrameRate::FPS_24, 4, Some(200_000_000), None);
    let s = transcode_settings(&c, fb, "/p.mov");
    assert_eq!((s.format, s.gpu_rendering, s.include_audio), (Format::ProRes, GpuRendering::Auto, true));
    assert_eq!(s.prores_profile, "proxy");
    assert_eq!(s.hevc_encoder, HevcEncoder::Auto, "H.265 fields untouched");
}

#[test]
fn full_size_transcodes_keep_the_cpu_renderer() {
    let c = clip(1920, 1080, FrameRate::FPS_24, 4, None, None);
    for p in PRESETS.iter().filter(|p| !p.proxy) {
        let s = transcode_settings(&c, p, "/t.mov");
        assert_eq!(s.gpu_rendering, GpuRendering::Off, "{}", p.id);
        assert!(s.include_audio, "{}", p.id);
    }
    for p in PRESETS.iter().filter(|p| p.proxy) {
        assert_eq!(transcode_settings(&c, p, "/t.mov").gpu_rendering, GpuRendering::Auto, "{}", p.id);
    }
}
