//! Fork U0.2.3b d / CLOUD-QUEUE MKV-HANDOFF: [`MediaSource::set_handoff_scale`] reaches
//! `hw::handoff_scale()` while each container source makes its decoder (where a hardware decoder
//! factory reads it), and only for a sane scale. A recording factory registered ahead of the
//! built-in ones notes the scale it sees and declines, so our own decoders still do the work.
//! ffmpeg writes the fixtures (never linked).

mod common;

use std::cell::RefCell;

use common::*;
use filmcraft_codecs::{VideoDecoder, hw, register_video_decoder};
use filmcraft_isobmff::SampleEntry;
use filmcraft_media::FrameRequest;
use filmcraft_time::Tick;

thread_local! {
    /// The hand-off scale each decoder creation on this thread saw.
    static SEEN: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

fn recording_factory(_: &SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn VideoDecoder>>> {
    SEEN.with(|s| s.borrow_mut().push(hw::handoff_scale()));
    None
}

/// The scales seen by the decoder creations of one source (opened fresh) after
/// `set_handoff_scale(scale)` (`None`: never called), for its first frame.
fn seen(name: &str, args: &[&str], scale: Option<f32>) -> Option<Vec<f32>> {
    let ff = filmcraft_testkit::ffmpeg_or_skip("handoff_scale")?;
    let mut all = vec!["-f", "lavfi", "-i", "testsrc2=size=256x144:rate=25", "-t", "0.2"];
    all.extend_from_slice(args);
    // ffmpeg is here, so a fixture it can't write is this test's bug, not a reason to skip
    let Some(p) = fixture(&ff, name, &all) else { panic!("{name}: ffmpeg could not write the fixture") };
    register_video_decoder(recording_factory);
    let src = filmcraft_codecs::open_bytes(name, bytes(&p)).unwrap();
    if let Some(s) = scale {
        src.set_handoff_scale(s);
    }
    SEEN.with(|s| s.borrow_mut().clear());
    let f = src.video_frame(FrameRequest::full(Tick::ZERO));
    assert!(f.is_ok(), "{name}: {:?}", f.err());
    let v = SEEN.with(|s| s.take());
    assert!(!v.is_empty(), "{name}: no decoder was made through the factories");
    // the scale is scoped to decoder creation: this thread is back to the default
    assert_eq!(hw::handoff_scale(), 1.0, "{name}: scale leaked out of decoder creation");
    Some(v)
}

fn check(name: &str, args: &[&str]) {
    let Some(unset) = seen(name, args, None) else { return };
    assert!(unset.iter().all(|&s| s == 1.0), "{name} unset: {unset:?}");
    let Some(half) = seen(name, args, Some(0.5)) else { return };
    assert!(half.iter().all(|&s| s == 0.5), "{name} 0.5: {half:?}");
    // hostile or meaningless scales are ignored (unscaled)
    for bad in [f32::NAN, f32::INFINITY, 0.0, -0.5, 1.0, 2.0] {
        let Some(v) = seen(name, args, Some(bad)) else { return };
        assert!(v.iter().all(|&s| s == 1.0), "{name} {bad}: {v:?}");
    }
}

#[test]
fn mp4_scale_reaches_decoder_creation() {
    check("handoff_mjpeg.mov", &["-c:v", "mjpeg", "-pix_fmt", "yuvj420p"]);
}

#[test]
fn mkv_scale_reaches_decoder_creation() {
    check("handoff_mjpeg.mkv", &["-c:v", "mjpeg", "-pix_fmt", "yuvj420p"]);
}

#[test]
fn mxf_scale_reaches_decoder_creation() {
    check("handoff_dnxhr.mxf", &["-c:v", "dnxhd", "-profile:v", "dnxhr_lb", "-pix_fmt", "yuv422p"]);
}
