//! Two-GPU HEVC export, static-shot join (fork 6E.3 S10, DESIGN-6E3 §2.7): a still picture
//! (bars) exported on one GPU and on two, decoded with our own decoder. The two-GPU file has the
//! same frame count, presentation times and keyframes as the one-GPU file, and next to every join
//! the picture is as close to the source as the one-GPU export's (luma PSNR within 0.5 dB), so a
//! chunk boundary is invisible on a shot where any step would show. The colours match too: every
//! frame's chroma is as close to the source's as the one-GPU export's, with no shift in its mean
//! (a lane with another matrix or range would show; 6E.3 "colour vs Program monitor"). Skips
//! without two VA-API HEVC encoders.
#![cfg(target_os = "linux")]

use std::sync::Arc;

use filmcraft_export::{BitrateMode, ExportEngines, ExportSettings, Format, Progress, export, lanes};
use filmcraft_frame::PixelData;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{FrameRequest, Generator, MediaSource, SharedSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_render::{SourceMap, SourceProvider};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const W: u32 = 640;
const H: u32 = 360;
const FRAMES: usize = 144;

struct Lanes(Arc<SourceMap>);

impl SourceProvider for Lanes {
    fn source(&self, item: ItemId) -> Option<SharedSource> {
        self.0.source(item)
    }
    fn lane_copy(&self) -> Option<Arc<dyn SourceProvider + Send + Sync>> {
        Some(Arc::new(Lanes(self.0.clone())))
    }
}

/// One 6 s still (bars), 24 fps, and the source's luma (BT.709, limited range).
fn project() -> (Arc<Project>, ItemId, Lanes, Vec<f64>) {
    let mut p = Project::new("static");
    let r = FrameRate::FPS_24;
    let len = Tick(6 * TICKS_PER_SECOND);
    let g = GeneratorSource::new(Generator::BarsAndTone, W, H, r, len);
    let info = g.info().clone();
    let rgba = g.video_frame(FrameRequest::full(Tick::ZERO)).unwrap().to_rgba8();
    let luma = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| 16.0 + 219.0 * (0.2126 * px[0] as f64 + 0.7152 * px[1] as f64 + 0.0722 * px[2] as f64) / 255.0)
        .collect::<Vec<_>>();
    let clip = MediaClip {
        media: MediaRef::Generator(g.generator.clone()),
        info: info.clone(),
        interpret: Default::default(),
        mark_in: None,
        mark_out: None,
        markers: vec![],
        offline: false,
        proxy: None,
        identity: None,
    };
    let id = p.add_item(&info.name, Label::Iris, ItemKind::Media(clip), None);
    let mut m = SourceMap::default();
    m.0.insert(id, Arc::new(g));
    let seq = p.new_sequence("s", SequenceSettings { width: W, height: H, frame_rate: r, ..Default::default() }, 1, 0, None);
    let v = p.make_track_item(id, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, len), r).unwrap();
    p.sequence_mut(seq).unwrap().video_tracks[0].items.push(v);
    (Arc::new(p), seq, Lanes(Arc::new(m)), luma)
}

/// The source's BT.709 limited-range Cb and Cr at 4:2:0 (each 2×2 block averaged).
fn source_chroma() -> [Vec<f64>; 2] {
    let g = GeneratorSource::new(Generator::BarsAndTone, W, H, FrameRate::FPS_24, Tick(TICKS_PER_SECOND));
    let rgba = g.video_frame(FrameRequest::full(Tick::ZERO)).unwrap().to_rgba8();
    let px = rgba.as_chunks::<4>().0;
    let (w, h) = (W as usize, H as usize);
    let mut out = [vec![0.0; w / 2 * h / 2], vec![0.0; w / 2 * h / 2]];
    for y in 0..h {
        for x in 0..w {
            let [r, g, b, _] = px[y * w + x].map(|v| v as f64 / 255.0);
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let i = y / 2 * (w / 2) + x / 2;
            out[0][i] += (128.0 + 224.0 * (b - l) / 1.8556) / 4.0;
            out[1][i] += (128.0 + 224.0 * (r - l) / 1.5748) / 4.0;
        }
    }
    out
}

/// Per decoded frame of `path`: (Cb, Cr) PSNR to `source`, and (Cb, Cr) mean minus the source's.
fn read_chroma(path: &str, source: &[Vec<f64>; 2]) -> Vec<[(f64, f64); 2]> {
    let bytes = std::fs::read(path).unwrap();
    let file = filmcraft_isobmff::open(bytes.clone()).unwrap();
    let vt = file.tracks.iter().position(|t| t.kind == filmcraft_isobmff::TrackKind::Video).unwrap();
    let track = &file.tracks[vt];
    let mut d = filmcraft_codecs::software_video_decoder(&track.entries[0]).unwrap();
    let mut out = Vec::new();
    for (k, s) in track.samples.iter().enumerate() {
        let data = file.read_sample(&bytes, vt, k).unwrap();
        out.extend(d.decode(&data, s.pts).unwrap());
    }
    out.extend(d.flush());
    out.iter()
        .map(|f| {
            let planar = f.frame.to_planar().unwrap_or_else(|| f.frame.clone());
            [1, 2].map(|p| {
                let c: Vec<f64> = match &planar.data {
                    PixelData::Yuv8 { planes, .. } => planes[p].iter().map(|&v| v as f64).collect(),
                    PixelData::Yuv16 { planes, bits, .. } => planes[p].iter().map(|&v| v as f64 / f64::from(1u32 << (bits - 8))).collect(),
                    other => panic!("unexpected decoded format {:?}", std::mem::discriminant(other)),
                };
                let src = &source[p - 1];
                assert_eq!(c.len(), src.len(), "4:2:0 chroma");
                let n = c.len() as f64;
                let mse = c.iter().zip(src).map(|(a, b)| (a - b) * (a - b)).sum::<f64>() / n;
                let bias = (c.iter().sum::<f64>() - src.iter().sum::<f64>()) / n;
                (10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10(), bias)
            })
        })
        .collect()
}

fn settings(path: &str, engines: ExportEngines) -> ExportSettings {
    ExportSettings {
        format: Format::Hevc,
        path: path.into(),
        hevc_bit_depth: 8,
        bitrate_mode: BitrateMode::Crf,
        crf: 28.0,
        keyframe_distance: Some(24),
        export_engines: engines,
        ..Default::default()
    }
}

/// Per frame (presentation order): pts, sync flag, luma PSNR to `source`.
fn read(path: &str, source: &[f64]) -> (Vec<i64>, Vec<bool>, Vec<f64>) {
    let bytes = std::fs::read(path).unwrap();
    let file = filmcraft_isobmff::open(bytes.clone()).unwrap();
    let vt = file.tracks.iter().position(|t| t.kind == filmcraft_isobmff::TrackKind::Video).unwrap();
    let track = &file.tracks[vt];
    let mut d = filmcraft_codecs::software_video_decoder(&track.entries[0]).unwrap();
    let mut out = Vec::new();
    for (k, s) in track.samples.iter().enumerate() {
        let data = file.read_sample(&bytes, vt, k).unwrap();
        out.extend(d.decode(&data, s.pts).unwrap());
    }
    out.extend(d.flush());
    let psnr = out
        .iter()
        .map(|f| {
            let planar = f.frame.to_planar().unwrap_or_else(|| f.frame.clone());
            let y: Vec<f64> = match &planar.data {
                PixelData::Yuv8 { planes, .. } => planes[0].iter().map(|&v| v as f64).collect(),
                PixelData::Yuv16 { planes, bits, .. } => planes[0].iter().map(|&v| v as f64 / f64::from(1u32 << (bits - 8))).collect(),
                other => panic!("unexpected decoded format {:?}", std::mem::discriminant(other)),
            };
            assert_eq!(y.len(), source.len());
            let mse = y.iter().zip(source).map(|(a, b)| (a - b) * (a - b)).sum::<f64>() / y.len() as f64;
            10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
        })
        .collect();
    let mut pts: Vec<i64> = track.samples.iter().map(|s| s.pts).collect();
    pts.sort_unstable();
    let mut keys = vec![false; track.samples.len()];
    for s in &track.samples {
        if let Ok(i) = pts.binary_search(&s.pts) {
            keys[i] = s.is_sync;
        }
    }
    (pts, keys, psnr)
}

#[test]
fn a_static_shot_shows_no_step_at_the_joins() {
    let _ = filmcraft_platform::register();
    let Some(second) = filmcraft_platform::export_lanes::second_encode_node() else {
        eprintln!("SKIPPED: no second GPU with an HEVC encoder");
        return;
    };
    let (p, seq, src, luma) = project();
    let dir = std::env::temp_dir().join(format!("fc-dual-static-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let one = dir.join("one.mp4").to_string_lossy().into_owned();
    let two = dir.join("two.mp4").to_string_lossy().into_owned();
    if let Err(e) = export(&p, seq, &settings(&one, ExportEngines::Single), &src, &Progress::default()) {
        eprintln!("SKIPPED: no HEVC export here: {e}");
        return;
    }
    export(&p, seq, &settings(&two, ExportEngines::Dual), &src, &Progress::default()).unwrap();
    let report = lanes::last_lanes();
    eprintln!("second node {second}: {report:?}");
    if report.lanes.is_empty() {
        eprintln!("SKIPPED: {}", report.note.unwrap_or_default());
        return;
    }
    // a lane may be retired by the slow-lane rule on a loaded machine, never fail
    assert!(
        report.lanes.iter().all(|l| l.failed.as_deref().is_none_or(|f| f.contains("too slow") || f.contains("last chunk")) && l.frames > 0),
        "both lanes worked: {report:?}"
    );
    let (pts1, keys1, q1) = read(&one, &luma);
    let (pts2, keys2, q2) = read(&two, &luma);
    assert_eq!((q1.len(), q2.len()), (FRAMES, FRAMES), "every frame decodes");
    assert_eq!(pts2, pts1, "the same presentation times");
    assert_eq!(keys2, keys1, "the same keyframes");
    // every chunk starts on a keyframe: check both sides of every keyframe after the first
    let mut worst = 0.0f64;
    for k in (1..FRAMES).filter(|&k| keys2[k]) {
        for i in [k - 1, k] {
            let d = q1[i] - q2[i];
            eprintln!("frame {i}: one GPU {:.2} dB, two GPUs {:.2} dB", q1[i], q2[i]);
            worst = worst.max(d);
        }
    }
    assert!(worst <= 0.5, "a join is {worst:.2} dB worse than the one-GPU export");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_gpus_keep_the_colours_of_one() {
    let _ = filmcraft_platform::register();
    if filmcraft_platform::export_lanes::second_encode_node().is_none() {
        eprintln!("SKIPPED: no second GPU with an HEVC encoder");
        return;
    }
    let (p, seq, src, _) = project();
    let dir = std::env::temp_dir().join(format!("fc-dual-colour-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let one = dir.join("one.mp4").to_string_lossy().into_owned();
    let two = dir.join("two.mp4").to_string_lossy().into_owned();
    if let Err(e) = export(&p, seq, &settings(&one, ExportEngines::Single), &src, &Progress::default()) {
        eprintln!("SKIPPED: no HEVC export here: {e}");
        return;
    }
    export(&p, seq, &settings(&two, ExportEngines::Dual), &src, &Progress::default()).unwrap();
    let report = lanes::last_lanes();
    if report.lanes.is_empty() {
        eprintln!("SKIPPED: {}", report.note.unwrap_or_default());
        return;
    }
    assert!(report.lanes.iter().filter(|l| l.frames > 0).count() == 2, "both lanes encoded frames: {report:?}");
    let source = source_chroma();
    let (c1, c2) = (read_chroma(&one, &source), read_chroma(&two, &source));
    assert_eq!((c1.len(), c2.len()), (FRAMES, FRAMES), "every frame decodes");
    let (mut worst_psnr, mut worst_bias) = (0.0f64, 0.0f64);
    for (a, b) in c1.iter().zip(&c2) {
        for p in 0..2 {
            worst_psnr = worst_psnr.max(a[p].0 - b[p].0);
            worst_bias = worst_bias.max(b[p].1.abs()).max(a[p].1.abs());
        }
    }
    eprintln!("first frame one GPU {:?}, two GPUs {:?}; worst chroma PSNR loss {worst_psnr:.2} dB, worst mean shift {worst_bias:.2}", c1[0], c2[0]);
    assert!(worst_psnr <= 0.5, "a two-GPU frame's chroma is {worst_psnr:.2} dB further from the source than one GPU's");
    assert!(worst_bias <= 1.0, "a frame's mean chroma is {worst_bias:.2} code values off the source's");
    let _ = std::fs::remove_dir_all(&dir);
}
