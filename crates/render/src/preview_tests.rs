//! Render-bar segments: splitting, hashing, invalidation and cost classification.

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{DemoScene, Generator, MediaSource};
use filmcraft_project::{ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, ParamValue, Project, SequenceSettings, TrackKind, find_effect};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

use crate::preview::{Need, Segment, effect_cost_ms, segment_at, video_segments};

fn add_gen(p: &mut Project, g: GeneratorSource) -> ItemId {
    let info = g.info().clone();
    p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(g.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    )
}

fn setup() -> (Project, ItemId, ItemId, ItemId) {
    let mut p = Project::new("r");
    let matte = add_gen(
        &mut p,
        GeneratorSource::new(Generator::ColorMatte { color: [1.0, 0.0, 0.0, 1.0] }, 1920, 1080, FrameRate::FPS_24, Tick(60 * TICKS_PER_SECOND)),
    );
    let ocean = add_gen(&mut p, GeneratorSource::demo(DemoScene::OceanSunset));
    let seq = p.new_sequence("s", SequenceSettings { width: 1920, height: 1080, frame_rate: FrameRate::FPS_24, ..Default::default() }, 3, 1, None);
    (p, matte, ocean, seq)
}

fn place(p: &mut Project, seq: ItemId, track: usize, item: ItemId, start_f: i64, dur_f: i64) -> ClipId {
    let r = FrameRate::FPS_24;
    let mut ti = p.make_track_item(item, TrackKind::Video, r.tick_of(start_f), TimeRange::new(Tick::ZERO, r.tick_of(dur_f)), r).unwrap();
    for e in ti.effects.iter_mut() {
        filmcraft_project::resolve_auto_points(e, (1920, 1080), (1920, 1080));
    }
    let id = ti.id;
    let t = &mut p.sequence_mut(seq).unwrap().video_tracks[track];
    t.items.push(ti);
    t.sort();
    id
}

fn add_fx(p: &mut Project, seq: ItemId, clip: ClipId, id: &str) {
    let mut inst = find_effect(id).unwrap().instance();
    filmcraft_project::resolve_auto_points(&mut inst, (1920, 1080), (1920, 1080));
    p.sequence_mut(seq).unwrap().find_item_mut(clip).unwrap().1.effects.push(inst);
}

fn bounds(s: &[Segment]) -> Vec<(i64, i64)> {
    s.iter().map(|s| (s.first_frame, s.first_frame + s.frames)).collect()
}

fn by_frame(s: &[Segment], f: i64) -> String {
    segment_at(s, f).unwrap().hash.clone()
}

#[test]
fn segments_split_at_every_edit_point_and_skip_gaps() {
    let (mut p, matte, ocean, seq) = setup();
    place(&mut p, seq, 0, ocean, 0, 48);
    place(&mut p, seq, 0, matte, 48, 24);
    // gap 72..96 on every track
    place(&mut p, seq, 0, ocean, 96, 24);
    place(&mut p, seq, 1, matte, 24, 48); // spans the V1 cut at 48
    let s = video_segments(&p, seq);
    assert_eq!(bounds(&s), vec![(0, 24), (24, 48), (48, 72), (96, 120)]);
    assert_eq!(s[1].clips.len(), 2, "two visible layers in 24..48");
    assert!(segment_at(&s, 80).is_none(), "gap has no segment");
    assert_eq!(segment_at(&s, 30).unwrap().first_frame, 24);
    // segments are frame-aligned ranges
    let r = FrameRate::FPS_24;
    for seg in &s {
        assert_eq!(seg.start, r.tick_of(seg.first_frame));
        assert_eq!(seg.end, r.tick_of(seg.first_frame + seg.frames));
    }
}

#[test]
fn transitions_are_their_own_segments() {
    let (mut p, matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    let b = place(&mut p, seq, 0, matte, 48, 48);
    let r = FrameRate::FPS_24;
    let tr = filmcraft_project::Transition {
        id: Default::default(),
        effect: find_effect("cross_dissolve").unwrap().instance(),
        start: r.tick_of(36),
        duration: r.tick_of(24),
        from: Some(a),
        to: Some(b),
        align: Default::default(),
        reverse: false,
    };
    p.sequence_mut(seq).unwrap().video_tracks[0].transitions.push(tr);
    let s = video_segments(&p, seq);
    assert_eq!(bounds(&s), vec![(0, 36), (36, 48), (48, 60), (60, 96)]);
    assert_eq!(s[1].clips, vec![a, b]);
    assert_eq!(s[1].need, Need::Realtime);
}

#[test]
fn editing_one_clip_invalidates_only_its_segments() {
    let (mut p, matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    let b = place(&mut p, seq, 0, matte, 48, 48);
    add_fx(&mut p, seq, a, "gaussian_blur");
    add_fx(&mut p, seq, b, "tint");
    let before = video_segments(&p, seq);
    // change a parameter of A's effect
    let q = p.sequence_mut(seq).unwrap();
    let (_, it) = q.find_item_mut(a).unwrap();
    it.effect_mut("gaussian_blur").unwrap().params.values_mut().next().unwrap().value = ParamValue::Float(42.0);
    let after = video_segments(&p, seq);
    assert_ne!(by_frame(&before, 10), by_frame(&after, 10), "A changed");
    assert_eq!(by_frame(&before, 60), by_frame(&after, 60), "B untouched");
    // a keyframe on B changes B only
    let q = p.sequence_mut(seq).unwrap();
    let (_, it) = q.find_item_mut(b).unwrap();
    it.effect_mut("opacity").unwrap().params.get_mut("opacity").unwrap().toggle_animation(Tick(1000));
    let kf = video_segments(&p, seq);
    assert_eq!(by_frame(&after, 10), by_frame(&kf, 10));
    assert_ne!(by_frame(&after, 60), by_frame(&kf, 60));
    // sequence settings affect everything
    p.sequence_mut(seq).unwrap().settings.max_render_quality = true;
    let st = video_segments(&p, seq);
    assert_ne!(by_frame(&kf, 10), by_frame(&st, 10));
    assert_ne!(by_frame(&kf, 60), by_frame(&st, 60));
}

#[test]
fn renaming_relabelling_and_moving_keep_hashes() {
    let (mut p, matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    let b = place(&mut p, seq, 0, matte, 48, 48);
    add_fx(&mut p, seq, a, "tint");
    let before = video_segments(&p, seq);
    let q = p.sequence_mut(seq).unwrap();
    for it in q.video_tracks[0].items.iter_mut() {
        it.name = "renamed".into();
        it.label = Label::Mango;
        // ripple both clips 10 frames later
        it.start += FrameRate::FPS_24.tick_of(10);
    }
    let after = video_segments(&p, seq);
    assert_eq!(by_frame(&before, 0), by_frame(&after, 10), "moved segment keeps its preview");
    assert_eq!(by_frame(&before, 50), by_frame(&after, 60));
    let _ = b;
    // trimming B changes B's segment only (its frame count changed)
    let q = p.sequence_mut(seq).unwrap();
    q.find_item_mut(b).unwrap().1.duration = FrameRate::FPS_24.tick_of(40);
    let trimmed = video_segments(&p, seq);
    assert_eq!(by_frame(&after, 10), by_frame(&trimmed, 10));
    assert_ne!(by_frame(&after, 60), by_frame(&trimmed, 60));
}

#[test]
fn hashes_are_stable_across_save_and_load() {
    let (mut p, matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    let b = place(&mut p, seq, 1, matte, 24, 48);
    add_fx(&mut p, seq, a, "lumetri");
    add_fx(&mut p, seq, b, "gaussian_blur");
    let q = p.sequence_mut(seq).unwrap();
    let (_, it) = q.find_item_mut(b).unwrap();
    let m = it.effect_mut("motion").unwrap();
    m.params.get_mut("scale").unwrap().toggle_animation(Tick(0));
    m.params.get_mut("scale").unwrap().set_at(Tick(TICKS_PER_SECOND), ParamValue::Float(80.0));
    let before = video_segments(&p, seq);
    let loaded = Project::from_json(&p.to_json()).unwrap();
    let after = video_segments(&loaded, seq);
    assert_eq!(before, after);
    // and identical across repeated computation
    assert_eq!(video_segments(&p, seq), before);
}

#[test]
fn need_classification_none_yellow_red() {
    let (mut p, matte, _ocean, seq) = setup();
    let a = place(&mut p, seq, 0, matte, 0, 24);
    let b = place(&mut p, seq, 0, matte, 24, 24);
    let c = place(&mut p, seq, 0, matte, 48, 24);
    add_fx(&mut p, seq, b, "tint");
    for fx in ["gaussian_blur", "sharpen", "lumetri", "noise"] {
        add_fx(&mut p, seq, c, fx);
    }
    let s = video_segments(&p, seq);
    let need = |f| segment_at(&s, f).unwrap().need;
    assert_eq!(need(0), Need::None, "untouched clip matching the sequence plays natively");
    assert_eq!(need(30), Need::Realtime, "one cheap effect: yellow");
    assert_eq!(need(60), Need::Render, "four heavy effects: red ({} ms)", segment_at(&s, 60).unwrap().cost_ms);
    let _ = a;
    // a clip at a different frame size than the sequence needs scaling: yellow
    let (mut p, _m, ocean, seq) = setup();
    place(&mut p, seq, 0, ocean, 0, 24);
    let s = video_segments(&p, seq);
    let ocean_matches = p.item(ocean).unwrap().as_media().unwrap().info.video.as_ref().is_some_and(|v| v.width == 1920 && v.height == 1080);
    let want = if ocean_matches && p.item(ocean).unwrap().frame_rate() == FrameRate::FPS_24 { Need::None } else { Need::Realtime };
    assert_eq!(s[0].need, want);
}

#[test]
fn effect_tiers_are_ordered() {
    assert!(effect_cost_ms("lumetri") > effect_cost_ms("tint"));
    assert!(effect_cost_ms("tint") > effect_cost_ms("crop"));
    assert_eq!(effect_cost_ms("gaussian_blur"), effect_cost_ms("lumetri"));
}

/// Times every standard video effect on a 1080p frame (run with `--release --ignored --nocapture`)
/// to calibrate [`effect_cost_ms`].
#[test]
#[ignore]
fn effect_costs() {
    use crate::effects::{FxCtx, apply};
    let mut rows = Vec::new();
    for d in filmcraft_project::effect_defs().iter().filter(|d| d.kind == filmcraft_project::EffectKind::Video && !d.intrinsic) {
        let mut img = crate::Image::filled(1920, 1080, [0.4, 0.5, 0.6, 1.0]);
        let e = d.instance();
        let cx = FxCtx {
            t: Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "00:00:00:00",
            clip_name: "x",
            project: None,
            env: None,
            working: filmcraft_color::WorkingSpace::Rec709,
        };
        apply(&mut img, &e, &cx);
        let t0 = std::time::Instant::now();
        for _ in 0..3 {
            apply(&mut img, &e, &cx);
        }
        rows.push((t0.elapsed().as_secs_f64() * 1000.0 / 3.0, d.id));
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (ms, id) in rows {
        println!("{ms:8.2} ms  {id:24} tier {:.0}", effect_cost_ms(id));
    }
}

fn media_mut(p: &mut Project, id: ItemId) -> &mut MediaClip {
    match &mut p.item_mut(id).unwrap().kind {
        ItemKind::Media(m) => m,
        _ => panic!("not media"),
    }
}

#[test]
fn attaching_or_detaching_a_proxy_keeps_hashes() {
    let (mut p, matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    place(&mut p, seq, 1, matte, 24, 48);
    add_fx(&mut p, seq, a, "lumetri");
    let before = video_segments(&p, seq);
    media_mut(&mut p, ocean).proxy = Some(MediaRef::File { path: "/proxies/ocean_Proxy.0123456789abcdef.mov".into() });
    media_mut(&mut p, matte).proxy = Some(MediaRef::File { path: "/proxies/matte_Proxy.mov".into() });
    assert_eq!(before, video_segments(&p, seq), "attaching a proxy keeps every preview");
    media_mut(&mut p, ocean).proxy = None;
    assert_eq!(before, video_segments(&p, seq), "detaching too");
    // and across save / load with a proxy attached
    let loaded = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(before, video_segments(&loaded, seq));
}

#[test]
fn grade_output_size_and_source_identity_change_hashes() {
    let (mut p, _matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    let before = by_frame(&video_segments(&p, seq), 10);
    // a grade on the clip
    p.sequence_mut(seq).unwrap().find_item_mut(a).unwrap().1.grade = Some(filmcraft_project::grade::Grade { bypass: true, ..Default::default() });
    assert_ne!(before, by_frame(&video_segments(&p, seq), 10), "a grade changes the hash");
    p.sequence_mut(seq).unwrap().find_item_mut(a).unwrap().1.grade = None;
    assert_eq!(before, by_frame(&video_segments(&p, seq), 10), "removing it restores the hash");
    // output (sequence) size
    p.sequence_mut(seq).unwrap().settings.width = 1280;
    p.sequence_mut(seq).unwrap().settings.height = 720;
    assert_ne!(before, by_frame(&video_segments(&p, seq), 10), "output size changes the hash");
    p.sequence_mut(seq).unwrap().settings.width = 1920;
    p.sequence_mut(seq).unwrap().settings.height = 1080;
    assert_eq!(before, by_frame(&video_segments(&p, seq), 10));
    // source identity (a different file behind the same media item)
    media_mut(&mut p, ocean).identity = Some(filmcraft_project::MediaIdentity { size: 1000, fingerprint: 0x1234 });
    let id1 = by_frame(&video_segments(&p, seq), 10);
    assert_ne!(before, id1, "identity changes the hash");
    media_mut(&mut p, ocean).identity = Some(filmcraft_project::MediaIdentity { size: 1000, fingerprint: 0x5678 });
    assert_ne!(id1, by_frame(&video_segments(&p, seq), 10), "a different fingerprint changes the hash");
}

/// Temporal noise reduction (fork 6.7, a clip setting since 6.7b) renders only in the cache, at
/// export and on the paused frame: every edit of the clip's `temporal_nr` changes the segment hash
/// (the cached part is re-rendered) and a clip with it needs rendering.
#[test]
fn temporal_nr_edits_change_the_hash_and_need_rendering() {
    use filmcraft_project::{Param, TemporalNr};
    let (mut p, _matte, ocean, seq) = setup();
    let a = place(&mut p, seq, 0, ocean, 0, 48);
    let set = |p: &mut Project, params: &[(&str, f64)]| {
        let mut n = TemporalNr::default();
        for (k, v) in params {
            n.params.insert((*k).into(), Param::new(ParamValue::Float(*v)));
        }
        p.sequence_mut(seq).unwrap().find_item_mut(a).unwrap().1.temporal_nr = (!params.is_empty()).then_some(n);
        let s = video_segments(p, seq);
        let seg = segment_at(&s, 10).unwrap();
        (seg.hash.clone(), seg.need)
    };
    let (none, need0) = set(&mut p, &[]);
    let (luma, need1) = set(&mut p, &[("luma", 0.5)]);
    let (chroma, _) = set(&mut p, &[("luma", 0.5), ("chroma", 0.5)]);
    let (frames, _) = set(&mut p, &[("luma", 0.5), ("chroma", 0.5), ("frames", 2.0)]);
    let (motion, _) = set(&mut p, &[("luma", 0.5), ("chroma", 0.5), ("frames", 2.0), ("motion", 0.9)]);
    let all = [&none, &luma, &chroma, &frames, &motion];
    for (i, x) in all.iter().enumerate() {
        for y in all.iter().skip(i + 1) {
            assert_ne!(x, y, "every tnr edit changes the hash");
        }
    }
    assert_ne!(need0, Need::Render);
    assert_eq!(need1, Need::Render, "temporal NR never plays live: the segment needs rendering");
    // back to no temporal NR: the hash and the need return
    assert_eq!(set(&mut p, &[]), (none, need0));
}
