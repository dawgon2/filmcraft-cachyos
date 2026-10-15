//! The H.264 and HEVC front ends against a recording stand-in for the hardware, on every system:
//! for x264 streams with B-pyramids, several references, weighted prediction, several slices and
//! open GOPs, and x265 streams (Main and Main 10, open GOP with RASL pictures, B-pyramids),
//! the pictures come out in the software decoder's order with its pts (whole stream, after seeks,
//! after a mid-stream flush), and every buffer handed to the hardware is consistent: the current
//! picture's surface is free, every reference is a decoded surface listed in the DPB, reference
//! lists have the active number of entries, and slice data offsets lie inside the slice.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use filmcraft_codecs::DecodedFrame;
use filmcraft_codecs::hw::NalStreamInfo;
use filmcraft_frame::VideoFrame;

use super::Accel;
use super::ffi::{self, VAIQMatrixBufferH264, VAPictureH264, VAPictureParameterBufferH264, VASliceParameterBufferH264};
use super::h264::Front;
use crate::biplanar::{self, Biplanar, Geometry};

/// Records what it is asked to do and checks it; "decodes" nothing.
struct Recorder {
    ids: Vec<ffi::VASurfaceID>,
    decoded: Vec<bool>,
    pictures: usize,
    frame: VideoFrame,
    /// Every picture read out keeps its surface (as frames of a zero-copy decoder do) until
    /// [`Recorder::release`]; `pick` then skips them and adds surfaces instead.
    holding: bool,
    held: BTreeSet<usize>,
    /// Surfaces made mid-grey (`fill(_, None)`: stand-ins for missing references).
    grey_fills: usize,
    /// Surfaces picked for stand-ins (`pick_gray`).
    grey_picks: usize,
    /// Pictures decoded with the range-extension buffers (4:2:2, fork U0.2.3).
    rext_pictures: usize,
    /// Every H.264 picture: target surface, CurrPic, the frame list and the first slice's
    /// RefPicList0 (active entries).
    h264_log: Vec<(usize, VAPictureH264, Vec<VAPictureH264>, Vec<VAPictureH264>)>,
}

impl Recorder {
    fn new(count: usize) -> Self {
        let g = Geometry { crop: (0, 0, 2, 2), bits: 8, color: Default::default(), par: (1, 1) };
        let frame = biplanar::to_frame(&Biplanar { luma: &[128; 4], chroma: &[128; 2], stride: 2, rows: 2 }, &g).unwrap();
        Self {
            ids: (0..count as u32).map(|i| 0x100 + i).collect(),
            decoded: vec![false; count],
            pictures: 0,
            frame,
            holding: false,
            held: BTreeSet::new(),
            grey_fills: 0,
            grey_picks: 0,
            rext_pictures: 0,
            h264_log: Vec::new(),
        }
    }

    fn holding(count: usize) -> Self {
        Self { holding: true, ..Self::new(count) }
    }

    /// The frames handed out so far are dropped: their surfaces are free again.
    fn release(&mut self) {
        self.held.clear();
    }
}

fn valid(p: &VAPictureH264) -> bool {
    p.flags & ffi::VA_PICTURE_H264_INVALID == 0
}

impl Accel for Recorder {
    fn surfaces(&self) -> &[ffi::VASurfaceID] {
        &self.ids
    }

    fn decode_hevc(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferHEVC,
        _iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        slices: &[(ffi::VASliceParameterBufferHEVC, Vec<u8>)],
    ) -> Result<(), String> {
        assert_eq!(pic.CurrPic.picture_id, self.ids[target], "current picture surface");
        assert!(!self.held.contains(&target), "surface {target} is held by a frame");
        for (sp, data) in slices {
            assert!(data.len() > sp.slice_data_byte_offset as usize, "a slice without data after its header reached the hardware");
        }
        let valid = |p: &ffi::VAPictureHEVC| p.flags & ffi::VA_PICTURE_HEVC_INVALID == 0;
        let n = pic.ReferenceFrames.iter().take_while(|p| valid(p)).count();
        assert!(pic.ReferenceFrames[n..].iter().all(|p| !valid(p)), "reference frames packed at the front");
        let ids: BTreeSet<_> = pic.ReferenceFrames[..n].iter().map(|r| r.picture_id).collect();
        assert_eq!(ids.len(), n, "each reference frame once");
        assert!(!ids.contains(&pic.CurrPic.picture_id), "the target is not a reference");
        for r in &pic.ReferenceFrames[..n] {
            let i = self.ids.iter().position(|&id| id == r.picture_id).expect("reference is a decoder surface");
            assert!(self.decoded[i], "reference surface {i} holds a picture");
        }
        let rps = ffi::VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE | ffi::VA_PICTURE_HEVC_RPS_ST_CURR_AFTER | ffi::VA_PICTURE_HEVC_RPS_LT_CURR;
        assert!(!slices.is_empty());
        let last = ffi::hevc_slice_bits::LAST_SLICE_OF_PIC;
        for (k, (s, data)) in slices.iter().enumerate() {
            assert_eq!(s.slice_data_size as usize, data.len());
            assert!((s.slice_data_byte_offset as usize) <= data.len(), "slice data starts inside the slice");
            assert_eq!(s.LongSliceFlags >> last & 1 == 1, k + 1 == slices.len(), "LastSliceOfPic on the last slice only");
            let ty = (s.LongSliceFlags >> ffi::hevc_slice_bits::SLICE_TYPE) & 3;
            let lists: &[(usize, u8)] = match ty {
                2 => &[],
                1 => &[(0, s.num_ref_idx_l0_active_minus1)],
                _ => &[(0, s.num_ref_idx_l0_active_minus1), (1, s.num_ref_idx_l1_active_minus1)],
            };
            for &(l, minus1) in lists {
                let len = minus1 as usize + 1;
                for &i in &s.RefPicList[l][..len] {
                    assert!((i as usize) < n, "list entry {i} is a reference frame");
                    assert!(pic.ReferenceFrames[i as usize].flags & rps != 0, "list entries are in the current RPS");
                }
                assert!(s.RefPicList[l][len..].iter().all(|&i| i == 0xFF), "nothing past the active entries");
            }
        }
        self.decoded[target] = true;
        self.pictures += 1;
        Ok(())
    }

    /// The range-extension buffers carry the base ones unchanged (checked as such), a 4:2:2
    /// picture, no screen-content tools, and the base slices' weighted-prediction offsets at
    /// full width.
    fn decode_hevc_rext(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferHEVCExtension,
        iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        slices: &[(ffi::VASliceParameterBufferHEVCExtension, Vec<u8>)],
    ) -> Result<(), String> {
        assert_eq!(pic.base.pic_fields >> ffi::hevc_pic_bits::CHROMA_FORMAT_IDC & 3, 2, "a 4:2:2 picture");
        assert_eq!(pic.rext.range_extension_pic_fields, 0, "no range-extension tool");
        assert_eq!(pic.scc, ffi::VAPictureParameterBufferHEVCScc::NONE, "no screen-content tool");
        for (s, _) in slices {
            let (b, r) = (&s.base, &s.rext);
            assert!(b.luma_offset_l0.iter().zip(&r.luma_offset_l0).all(|(&a, &w)| i16::from(a) == w), "l0 luma offsets");
            assert!(b.luma_offset_l1.iter().zip(&r.luma_offset_l1).all(|(&a, &w)| i16::from(a) == w), "l1 luma offsets");
            let chroma = |a: &[[i8; 2]; 15], w: &[[i16; 2]; 15]| a.iter().flatten().zip(w.iter().flatten()).all(|(&a, &w)| i16::from(a) == w);
            assert!(chroma(&b.ChromaOffsetL0, &r.ChromaOffsetL0) && chroma(&b.ChromaOffsetL1, &r.ChromaOffsetL1), "chroma offsets");
            assert_eq!(r.slice_ext_flags, 0);
        }
        let base: Vec<_> = slices.iter().map(|(s, d)| (s.base, d.clone())).collect();
        self.decode_hevc(target, &pic.base, iq, &base)?;
        self.rext_pictures += 1;
        Ok(())
    }

    fn decode_h264(
        &mut self,
        target: usize,
        pic: &VAPictureParameterBufferH264,
        _iq: &VAIQMatrixBufferH264,
        slices: &[(VASliceParameterBufferH264, Vec<u8>)],
    ) -> Result<(), String> {
        assert_eq!(pic.CurrPic.picture_id, self.ids[target], "current picture surface");
        assert!(!self.held.contains(&target), "surface {target} is held by a frame");
        let refs: Vec<_> = pic.ReferenceFrames.iter().filter(|r| valid(r)).collect();
        let ref_ids: BTreeSet<_> = refs.iter().map(|r| r.picture_id).collect();
        // (frame_num gap stand-ins share a decoded picture's surface, under their own frame_num)
        let keys: BTreeSet<_> = refs.iter().map(|r| (r.picture_id, r.frame_idx)).collect();
        assert_eq!(keys.len(), refs.len(), "each reference frame once");
        // the target is not a reference, except that a second field decodes into the surface
        // holding its first field, listed as that (opposite) field alone
        let fields = ffi::VA_PICTURE_H264_TOP_FIELD | ffi::VA_PICTURE_H264_BOTTOM_FIELD;
        for r in refs.iter().filter(|r| r.picture_id == pic.CurrPic.picture_id) {
            let cur = pic.CurrPic.flags & fields;
            assert!(cur != 0 && cur != fields, "the target is not a reference");
            assert_eq!(r.flags & fields, fields & !cur, "only the first field of the target is a reference");
        }
        for r in &refs {
            let i = self.ids.iter().position(|&id| id == r.picture_id).expect("reference is a decoder surface");
            assert!(self.decoded[i], "reference surface {i} holds a picture");
            assert!(r.flags & (ffi::VA_PICTURE_H264_SHORT_TERM_REFERENCE | ffi::VA_PICTURE_H264_LONG_TERM_REFERENCE) != 0, "reference flags");
        }
        assert!(!slices.is_empty());
        for (s, data) in slices {
            assert_eq!(s.slice_data_size as usize, data.len());
            assert!((s.slice_data_bit_offset as usize) < data.len() * 8, "slice data starts inside the slice");
            let ty = s.slice_type;
            let lists: &[(&[VAPictureH264; 32], u8)] = match ty {
                2 => &[],
                0 => &[(&s.RefPicList0, s.num_ref_idx_l0_active_minus1)],
                _ => &[(&s.RefPicList0, s.num_ref_idx_l0_active_minus1), (&s.RefPicList1, s.num_ref_idx_l1_active_minus1)],
            };
            for (list, minus1) in lists {
                let n = *minus1 as usize + 1;
                assert!(list[..n].iter().all(valid), "{n} active entries");
                assert!(list[n..].iter().all(|p| !valid(p)), "nothing past the active entries");
                assert!(list[..n].iter().all(|p| ref_ids.contains(&p.picture_id)), "list entries are reference frames");
            }
        }
        let list0 = slices.first().map_or(Vec::new(), |(s, _)| {
            let n = if s.slice_type == 2 { 0 } else { s.num_ref_idx_l0_active_minus1 as usize + 1 };
            s.RefPicList0.iter().take(n).copied().collect()
        });
        self.h264_log.push((target, pic.CurrPic, refs.iter().map(|r| **r).collect(), list0));
        self.decoded[target] = true;
        self.pictures += 1;
        Ok(())
    }

    fn read(&mut self, index: usize) -> Result<VideoFrame, String> {
        assert!(self.decoded[index], "read back a decoded surface");
        if self.holding {
            self.held.insert(index);
        }
        Ok(self.frame.clone())
    }

    fn fill(&mut self, index: usize, from: Option<usize>) -> Result<(), String> {
        assert!(from.is_none_or(|f| self.decoded[f]));
        assert!(!self.held.contains(&index), "surface {index} is held by a frame");
        self.decoded[index] = true;
        self.grey_fills += usize::from(from.is_none());
        Ok(())
    }

    fn pick_gray(&mut self, candidates: &[usize]) -> Option<usize> {
        self.grey_picks += 1;
        self.pick(candidates)
    }

    fn pick(&mut self, candidates: &[usize]) -> Option<usize> {
        if !self.holding {
            return candidates.first().copied();
        }
        if let Some(&i) = candidates.iter().find(|i| !self.held.contains(i)) {
            return Some(i);
        }
        // every free surface is in a frame: add one (the zero-copy accelerator's way)
        self.ids.push(0x100 + self.ids.len() as u32);
        self.decoded.push(false);
        Some(self.ids.len() - 1)
    }
}

/// An accelerator that only has surfaces: the trait's default `pick`.
struct Bare(Vec<ffi::VASurfaceID>);

impl Accel for Bare {
    fn surfaces(&self) -> &[ffi::VASurfaceID] {
        &self.0
    }
    fn decode_h264(
        &mut self,
        _: usize,
        _: &VAPictureParameterBufferH264,
        _: &VAIQMatrixBufferH264,
        _: &[(VASliceParameterBufferH264, Vec<u8>)],
    ) -> Result<(), String> {
        Err("bare".into())
    }
    fn decode_hevc(
        &mut self,
        _: usize,
        _: &ffi::VAPictureParameterBufferHEVC,
        _: Option<&ffi::VAIQMatrixBufferHEVC>,
        _: &[(ffi::VASliceParameterBufferHEVC, Vec<u8>)],
    ) -> Result<(), String> {
        Err("bare".into())
    }
    fn read(&mut self, _: usize) -> Result<VideoFrame, String> {
        Err("bare".into())
    }
    fn fill(&mut self, _: usize, _: Option<usize>) -> Result<(), String> {
        Err("bare".into())
    }
}

#[test]
fn the_default_pick_takes_the_first_free_surface() {
    let mut a = Bare(vec![1, 2, 3]);
    assert_eq!(a.pick(&[2, 0]), Some(2));
    assert_eq!(a.pick(&[]), None);
}

/// Surfaces still held by frames (a zero-copy decoder's output) are never decoded or filled
/// into: `pick` skips them and the accelerator grows instead, and the pictures still come out
/// in the software decoder's order; released, the surfaces are reused (no further growth).
#[test]
fn pick_skips_the_surfaces_frames_still_hold() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let (name, profile, params) = STREAMS[2];
    if let Some(path) = fixture(&ff, name, profile, params) {
        let s = read_stream(&path);
        let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
        let mbs = (info.coded.0 / 16, info.coded.1 / 16);
        let mut f = Front::new(Recorder::holding(18), info.length_size, mbs, &info.parameter_sets).unwrap();
        let mut sw = filmcraft_codecs::software_video_decoder(&s.entry).unwrap();
        assert_eq!(run_front(&mut f, &s.samples), run_software(sw.as_mut(), &s.samples), "{name}");
        let grown = f.accel().ids.len();
        assert!(grown > 18 && f.accel().held.len() == s.samples.len(), "{name}: {grown} surfaces, {} held", f.accel().held.len());
        // seeks into the open GOP (frame_num gap fills) with every frame held
        for k in (1..s.samples.len()).filter(|&i| s.sync[i]) {
            f.reset();
            sw.reset();
            let end = (k + 13).min(s.samples.len());
            assert_eq!(run_front(&mut f, &s.samples[k..end]), run_software(sw.as_mut(), &s.samples[k..end]), "{name} from sample {k}");
        }
        let grown = f.accel().ids.len();
        f.accel_mut().release();
        f.reset();
        sw.reset();
        assert_eq!(run_front(&mut f, &s.samples), run_software(sw.as_mut(), &s.samples), "{name} after the release");
        assert_eq!(f.accel().ids.len(), grown, "{name}: released surfaces are reused");
    }
    let (name, pix_fmt, params) = HEVC_STREAMS[0];
    if let Some(path) = hevc_fixture(&ff, name, pix_fmt, params) {
        let s = read_stream(&path);
        let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
        let mut f = super::hevc::Front::new(Recorder::holding(18), info.length_size, info.coded, info.bit_depth_luma, &info.parameter_sets).unwrap();
        let mut sw = filmcraft_codecs::software_video_decoder(&s.entry).unwrap();
        assert_eq!(run_hevc(&mut f, &s.samples), run_software(sw.as_mut(), &s.samples), "{name}");
        assert!(f.accel().ids.len() > 18, "{name}: grew");
    }
}

/// (file, x264 profile, x264 parameters).
const STREAMS: &[(&str, &str, &str)] = &[
    ("va_front_high.mp4", "high", "bframes=3:b-pyramid=normal:keyint=12:min-keyint=12:scenecut=0:ref=4:weightp=2"),
    ("va_front_slices.mp4", "baseline", "keyint=12:min-keyint=12:scenecut=0:slices=4:ref=2"),
    ("va_front_open_gop.mp4", "high", "bframes=3:b-pyramid=strict:keyint=12:min-keyint=12:scenecut=0:open-gop=1:direct=temporal"),
];

fn fixture(ff: &std::path::Path, name: &str, profile: &str, params: &str) -> Option<PathBuf> {
    let out = filmcraft_testkit::fixtures_dir("platform").join(name);
    let src = "testsrc2=s=320x184:r=24:d=2";
    filmcraft_testkit::fixtures::generate(&out, |tmp| {
        Command::new(ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", src, "-c:v", "libx264", "-profile:v", profile, "-x264-params", params, "-pix_fmt", "yuv420p"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

struct Stream {
    entry: filmcraft_isobmff::SampleEntry,
    samples: Vec<(Vec<u8>, i64)>,
    sync: Vec<bool>,
}

fn read_stream(path: &std::path::Path) -> Stream {
    let bytes = std::fs::read(path).unwrap();
    let file = filmcraft_isobmff::open(bytes.clone()).unwrap();
    let t = file.track_of_kind(filmcraft_isobmff::TrackKind::Video).unwrap();
    let track = &file.tracks[t];
    let samples = (0..track.samples.len()).map(|i| (file.read_sample(&bytes, t, i).unwrap(), track.samples[i].pts)).collect();
    Stream { entry: track.entries[0].clone(), samples, sync: track.samples.iter().map(|s| s.is_sync).collect() }
}

fn front(s: &Stream) -> Front<Recorder> {
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    let mbs = (info.coded.0 / 16, info.coded.1 / 16);
    Front::new(Recorder::new(18), info.length_size, mbs, &info.parameter_sets).unwrap()
}

fn pts(frames: &[DecodedFrame]) -> Vec<i64> {
    frames.iter().map(|f| f.pts).collect()
}

fn run_front(f: &mut Front<Recorder>, samples: &[(Vec<u8>, i64)]) -> Vec<i64> {
    let mut out = Vec::new();
    for (s, p) in samples {
        out.extend(pts(&f.decode(s, *p).unwrap()));
    }
    out.extend(pts(&f.flush().unwrap()));
    out
}

fn run_software(d: &mut dyn filmcraft_codecs::VideoDecoder, samples: &[(Vec<u8>, i64)]) -> Vec<i64> {
    let mut out = Vec::new();
    for (s, p) in samples {
        out.extend(pts(&d.decode(s, *p).unwrap()));
    }
    out.extend(pts(&d.flush()));
    out
}

#[test]
fn output_order_and_buffers_match_the_software_decoder() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (name, profile, params) in STREAMS {
        let Some(path) = fixture(&ff, name, profile, params) else { continue };
        let s = read_stream(&path);
        let mut f = front(&s);
        let mut sw = filmcraft_codecs::software_video_decoder(&s.entry).unwrap();
        let a = run_front(&mut f, &s.samples);
        assert_eq!(a, run_software(sw.as_mut(), &s.samples), "{name}");
        assert_eq!(f.accel().pictures, s.samples.len(), "{name}: every picture decoded");
        // seeks to every sync sample (open-GOP I pictures included: frame_num gap stand-ins)
        for k in (1..s.samples.len()).filter(|&i| s.sync[i]) {
            f.reset();
            sw.reset();
            let end = (k + 13).min(s.samples.len());
            assert_eq!(run_front(&mut f, &s.samples[k..end]), run_software(sw.as_mut(), &s.samples[k..end]), "{name} from sample {k}");
        }
        // a flush mid-GOP, then on
        f.reset();
        sw.reset();
        let mid = s.samples.len() / 2 + 1;
        let mut a = run_front(&mut f, &s.samples[..mid]);
        a.extend(run_front(&mut f, &s.samples[mid..]));
        let mut b = run_software(sw.as_mut(), &s.samples[..mid]);
        b.extend(run_software(sw.as_mut(), &s.samples[mid..]));
        assert_eq!(a, b, "{name} with a flush at {mid}");
    }
}

/// Damaged samples give errors (the hybrid decoder then continues in software), never a panic.
#[test]
fn damaged_samples_are_errors() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let (name, profile, params) = STREAMS[0];
    let Some(path) = fixture(&ff, name, profile, params) else { return };
    let s = read_stream(&path);
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..40 {
        let mut f = front(&s);
        for (i, (smp, p)) in s.samples.iter().enumerate() {
            let mut m = smp.clone();
            if i % 3 == round % 3 {
                match next() % 3 {
                    0 => m.truncate(next() as usize % m.len().max(1)),
                    1 => {
                        let at = next() as usize % m.len().max(1);
                        if let Some(b) = m.get_mut(at) {
                            *b ^= 1 << (next() % 8);
                        }
                    }
                    _ => m.iter_mut().skip(4).take(6).for_each(|b| *b = next() as u8),
                }
            }
            if f.decode(&m, *p).is_err() {
                f.reset();
            }
        }
        let _ = f.flush();
    }
}

/// (file, pixel format, x265 parameters).
const HEVC_STREAMS: &[(&str, &str, &str)] = &[
    ("va_front_hevc_open_gop.mp4", "yuv420p", "log-level=error:bframes=4:b-pyramid=1:keyint=12:min-keyint=12:scenecut=0:open-gop=1:ref=3"),
    ("va_front_hevc_main10.mp4", "yuv420p10le", "log-level=error:bframes=3:keyint=12:min-keyint=12:scenecut=0:slices=2:weightp=1"),
];

fn hevc_fixture(ff: &std::path::Path, name: &str, pix_fmt: &str, params: &str) -> Option<PathBuf> {
    let out = filmcraft_testkit::fixtures_dir("platform").join(name);
    let src = "testsrc2=s=320x184:r=24:d=2";
    let profile = if pix_fmt.contains("422") {
        "main422-10"
    } else if pix_fmt.contains("10") {
        "main10"
    } else {
        "main"
    };
    filmcraft_testkit::fixtures::generate(&out, |tmp| {
        Command::new(ff)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                src,
                "-c:v",
                "libx265",
                "-profile:v",
                profile,
                "-tag:v",
                "hvc1",
                "-x265-params",
                params,
                "-pix_fmt",
                pix_fmt,
            ])
            .arg(tmp)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn hevc_front(s: &Stream) -> super::hevc::Front<Recorder> {
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    super::hevc::Front::new(Recorder::new(18), info.length_size, info.coded, info.bit_depth_luma, &info.parameter_sets).unwrap()
}

fn run_hevc(f: &mut super::hevc::Front<Recorder>, samples: &[(Vec<u8>, i64)]) -> Vec<i64> {
    let mut out = Vec::new();
    for (s, p) in samples {
        out.extend(pts(&f.decode(s, *p).unwrap()));
    }
    out.extend(pts(&f.flush().unwrap()));
    out
}

#[test]
fn hevc_output_order_and_buffers_match_the_software_decoder() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (name, pix_fmt, params) in HEVC_STREAMS {
        let Some(path) = hevc_fixture(&ff, name, pix_fmt, params) else { continue };
        let s = read_stream(&path);
        let mut f = hevc_front(&s);
        let mut sw = filmcraft_codecs::software_video_decoder(&s.entry).unwrap();
        assert_eq!(run_hevc(&mut f, &s.samples), run_software(sw.as_mut(), &s.samples), "{name}");
        // seeks to every sync sample (open-GOP CRA pictures included: RASL pictures left out)
        for k in (1..s.samples.len()).filter(|&i| s.sync[i]) {
            f.reset();
            sw.reset();
            let end = (k + 13).min(s.samples.len());
            assert_eq!(run_hevc(&mut f, &s.samples[k..end]), run_software(sw.as_mut(), &s.samples[k..end]), "{name} from sample {k}");
        }
    }
}

/// Damaged HEVC samples give errors, never a panic.
#[test]
fn hevc_damaged_samples_are_errors() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let (name, pix_fmt, params) = HEVC_STREAMS[0];
    let Some(path) = hevc_fixture(&ff, name, pix_fmt, params) else { return };
    let s = read_stream(&path);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..40 {
        let mut f = hevc_front(&s);
        for (i, (smp, p)) in s.samples.iter().enumerate() {
            let mut m = smp.clone();
            if i % 3 == round % 3 {
                match next() % 3 {
                    0 => m.truncate(next() as usize % m.len().max(1)),
                    1 => {
                        let at = next() as usize % m.len().max(1);
                        if let Some(b) = m.get_mut(at) {
                            *b ^= 1 << (next() % 8);
                        }
                    }
                    _ => m.iter_mut().skip(4).take(6).for_each(|b| *b = next() as u8),
                }
            }
            if f.decode(&m, *p).is_err() {
                f.reset();
            }
        }
        let _ = f.flush();
    }
}

/// Tile layouts the driver would trust are checked first (fork U0.2.2, from `va_hevc`): more
/// tiles than VA's arrays or than CTBs, explicit sizes that overrun the picture or leave a tile
/// empty are errors; valid layouts resolve to sizes that add up to the picture.
#[test]
fn hevc_hostile_tile_layouts_are_refused() {
    use super::hevc::tile_sizes;
    assert_eq!(tile_sizes(3, &[], 10, true, 19).unwrap(), vec![3, 3, 4]);
    assert_eq!(tile_sizes(3, &[2, 5], 10, false, 19).unwrap(), vec![2, 5, 3]);
    assert_eq!(tile_sizes(1, &[], 7, false, 19).unwrap(), vec![7]);
    let hostile: [(u32, &[u32], u32, bool); 9] = [
        (20, &[], 64, true),               // more columns than VA's 19
        (5, &[], 4, true),                 // more tiles than CTBs
        (0, &[], 4, true),                 // no tile
        (u32::MAX, &[], u32::MAX, true),   // more than VA's arrays
        (3, &[2, 8], 10, false),           // the last tile would be empty
        (3, &[2, 9], 10, false),           // past the picture
        (3, &[0, 5], 10, false),           // an empty tile
        (3, &[2], 10, false),              // too few sizes
        (2, &[u32::MAX], u32::MAX, false), // the last tile would be empty
    ];
    for (n, explicit, total, uniform) in hostile {
        assert!(tile_sizes(n, explicit, total, uniform, 19).is_err(), "{n} tiles {explicit:?} in {total}");
    }
}

/// A slice whose entry points announce WPP substreams past its end (a truncated sample) is an
/// error before any buffer reaches the hardware (fork U0.2.2, from `va_hevc`: Intel's media
/// engine resets on such slices).
#[test]
fn hevc_entry_points_past_the_slice_never_reach_the_hardware() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let params = "log-level=error:wpp=1:ctu=16:bframes=0:keyint=12:min-keyint=12:scenecut=0";
    let Some(path) = hevc_fixture(&ff, "va_front_hevc_wpp.mp4", "yuv420p", params) else { return };
    let s = read_stream(&path);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    // the whole stream decodes
    let mut f = hevc_front(&s);
    assert_eq!(run_hevc(&mut f, &s.samples).len(), s.samples.len());
    // the first picture with its slice cut in half
    let ls = info.length_size;
    let nals = filmcraft_bitstream::length_prefixed_nals(&s.samples[0].0, ls).unwrap();
    let mut cut = Vec::new();
    let mut truncated = false;
    for nal in nals {
        let vcl = (nal[0] >> 1) & 0x3f < 32;
        let nal = if vcl && !truncated {
            truncated = true;
            &nal[..nal.len() / 2]
        } else {
            nal
        };
        cut.extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - ls..]);
        cut.extend_from_slice(nal);
    }
    assert!(truncated);
    let mut f = hevc_front(&s);
    let Err(err) = f.decode(&cut, 0) else { panic!("refused") };
    assert!(err.to_string().contains("entry points"), "{err}");
    assert_eq!(f.accel().pictures, 0, "nothing was handed to the hardware");
}

/// A slice cut anywhere, including exactly at the end of its header (no data bytes left), is
/// refused or decoded with data; it never reaches the hardware empty (the `Recorder` checks every
/// slice). Today the slice-header parser already refuses the cut at the header's end, so
/// `slice_params`'s `data_start >= len` (fork U0.2.2 review, as `va_hevc`) is a second guard this
/// property test does not reach on its own.
#[test]
fn hevc_slice_cut_at_its_header_never_reaches_the_hardware() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let (name, pix_fmt, params) = HEVC_STREAMS[0];
    let Some(path) = hevc_fixture(&ff, name, pix_fmt, params) else { return };
    let s = read_stream(&path);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    let ls = info.length_size;
    let nals = filmcraft_bitstream::length_prefixed_nals(&s.samples[0].0, ls).unwrap();
    let vcl_len = nals.iter().find(|n| (n[0] >> 1) & 0x3f < 32).unwrap().len();
    let mut refused = 0;
    for keep in 3..vcl_len {
        let mut cut = Vec::new();
        let mut done = false;
        for nal in &nals {
            let nal: &[u8] = if (nal[0] >> 1) & 0x3f < 32 && !done {
                done = true;
                &nal[..keep]
            } else {
                nal
            };
            cut.extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - ls..]);
            cut.extend_from_slice(nal);
        }
        // the picture reaches the hardware at the flush (the `Recorder` checks its slices)
        let mut f = hevc_front(&s);
        if f.decode(&cut, 0).is_err() || f.flush().is_err() {
            refused += 1;
        }
    }
    assert!(refused > 0, "{name}: short cuts are refused");
}

/// Missing reference pictures (decoding starts at a picture that is not a random-access point)
/// are an error by default (the hybrid decoder replays in software) and grey stand-in surfaces
/// with [`super::hevc::Front::set_conceal_missing`] (fork U0.2.2, the zero-copy decoder: as the
/// fork's decoder and the software decoder conceal them); every buffer stays consistent.
#[test]
fn hevc_missing_references_get_grey_stand_ins() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (name, pix_fmt, params) in HEVC_STREAMS {
        let Some(path) = hevc_fixture(&ff, name, pix_fmt, params) else { continue };
        let s = read_stream(&path);
        let k = (1..s.samples.len()).find(|&i| !s.sync[i]).unwrap();
        let mut f = hevc_front(&s);
        let refused = s.samples[k..].iter().any(|(smp, p)| f.decode(smp, *p).is_err());
        assert!(refused, "{name}: missing references refused by default");
        let mut f = hevc_front(&s);
        f.set_conceal_missing(true);
        let out = run_hevc(&mut f, &s.samples[k..]);
        assert!(!out.is_empty(), "{name}: pictures come out");
        assert!(f.accel().grey_fills > 0, "{name}: stand-ins were made grey");
        assert_eq!(f.accel().grey_picks, f.accel().grey_fills, "{name}: stand-in surfaces come from `pick_gray` (fork U0.2.3)");
        // after a seek to a random-access point nothing is missing
        f.reset();
        let fills = f.accel().grey_fills;
        let sync = (1..s.samples.len()).find(|&i| s.sync[i]).unwrap();
        run_hevc(&mut f, &s.samples[sync..]);
        assert_eq!(f.accel().grey_fills, fills, "{name}: no stand-ins from a sync sample on");
    }
}

/// Fork U0.2.3: HEVC Main 4:2:2 10 through the front end set to 4:2:2 decodes every picture with
/// the range-extension buffers, in the software decoder's order, also after seeks; surfaces made
/// for 4:2:0 refuse it as a format change before anything reaches the hardware.
#[test]
fn hevc_422_takes_the_range_extension_buffers() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let params = "log-level=error:bframes=3:keyint=12:min-keyint=12:scenecut=0:weightp=1:open-gop=1";
    let Some(path) = hevc_fixture(&ff, "va_front_hevc_422.mp4", "yuv422p10le", params) else { return };
    let s = read_stream(&path);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    assert_eq!((info.chroma_format_idc, info.bit_depth_luma), (2, 10));
    let mut f = hevc_front(&s);
    f.set_chroma_format(2);
    let mut sw = filmcraft_codecs::software_video_decoder(&s.entry).unwrap();
    assert_eq!(run_hevc(&mut f, &s.samples), run_software(sw.as_mut(), &s.samples));
    assert!(f.accel().pictures >= s.samples.len() && f.accel().rext_pictures == f.accel().pictures, "every picture with the rext buffers");
    for k in (1..s.samples.len()).filter(|&i| s.sync[i]) {
        f.reset();
        sw.reset();
        assert_eq!(run_hevc(&mut f, &s.samples[k..]), run_software(sw.as_mut(), &s.samples[k..]), "from sample {k}");
    }
    let mut g = hevc_front(&s);
    let err = s.samples.iter().find_map(|(smp, p)| g.decode(smp, *p).err()).expect("refused");
    assert!(err.to_string().contains("format change"), "{err}");
    assert_eq!(g.accel().pictures, 0, "nothing reached the hardware");
}

/// The PPS NAL unit `pps` with tiles enabled: `columns` x `rows` uniformly spaced (the rest of
/// the PPS as it was). The stream must already have entry points (WPP), so its slice headers
/// parse alike.
fn with_tiles(pps: &[u8], columns: u32, rows: u32) -> Vec<u8> {
    use filmcraft_bitstream::{BitReader, BitWriter, escape_rbsp, unescape_rbsp};
    let rbsp = unescape_rbsp(&pps[2..]);
    let mut r = BitReader::new(&rbsp);
    r.read_ue().unwrap(); // pps_pic_parameter_set_id
    r.read_ue().unwrap(); // pps_seq_parameter_set_id
    r.read_bits(2 + 3 + 2).unwrap(); // dependent slices, output flag, extra bits, sign hiding, cabac init
    r.read_ue().unwrap();
    r.read_ue().unwrap(); // num_ref_idx defaults
    r.read_se().unwrap(); // init_qp
    r.read_bits(2).unwrap(); // constrained intra, transform skip
    if r.read_flag().unwrap() {
        r.read_ue().unwrap(); // diff_cu_qp_delta_depth
    }
    r.read_se().unwrap();
    r.read_se().unwrap(); // chroma QP offsets
    r.read_bits(4).unwrap(); // slice chroma offsets, weighted pred / bipred, transquant bypass
    let at = r.position();
    assert!(!r.read_flag().unwrap(), "no tiles yet");
    assert!(r.read_flag().unwrap(), "WPP");
    let rest = r.position();
    let stop = rbsp.len() * 8 - 1 - rbsp.last().unwrap().trailing_zeros() as usize;
    let mut w = BitWriter::new();
    let bit = |i: usize| rbsp[i / 8] >> (7 - i % 8) & 1 == 1;
    (0..at).for_each(|i| w.write_bit(bit(i)));
    w.write_bit(true); // tiles_enabled_flag
    w.write_bit(true); // entropy_coding_sync_enabled_flag
    w.write_ue(columns - 1);
    w.write_ue(rows - 1);
    w.write_bit(true); // uniform_spacing_flag
    w.write_bit(true); // loop_filter_across_tiles_enabled_flag
    (rest..stop).for_each(|i| w.write_bit(bit(i)));
    w.rbsp_trailing();
    let mut nal = pps[..2].to_vec();
    nal.extend(escape_rbsp(&w.finish()));
    nal
}

/// Fork U0.2.3 (U0.2.2 review): a hostile tile layout in a PPS (more tile columns than VA's 19
/// slots, still within the CTBs and the parser's limit) is refused through `Front::decode`
/// before any buffer reaches the hardware, not only by `tile_sizes` on its own; a valid tile
/// layout made the same way decodes (the buffers are consistent).
#[test]
fn hevc_hostile_tile_layout_is_refused_through_decode() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let params = "log-level=error:wpp=1:ctu=16:bframes=0:keyint=12:min-keyint=12:scenecut=0";
    let Some(path) = hevc_fixture(&ff, "va_front_hevc_wpp.mp4", "yuv420p", params) else { return };
    let s = read_stream(&path);
    let info = NalStreamInfo::from_entry(&s.entry).unwrap().unwrap();
    // 320x184 in 16x16 CTBs: 20 x 12
    let sets = |columns: u32, rows: u32| -> Vec<Vec<u8>> {
        info.parameter_sets.iter().map(|n| if (n[0] >> 1) & 0x3f == 34 { with_tiles(n, columns, rows) } else { n.clone() }).collect()
    };
    let front = |sets: &[Vec<u8>]| super::hevc::Front::new(Recorder::new(18), info.length_size, info.coded, info.bit_depth_luma, sets).unwrap();
    // the first sample, its own parameter sets (if any) altered alike
    let first = |columns: u32, rows: u32| -> Vec<u8> {
        let ls = info.length_size;
        let mut out = Vec::new();
        for nal in filmcraft_bitstream::length_prefixed_nals(&s.samples[0].0, ls).unwrap() {
            let nal = if (nal[0] >> 1) & 0x3f == 34 { with_tiles(nal, columns, rows) } else { nal.to_vec() };
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - ls..]);
            out.extend_from_slice(&nal);
        }
        out
    };
    // valid: 4 x 3 tiles
    let mut f = front(&sets(4, 3));
    assert!(f.decode(&first(4, 3), 0).is_ok() && f.flush().is_ok(), "a valid tile layout decodes");
    assert_eq!(f.accel().pictures, 1);
    // hostile: 20 columns (VA has 19 slots)
    let mut f = front(&sets(20, 1));
    let err = f.decode(&first(20, 1), 0).err().expect("refused");
    assert!(err.to_string().contains("tiles"), "{err}");
    assert_eq!(f.accel().pictures, 0, "nothing reached the hardware");
}

/// A synthetic PAFF (field picture) stream for the front end: Main profile, POC type 0, CAVLC,
/// headers only (the recording accelerator decodes nothing, so slice data is filler). No free
/// encoder writes PAFF (fork OWNER-QUESTIONS Q31); real streams are checked on hardware (H1.5).
mod paff {
    use super::*;
    use filmcraft_bitstream::BitWriter;

    /// Macroblocks per frame (4 × 4: two map-unit rows of field macroblock pairs).
    pub const MBS: (u32, u32) = (4, 4);

    fn nal(ref_idc: u8, ty: u8, w: BitWriter) -> Vec<u8> {
        let mut out = vec![(ref_idc & 3) << 5 | ty];
        out.extend(w.finish());
        out
    }

    pub fn sps(mbaff: bool) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(77, 8);
        w.write_bits(0, 8);
        w.write_bits(30, 8);
        w.write_ue(0); // sps_id
        w.write_ue(0); // log2_max_frame_num_minus4
        w.write_ue(0); // poc type 0
        w.write_ue(2); // log2_max_poc_lsb_minus4
        w.write_ue(2); // max_num_ref_frames
        w.write_bit(false); // gaps
        w.write_ue(MBS.0 - 1);
        w.write_ue(MBS.1 / 2 - 1); // map units are field macroblock pair rows
        w.write_bit(false); // frame_mbs_only
        w.write_bit(mbaff);
        w.write_bit(true); // direct_8x8
        w.write_bit(false); // cropping
        w.write_bit(false); // vui
        w.rbsp_trailing();
        nal(3, 7, w)
    }

    pub fn pps() -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_ue(0); // pps_id
        w.write_ue(0); // sps_id
        w.write_bit(false); // CAVLC
        w.write_bit(false); // bottom_field_pic_order_in_frame_present
        w.write_ue(0); // one slice group
        w.write_ue(0); // num_ref_idx_l0_default_active_minus1
        w.write_ue(0); // l1
        w.write_bit(false); // weighted_pred
        w.write_bits(0, 2); // weighted_bipred_idc
        w.write_se(0); // pic_init_qp_minus26
        w.write_se(0); // pic_init_qs_minus26
        w.write_se(0); // chroma_qp_index_offset
        w.write_bit(true); // deblocking_filter_control_present
        w.write_bit(false); // constrained_intra_pred
        w.write_bit(false); // redundant_pic_cnt_present
        w.rbsp_trailing();
        nal(3, 8, w)
    }

    /// One reference slice: `field` None = frame, Some(bottom); `refs` = active list-0 entries
    /// (0 = an I slice).
    pub fn slice(frame_num: u32, field: Option<bool>, lsb: u32, refs: u32, idr: bool) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_ue(0); // first_mb_in_slice
        w.write_ue(if refs == 0 { 7 } else { 5 }); // all I / all P
        w.write_ue(0); // pps_id
        w.write_bits(frame_num, 4);
        w.write_bit(field.is_some());
        if let Some(bottom) = field {
            w.write_bit(bottom);
        }
        if idr {
            w.write_ue(0); // idr_pic_id
        }
        w.write_bits(lsb, 6);
        if refs > 0 {
            w.write_bit(true); // num_ref_idx_active_override
            w.write_ue(refs - 1);
            w.write_bit(false); // no list modification
        }
        if idr {
            w.write_bit(false); // no_output_of_prior_pics
            w.write_bit(false); // long_term_reference
        } else {
            w.write_bit(false); // sliding window
        }
        w.write_se(0); // slice_qp_delta
        w.write_ue(1); // deblocking off
        for _ in 0..8 {
            w.write_bits(0xA5, 8); // filler "slice data"
        }
        w.rbsp_trailing();
        nal(2, if idr { 5 } else { 1 }, w)
    }

    /// A sample of 4-byte length-prefixed NAL units.
    pub fn sample(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in nals {
            out.extend((n.len() as u32).to_be_bytes());
            out.extend(n);
        }
        out
    }

    /// Frame 0: IDR top field + P bottom field; frame 1: P fields with 2 / 3 references; frame 2:
    /// a P frame picture (PAFF streams mix both); frame 3: P fields again.
    pub fn stream() -> Vec<(Vec<u8>, i64)> {
        vec![
            (sample(&[slice(0, Some(false), 0, 0, true), slice(0, Some(true), 1, 1, false)]), 0),
            (sample(&[slice(1, Some(false), 4, 2, false), slice(1, Some(true), 5, 3, false)]), 1),
            (sample(&[slice(2, None, 8, 1, false)]), 2),
            (sample(&[slice(3, Some(false), 12, 2, false), slice(3, Some(true), 13, 2, false)]), 3),
        ]
    }

    pub fn front(mbaff: bool) -> Front<Recorder> {
        Front::new(Recorder::new(18), 4, MBS, &[sps(mbaff), pps()]).unwrap()
    }
}

const TOP: u32 = ffi::VA_PICTURE_H264_TOP_FIELD;
const BOTTOM: u32 = ffi::VA_PICTURE_H264_BOTTOM_FIELD;
const SHORT: u32 = ffi::VA_PICTURE_H264_SHORT_TERM_REFERENCE;

#[test]
fn paff_fields_pair_into_one_surface_with_field_flags_and_lists() {
    let mut f = paff::front(false);
    // four frames out, each once, in order
    assert_eq!(run_front(&mut f, &paff::stream()), vec![0, 1, 2, 3]);
    let log = &f.accel().h264_log;
    assert_eq!(log.len(), 7, "six fields and a frame decoded");
    let surf = |k: usize| f.accel().ids[log[k].0];
    // second fields decode into their first field's surface
    assert_eq!(log[0].0, log[1].0);
    assert_eq!(log[2].0, log[3].0);
    assert_eq!(log[5].0, log[6].0);
    assert_ne!(log[0].0, log[2].0);
    // CurrPic: the field flag and only its own order count
    let cur = |k: usize| (log[k].1.flags, log[k].1.TopFieldOrderCnt, log[k].1.BottomFieldOrderCnt);
    assert_eq!(cur(0), (TOP, 0, 0));
    assert_eq!(cur(1), (BOTTOM, 0, 1));
    assert_eq!(cur(2), (TOP, 4, 0));
    assert_eq!(cur(4), (0, 8, 8), "a frame picture");
    let entries = |v: &[VAPictureH264]| v.iter().map(|p| (p.picture_id, p.flags, p.TopFieldOrderCnt, p.BottomFieldOrderCnt)).collect::<Vec<_>>();
    // frame 0's bottom field refers to its own top field, the frame list names that field alone
    assert_eq!(entries(&log[1].3), vec![(surf(0), TOP | SHORT, 0, 0)]);
    assert_eq!(entries(&log[1].2), vec![(surf(0), TOP | SHORT, 0, 0)]);
    // frame 1's top field: same parity first (8.2.4.2.5); a complete pair is a frame entry
    assert_eq!(entries(&log[2].3), vec![(surf(0), TOP | SHORT, 0, 0), (surf(0), BOTTOM | SHORT, 0, 1)]);
    assert_eq!(entries(&log[2].2), vec![(surf(0), SHORT, 0, 1)]);
    // frame 1's bottom field: frame 0's bottom, then its own top field, then frame 0's top
    assert_eq!(entries(&log[3].3), vec![(surf(0), BOTTOM | SHORT, 0, 1), (surf(2), TOP | SHORT, 4, 0), (surf(0), TOP | SHORT, 0, 0)]);
    // the frame picture refers to frame 1 as a frame
    assert_eq!(entries(&log[4].3), vec![(surf(2), SHORT, 4, 5)]);
    // two reference frames (max_num_ref_frames 2): frame 2 slid frame 0 out
    assert!(log[5].2.iter().all(|r| r.picture_id != surf(0)));
    assert_eq!(entries(&log[5].3), vec![(surf(4), TOP | SHORT, 8, 0), (surf(4), BOTTOM | SHORT, 0, 8)]);
}

#[test]
fn mbaff_frames_take_the_hardware_path() {
    let mut f = paff::front(true);
    let samples = vec![(paff::sample(&[paff::slice(0, None, 0, 0, true)]), 0), (paff::sample(&[paff::slice(1, None, 4, 1, false)]), 1)];
    assert_eq!(run_front(&mut f, &samples), vec![0, 1]);
    let log = &f.accel().h264_log;
    assert_eq!(log.len(), 2);
    assert_eq!(log[1].1.flags, 0);
    assert_eq!(log[1].3.len(), 1);
}

/// Hostile field sequences (unpaired fields, repeated parities, frame_num jumps, frames and
/// fields of one frame_num, damaged bytes) are decoded or refused, never a panic, and every
/// buffer the hardware gets passes the recorder's checks.
#[test]
fn hostile_field_sequences_never_panic() {
    use paff::{sample, slice};
    let odd = vec![
        (sample(&[slice(0, Some(true), 0, 0, true)]), 0),
        (sample(&[slice(0, Some(true), 1, 1, false)]), 1),
        (sample(&[slice(5, Some(false), 9, 2, false), slice(5, Some(false), 9, 2, false)]), 2),
        (sample(&[slice(5, Some(true), 9, 3, false)]), 3),
        (sample(&[slice(6, None, 12, 2, false), slice(6, Some(true), 13, 2, false)]), 4),
        (sample(&[slice(7, Some(true), 15, 1, false)]), 5),
    ];
    let mut f = paff::front(false);
    for (s, p) in &odd {
        if f.decode(s, *p).is_err() {
            f.reset();
        }
    }
    let _ = f.flush();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let stream = paff::stream();
    for _ in 0..200 {
        let mut f = paff::front(false);
        for (smp, p) in &stream {
            let mut m = smp.clone();
            let at = next() as usize % m.len().max(1);
            if let Some(b) = m.get_mut(at) {
                *b ^= 1 << (next() % 8);
            }
            if f.decode(&m, *p).is_err() {
                f.reset();
            }
        }
        let _ = f.flush();
    }
}
