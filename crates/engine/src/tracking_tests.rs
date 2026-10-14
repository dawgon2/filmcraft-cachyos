//! `tracking.cornerPin` (fork Y3.3b): a textured plane moving in perspective on V1, a clip on V2
//! whose Corner Pin follows it.

use crate::Session;
use filmcraft_geom::Vec2;
use filmcraft_project::ClipId;
use filmcraft_render::track::Homography;
use filmcraft_time::{FrameRate, Tick};
use serde_json::json;

const RATE: FrameRate = FrameRate::FPS_24;
const W: usize = 320;
const H: usize = 240;

fn texture(u: f64, v: f64) -> f64 {
    let s = (u * 0.21).sin() * (v * 0.17).cos() + 0.5 * ((u + v) * 0.43).sin() + 0.35 * ((u * 0.9 - v * 0.6).sin() * (v * 0.75).cos());
    let (iu, iv) = ((u / 6.0).floor() as i64, (v / 6.0).floor() as i64);
    let mut x = (iu.wrapping_mul(73_856_093) ^ iv.wrapping_mul(19_349_663)) as u64;
    x ^= x >> 13;
    x = x.wrapping_mul(0x5bd1_e995);
    let h = (x >> 40) as f64 / (1u64 << 24) as f64;
    (0.5 + 0.22 * s + 0.12 * h).clamp(0.0, 1.0)
}

/// The plane at frame `k`: object coordinates (a 160 × 160 square around 0) → frame pixels.
fn truth(k: f64) -> Homography {
    let a = filmcraft_geom::Affine::motion(Vec2::new(160.0 + 2.5 * k, 120.0 - 1.2 * k), Vec2::new(1.0 + 0.01 * k, 1.0), 1.5 * k, Vec2::ZERO);
    Homography { m: [a.a, a.c, a.e, a.b, a.d, a.f, 0.00012 * k, -0.00008 * k, 1.0] }
}

fn inverse(h: &Homography) -> Homography {
    let m = &h.m;
    Homography {
        m: [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ],
    }
}

struct Plane {
    info: filmcraft_media::MediaInfo,
    flat: bool,
    pose: fn(f64) -> Homography,
}

impl filmcraft_media::MediaSource for Plane {
    fn info(&self) -> &filmcraft_media::MediaInfo {
        &self.info
    }
    fn video_frame(&self, req: filmcraft_media::FrameRequest) -> filmcraft_media::Result<std::sync::Arc<filmcraft_frame::VideoFrame>> {
        let inv = inverse(&(self.pose)(RATE.frame_at(req.time) as f64));
        let mut px = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let mut acc = 0.0;
                for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let p = Vec2::new(x as f64 + ox, y as f64 + oy);
                    acc += match inv.apply(p) {
                        _ if self.flat => 0.5,
                        Some(o) if o.x.abs() < 80.0 && o.y.abs() < 80.0 => texture(o.x + 200.0, o.y + 300.0),
                        _ => 0.25 + 0.1 * texture(p.x * 0.5 + 900.0, p.y * 0.5),
                    };
                }
                let v = (acc / 4.0 * 255.0).round() as u8;
                px[(y * W + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        Ok(std::sync::Arc::new(filmcraft_frame::VideoFrame::rgba8(W as u32, H as u32, px)))
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<filmcraft_frame::AudioBuffer> {
        Ok(filmcraft_frame::AudioBuffer::silence(sample_rate, 2, frames))
    }
}

/// (session, plate clip on V1, pinned clip on V2 with a Corner Pin on the plane's corners at
/// frame 10, the Corner Pin's effect index)
fn session(flat: bool) -> (Session, ClipId, ClipId, usize) {
    session_with(flat, truth)
}

fn session_with(flat: bool, pose: fn(f64) -> Homography) -> (Session, ClipId, ClipId, usize) {
    use filmcraft_media::MediaSource;
    let g = |c: f32| {
        filmcraft_media::generators::GeneratorSource::new(
            filmcraft_media::Generator::ColorMatte { color: [c, c, c, 1.0] },
            W as u32,
            H as u32,
            RATE,
            RATE.tick_of(48),
        )
    };
    let mut p = filmcraft_project::Project::new("pin");
    let mut add = |name: &str, g: &filmcraft_media::generators::GeneratorSource| {
        p.add_item(
            name,
            filmcraft_project::Label::Iris,
            filmcraft_project::ItemKind::Media(filmcraft_project::MediaClip {
                media: filmcraft_project::MediaRef::Generator(g.generator.clone()),
                info: g.info().clone(),
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
    };
    let (gp, gi) = (g(0.5), g(0.9));
    let (plate_item, pin_item) = (add("plate", &gp), add("insert", &gi));
    let seq =
        p.new_sequence("s", filmcraft_project::SequenceSettings { width: W as u32, height: H as u32, frame_rate: RATE, ..Default::default() }, 2, 0, None);
    let range = filmcraft_time::TimeRange::new(Tick::ZERO, RATE.tick_of(48));
    let plate = p.make_track_item(plate_item, filmcraft_project::TrackKind::Video, Tick::ZERO, range, RATE).unwrap();
    let pin = p.make_track_item(pin_item, filmcraft_project::TrackKind::Video, Tick::ZERO, range, RATE).unwrap();
    let (plate_id, pin_id) = (plate.id, pin.id);
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(plate);
    q.video_tracks[1].items.push(pin);
    let mut s = Session { project: std::sync::Arc::new(p), ..Default::default() };
    s.state.active_sequence = Some(seq);
    s.state.selection = vec![pin_id];
    s.media.insert(plate_item, std::sync::Arc::new(Plane { info: gp.info().clone(), flat, pose }));
    s.execute("effects.apply", json!({"clips": [pin_id.0], "effect": "corner_pin"})).unwrap();
    let fx = s.active_sequence().unwrap().find_item(pin_id).unwrap().1.effects.iter().position(|e| e.effect == "corner_pin").unwrap();
    s.set_playhead(RATE.tick_of(10));
    for (id, c) in ["upper_left", "upper_right", "lower_right", "lower_left"].iter().zip(corners(10.0)) {
        s.execute("effects.setParam", json!({"clip": pin_id.0, "effect": fx, "param": id, "value": [c.x, c.y]})).unwrap();
    }
    (s, plate_id, pin_id, fx)
}

/// The plane's corners (inset 10 px) in frame pixels at frame `k`, clockwise from the top left.
fn corners(k: f64) -> [Vec2; 4] {
    [Vec2::new(-70.0, -70.0), Vec2::new(70.0, -70.0), Vec2::new(70.0, 70.0), Vec2::new(-70.0, 70.0)].map(|c| truth(k).apply(c).unwrap())
}

fn pins_at(s: &Session, pin: ClipId, fx: usize, frame: i64) -> [Vec2; 4] {
    let it = s.active_sequence().unwrap().find_item(pin).unwrap().1.clone();
    let e = &it.effects[fx];
    ["upper_left", "upper_right", "lower_right", "lower_left"].map(|id| e.param(id).unwrap().vec2_at(it.source_time_at(RATE.tick_of(frame))))
}

#[test]
fn corner_pin_follows_a_plane_in_perspective() {
    let (mut s, plate, pin, fx) = session(false);
    // (the plate below is the default source)
    let r = s.execute("tracking.cornerPin", json!({"frames": 12, "wait": true})).unwrap();
    assert_eq!(r["frames"], 12, "{r}");
    let r = s.execute("tracking.cornerPin", json!({"source": plate.0, "direction": "backward", "frames": 6, "wait": true})).unwrap();
    assert_eq!(r["frames"], 6, "{r}");
    assert!(s.pin_jobs.is_empty());
    let mut worst = 0.0f64;
    for f in [4i64, 7, 10, 13, 16, 19, 22] {
        for (got, want) in pins_at(&s, pin, fx, f).iter().zip(corners(f as f64)) {
            worst = worst.max((*got - want).length());
        }
    }
    eprintln!("corner pin tracking: max corner error {worst:.3} px over -6..+12 frames");
    assert!(worst < 1.0, "max corner error {worst} px");
    // two runs, two undo steps; undoing the backward run leaves frame 4 unkeyed (the static value
    // there is the first keyframe's)
    let h = s.execute("history.list", json!({})).unwrap();
    let undo: Vec<&str> = h["undo"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(&undo[undo.len() - 2..], ["Track Corner Pin", "Track Corner Pin"], "{undo:?}");
    s.undo();
    let e = &s.active_sequence().unwrap().find_item(pin).unwrap().1.effects[fx];
    assert_eq!(e.param("upper_left").unwrap().keyframes.len(), 13, "frames 10..=22 stay after one undo");
}

#[test]
fn corner_pin_tracking_refuses_what_it_cannot_do() {
    let (mut s, plate, pin, fx) = session(false);
    // no Corner Pin on the plate itself
    assert!(s.execute("tracking.cornerPin", json!({"clip": plate.0, "wait": true})).is_err());
    assert!(s.execute("tracking.cornerPin", json!({"clip": pin.0, "effect": fx + 1, "wait": true})).is_err(), "no effect there");
    assert!(s.execute("tracking.cornerPin", json!({"clip": pin.0, "effect": u64::MAX, "wait": true})).is_err());
    assert!(s.execute("tracking.cornerPin", json!({"source": 999_999, "wait": true})).is_err());
    assert!(s.execute("tracking.cornerPin", json!({"direction": "sideways", "wait": true})).is_err());
    assert!(s.execute("tracking.cornerPin", json!({"time": RATE.tick_of(500).0, "wait": true})).is_err(), "outside the clips");
    s.set_playhead(RATE.tick_of(47));
    assert!(s.execute("tracking.cornerPin", json!({"source": plate.0, "wait": true})).is_err(), "nothing after the last frame");
    // a huge frame count is clamped to the clips
    s.set_playhead(RATE.tick_of(40));
    let r = s.execute("tracking.cornerPin", json!({"source": plate.0, "frames": u64::MAX, "wait": true})).unwrap();
    assert_eq!(r["frames"], 7);
}

#[test]
fn a_flat_picture_loses_the_track_without_keyframes_or_a_panic() {
    let (mut s, plate, pin, fx) = session(true);
    let r = s.execute("tracking.cornerPin", json!({"source": plate.0, "frames": 5, "wait": true})).unwrap();
    assert!(r["status"]["status"].as_str().unwrap_or("").contains("lost track"), "{r}");
    assert!(s.pin_jobs.is_empty());
    let e = &s.active_sequence().unwrap().find_item(pin).unwrap().1.effects[fx];
    assert!(e.param("upper_left").unwrap().keyframes.len() <= 1, "only the start frame");
}

/// A rigid-plus-scale motion (no perspective): what a layer attached with Position, Scale &
/// Rotation can follow exactly.
fn similar(k: f64) -> Homography {
    let a = filmcraft_geom::Affine::motion(Vec2::new(150.0 + 3.0 * k, 110.0 + 1.2 * k), Vec2::new(1.0 + 0.008 * k, 1.0 + 0.008 * k), 1.2 * k, Vec2::ZERO);
    Homography { m: [a.a, a.c, a.e, a.b, a.d, a.f, 0.0, 0.0, 1.0] }
}

/// A pure translation (what Position-only attaching follows exactly).
fn sliding(k: f64) -> Homography {
    Homography { m: [1.0, 0.0, 150.0 + 3.0 * k, 0.0, 1.0, 110.0 + 1.2 * k, 0.0, 0.0, 1.0] }
}

fn motion_at(s: &Session, clip: ClipId, frame: i64) -> (Vec2, f64, f64) {
    let it = s.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
    let m = it.effects.iter().find(|e| e.effect == "motion").unwrap();
    let t = it.source_time_at(RATE.tick_of(frame));
    (m.vec2_at("position", t), m.f64_at("rotation", t), m.f64_at("scale", t))
}

#[test]
fn a_layer_attached_to_a_track_follows_it() {
    for (method, pose) in [("position", sliding as fn(f64) -> Homography), ("positionScaleRotation", similar)] {
        let (mut s, _, layer, _) = session_with(false, pose);
        // the layer's Position on a point of the plane at frame 10 (object (20, -15))
        let obj = Vec2::new(20.0, -15.0);
        let p10 = pose(10.0).apply(obj).unwrap();
        s.execute("effects.setParam", json!({"clip": layer.0, "effect": "motion", "param": "position", "value": [p10.x, p10.y]})).unwrap();
        let r = s.execute("tracking.attach", json!({"clip": layer.0, "method": method, "frames": 30, "wait": true})).unwrap();
        assert_eq!(r["frames"], 30, "{r}");
        let r = s.execute("tracking.attach", json!({"clip": layer.0, "method": method, "direction": "backward", "frames": 8, "wait": true})).unwrap();
        assert_eq!(r["frames"], 8, "{r}");
        assert!(s.attach_jobs.is_empty());
        let (mut worst, mut worst_rot, mut worst_scale) = (0.0f64, 0.0f64, 0.0f64);
        for f in [2i64, 6, 10, 16, 25, 33, 40] {
            let (pos, rot, scale) = motion_at(&s, layer, f);
            worst = worst.max((pos - pose(f as f64).apply(obj).unwrap()).length());
            if method != "position" {
                worst_rot = worst_rot.max((rot - 1.2 * (f - 10) as f64).abs());
                worst_scale = worst_scale.max((scale - 100.0 * (1.0 + 0.008 * f as f64) / (1.0 + 0.008 * 10.0)).abs());
            } else {
                assert_eq!((rot, scale), (0.0, 100.0), "position only leaves rotation and scale");
            }
        }
        eprintln!("attach {method}: position error {worst:.3} px, rotation {worst_rot:.3}°, scale {worst_scale:.3} % over -8..+30 frames");
        assert!(worst < 1.0, "{method}: position error {worst} px");
        assert!(worst_rot < 0.5 && worst_scale < 0.5, "{method}: rotation {worst_rot}°, scale {worst_scale} %");
        let h = s.execute("history.list", json!({})).unwrap();
        let undo: Vec<&str> = h["undo"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(&undo[undo.len() - 2..], ["Attach to Track", "Attach to Track"], "{undo:?}");
    }
}

#[test]
fn attach_refuses_what_it_cannot_do() {
    let (mut s, plate, layer, _) = session(false);
    assert!(s.execute("tracking.attach", json!({"clip": plate.0, "wait": true})).is_err(), "nothing below the bottom clip");
    assert!(s.execute("tracking.attach", json!({"clip": layer.0, "source": layer.0, "wait": true})).is_err(), "a clip can't follow itself");
    assert!(s.execute("tracking.attach", json!({"clip": layer.0, "method": "zoom", "wait": true})).is_err());
    assert!(s.execute("tracking.attach", json!({"clip": layer.0, "region": [[1, 2], [3, "x"]], "wait": true})).is_err());
    let flat = session(true).0;
    let mut f = flat;
    let r = f.execute("tracking.attach", json!({"clip": layer.0, "frames": 3, "size": [-5, f64::MAX], "wait": true})).unwrap();
    assert!(r["status"]["status"].as_str().unwrap_or("").contains("lost track"), "{r}");
    assert!(f.attach_jobs.is_empty());
}

#[test]
fn blur_and_track_hides_a_moving_region_and_only_it() {
    for kind in ["gaussian_blur", "mosaic"] {
        let (mut s, plate, _, _) = session_with(false, sliding);
        // only the plate: the clip above would cover it
        let top = s.active_sequence().unwrap().video_tracks[1].items[0].id;
        s.execute("timeline.select", json!({"clips": [top.0]})).unwrap();
        s.execute("edit.clear", json!({})).unwrap();
        assert!(s.active_sequence().unwrap().video_tracks[1].items.is_empty());
        let obj = Vec2::new(0.0, 0.0);
        s.set_playhead(RATE.tick_of(10));
        let before: Vec<_> = [10i64, 30]
            .iter()
            .map(|f| {
                s.set_playhead(RATE.tick_of(*f));
                s.render_program(1.0).unwrap()
            })
            .collect();
        s.set_playhead(RATE.tick_of(10));
        let c = sliding(10.0).apply(obj).unwrap();
        let r = s
            .execute("tracking.blurRegion", json!({"clip": plate.0, "effect": kind, "center": [c.x, c.y], "size": [60, 60], "frames": 20, "wait": true}))
            .unwrap();
        assert_eq!(r["track"]["frames"], 20, "{kind}: {r}");
        for (f, b) in [10i64, 30].iter().zip(&before) {
            s.set_playhead(RATE.tick_of(*f));
            let a = s.render_program(1.0).unwrap();
            let at = |img: &filmcraft_render::Image, p: Vec2| img.get(p.x as usize, p.y as usize);
            let inside = sliding(*f as f64).apply(Vec2::new(8.0, 5.0)).unwrap();
            let outside = sliding(*f as f64).apply(Vec2::new(70.0, 0.0)).unwrap();
            let d = |x: [f32; 4], y: [f32; 4]| x.iter().zip(&y).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
            assert!(d(at(&a, inside), at(b, inside)) > 1e-3, "{kind} frame {f}: inside the moving mask is changed");
            assert!(d(at(&a, outside), at(b, outside)) < 1e-4, "{kind} frame {f}: outside is untouched");
        }
    }
    let (mut s, plate, _, _) = session(false);
    assert!(s.execute("tracking.blurRegion", json!({"clip": plate.0, "wait": true})).is_err(), "needs a centre");
    assert!(s.execute("tracking.blurRegion", json!({"clip": plate.0, "effect": "invert", "center": [1, 2]})).is_err());
    assert!(s.execute("tracking.blurRegion", json!({"clip": plate.0, "center": [f64::NAN, 2]})).is_err());
}
