//! MP4 audio against ffmpeg: ffmpeg writes an AAC track (with the edit list that skips the encoder
//! priming), and reads at another sample rate must land on the same source samples as the native
//! decode, however the requests are cut.

mod common;

use common::*;

/// A source whose rate differs from the sequence's is resampled per request. An AAC track in MP4
/// carries an edit list that skips the encoder priming (ffmpeg: 1024 samples), and every request
/// must still land on the same source samples: reading at 44.1 kHz in video-frame-sized pieces (as
/// an export does) equals the 48 kHz decode interpolated at the same times.
#[test]
fn aac_mp4_resampled_reads_follow_the_edit_list() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let Some(f) = fixture(&ff, "tone_aac_48k.mp4", &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:d=3", "-c:a", "aac", "-b:a", "128k"]) else {
        panic!("could not write the AAC fixture")
    };
    let src = filmcraft_codecs::open_bytes("tone_aac_48k.mp4", bytes(&f)).unwrap();
    let native = src.audio(0, 2 * 48_000, 48_000).unwrap().channels[0].clone();
    let ratio = 48_000.0 / 44_100.0;
    // 30 fps at 44.1 kHz, starting mid-file and off the 48 kHz grid
    let (mut start, piece) = (22_051i64, 1470usize);
    let (mut worst, mut peak) = (0f32, 0f32);
    for _ in 0..20 {
        let got = src.audio(start, piece, 44_100).unwrap();
        for (k, &v) in got.channels[0].iter().enumerate() {
            let pos = (start + k as i64) as f64 * ratio;
            let i0 = pos.floor() as usize;
            let want = native[i0] + (native[i0 + 1] - native[i0]) * (pos - i0 as f64) as f32;
            worst = worst.max((v - want).abs());
            peak = peak.max(v.abs());
        }
        start += piece as i64;
    }
    let level = native.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak <= level * 1.01, "resampled peak {peak} above the source's {level}");
    assert!(worst < 1e-4, "resampled read differs from the source by {worst}");
}

/// ffmpeg's MP4 muxer skips the B-frame reorder delay with an edit list, so the media (`mdhd`) runs
/// one frame past what plays. The clip must be as long as ffmpeg decodes it: 60 frames, not 61.
#[test]
fn bframe_mp4_duration_is_the_decoded_frame_count() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let args = ["-f", "lavfi", "-i", "testsrc2=size=160x96:rate=30:duration=2", "-c:v", "libx264", "-bf", "2", "-pix_fmt", "yuv420p"];
    let Some(f) = fixture(&ff, "bframes_x264.mp4", &args) else {
        eprintln!("SKIPPED: ffmpeg without libx264");
        return;
    };
    let n = ffmpeg_frames(&ff, &f, "yuv420p").len() / (160 * 96 * 3 / 2);
    assert_eq!(n, 60);
    let src = filmcraft_codecs::open_bytes("bframes_x264.mp4", bytes(&f)).unwrap();
    assert_eq!(src.info().duration, filmcraft_time::FrameRate::FPS_30.tick_of(n as i64));
}

/// The X-M5 records 24-bit LPCM in MOV (`in24`; little-endian with an `enda` box, as ffmpeg
/// writes `pcm_s24le`). Full-scale noise uses all 24 bits; every sample must equal ffmpeg's (a
/// 24-bit sample is exact in f32), in both byte orders, and a window read at random equals the
/// same span of a full read.
#[test]
fn mov_24bit_lpcm_is_sample_exact() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for codec in ["pcm_s24le", "pcm_s24be"] {
        let name = format!("noise_{codec}.mov");
        let Some(f) = fixture(&ff, &name, &["-f", "lavfi", "-i", "anoisesrc=color=white:amplitude=1:sample_rate=48000:d=2", "-ac", "2", "-c:a", codec]) else {
            panic!("could not write {name}")
        };
        let src = filmcraft_codecs::open_bytes(&name, bytes(&f)).unwrap();
        let a = src.info().audio.clone().unwrap();
        assert_eq!((a.channels, a.sample_rate, a.bits_per_sample), (2, 48_000, Some(24)), "{name}");
        let want = ffmpeg_audio_f32(&ff, &f, &[]);
        let n = want.len() / 2;
        let got = src.audio(0, n, 48_000).unwrap();
        let mut differ = 0;
        for i in 0..n {
            for c in 0..2 {
                differ += (got.channels[c][i] != want[i * 2 + c]) as usize;
            }
        }
        assert_eq!(differ, 0, "{name}: {differ} of {} samples differ from ffmpeg's", 2 * n);
        assert!(want.iter().any(|v| (v * 8_388_608.0).rem_euclid(256.0) != 0.0), "{name}: the low byte is exercised");
        let win = src.audio(31_337, 4_800, 48_000).unwrap();
        assert_eq!(win.channels[1][..], got.channels[1][31_337..31_337 + 4_800], "{name}: random access");
    }
}
