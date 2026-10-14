//! Tracking that drives other parameters. `tracking.attach` (fork Y3.2): a layer's Motion
//! (Position, and optionally Rotation and Scale) follows a region tracked in the clip below.
//!
//! Corner Pin tracking (fork Y3.3, screen replacement): the four Corner Pin points of a clip
//! follow a flat surface (a screen, a sign) in a clip below it, or in the same clip, frame by
//! frame. The surface is tracked as a homography (`filmcraft_render::track`, perspective step)
//! inside the quad the points cover at the start frame, and every tracked frame becomes four
//! point keyframes (one undo step per tracking run, written as the frames arrive, as
//! `masks.track` does).
//!
//! Coordinates: Corner Pin's points are in the pinned clip's source pixels; the tracked clip's
//! pixels go to sequence pixels through its Motion and back into the pinned clip's pixels through
//! the inverse of the pinned clip's Motion, both evaluated at each frame.

use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ClipId, ParamValue, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::commands::{CommandSpec, bad, bool_p, clip_p, has_seq, str_p, time_p, u64_p};
use crate::{EngineError, Result, Session};

/// Corner Pin's point parameters, clockwise from the top left, and where each sits by default
/// (fractions of the layer).
const PINS: [(&str, f64, f64); 4] = [("upper_left", 0.0, 0.0), ("upper_right", 1.0, 0.0), ("lower_right", 1.0, 1.0), ("lower_left", 0.0, 1.0)];

/// Most frames one run tracks (about 2 h 20 min at 24 fps; bounds the per-frame table).
const MAX_FRAMES: i64 = 200_000;

/// Longest side of the frames the tracker works on (as `masks.track`).
const TRACK_MAX_WIDTH: f64 = 960.0;

/// A running Corner Pin tracking job; see [`poll`].
pub struct PendingPin {
    pub job: u64,
    pub seq: filmcraft_project::ItemId,
    pub clip: ClipId,
    /// Index of the Corner Pin in the clip's effects.
    pub effect: usize,
    /// (pinned clip's media time, the four points in its pixels)
    pub keys: Arc<Mutex<Vec<(Tick, [Vec2; 4])>>>,
    applied: usize,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "tracking.cornerPin",
            label: "Track Corner Pin",
            menu: &[],
            shortcut: None,
            params: r#"{"clip":id? (the clip with the Corner Pin; default: the selected clip),"effect":index? (default: its first Corner Pin),"source":id? (the clip whose picture is tracked; default: the nearest clip below at the start frame, else the same clip),"direction":"forward"|"backward"?,"frames":n?,"time":ticks? (start; default: the playhead),"wait":bool?}"#,
            enabled: has_seq,
            run: track_corner_pin,
            journal: true,
        },
        CommandSpec {
            id: "tracking.attach",
            label: "Attach to Track",
            menu: &[],
            shortcut: None,
            params: r#"{"clip":id? (the layer that follows: text, a sticker, a callout; default: the selected clip),"source":id? (the clip whose picture is tracked; default: the nearest clip below at the start frame),"region":[[x,y]…]? (what to follow, sequence pixels; default: a box of `size` around the layer's Position),"size":[w,h]? (default [120,120]),"method":"position|positionRotation|positionScaleRotation"? (default position),"direction":"forward"|"backward"?,"frames":n?,"time":ticks?,"wait":bool?}"#,
            enabled: has_seq,
            run: attach,
            journal: true,
        },
        CommandSpec {
            id: "tracking.blurRegion",
            label: "Blur and Track",
            menu: &[],
            shortcut: None,
            params: r#"{"clip":id? (default: the selected clip),"center":[x,y] (clip pixels: what to hide, e.g. a face or a number plate),"size":[w,h]? (default [160,160]),"effect":"gaussian_blur"|"mosaic"? (default gaussian_blur),"direction":"forward"|"backward"?,"frames":n?,"method":"position|positionRotation|positionScaleRotation"?,"wait":bool?}"#,
            enabled: has_seq,
            run: blur_region,
            journal: true,
        },
    ]
}

/// One click for a tracked blur (fork Y3.2c): Gaussian Blur or Mosaic on the clip, an ellipse mask
/// on it around `center`, and the mask tracked (`masks.track`). Three undo steps (the effect, the
/// mask, the track), each its own so a failed track still leaves the blur to place by hand.
fn blur_region(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "tracking.blurRegion";
    let clip = match clip_p(p, "clip") {
        Some(c) => c,
        None => *s.state.selection.first().ok_or_else(|| bad(ID, "need `clip` (or select the clip)"))?,
    };
    let kind = str_p(p, "effect").unwrap_or("gaussian_blur");
    if !matches!(kind, "gaussian_blur" | "mosaic") {
        return Err(bad(ID, "`effect` is gaussian_blur or mosaic"));
    }
    let pair = |k: &str| -> Option<[f64; 2]> {
        let a = p.get(k)?.as_array()?;
        let v = [a.first()?.as_f64()?, a.get(1)?.as_f64()?];
        v.iter().all(|x| x.is_finite()).then_some(v)
    };
    let center = pair("center").ok_or_else(|| bad(ID, "need `center` [x, y] in clip pixels"))?;
    let size = pair("size").filter(|v| v.iter().all(|x| *x > 0.0)).unwrap_or([160.0, 160.0]);
    let it = s.active_sequence().ok_or(EngineError::NoSequence)?.find_item(clip).ok_or_else(|| bad(ID, "no such clip"))?.1.clone();
    let (w, h) = filmcraft_render::source_size(&s.project, it.item).ok_or_else(|| bad(ID, "the clip has no picture"))?;
    s.execute("effects.apply", json!({"clips": [clip.0], "effect": kind}))?;
    let idx = s
        .active_sequence()
        .and_then(|q| q.find_item(clip))
        .and_then(|(_, it)| it.effects.iter().rposition(|e| e.effect == kind))
        .ok_or_else(|| bad(ID, "the effect was not added"))?;
    match kind {
        "mosaic" => {
            // blocks of about 16 source pixels
            let blocks = |n: u32| (f64::from(n) / 16.0).round().clamp(1.0, 4000.0);
            s.execute("effects.setParam", json!({"clip": clip.0, "effect": idx, "param": "horizontal", "value": blocks(w)}))?;
            s.execute("effects.setParam", json!({"clip": clip.0, "effect": idx, "param": "vertical", "value": blocks(h)}))?;
        }
        _ => {
            s.execute("effects.setParam", json!({"clip": clip.0, "effect": idx, "param": "blurriness", "value": 40.0}))?;
        }
    }
    s.execute("masks.add", json!({"clip": clip.0, "effect": idx, "shape": "ellipse", "center": center, "size": size}))?;
    let mask = s
        .active_sequence()
        .and_then(|q| q.find_item(clip))
        .and_then(|(_, it)| it.effects.get(idx))
        .map(|e| e.masks.len().saturating_sub(1))
        .ok_or_else(|| bad(ID, "the mask was not added"))?;
    let mut tp = json!({"clip": clip.0, "effect": idx, "mask": mask, "direction": str_p(p, "direction").unwrap_or("forward")});
    for k in ["frames", "method", "wait", "time"] {
        if let Some(v) = p.get(k) {
            tp[k] = v.clone();
        }
    }
    let track = s.execute("masks.track", tp)?;
    Ok(json!({"effect": idx, "mask": mask, "track": track}))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The Motion map of `it` (source pixels → sequence pixels) at sequence time `t`.
fn motion_at(s: &Session, seq: &filmcraft_project::Sequence, it: &TrackItem, t: Tick) -> Option<Affine> {
    let size = filmcraft_render::source_size(&s.project, it.item)?;
    let mt = it.source_time_at(t);
    Some(filmcraft_render::motion_matrix(seq, it, size, filmcraft_render::source_par(&s.project, it.item), mt))
}

/// One frame of the run: sequence time, the tracked clip's media time, the pinned clip's media
/// time, and the map from the tracked clip's pixels to the pinned clip's.
struct Frame {
    plate_t: Tick,
    pin_t: Tick,
    plate_to_pin: Affine,
}

fn track_corner_pin(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "tracking.cornerPin";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let clip = match clip_p(p, "clip") {
        Some(c) => c,
        None => *s.state.selection.first().ok_or_else(|| bad(ID, "need `clip` (or select the clip with the Corner Pin)"))?,
    };
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    let pin_it = seq.find_item(clip).ok_or_else(|| bad(ID, "no such clip"))?.1.clone();
    let fx = match u64_p(p, "effect") {
        Some(i) => usize::try_from(i).ok().filter(|i| pin_it.effects.get(*i).is_some_and(|e| e.effect == "corner_pin")),
        None => pin_it.effects.iter().position(|e| e.effect == "corner_pin"),
    }
    .ok_or_else(|| bad(ID, "the clip has no Corner Pin there: apply Corner Pin first"))?;
    if s.pin_jobs.iter().any(|j| j.clip == clip && j.effect == fx) {
        return Err(bad(ID, "this Corner Pin is already being tracked"));
    }
    let t0 = time_p(s, p, "").unwrap_or(s.playhead());
    // default source: the nearest clip below at the start frame (the plate a screen replacement
    // sits on), else the clip itself
    let below = || {
        let ti = seq.video_tracks.iter().position(|tr| tr.items.iter().any(|i| i.id == clip))?;
        seq.video_tracks.get(..ti)?.iter().rev().find_map(|tr| tr.items.iter().find(|i| i.start <= t0 && t0 < i.end()).map(|i| i.id))
    };
    let plate_id = clip_p(p, "source").or_else(below).unwrap_or(clip);
    let plate_it = seq.find_item(plate_id).ok_or_else(|| bad(ID, "no such source clip"))?.1.clone();
    let backward = match str_p(p, "direction").unwrap_or("forward") {
        "forward" => false,
        "backward" => true,
        d => return Err(bad(ID, format!("direction `{d}`: forward or backward"))),
    };
    let fd = seq.settings.frame_rate.frame_duration();
    let (lo, hi) = (pin_it.start.max(plate_it.start), (pin_it.end().min(plate_it.end()) - Tick(1)));
    if hi < lo {
        return Err(bad(ID, "the two clips don't overlap in time"));
    }
    if t0 < lo || t0 > hi {
        return Err(bad(ID, "the start frame must be where both clips are"));
    }
    let room = if backward { (t0 - lo).0 / fd.0.max(1) } else { (hi - t0).0 / fd.0.max(1) };
    let steps = match u64_p(p, "frames") {
        Some(n) => i64::try_from(n).unwrap_or(i64::MAX).min(room),
        None => room,
    }
    .clamp(0, MAX_FRAMES) as usize;
    if steps == 0 {
        return Err(bad(ID, if backward { "already at the first frame both clips share" } else { "already at the last frame both clips share" }));
    }
    let pin_size = filmcraft_render::source_size(&s.project, pin_it.item).ok_or_else(|| bad(ID, "the clip has no picture"))?;
    let plate_size = filmcraft_render::source_size(&s.project, plate_it.item).ok_or_else(|| bad(ID, "the source clip has no picture"))?;
    let src = s.source(plate_it.item).ok_or_else(|| EngineError::Other("the source clip has no media to track".into()))?;
    // every frame's maps, worked out here (they need the project)
    let mut frames = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let d = Tick(fd.0.saturating_mul(k as i64));
        let t = if backward { t0 - d } else { t0 + d };
        let (Some(m_plate), Some(m_pin)) = (motion_at(s, &seq, &plate_it, t), motion_at(s, &seq, &pin_it, t)) else {
            return Err(bad(ID, "the clips have no picture"));
        };
        let pin_inv = m_pin.inverse().ok_or_else(|| bad(ID, "the clip's Motion collapses it (scale 0)"))?;
        frames.push(Frame { plate_t: plate_it.source_time_at(t), pin_t: pin_it.source_time_at(t), plate_to_pin: pin_inv.then_apply(&m_plate) });
    }
    let first = frames.first().ok_or_else(|| bad(ID, "nothing to track"))?;
    // the points at the start frame (unset points sit on the layer's corners)
    let e = pin_it.effects.get(fx).ok_or_else(|| bad(ID, "no Corner Pin"))?;
    let start: [Vec2; 4] = PINS.map(|(id, fx_, fy_)| {
        let v = e.param(id).map(|q| q.vec2_at(first.pin_t)).unwrap_or(Vec2::new(f64::NAN, f64::NAN));
        Vec2::new(if v.x.is_finite() { v.x } else { pin_size.0 as f64 * fx_ }, if v.y.is_finite() { v.y } else { pin_size.1 as f64 * fy_ })
    });
    let pin_to_plate0 = first.plate_to_pin.inverse().ok_or_else(|| bad(ID, "the source clip's Motion collapses it (scale 0)"))?;
    let quad0 = start.map(|q| pin_to_plate0.apply(q));
    if quad0.iter().any(|q| !(q.x.is_finite() && q.y.is_finite())) {
        return Err(bad(ID, "the Corner Pin points are not usable"));
    }
    let scale = (TRACK_MAX_WIDTH / plate_size.0.max(1) as f64).min(1.0) as f32;
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let label = format!("Track Corner Pin ({})", if backward { "backward" } else { "forward" });
    let job = crate::Job { id, label, progress: Default::default(), result: Default::default() };
    job.progress.total.store(steps as u64, std::sync::atomic::Ordering::Relaxed);
    let keys: Arc<Mutex<Vec<(Tick, [Vec2; 4])>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), keys.clone());
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let gray = |t: Tick| -> Option<(filmcraft_render::track::Prepared, f64)> {
            let f = src.video_frame(filmcraft_media::FrameRequest { time: t, scale }).ok()?;
            let g = filmcraft_render::track::Gray::from_rgba8(f.width as usize, f.height as usize, &f.to_rgba8());
            Some((filmcraft_render::track::Prepared::new(g), f.width as f64 / plate_size.0.max(1) as f64))
        };
        let mut quad = quad0;
        let mut err: Option<String> = None;
        let mut done = 0u64;
        let mut it = frames.into_iter();
        let first = it.next();
        match first.as_ref().and_then(|f| gray(f.plate_t).map(|g| (f, g))) {
            None => err = Some("can't decode the source clip".into()),
            Some((f0, (mut prev, mut k_prev))) => {
                lock(&out).push((f0.pin_t, quad.map(|q| f0.plate_to_pin.apply(q))));
                for (step, f) in it.enumerate().map(|(i, f)| (i + 1, f)) {
                    if prog.cancel.load(Ordering::Relaxed) {
                        err = Some("stopped".into());
                        break;
                    }
                    let Some((next, k)) = gray(f.plate_t) else {
                        err = Some(format!("can't decode frame {step}"));
                        break;
                    };
                    let region: Vec<Vec2> = quad.iter().map(|q| *q * k_prev).collect();
                    let moved = filmcraft_render::track::track_step_perspective(&prev, &next, &region).and_then(|st| {
                        let m: Vec<Vec2> = region.iter().filter_map(|q| st.transform.apply(*q).map(|r| r * (1.0 / k))).collect();
                        <[Vec2; 4]>::try_from(m).ok().map(|m| (m, st.inliers))
                    });
                    let Some((m, inliers)) = moved else {
                        err = Some(format!("lost track after {} frame(s): not enough detail inside the corners", step - 1));
                        break;
                    };
                    quad = m;
                    lock(&out).push((f.pin_t, quad.map(|q| f.plate_to_pin.apply(q))));
                    done += 1;
                    prog.done.store(done, Ordering::Relaxed);
                    *lock(&prog.status) = format!("Frame {step} of {steps} · {inliers} features");
                    prev = next;
                    k_prev = k;
                }
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let r = match &err {
            Some(e) if e != "stopped" => Err(e.clone()),
            _ => Ok(filmcraft_export::Report {
                path: String::new(),
                frames: done,
                seconds: secs,
                bytes: 0,
                render_fps: done as f64 / secs.max(1e-6),
                extra_files: Vec::new(),
            }),
        };
        *lock(&prog.status) = match &err {
            Some(e) => format!("{e} ({done} frame(s) tracked)"),
            None => format!("Tracked {done} frame(s) in {secs:.1}s"),
        };
        prog.finished.store(true, Ordering::Relaxed);
        *lock(&res) = Some(r);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.pin_jobs.push(PendingPin { job: id, seq: seq_id, clip, effect: fx, keys, applied: 0 });
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
    } else {
        std::thread::Builder::new().name("filmcraft-pin-track".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    let st = s.jobs.iter().find(|j| j.id == id).map(crate::Job::to_json).unwrap_or(Value::Null);
    Ok(json!({"job": id, "frames": steps, "status": st}))
}

/// A running attach-to-track job: the layer's Motion follows a tracked region (fork Y3.2).
pub struct PendingAttach {
    pub job: u64,
    pub seq: filmcraft_project::ItemId,
    pub clip: ClipId,
    /// The layer's Position (sequence px), Scale, Scale Width (%) and Rotation (°) at the start.
    start: (Vec2, f64, f64, f64),
    method: filmcraft_project::TrackMethod,
    /// (the layer's media time, the tracked motion in sequence pixels since the start frame)
    pub keys: Arc<Mutex<Vec<(Tick, Affine)>>>,
    applied: usize,
}

/// The rotation (degrees) and uniform scale of a similarity map.
fn rot_scale(m: &Affine) -> (f64, f64) {
    (m.b.atan2(m.a).to_degrees(), (m.a * m.a + m.b * m.b).sqrt())
}

fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "tracking.attach";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let clip = match clip_p(p, "clip") {
        Some(c) => c,
        None => *s.state.selection.first().ok_or_else(|| bad(ID, "need `clip` (or select the layer that should follow)"))?,
    };
    if s.attach_jobs.iter().any(|j| j.clip == clip) {
        return Err(bad(ID, "this clip is already being attached"));
    }
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    let layer = seq.find_item(clip).ok_or_else(|| bad(ID, "no such clip"))?.1.clone();
    let method = match str_p(p, "method") {
        Some(m) => filmcraft_project::TrackMethod::from_name(m).ok_or_else(|| bad(ID, format!("unknown method `{m}`")))?,
        None => filmcraft_project::TrackMethod::Position,
    };
    let backward = match str_p(p, "direction").unwrap_or("forward") {
        "forward" => false,
        "backward" => true,
        d => return Err(bad(ID, format!("direction `{d}`: forward or backward"))),
    };
    let t0 = time_p(s, p, "").unwrap_or(s.playhead());
    let below = || {
        let ti = seq.video_tracks.iter().position(|tr| tr.items.iter().any(|i| i.id == clip))?;
        seq.video_tracks.get(..ti)?.iter().rev().find_map(|tr| tr.items.iter().find(|i| i.start <= t0 && t0 < i.end()).map(|i| i.id))
    };
    let plate_id = clip_p(p, "source").or_else(below).ok_or_else(|| bad(ID, "nothing below the clip to follow: put it above the shot, or pass `source`"))?;
    if plate_id == clip {
        return Err(bad(ID, "a clip can't follow itself"));
    }
    let plate = seq.find_item(plate_id).ok_or_else(|| bad(ID, "no such source clip"))?.1.clone();
    let fd = seq.settings.frame_rate.frame_duration();
    let (lo, hi) = (layer.start.max(plate.start), (layer.end().min(plate.end()) - Tick(1)));
    if hi < lo {
        return Err(bad(ID, "the two clips don't overlap in time"));
    }
    if t0 < lo || t0 > hi {
        return Err(bad(ID, "the start frame must be where both clips are"));
    }
    let room = if backward { (t0 - lo).0 / fd.0.max(1) } else { (hi - t0).0 / fd.0.max(1) };
    let steps = match u64_p(p, "frames") {
        Some(n) => i64::try_from(n).unwrap_or(i64::MAX).min(room),
        None => room,
    }
    .clamp(0, MAX_FRAMES) as usize;
    if steps == 0 {
        return Err(bad(ID, if backward { "already at the first frame both clips share" } else { "already at the last frame both clips share" }));
    }
    // the layer's Motion at the start
    let motion = layer.effects.iter().find(|e| e.effect == "motion");
    let lt0 = layer.source_time_at(t0);
    let (sw, sh) = (seq.settings.width as f64, seq.settings.height as f64);
    let pos = motion.map(|m| m.vec2_at("position", lt0)).filter(|v| v.x.is_finite() && v.y.is_finite()).unwrap_or(Vec2::new(sw / 2.0, sh / 2.0));
    let f = |id: &str, d: f64| motion.map(|m| m.f64_at(id, lt0)).filter(|v| v.is_finite()).unwrap_or(d);
    let start = (pos, f("scale", 100.0), f("scale_width", 100.0), f("rotation", 0.0));
    // what to follow, in sequence pixels
    let region: Vec<Vec2> = match p.get("region").and_then(Value::as_array) {
        Some(pts) => pts
            .iter()
            .take(1024)
            .filter_map(|q| Some(Vec2::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?)))
            .filter(|v| v.x.is_finite() && v.y.is_finite())
            .collect(),
        None => {
            let size = p.get("size").and_then(Value::as_array);
            let g = |i: usize| size.and_then(|a| a.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(120.0).min(sw.max(sh));
            let (hw, hh) = (g(0) / 2.0, g(1) / 2.0);
            vec![pos + Vec2::new(-hw, -hh), pos + Vec2::new(hw, -hh), pos + Vec2::new(hw, hh), pos + Vec2::new(-hw, hh)]
        }
    };
    if region.len() < 3 {
        return Err(bad(ID, "`region` needs at least three points"));
    }
    let plate_size = filmcraft_render::source_size(&s.project, plate.item).ok_or_else(|| bad(ID, "the source clip has no picture"))?;
    let src = s.source(plate.item).ok_or_else(|| EngineError::Other("the source clip has no media to track".into()))?;
    let mut frames = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let d = Tick(fd.0.saturating_mul(k as i64));
        let t = if backward { t0 - d } else { t0 + d };
        let m = motion_at(s, &seq, &plate, t).ok_or_else(|| bad(ID, "the source clip has no picture"))?;
        frames.push((plate.source_time_at(t), layer.source_time_at(t), m));
    }
    let m0_inv = frames.first().and_then(|f| f.2.inverse()).ok_or_else(|| bad(ID, "the source clip's Motion collapses it (scale 0)"))?;
    let region0: Vec<Vec2> = region.iter().map(|q| m0_inv.apply(*q)).collect();
    let scale = (TRACK_MAX_WIDTH / plate_size.0.max(1) as f64).min(1.0) as f32;
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let label = format!("Attach to Track ({})", if backward { "backward" } else { "forward" });
    let job = crate::Job { id, label, progress: Default::default(), result: Default::default() };
    job.progress.total.store(steps as u64, std::sync::atomic::Ordering::Relaxed);
    let keys: Arc<Mutex<Vec<(Tick, Affine)>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), keys.clone());
    let run = move || {
        use std::sync::atomic::Ordering;
        let started = web_time::Instant::now();
        let gray = |t: Tick| -> Option<(filmcraft_render::track::Prepared, f64)> {
            let f = src.video_frame(filmcraft_media::FrameRequest { time: t, scale }).ok()?;
            let g = filmcraft_render::track::Gray::from_rgba8(f.width as usize, f.height as usize, &f.to_rgba8());
            Some((filmcraft_render::track::Prepared::new(g), f.width as f64 / plate_size.0.max(1) as f64))
        };
        let mut region = region0;
        // the tracked motion in the source clip's pixels since the start frame
        let mut acc = Affine::IDENTITY;
        let mut err: Option<String> = None;
        let mut done = 0u64;
        let mut it = frames.into_iter();
        match it.next().and_then(|f| gray(f.0).map(|g| (f, g))) {
            None => err = Some("can't decode the source clip".into()),
            Some(((_, lt, _), (mut prev, mut k_prev))) => {
                lock(&out).push((lt, Affine::IDENTITY));
                for (step, (pt, lt, m)) in it.enumerate().map(|(i, f)| (i + 1, f)) {
                    if prog.cancel.load(Ordering::Relaxed) {
                        err = Some("stopped".into());
                        break;
                    }
                    let Some((next, k)) = gray(pt) else {
                        err = Some(format!("can't decode frame {step}"));
                        break;
                    };
                    let scaled: Vec<Vec2> = region.iter().map(|q| *q * k_prev).collect();
                    let Some(st) = filmcraft_render::track::track_step(&prev, &next, &scaled, method) else {
                        err = Some(format!("lost track after {} frame(s): not enough detail to follow", step - 1));
                        break;
                    };
                    let d = Affine::scale(1.0 / k, 1.0 / k).then_apply(&st.transform).then_apply(&Affine::scale(k_prev, k_prev));
                    region = region.iter().map(|q| d.apply(*q)).collect();
                    acc = d.then_apply(&acc);
                    // in sequence pixels: back to the start frame's source pixels, the motion, out
                    // through this frame's Motion
                    lock(&out).push((lt, m.then_apply(&acc).then_apply(&m0_inv)));
                    done += 1;
                    prog.done.store(done, Ordering::Relaxed);
                    *lock(&prog.status) = format!("Frame {step} of {steps} · {} features", st.inliers);
                    prev = next;
                    k_prev = k;
                }
            }
        }
        let secs = started.elapsed().as_secs_f64();
        let r = match &err {
            Some(e) if e != "stopped" => Err(e.clone()),
            _ => Ok(filmcraft_export::Report {
                path: String::new(),
                frames: done,
                seconds: secs,
                bytes: 0,
                render_fps: done as f64 / secs.max(1e-6),
                extra_files: Vec::new(),
            }),
        };
        *lock(&prog.status) = match &err {
            Some(e) => format!("{e} ({done} frame(s) tracked)"),
            None => format!("Tracked {done} frame(s) in {secs:.1}s"),
        };
        prog.finished.store(true, Ordering::Relaxed);
        *lock(&res) = Some(r);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.attach_jobs.push(PendingAttach { job: id, seq: seq_id, clip, start, method, keys, applied: 0 });
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
    } else {
        std::thread::Builder::new().name("filmcraft-attach-track".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    let st = s.jobs.iter().find(|j| j.id == id).map(crate::Job::to_json).unwrap_or(Value::Null);
    Ok(json!({"job": id, "frames": steps, "status": st}))
}

/// Write newly tracked frames of attach jobs as Motion keyframes (one merged undo step per job).
fn poll_attach(s: &mut Session) {
    use filmcraft_project::TrackMethod;
    use std::sync::atomic::Ordering;
    let mut i = 0;
    while let Some(pj) = s.attach_jobs.get(i) {
        let finished = s.jobs.iter().find(|j| j.id == pj.job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let new: Vec<(Tick, Affine)> = lock(&pj.keys).get(pj.applied..).map(<[_]>::to_vec).unwrap_or_default();
        let total = lock(&pj.keys).len();
        let (job, seq_id, clip, (pos, sc, scw, rot), method) = (pj.job, pj.seq, pj.clip, pj.start, pj.method);
        if !new.is_empty() {
            let n_new = new.len();
            let r = s.edit_merged("Attach to Track", &format!("attach-track-{job}"), move |pr, _| {
                let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
                let (_, it) = q.find_item_mut(clip).ok_or(filmcraft_edit::EditError::NoItem(clip))?;
                let e = it.effects.iter_mut().find(|e| e.effect == "motion").ok_or_else(|| bad("tracking.attach", "the clip has no Motion"))?;
                e.enabled = true;
                let mut ids = vec!["position"];
                if method != TrackMethod::Position {
                    ids.push("rotation");
                }
                if method == TrackMethod::PositionScaleRotation {
                    ids.extend(["scale", "scale_width"]);
                }
                for id in &ids {
                    if e.param(id).is_none()
                        && let Some(d) = e.def().and_then(|d| d.param(id))
                    {
                        e.params.insert((*id).to_string(), filmcraft_project::Param::new(d.default.clone()));
                    }
                }
                for (t, m) in new {
                    let (dr, ds) = rot_scale(&m);
                    for id in &ids {
                        let v = match *id {
                            "position" => ParamValue::Vec2(m.apply(pos)),
                            "rotation" => ParamValue::Float(rot + dr),
                            "scale" => ParamValue::Float(sc * ds),
                            _ => ParamValue::Float(scw * ds),
                        };
                        if let Some(prm) = e.param_mut(id) {
                            prm.put_keyframe(t, v);
                        }
                    }
                }
                Ok(())
            });
            match r {
                Ok(()) => {
                    if let Some(pj) = s.attach_jobs.get_mut(i) {
                        pj.applied += n_new;
                    }
                }
                Err(e) => {
                    s.error_toast("tracking.attach", e.to_string());
                    if let Some(j) = s.jobs.iter().find(|j| j.id == job) {
                        j.progress.cancel.store(true, Ordering::Relaxed);
                    }
                    s.attach_jobs.remove(i);
                    continue;
                }
            }
        }
        let (applied, total_now) = s.attach_jobs.get(i).map_or((0, 0), |pj| (pj.applied, lock(&pj.keys).len()));
        if finished && applied >= total.max(total_now) {
            s.attach_jobs.remove(i);
            continue;
        }
        i += 1;
    }
}

/// Write newly tracked frames as Corner Pin point keyframes (one merged undo step per job) and
/// drop finished jobs. Called from [`Session::poll_persistence`] and after synchronous runs.
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    poll_attach(s);
    let mut i = 0;
    while let Some(pj) = s.pin_jobs.get(i) {
        let finished = s.jobs.iter().find(|j| j.id == pj.job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let new: Vec<(Tick, [Vec2; 4])> = lock(&pj.keys).get(pj.applied..).map(<[_]>::to_vec).unwrap_or_default();
        let total = lock(&pj.keys).len();
        let (job, seq_id, clip, fx) = (pj.job, pj.seq, pj.clip, pj.effect);
        if !new.is_empty() {
            let n_new = new.len();
            let r = s.edit_merged("Track Corner Pin", &format!("pin-track-{job}"), move |pr, _| {
                let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
                let (_, it) = q.find_item_mut(clip).ok_or(filmcraft_edit::EditError::NoItem(clip))?;
                let e = it.effects.get_mut(fx).filter(|e| e.effect == "corner_pin").ok_or_else(|| bad("tracking.cornerPin", "the Corner Pin is gone"))?;
                for (id, _, _) in PINS {
                    if e.param(id).is_none() {
                        let d = e.def().and_then(|d| d.param(id)).map(|d| d.default.clone()).unwrap_or(ParamValue::Vec2(Vec2::new(f64::NAN, f64::NAN)));
                        e.params.insert(id.to_string(), filmcraft_project::Param::new(d));
                    }
                }
                for (t, pts) in new {
                    for ((id, _, _), v) in PINS.iter().zip(pts) {
                        if let Some(prm) = e.param_mut(id) {
                            prm.put_keyframe(t, ParamValue::Vec2(v));
                        }
                    }
                }
                Ok(())
            });
            match r {
                Ok(()) => {
                    if let Some(pj) = s.pin_jobs.get_mut(i) {
                        pj.applied += n_new;
                    }
                }
                Err(e) => {
                    s.error_toast("tracking.cornerPin", e.to_string());
                    if let Some(j) = s.jobs.iter().find(|j| j.id == job) {
                        j.progress.cancel.store(true, Ordering::Relaxed);
                    }
                    s.pin_jobs.remove(i);
                    continue;
                }
            }
        }
        let (applied, total_now) = s.pin_jobs.get(i).map_or((0, 0), |pj| (pj.applied, lock(&pj.keys).len()));
        if finished && applied >= total.max(total_now) {
            s.pin_jobs.remove(i);
            continue;
        }
        i += 1;
    }
}
