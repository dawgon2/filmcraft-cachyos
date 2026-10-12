//! Temporal noise reduction in the renderer (fork 6.7 integration): the grade compiles `tnr.*`
//! on the first active node only, neighbours follow the clip (edges, cuts, speed, reverse,
//! holds), only temporal renders blend, the GPU plan and the CPU reference agree, and projects
//! without temporal NR render exactly as before.

use super::*;
use crate::gpufx::{FxOp, NeighbourPicture};
use crate::temporal::{NeighbourTime, neighbour_times};
use crate::tnr::fixture;
use filmcraft_frame::AudioBuffer;
use filmcraft_media::{MediaInfo, MediaKind, MediaSource, VideoStreamInfo};
use filmcraft_project::grade::{Grade, GradeNode};
use filmcraft_project::{Label, MediaClip, MediaRef, Param, SequenceSettings, TrackKind, resolve_auto_points};
use filmcraft_time::{FrameRate, TimeRange};

const FPS: FrameRate = FrameRate::FPS_24;

/// The design's synthetic sequence (moving textured disc, encoded noise) as a 7-frame movie.
struct Fixture {
    info: MediaInfo,
    frames: Vec<Arc<VideoFrame>>,
}

impl MediaSource for Fixture {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        let i = FPS.frame_at(req.time).clamp(0, fixture::FRAMES as i64 - 1) as usize;
        self.frames.get(i).cloned().ok_or(filmcraft_media::MediaError::NoStream("video"))
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        Ok(AudioBuffer::silence(sample_rate, 2, frames))
    }
}

fn info(name: &str) -> MediaInfo {
    MediaInfo {
        name: name.into(),
        kind: MediaKind::Movie,
        duration: FPS.tick_of(fixture::FRAMES as i64),
        video: Some(VideoStreamInfo {
            width: fixture::W as u32,
            height: fixture::H as u32,
            frame_rate: FPS,
            par: (1, 1),
            codec: "test".into(),
            pixel_format: "RGBA float".into(),
            color: filmcraft_color::ColorInfo::SRGB_FULL,
            has_alpha: false,
            bitrate: None,
            hdr: None,
        }),
        audio_streams: Vec::new(),
        container: "test".into(),
        start_timecode: None,
        file_size: None,
    }
}

/// The fixture with noise `sigma` (0: clean). `name` keys the flow cache: one per content.
fn source(name: &str, sigma: f32) -> Fixture {
    let frames = fixture::frames(sigma, 7).into_iter().map(|im| Arc::new(VideoFrame::rgba_f32(im.w as u32, im.h as u32, im.px))).collect();
    Fixture { info: info(name), frames }
}

fn tnr_node(id: u64, params: &[(&str, f64)]) -> GradeNode {
    let mut n = GradeNode::new(id);
    for (k, v) in params {
        n.params.insert((*k).into(), Param::new(ParamValue::Float(*v)));
    }
    n
}

/// A sequence the fixture's size with the clip on V1 at frame 0, all 7 frames; `grade` on it.
fn project(name: &str, sigma: f32, grade: Option<Grade>) -> (Project, ItemId, SourceMap) {
    let mut p = Project::new("tnr");
    let src = source(name, sigma);
    let clip = MediaClip {
        media: MediaRef::File { path: format!("{name}.mov") },
        info: src.info.clone(),
        interpret: Default::default(),
        mark_in: None,
        mark_out: None,
        markers: vec![],
        offline: false,
        proxy: None,
        identity: None,
    };
    let item = p.add_item(name, Label::Iris, ItemKind::Media(clip), None);
    let size = (fixture::W as u32, fixture::H as u32);
    let settings = SequenceSettings { width: size.0, height: size.1, frame_rate: FPS, ..Default::default() };
    let sid = p.new_sequence("s", settings, 1, 0, None);
    let mut ti = p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FPS.tick_of(fixture::FRAMES as i64)), FPS).unwrap();
    for e in &mut ti.effects {
        resolve_auto_points(e, size, size);
    }
    ti.grade = grade;
    p.sequence_mut(sid).unwrap().video_tracks[0].items.push(ti);
    let mut map = SourceMap::default();
    map.0.insert(item, Arc::new(src) as SharedSource);
    (p, sid, map)
}

fn opts(temporal: bool) -> RenderOptions {
    RenderOptions { temporal, ..Default::default() }
}

fn tnr_grade(luma: f64, frames: f64) -> Grade {
    Grade { nodes: vec![tnr_node(1, &[("tnr.luma", luma), ("tnr.chroma", luma), ("tnr.frames", frames)])], bypass: false }
}

/// Mean absolute difference over the colour channels.
fn mad(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.w, a.h), (b.w, b.h));
    let (mut s, mut n) = (0.0f64, 0usize);
    for (p, q) in a.px.as_chunks::<4>().0.iter().zip(b.px.as_chunks::<4>().0) {
        for k in 0..3 {
            s += f64::from((p[k] - q[k]).abs());
            n += 1;
        }
    }
    (s / n.max(1) as f64) as f32
}

fn max_diff(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

// ----------------------------------------------------------------------------- neighbours

/// A bare item for `neighbour_times`: `dur` frames at timeline frame `start`, source from 100.
fn item(start: i64, dur: i64) -> TrackItem {
    let mut p = Project::new("n");
    let it = p.add_item(
        "x",
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: "x.mov".into() },
            info: info("x"),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    p.make_track_item(it, TrackKind::Video, FPS.tick_of(start), TimeRange::new(FPS.tick_of(100), FPS.tick_of(dur)), FPS).unwrap()
}

/// (offset, source frame) of the neighbours at timeline frame `f`.
fn nb(it: &TrackItem, f: i64, radius: u8) -> Vec<(i8, i64)> {
    neighbour_times(it, FPS.tick_of(f), FPS, FPS, radius).iter().map(|n: &NeighbourTime| (n.offset, n.frame)).collect()
}

#[test]
fn neighbours_stay_inside_the_clip_and_follow_its_time() {
    let it = item(10, 20);
    // the middle: ±1 then ±2, source frames around 100 + 5
    assert_eq!(nb(&it, 15, 2), vec![(1, 106), (-1, 104), (2, 107), (-2, 103)]);
    assert_eq!(nb(&it, 15, 1), vec![(1, 106), (-1, 104)]);
    // hostile radius: the maximum
    assert_eq!(nb(&it, 15, 200).len(), crate::tnr::MAX_NEIGHBOURS);
    assert!(nb(&it, 15, 0).is_empty());
    // the clip's first frame: nothing before it (the cut), the last: nothing after
    assert_eq!(nb(&it, 10, 2), vec![(1, 101), (2, 102)]);
    assert_eq!(nb(&it, 11, 2), vec![(1, 102), (-1, 100), (2, 103)]);
    assert_eq!(nb(&it, 29, 2), vec![(-1, 118), (-2, 117)]);
    // a one-frame clip has no neighbours
    assert!(nb(&item(0, 1), 0, 2).is_empty());
    // double speed: every other source frame
    let mut fast = item(0, 20);
    fast.speed = 2.0;
    assert_eq!(nb(&fast, 5, 1), vec![(1, 112), (-1, 108)]);
    // half speed: repeated source frames collapse (dedup), the next distinct frame is ±1
    let mut slow = item(0, 20);
    slow.speed = 0.5;
    let got = nb(&slow, 5, 2);
    let cur = FPS.frame_at(crate::video_source_time(&slow, FPS.tick_of(5), FPS));
    assert!(got.iter().all(|(_, f)| *f != cur), "{got:?} repeats {cur}");
    let mut frames: Vec<i64> = got.iter().map(|g| g.1).collect();
    frames.dedup();
    assert_eq!(frames.len(), got.len());
    assert!(got.len() < 4 && got.iter().all(|(o, _)| o.unsigned_abs() <= 2), "{got:?}");
    // reverse: the next timeline frame is the previous source frame
    let mut rev = item(0, 20);
    rev.reverse = true;
    let r = nb(&rev, 5, 1);
    let cur = FPS.frame_at(crate::video_source_time(&rev, FPS.tick_of(5), FPS));
    assert_eq!(r, vec![(1, cur - 1), (-1, cur + 1)]);
    // a frame hold: one picture, nothing to blend
    let mut hold = item(0, 20);
    hold.frame_hold = Some(FPS.tick_of(103));
    assert!(nb(&hold, 5, 2).is_empty());
    // hostile rates (read as the default rate) and times never panic and stay bounded
    let hostile = FrameRate { num: 0, den: -5 };
    assert!(neighbour_times(&it, FPS.tick_of(15), hostile, hostile, 2).len() <= crate::tnr::MAX_NEIGHBOURS);
    assert!(neighbour_times(&it, Tick(i64::MAX), FPS, FPS, 2).is_empty());
    assert!(neighbour_times(&it, Tick(i64::MIN), FPS, FPS, 2).is_empty());
}

// ---------------------------------------------------------------------------------- grade

#[test]
fn the_grade_compiles_temporal_nr_on_the_first_active_node_only() {
    use crate::grade::{GradeCtx, compile_fx};
    use filmcraft_color::WorkingSpace;
    let fx = |g: &Grade| compile_fx(g, Tick(0), WorkingSpace::Rec709, GradeCtx::default(), 16, 16);
    let tnr = |fx: &[FxOp]| fx.iter().filter(|o| matches!(o, FxOp::TemporalDenoise { .. })).count();
    // first node: before its spatial NR
    let first = tnr_node(1, &[("tnr.luma", 0.5), ("tnr.frames", 2.0), ("nr.luma", 0.3)]);
    let ops = fx(&Grade { nodes: vec![first.clone()], bypass: false });
    assert!(
        matches!(ops.as_slice(), [FxOp::TemporalDenoise { luma: 0.5, chroma: 0.0, radius: 2, motion: 0.5, inputs: None }, FxOp::Denoise { .. }]),
        "{ops:?}"
    );
    // a later node's values are ignored (provisional rule, owner question Q9)
    let later = Grade { nodes: vec![tnr_node(1, &[("saturation", 0.5)]), tnr_node(2, &[("tnr.luma", 1.0)])], bypass: false };
    assert_eq!(tnr(&fx(&later)), 0);
    // ...unless the nodes before it are off: then it is the first active node
    let mut off = later.clone();
    off.nodes[0].enabled = false;
    assert_eq!(tnr(&fx(&off)), 1);
    // bypassed grade: nothing
    assert!(fx(&Grade { bypass: true, ..Grade { nodes: vec![first.clone()], bypass: false } }).is_empty());
    // windows on the node don't limit it: it stays outside the masked step (GPU-capable)
    let mut win = first;
    win.windows.push(filmcraft_project::Mask::new("w", filmcraft_project::MaskPath::rect(0.0, 0.0, 8.0, 8.0)));
    let ops = fx(&Grade { nodes: vec![win], bypass: false });
    assert!(matches!(ops.first(), Some(FxOp::TemporalDenoise { .. })) && matches!(ops.get(1), Some(FxOp::Masked { .. })), "{ops:?}");
    assert!(ops.iter().all(FxOp::gpu_ok));
}

#[test]
fn hostile_temporal_params_are_clamped_or_identity() {
    use crate::grade::temporal_node;
    let op = |ps: &[(&str, f64)]| temporal_node(&Grade { nodes: vec![tnr_node(1, ps)], bypass: false }, Tick(0)).map(|(_, o)| o);
    assert!(op(&[]).is_none(), "missing: identity");
    assert!(op(&[("tnr.luma", f64::NAN), ("tnr.chroma", -3.0)]).is_none());
    assert!(op(&[("tnr.luma", 0.0), ("tnr.frames", 2.0), ("tnr.motion", 1.0)]).is_none(), "zero strengths: identity");
    let Some(FxOp::TemporalDenoise { luma, chroma, radius, motion, .. }) =
        op(&[("tnr.luma", 1e30), ("tnr.chroma", f64::INFINITY), ("tnr.frames", 99.0), ("tnr.motion", f64::NAN)])
    else {
        panic!("an op")
    };
    assert_eq!((luma, chroma, radius, motion), (1.0, 0.0, 2, 0.5));
    for (frames, want) in [(1.4, 1), (1.5, 2), (-7.0, 1), (f64::NEG_INFINITY, 1)] {
        let Some(FxOp::TemporalDenoise { radius, .. }) = op(&[("tnr.luma", 0.2), ("tnr.frames", frames)]) else { panic!("an op") };
        assert_eq!(radius, want, "frames {frames}");
    }
    // a mistyped value
    let mut n = tnr_node(1, &[("tnr.luma", 0.4)]);
    n.params.insert("tnr.frames".into(), Param::new(ParamValue::Text("two".into())));
    let g = Grade { nodes: vec![n], bypass: false };
    assert!(matches!(temporal_node(&g, Tick(0)), Some((_, FxOp::TemporalDenoise { radius: 1, .. }))));
}

// --------------------------------------------------------------------------------- render

#[test]
fn only_temporal_renders_blend_and_they_clean_the_noise() {
    let (p, seq, map) = project("tnr-render", fixture::NOISE, Some(tnr_grade(1.0, 2.0)));
    let (clean_p, clean_seq, clean_map) = project("tnr-render-clean", 0.0, None);
    let t = FPS.tick_of(fixture::MID as i64);
    let clean = render_sequence(&clean_p, clean_seq, t, opts(false), &clean_map);
    let (none_p, none_seq, none_map) = project("tnr-render", fixture::NOISE, None);
    let noisy = render_sequence(&none_p, none_seq, t, opts(false), &none_map);
    // playback / scrubbing: the op is left out, the picture is the clip's own
    let live = render_sequence(&p, seq, t, opts(false), &map);
    assert_eq!(live.px, noisy.px, "no temporal access: no temporal NR");
    // a temporal render blends ±2 and gets closer to the clean picture
    let tnr = render_sequence(&p, seq, t, opts(true), &map);
    let (e0, e1) = (mad(&noisy, &clean), mad(&tnr, &clean));
    assert!(e1 < 0.75 * e0, "error to clean: {e0} → {e1}");
    // the GPU plan carries source-frame neighbours (GPU-capable) and its CPU reference agrees
    let plan = plan::plan_frame(&p, seq, t, opts(true), &map);
    let plan::FramePlan::Layers { layers, .. } = &plan else { panic!("a layer plan") };
    let fx = layers.first().and_then(|l| l.fx.clone()).expect("an effect stage");
    let Some(FxOp::TemporalDenoise { inputs: Some(inp), radius: 2, .. }) = fx.ops.iter().find(|o| matches!(o, FxOp::TemporalDenoise { .. })) else {
        panic!("{:?}", fx.ops)
    };
    assert_eq!(inp.neighbours.len(), 4);
    assert!(inp.neighbours.iter().all(|n| matches!(n.picture, NeighbourPicture::Frame { .. })));
    assert!(fx.ops.iter().all(FxOp::gpu_ok));
    let via_plan = plan::execute_cpu(&plan);
    assert!(max_diff(&via_plan, &tnr) < 2e-3, "plan vs CPU: {}", max_diff(&via_plan, &tnr));
    // a playback plan has no temporal op at all
    let live_plan = plan::plan_frame(&p, seq, t, opts(false), &map);
    let plan::FramePlan::Layers { layers, .. } = &live_plan else { panic!("a layer plan") };
    assert!(layers.iter().filter_map(|l| l.fx.as_ref()).all(|fx| !fx.ops.iter().any(|o| matches!(o, FxOp::TemporalDenoise { .. }))));
    // radius 1: two neighbours
    let (p1, s1, m1) = project("tnr-render", fixture::NOISE, Some(tnr_grade(1.0, 1.0)));
    let plan1 = plan::plan_frame(&p1, s1, t, opts(true), &m1);
    let plan::FramePlan::Layers { layers, .. } = &plan1 else { panic!("a layer plan") };
    let ops = &layers[0].fx.as_ref().expect("fx").ops;
    assert!(matches!(ops.first(), Some(FxOp::TemporalDenoise { inputs: Some(i), .. }) if i.neighbours.len() == 2), "{ops:?}");
}

#[test]
fn flows_are_cached_per_adjacent_pair() {
    // (a media name no other test uses: its own cache keys)
    let (p, seq, map) = project("tnr-cache-hit", fixture::NOISE, Some(tnr_grade(0.8, 2.0)));
    let t = FPS.tick_of(fixture::MID as i64);
    let it = &p.sequence(seq).unwrap().video_tracks[0].items[0];
    let src = map.0.values().next().unwrap().clone();
    let media = crate::temporal::media_key(&p, it, src.as_ref());
    let size = (fixture::W as u32, fixture::H as u32);
    let mid = fixture::MID as i64;
    let first = crate::temporal::pairs_estimated();
    let _ = plan::plan_frame(&p, seq, t, opts(true), &map);
    // ±1 and ±2 by chaining: four adjacent pairs estimated once (on this thread), all cached
    assert_eq!(crate::temporal::pairs_estimated() - first, 4);
    for (a, b) in [(mid, mid + 1), (mid + 1, mid + 2), (mid, mid - 1), (mid - 1, mid - 2)] {
        assert!(crate::temporal::pair_cached(media, a, b, size), "{a}–{b}");
    }
    assert!(!crate::temporal::pair_cached(media, mid, mid + 2, size), "±2 is chained, not estimated");
    // the same frame again, and the next frame: only the one new pair is estimated
    let before = crate::temporal::pairs_estimated();
    let _ = plan::plan_frame(&p, seq, t, opts(true), &map);
    assert_eq!(crate::temporal::pairs_estimated(), before, "the same frame: every pair from the cache");
    let _ = plan::plan_frame(&p, seq, FPS.tick_of(mid + 1), opts(true), &map);
    assert_eq!(crate::temporal::pairs_estimated() - before, 1, "the next frame: one new pair");
}

#[test]
fn edges_and_holds_render_without_temporal_neighbours_where_there_are_none() {
    let (p, seq, map) = project("tnr-edges", fixture::NOISE, Some(tnr_grade(1.0, 2.0)));
    // the first and last frames: one side only, still a valid picture
    for f in [0, fixture::FRAMES as i64 - 1] {
        let img = render_sequence(&p, seq, FPS.tick_of(f), opts(true), &map);
        assert!(img.px.iter().all(|v| v.is_finite()));
        let plain = render_sequence(&p, seq, FPS.tick_of(f), opts(false), &map);
        assert!(mad(&img, &plain) > 0.0, "frame {f}: the neighbours on one side still blend");
    }
    // a frame hold: no neighbour, the temporal render is the plain one
    let (mut hp, hseq, hmap) = project("tnr-edges", fixture::NOISE, Some(tnr_grade(1.0, 2.0)));
    hp.sequence_mut(hseq).unwrap().video_tracks[0].items[0].frame_hold = Some(FPS.tick_of(3));
    let t = FPS.tick_of(2);
    assert_eq!(render_sequence(&hp, hseq, t, opts(true), &hmap).px, render_sequence(&hp, hseq, t, opts(false), &hmap).px);
}

#[test]
fn projects_without_temporal_nr_render_the_same_with_temporal_access() {
    // no grade, a grade without tnr keys, and one with tnr frames / motion but zero strengths:
    // a temporal render (export, the paused frame) is bit-identical to a live one
    let grades = [
        None,
        Some(Grade { nodes: vec![tnr_node(1, &[("saturation", 0.7), ("nr.luma", 0.2)])], bypass: false }),
        Some(Grade { nodes: vec![tnr_node(1, &[("tnr.frames", 2.0), ("tnr.motion", 0.9), ("lift", 0.0)])], bypass: false }),
    ];
    let t = FPS.tick_of(3);
    for g in grades {
        let (p, seq, map) = project("tnr-old", fixture::NOISE, g.clone());
        let live = render_sequence(&p, seq, t, opts(false), &map);
        assert_eq!(render_sequence(&p, seq, t, opts(true), &map).px, live.px, "{g:?}");
        let a = plan::execute_cpu(&plan::plan_frame(&p, seq, t, opts(false), &map));
        let b = plan::execute_cpu(&plan::plan_frame(&p, seq, t, opts(true), &map));
        assert_eq!(a.px, b.px, "{g:?}");
    }
    // and an old project's JSON (no tnr keys) loads to the same grade
    let (p, _, _) = project("tnr-old", fixture::NOISE, Some(Grade { nodes: vec![tnr_node(1, &[("saturation", 0.7)])], bypass: false }));
    let back = Project::from_json(&p.to_json()).unwrap();
    assert!(!p.to_json().contains("tnr."));
    assert_eq!(back.to_json(), p.to_json());
}

#[test]
fn the_program_monitor_asks_for_temporal_frames_only_where_a_clip_uses_it() {
    let (p, seq, _) = project("tnr-at", fixture::NOISE, Some(tnr_grade(0.5, 1.0)));
    assert!(crate::grade::temporal_at(&p, seq, FPS.tick_of(2)));
    assert!(!crate::grade::temporal_at(&p, seq, FPS.tick_of(50)), "past the clip");
    let (q, qseq, _) = project("tnr-at", fixture::NOISE, Some(tnr_grade(0.0, 2.0)));
    assert!(!crate::grade::temporal_at(&q, qseq, FPS.tick_of(2)));
    assert!(!crate::grade::temporal_at(&q, ItemId(u64::MAX), FPS.tick_of(2)), "no such sequence");
    let it = &p.sequence(seq).unwrap().video_tracks[0].items[0];
    assert!(crate::grade::ever_temporal(&p, it));
}

/// A clip whose grade has only `tnr.*` keeps the plain fast path without temporal access
/// (playback, a reduced-resolution monitor): no effect stage, the same pixels as no grade.
#[test]
fn a_temporal_only_grade_keeps_the_plain_path_without_temporal_access() {
    let (p, seq, map) = project("tnr-plain", fixture::NOISE, Some(tnr_grade(1.0, 2.0)));
    let (q, qseq, qmap) = project("tnr-plain", fixture::NOISE, None);
    let it = &p.sequence(seq).unwrap().video_tracks[0].items[0];
    assert!(!crate::grade::changes_picture(&p, it, Tick(0), false));
    assert!(crate::grade::changes_picture(&p, it, Tick(0), true));
    let t = FPS.tick_of(3);
    for scale in [1.0, 0.5] {
        let o = RenderOptions { scale, ..Default::default() };
        let plan = plan::plan_frame(&p, seq, t, o, &map);
        let plan::FramePlan::Layers { layers, .. } = &plan else { panic!("a layer plan") };
        assert!(layers.iter().all(|l| l.fx.is_none()), "scale {scale}: drawn directly");
        let plain = plan::execute_cpu(&plan::plan_frame(&q, qseq, t, o, &qmap));
        assert_eq!(plan::execute_cpu(&plan).px, plain.px, "scale {scale}");
        assert_eq!(render_sequence(&p, seq, t, o, &map).px, render_sequence(&q, qseq, t, o, &qmap).px, "scale {scale}");
    }
}

/// During a dissolve the outgoing clip shows its handle frames after its own end (and the
/// incoming one before its start): those frames have neighbours in the transition's span.
#[test]
fn neighbours_continue_through_a_dissolve() {
    use crate::temporal::{clip_bounds, neighbour_times_in};
    let (mut p, seq, map) = project("tnr-dissolve", fixture::NOISE, Some(tnr_grade(1.0, 2.0)));
    let q = p.sequence_mut(seq).unwrap();
    // A: timeline 0..4 (source 0..4), B: the same media at 4..7 (source 4..7), dissolve 2..6
    let mut a = q.video_tracks[0].items[0].clone();
    a.duration = FPS.tick_of(4);
    let mut b = a.clone();
    b.id = filmcraft_project::ClipId(a.id.0.wrapping_add(1));
    b.start = FPS.tick_of(4);
    b.source_in = FPS.tick_of(4);
    b.duration = FPS.tick_of(3);
    q.video_tracks[0].items = vec![a.clone(), b.clone()];
    q.video_tracks[0].transitions.push(filmcraft_project::Transition {
        id: Default::default(),
        effect: filmcraft_project::find_effect("cross_dissolve").unwrap().instance(),
        start: FPS.tick_of(2),
        duration: FPS.tick_of(4),
        from: Some(a.id),
        to: Some(b.id),
        align: Default::default(),
        reverse: false,
    });
    let q = p.sequence(seq).unwrap();
    let t = FPS.tick_of(5);
    // A after its end: no neighbours by its own range, ±2 within the dissolve's
    assert!(neighbour_times(&a, t, FPS, FPS, 2).is_empty());
    assert_eq!(clip_bounds(q, &a, t), (FPS.tick_of(0), FPS.tick_of(6)));
    let fa: Vec<i64> = neighbour_times_in(&a, clip_bounds(q, &a, t), t, FPS, FPS, 2).iter().map(|n| n.frame).collect();
    assert_eq!(fa, vec![4, 3], "A's handle at 5: 4 and 3 before it, nothing past the dissolve");
    // B before its start, at 3: 4 and 5 after, 2 before (the dissolve starts at 2)
    let tb = FPS.tick_of(3);
    let fb: Vec<i64> = neighbour_times_in(&b, clip_bounds(q, &b, tb), tb, FPS, FPS, 2).iter().map(|n| n.frame).collect();
    assert_eq!(fb, vec![4, 2, 5]);
    // outside the dissolve the clip's own range bounds them
    assert_eq!(clip_bounds(q, &a, FPS.tick_of(1)), (a.start, a.end()));
    // and the temporal render inside the dissolve blends (differs from the live one)
    let live = render_sequence(&p, seq, t, opts(false), &map);
    let tnr = render_sequence(&p, seq, t, opts(true), &map);
    assert!(mad(&live, &tnr) > 0.0);
    let via_plan = plan::execute_cpu(&plan::plan_frame(&p, seq, t, opts(true), &map));
    assert!(max_diff(&via_plan, &tnr) < 2e-3, "plan vs CPU: {}", max_diff(&via_plan, &tnr));
}

/// The flow cache's media key changes with the file on disk (re-exported in place: same name,
/// metadata and project item, another modification time) and with the content identity.
#[test]
fn the_flow_cache_key_follows_the_file_and_its_identity() {
    let dir = std::env::temp_dir().join(format!("filmcraft-tnr-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip.mov");
    std::fs::write(&path, b"one").unwrap();
    let (mut p, seq, map) = project("tnr-key", fixture::NOISE, None);
    let id = *map.0.keys().next().unwrap();
    let src = map.0.get(&id).unwrap().clone();
    let set = |p: &mut Project, f: &dyn Fn(&mut MediaClip)| {
        if let Some(ItemKind::Media(m)) = p.item_mut(id).map(|i| &mut i.kind) {
            f(m);
        }
    };
    set(&mut p, &|m| m.media = MediaRef::File { path: path.to_string_lossy().into() });
    let key = |p: &Project| crate::temporal::media_key(p, &p.sequence(seq).unwrap().video_tracks[0].items[0], src.as_ref());
    let k1 = key(&p);
    assert_eq!(k1, key(&p), "stable");
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000)).unwrap();
    drop(file);
    let k2 = key(&p);
    assert_ne!(k1, k2, "another modification time: no reuse");
    set(&mut p, &|m| m.identity = Some(filmcraft_project::MediaIdentity { size: 3, fingerprint: 7 }));
    assert_ne!(k2, key(&p), "another content identity: no reuse");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A retimed clip with Frame Blending / Optical Flow: interpolated frames don't run temporal NR
/// (the flows are between source frames), and interpolated neighbours are left out.
#[test]
fn interpolated_frames_skip_temporal_nr() {
    let (mut p, seq, map) = project("tnr-blend", fixture::NOISE, Some(tnr_grade(1.0, 1.0)));
    {
        let it = &mut p.sequence_mut(seq).unwrap().video_tracks[0].items[0];
        it.speed = 0.5;
        it.time_interpolation = filmcraft_project::TimeInterpolation::FrameBlending;
    }
    // frame 3 at half speed is source 1.5: blended, so the temporal render is the live one
    let t = FPS.tick_of(3);
    assert!(crate::interpolation_blend(&p.sequence(seq).unwrap().video_tracks[0].items[0], t, FPS).is_some());
    assert_eq!(render_sequence(&p, seq, t, opts(true), &map).px, render_sequence(&p, seq, t, opts(false), &map).px);
}
