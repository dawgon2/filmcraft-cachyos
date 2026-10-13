//! Two-GPU HEVC export (fork 6E.3 S7): one sequence exported on the primary GPU and a second
//! render node at once, chunks joined into one file. The file must have every frame once, the
//! same keyframes as the one-GPU export, a sample entry the lanes agree on (`hvc1`) or `hev1`,
//! and decode from start to end with one decoder. Skips without two VA-API HEVC encoders.
#![cfg(target_os = "linux")]

use std::sync::Arc;

use filmcraft_export::{BitrateMode, ExportEngines, ExportSettings, Format, Progress, export, lanes};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource, SharedSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_render::{SourceMap, SourceProvider};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

/// A provider that can give each lane its own copy (the generators are stateless).
struct Lanes(Arc<SourceMap>);

impl SourceProvider for Lanes {
    fn source(&self, item: ItemId) -> Option<SharedSource> {
        self.0.source(item)
    }
    fn lane_copy(&self) -> Option<Arc<dyn SourceProvider + Send + Sync>> {
        Some(Arc::new(Lanes(self.0.clone())))
    }
}

/// Three 2 s colour mattes (cuts at 2 s and 4 s) over a tone, 24 fps, 320 x 180.
fn project() -> (Arc<Project>, ItemId, Lanes) {
    let mut p = Project::new("dual");
    let r = FrameRate::FPS_24;
    let len = Tick(2 * TICKS_PER_SECOND);
    let add = |p: &mut Project, g: GeneratorSource, m: &mut SourceMap| {
        let info = g.info().clone();
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
        m.0.insert(id, Arc::new(g));
        id
    };
    let mut m = SourceMap::default();
    let seq = p.new_sequence("s", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
    for (k, color) in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]].into_iter().enumerate() {
        let id = add(&mut p, GeneratorSource::new(Generator::ColorMatte { color }, 320, 180, r, len), &mut m);
        let at = Tick(len.0 * k as i64);
        let v = p.make_track_item(id, TrackKind::Video, at, TimeRange::new(Tick::ZERO, len), r).unwrap();
        p.sequence_mut(seq).unwrap().video_tracks[0].items.push(v);
    }
    let tone = add(&mut p, GeneratorSource::new(Generator::Tone { hz: 440.0, db: -6.0 }, 320, 180, r, Tick(6 * TICKS_PER_SECOND)), &mut m);
    let a = p.make_track_item(tone, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, Tick(6 * TICKS_PER_SECOND)), r).unwrap();
    p.sequence_mut(seq).unwrap().audio_tracks[0].items.push(a);
    (Arc::new(p), seq, Lanes(Arc::new(m)))
}

fn settings(path: &str, engines: ExportEngines) -> ExportSettings {
    ExportSettings {
        format: Format::Hevc,
        path: path.into(),
        hevc_bit_depth: 8,
        bitrate_mode: BitrateMode::Crf,
        crf: 24.0,
        keyframe_distance: Some(24),
        export_engines: engines,
        ..Default::default()
    }
}

/// Video samples (sync flags) and the entry's fourcc; every sample decoded with one decoder.
fn read(path: &str) -> (Vec<bool>, String, usize, usize) {
    let bytes = std::fs::read(path).unwrap();
    let file = filmcraft_isobmff::open(bytes.clone()).unwrap();
    let vt = file.tracks.iter().position(|t| t.kind == filmcraft_isobmff::TrackKind::Video).unwrap();
    let track = &file.tracks[vt];
    let entry = track.entries[0].clone();
    let mut d = filmcraft_codecs::software_video_decoder(&entry).unwrap();
    let mut decoded = 0;
    for (k, s) in track.samples.iter().enumerate() {
        let data = file.read_sample(&bytes, vt, k).unwrap();
        decoded += d.decode(&data, s.pts).unwrap().len();
    }
    decoded += d.flush().len();
    let audio = file.tracks.iter().filter(|t| t.kind == filmcraft_isobmff::TrackKind::Audio).map(|t| t.samples.len()).sum();
    (track.samples.iter().map(|s| s.is_sync).collect(), String::from_utf8_lossy(&entry.format.0).into_owned(), decoded, audio)
}

#[test]
fn a_sequence_exported_on_two_gpus_joins_into_one_file() {
    let _ = filmcraft_platform::register();
    let Some(second) = filmcraft_platform::export_lanes::second_encode_node() else {
        eprintln!("SKIPPED: no second GPU with an HEVC encoder");
        return;
    };
    let (p, seq, src) = project();
    let dir = std::env::temp_dir().join(format!("fc-dual-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let one = dir.join("one.mp4").to_string_lossy().into_owned();
    let two = dir.join("two.mp4").to_string_lossy().into_owned();
    let r1 = match export(&p, seq, &settings(&one, ExportEngines::Single), &src, &Progress::default()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("SKIPPED: no HEVC export here: {e}");
            return;
        }
    };
    assert!(lanes::last_lanes().lanes.is_empty(), "single stays on one GPU");
    let progress = Progress::default();
    let r2 = export(&p, seq, &settings(&two, ExportEngines::Dual), &src, &progress).unwrap();
    let report = lanes::last_lanes();
    eprintln!("second node {second}: {report:?}");
    if report.lanes.is_empty() {
        let note = report.note.unwrap_or_default();
        assert!(note.contains("memory"), "two GPUs were expected: {note}");
        eprintln!("SKIPPED: {note}");
        return;
    }
    assert_eq!((r1.frames, r2.frames), (144, 144));
    assert_eq!(report.lanes.len(), 2);
    assert_eq!(report.lanes.iter().map(|l| l.frames).sum::<u64>(), 144, "every frame encoded once: {report:?}");
    // a lane may be retired by the slow-lane rule on a loaded machine, never fail
    assert!(report.lanes.iter().all(|l| l.failed.as_deref().is_none_or(|f| f.contains("too slow") || f.contains("last chunk"))), "{report:?}");
    assert_eq!(progress.done.load(std::sync::atomic::Ordering::Relaxed), 144);
    let spill_dirs = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".chunks")).count();
    assert_eq!(spill_dirs, 0, "the spill directory is removed");
    let (keys1, entry1, n1, a1) = read(&one);
    let (keys2, entry2, n2, a2) = read(&two);
    assert_eq!((n1, n2), (144, 144), "every frame decodes");
    assert_eq!(keys2, keys1, "the same keyframes as the one-GPU export");
    assert_eq!(entry1, "hvc1");
    assert_eq!(Some(entry2.clone()), report.sample_entry, "{report:?}");
    assert!(entry2 == "hvc1" || entry2 == "hev1");
    assert_eq!(a1, a2, "the same audio samples");
    let _ = std::fs::remove_dir_all(&dir);
}
