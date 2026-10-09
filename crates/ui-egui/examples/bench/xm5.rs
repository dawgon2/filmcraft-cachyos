//! The fork's Fujifilm X-M5 6.2K HEVC scenarios (`FORK-PLAN.md` §4; results in
//! `docs/fork-benchmarks.md`): `cargo xtask bench --sections xm5 [--only play-full] [--quick]`.
//!
//! Clips come from `$FILMCRAFT_XM5_DIR` (real camera clips linked under the names below) or
//! `target/fixtures/xm5/`, where missing ones are generated with ffmpeg as synthetic stand-ins
//! (6240×4160 HEVC Main10 long GOP, ~200 Mbit/s, MOV, 24-bit LPCM; slow: ~6 fps with libx265).
//! Every row also carries what the GPU did meanwhile (AMD `gpu_busy_percent`, VRAM in use) and
//! the process's peak resident memory, sampled every 100 ms.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use filmcraft_engine::Session;
use filmcraft_project::{ItemId, ParamValue, Project, SequenceSettings, TrackKind, find_effect, resolve_auto_points};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use serde_json::{Value, json};

use crate::Opts;
use crate::playback::{self, Display, PlayReport, load_avg, ms, pct};
use crate::sections::new_bench;

/// A synthetic stand-in: file name, frame rate (num, den), seconds, pixel format, x265 profile,
/// lavfi picture source (`{S}` = size, `{R}` = rate), keyframe interval.
struct Spec {
    name: &'static str,
    rate: (u32, u32),
    secs: u32,
    pix_fmt: &'static str,
    profile: &'static str,
    src: &'static str,
    keyint: u32,
}

pub const PLAY: &str = "xm5_420_2997_60s.mov";
pub const P2398: &str = "xm5_420_2398_60s.mov";
pub const C422: &str = "xm5_422_2997_20s.mov";
pub const CLIP_B: &str = "xm5_420_2997_30s_b.mov";
pub const LONG: &str = "xm5_420_2997_5min.mov";

const SPECS: &[Spec] = &[
    Spec {
        name: PLAY,
        rate: (30000, 1001),
        secs: 60,
        pix_fmt: "yuv420p10le",
        profile: "main10",
        src: "testsrc2=s={S}:r={R},noise=alls=14:allf=t+u",
        keyint: 30,
    },
    Spec {
        name: P2398,
        rate: (24000, 1001),
        secs: 60,
        pix_fmt: "yuv420p10le",
        profile: "main10",
        src: "testsrc2=s={S}:r={R},hue=h=t*30,noise=alls=14:allf=t+u",
        keyint: 24,
    },
    Spec {
        name: C422,
        rate: (30000, 1001),
        secs: 20,
        pix_fmt: "yuv422p10le",
        profile: "main422-10",
        src: "testsrc2=s={S}:r={R},hue=h=t*60,noise=alls=14:allf=t+u",
        keyint: 30,
    },
    Spec {
        name: CLIP_B,
        rate: (30000, 1001),
        secs: 30,
        pix_fmt: "yuv420p10le",
        profile: "main10",
        src: "testsrc=s={S}:r={R},noise=alls=14:allf=t+u",
        keyint: 30,
    },
    Spec {
        name: LONG,
        rate: (30000, 1001),
        secs: 300,
        pix_fmt: "yuv420p10le",
        profile: "main10",
        src: "testsrc2=s={S}:r={R},hue=h=t*10,noise=alls=14:allf=t+u",
        keyint: 30,
    },
];

/// Where the clips live: `$FILMCRAFT_XM5_DIR`, else `<repo>/target/fixtures/xm5`.
pub fn xm5_dir() -> PathBuf {
    match std::env::var_os("FILMCRAFT_XM5_DIR").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/xm5"),
    }
}

/// The clip `name`, generating the synthetic stand-in on first use (None when ffmpeg or libx265
/// is missing).
pub fn clip(name: &str) -> Option<PathBuf> {
    let dir = xm5_dir();
    let path = dir.join(name);
    if path.exists() {
        return Some(path);
    }
    let spec = SPECS.iter().find(|s| s.name == name)?;
    let ff = crate::fixtures::ffmpeg()?;
    std::fs::create_dir_all(&dir).ok()?;
    eprintln!("generating {} (6.2K x265: this takes a while)", path.display());
    let rate = format!("{}/{}", spec.rate.0, spec.rate.1);
    let src = spec.src.replace("{S}", "6240x4160").replace("{R}", &rate);
    let x265 = format!("keyint={k}:min-keyint={k}:bitrate=200000:vbv-maxrate=200000:vbv-bufsize=400000:log-level=error", k = spec.keyint);
    let tmp = dir.join(format!(".{name}.{}.tmp.mov", std::process::id()));
    let ok = Command::new(ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &src, "-f", "lavfi", "-i", "sine=f=440:r=48000,aformat=channel_layouts=stereo"])
        .args(["-t", &spec.secs.to_string(), "-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", spec.pix_fmt, "-profile:v", spec.profile])
        .args(["-x265-params", &x265, "-color_range", "tv", "-tag:v", "hvc1", "-c:a", "pcm_s24le", "-shortest"])
        .arg(&tmp)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("  ffmpeg failed for {name}");
        return None;
    }
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

// ------------------------------------------------------------------------------------ sampling

/// The AMD GPU's sysfs device directory (`/sys/class/drm/cardN/device` with `gpu_busy_percent`).
fn amd_gpu_sysfs() -> Option<PathBuf> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm").ok()?.flatten().map(|e| e.path()).collect();
    cards.sort();
    cards
        .into_iter()
        .map(|c| c.join("device"))
        .find(|d| std::fs::read_to_string(d.join("vendor")).is_ok_and(|v| v.trim() == "0x1002") && d.join("gpu_busy_percent").exists())
}

fn read_num(p: &Path) -> Option<f64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// Resident set size of this process (bytes).
fn rss_bytes() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096.0)
}

#[derive(Default, Clone)]
pub struct Sampled {
    pub gpu_busy_mean: f64,
    pub gpu_busy_max: f64,
    pub vram_max_mb: f64,
    pub vram_start_mb: f64,
    pub rss_max_mb: f64,
    pub rss_start_mb: f64,
    pub samples: usize,
}

impl Sampled {
    fn json(&self) -> Value {
        json!({
            "gpu_busy_pct_mean": self.gpu_busy_mean, "gpu_busy_pct_max": self.gpu_busy_max,
            "vram_mb_max": self.vram_max_mb, "vram_mb_start": self.vram_start_mb,
            "rss_mb_max": self.rss_max_mb, "rss_mb_start": self.rss_start_mb,
        })
    }
}

/// Samples GPU busy %, VRAM and RSS every 100 ms on a thread until dropped / finished.
pub struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Sampled>>,
}

impl Sampler {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::spawn(move || {
            let dev = amd_gpu_sysfs();
            let vram = |d: &Option<PathBuf>| d.as_ref().and_then(|d| read_num(&d.join("mem_info_vram_used"))).unwrap_or(0.0) / 1e6;
            let mut out = Sampled { vram_start_mb: vram(&dev), rss_start_mb: rss_bytes().unwrap_or(0.0) / 1e6, ..Default::default() };
            let mut busy_sum = 0.0;
            while !s2.load(Ordering::Relaxed) {
                if let Some(d) = &dev {
                    let b = read_num(&d.join("gpu_busy_percent")).unwrap_or(0.0);
                    busy_sum += b;
                    out.gpu_busy_max = out.gpu_busy_max.max(b);
                }
                out.vram_max_mb = out.vram_max_mb.max(vram(&dev));
                out.rss_max_mb = out.rss_max_mb.max(rss_bytes().unwrap_or(0.0) / 1e6);
                out.samples += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            out.gpu_busy_mean = busy_sum / out.samples.max(1) as f64;
            out
        });
        Self { stop, handle: Some(handle) }
    }

    pub fn finish(mut self) -> Sampled {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().and_then(|h| h.join().ok()).unwrap_or_default()
    }
}

// ------------------------------------------------------------------------------------ sequences

const FPS_29_97: FrameRate = FrameRate::FPS_29_97;

/// Fill scale (%) of a 6240×4160 clip on a `w`×`h` frame (Scale to Frame Fill).
fn fill_scale(w: u32, h: u32) -> f64 {
    100.0 * (w as f64 / 6240.0).max(h as f64 / 4160.0)
}

/// A Lumetri grade with Basic, Curves, HSL Secondary and Vignette in use, plus Sharpen, on every
/// clip of V1. (F-Log2 → Rec.709 conversion arrives in Phase 4; until then this is the grade.)
/// Interpret the media of `seq`'s V1 clips as F-Log2 / F-Gamut (the X-M5's log profile; the
/// synthetic clips are not log-encoded, but the conversion costs the same).
fn interpret_flog2(s: &mut Session, seq: ItemId) {
    s.edit("interpret", |p: &mut Project, _| {
        let media: Vec<ItemId> = p.sequence(seq).map(|q| q.video_tracks[0].items.iter().map(|it| it.item).collect()).unwrap_or_default();
        for id in media {
            if let Some(filmcraft_project::ItemKind::Media(m)) = p.item_mut(id).map(|i| &mut i.kind) {
                m.interpret.color_space = Some(filmcraft_color::ColorSpace::FLog2FGamut);
            }
        }
        Ok(())
    })
    .expect("interpret");
}

/// Make `seq` a DaVinci Wide Gamut / Intermediate sequence (the fork's grading space).
fn set_dwg(s: &mut Session, seq: ItemId) {
    s.edit("dwg", |p: &mut Project, _| {
        if let Some(q) = p.sequence_mut(seq) {
            q.settings.color.working = filmcraft_color::WorkingSpace::DaVinciWide;
            q.settings.working_space = filmcraft_color::WorkingSpace::DaVinciWide.label().into();
        }
        Ok(())
    })
    .expect("dwg");
}

fn grade(s: &mut Session, seq: ItemId) {
    interpret_flog2(s, seq);
    s.edit("grade", |p: &mut Project, _| {
        let q = p.sequence_mut(seq).expect("seq");
        for it in &mut q.video_tracks[0].items {
            let mut l = find_effect("lumetri").expect("lumetri").instance();
            let set: [(&str, ParamValue); 9] = [
                ("temperature", ParamValue::Float(12.0)),
                ("exposure", ParamValue::Float(0.3)),
                ("contrast", ParamValue::Float(20.0)),
                ("saturation", ParamValue::Float(115.0)),
                ("curve_luma", ParamValue::Curve(vec![[0.0, 0.0], [0.25, 0.2], [0.75, 0.82], [1.0, 1.0]])),
                ("hsl_on", ParamValue::Bool(true)),
                ("hsl_hue", ParamValue::Float(30.0)),
                ("hsl_sat", ParamValue::Float(130.0)),
                ("vignette_amount", ParamValue::Float(-1.5)),
            ];
            for (k, v) in set {
                if let Some(x) = l.params.get_mut(k) {
                    x.value = v;
                }
            }
            let mut sh = find_effect("sharpen").expect("sharpen").instance();
            if let Some(x) = sh.params.get_mut("amount") {
                x.value = ParamValue::Float(40.0);
            }
            it.effects.insert(0, l);
            it.effects.insert(1, sh);
        }
        Ok(())
    })
    .expect("grade");
}

/// V1 holds `cuts` (clip, source in-point s, length s) back to back on a `w`×`h` 29.97 sequence,
/// each scaled to fill the frame. Audio of the first cut's clip on A1 when `audio`.
fn cuts_sequence(s: &mut Session, w: u32, h: u32, cuts: &[(ItemId, f64, f64)], audio: bool) -> ItemId {
    let rate = FPS_29_97;
    let scale = fill_scale(w, h);
    s.edit("xm5 sequence", |p: &mut Project, st| {
        let settings = SequenceSettings { width: w, height: h, frame_rate: rate, ..SequenceSettings::default() };
        let seq = p.new_sequence("XM5", settings, 1, 1, None);
        let mut at = Tick::ZERO;
        let (mut vs, mut aus) = (Vec::new(), Vec::new());
        for &(item, src_in, len) in cuts {
            let range = TimeRange::new(rate.snap(Tick::from_seconds_f64(src_in)), rate.snap(Tick::from_seconds_f64(len)));
            let mut v = p.make_track_item(item, TrackKind::Video, at, range, rate).expect("track item");
            for e in &mut v.effects {
                resolve_auto_points(e, (w, h), (6240, 4160));
            }
            if let Some(x) = v.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) {
                x.value = ParamValue::Float(scale);
            }
            if audio && let Some(a) = p.make_track_item(item, TrackKind::Audio, at, range, rate) {
                aus.push(a);
            }
            at += range.duration;
            vs.push(v);
        }
        let q = p.sequence_mut(seq).expect("seq");
        q.video_tracks[0].items = vs;
        q.audio_tracks[0].items = aus;
        st.active_sequence = Some(seq);
        st.open_sequences = vec![seq];
        st.playheads.insert(seq, Tick::ZERO);
        Ok(seq)
    })
    .expect("sequence")
}

fn import(s: &mut Session, name: &str) -> Option<ItemId> {
    let path = clip(name)?;
    Some(playback::import(s, &path))
}

// ------------------------------------------------------------------------------------ section

fn play_row(name: &str, r: &PlayReport, fps: f64, smp: &Sampled, adapter: &str) -> Value {
    let frames = (r.shown + r.dropped).max(1) as f64;
    let mut v = playback::to_json(r, name, "full", if adapter.is_empty() { "cpu" } else { "gpu" });
    v["case"] = json!(name);
    v["adapter"] = json!(adapter);
    v["drop_pct"] = json!(100.0 * r.dropped as f64 / frames);
    v["dropped_per_s"] = json!(r.dropped as f64 / r.seconds.max(1e-9));
    v["cores_needed"] = json!(r.process_cpu / frames * fps / 1000.0);
    v["hw_frames"] = json!(filmcraft_codecs::hw::hw_stats().frames);
    v["sampled"] = smp.json();
    #[cfg(target_os = "linux")]
    {
        v["va_surfaces"] = json!(filmcraft_platform::va::live_surfaces());
    }
    v["decode_vram_mb"] = json!(filmcraft_codecs::vram_usage().0 as f64 / 1e6);
    v["load"] = json!(r.load);
    v["steps"] = json!(r.steps);
    v["gap_ms"] = json!({"p50": playback::pct(&r.gaps, 0.5), "p95": playback::pct(&r.gaps, 0.95), "max": r.gaps.iter().copied().fold(0.0, f64::max)});
    if r.steps.iter().any(|s| *s > 1) || r.label.contains("jkl") {
        eprintln!(
            "xm5 {name}: steps {:?}, new picture every p50 {:.0} / p95 {:.0} / max {:.0} ms ({} pictures)",
            r.steps,
            playback::pct(&r.gaps, 0.5),
            playback::pct(&r.gaps, 0.95),
            r.gaps.iter().copied().fold(0.0, f64::max),
            r.gaps.len() + 1
        );
    }
    eprintln!(
        "xm5 {name}: {}/{} shown/dropped, CPU {:.1} ms/frame, GPU {:.0} %, VRAM {:.0} MB, RSS {:.0} MB, VA surfaces {}, cache VRAM {:.0} MB, GOP seeks {} decoded {} skipped {} evicted {} (load {})",
        r.shown,
        r.dropped,
        r.process_cpu / frames,
        smp.gpu_busy_mean,
        smp.vram_max_mb,
        smp.rss_max_mb,
        v["va_surfaces"],
        filmcraft_codecs::vram_usage().0 as f64 / 1e6,
        r.gop.seeks,
        r.gop.decoded,
        r.gop.skipped,
        r.gop.evicted,
        r.load
    );
    v
}

/// Play `secs` of `seq` at `speed` from `start` (Program monitor at Full, GPU path).
fn play(s: &Session, seq: ItemId, o: &Opts, name: &str, secs: f64, speed: f64, start: Tick, rows: &mut Vec<Value>) {
    play_after(s, seq, o, name, secs, speed, start, None, rows);
}

/// [`play`], first playing 3 s at the same speed from `warm_from` (another part of the timeline)
/// with the same frame server: as the app plays a speed it has played before (it remembers the
/// skimming step per speed).
#[allow(clippy::too_many_arguments)]
fn play_after(s: &Session, seq: ItemId, o: &Opts, name: &str, secs: f64, speed: f64, start: Tick, warm_from: Option<Tick>, rows: &mut Vec<Value>) {
    let fps = s.project.sequence(seq).map(|q| q.settings.frame_rate.as_f64()).unwrap_or(29.97);
    let mut bench = new_bench(s, secs);
    bench.speed = speed;
    let mut display = Display::new(o.gpu);
    if let Some(from) = warm_from {
        bench.seconds = 3.0;
        let _ = bench.play(s, seq, &mut display, 1.0, from, "warm-up");
        bench.server.stop_prefetch();
        bench.seconds = secs;
    }
    let smp = Sampler::start();
    let r = bench.play(s, seq, &mut display, 1.0, start, name);
    let smp = smp.finish();
    rows.push(play_row(name, &r, fps, &smp, &display.adapter));
}

pub fn xm5(o: &Opts) -> Vec<Value> {
    let mut rows = Vec::new();
    let play_s = if o.quick { 8.0 } else { 60.0 };
    if o.wants("ceilings") {
        rows.extend(ceilings(o));
    }
    if o.wants("decode") {
        rows.extend(decode(o));
    }
    if o.only.as_deref().is_some_and(|x| x.contains("open")) {
        // cold source open + first frame per clip (what a cut to a clip not used yet costs)
        let mut s = Session::default();
        for name in [PLAY, P2398, CLIP_B, LONG] {
            let Some(id) = import(&mut s, name) else { continue };
            let t0 = Instant::now();
            let src = s.media.source_for(&s.project, id, s.services.as_ref());
            let opened = ms(t0.elapsed());
            let t1 = Instant::now();
            let ok =
                src.as_ref().map(|x| x.video_frame(filmcraft_media::FrameRequest::full(Tick::from_seconds_f64(1.0))).map(|_| ()).map_err(|e| e.to_string()));
            eprintln!("open {name}: source {opened:.0} ms, first frame {:.0} ms ({ok:?})", ms(t1.elapsed()));
        }
    }
    // (`--only jkl` runs all four JKL cases, `--only "jkl J 1x"` one of them)
    // (name, speed, from (s), warm-up from (s): the speed played before elsewhere)
    const JKL: [(&str, f64, f64, Option<f64>); 6] = [
        ("xm5-jkl L 2x", 2.0, 0.0, None),
        ("xm5-jkl L 4x", 4.0, 0.0, None),
        ("xm5-jkl L 4x warm", 4.0, 0.0, Some(45.0)),
        ("xm5-jkl J 1x", -1.0, 50.0, None),
        ("xm5-jkl J 2x", -2.0, 50.0, None),
        ("xm5-jkl J 2x warm", -2.0, 50.0, Some(15.0)),
    ];
    if o.wants("play-full")
        || o.wants("play-half-draft")
        || o.wants("play-flog2")
        || o.wants("play-dwg")
        || o.wants("play-graded")
        || JKL.iter().any(|(name, ..)| o.wants(name))
    {
        let mut s = Session::default();
        if let Some(a) = import(&mut s, PLAY) {
            let seq = cuts_sequence(&mut s, 3840, 2160, &[(a, 0.0, 60.0)], true);
            for _ in 0..o.repeat {
                if o.wants("play-full") {
                    play(&s, seq, o, "xm5-play-full", play_s, 1.0, Tick::ZERO, &mut rows);
                }
                if o.wants("play-half-draft") {
                    // Program monitor at 1/2 with Settings ▸ Playback ▸ Draft decoding (the GPU
                    // scales hardware frames: no more CPU work than at Full)
                    let fps = s.project.sequence(seq).map(|q| q.settings.frame_rate.as_f64()).unwrap_or(29.97);
                    let mut bench = new_bench(&s, play_s);
                    bench.draft = true;
                    let mut display = Display::new(o.gpu);
                    let smp = Sampler::start();
                    let r = bench.play(&s, seq, &mut display, 0.5, Tick::ZERO, "xm5-play-half-draft");
                    rows.push(play_row("xm5-play-half-draft", &r, fps, &smp.finish(), &display.adapter));
                }
                let secs = if o.quick { 4.0 } else { 10.0 };
                for (name, speed, from, warm) in JKL {
                    if o.wants(name) {
                        play_after(&s, seq, o, name, secs, speed, Tick::from_seconds_f64(from), warm.map(Tick::from_seconds_f64), &mut rows);
                    }
                }
            }
            if o.wants("play-flog2") {
                interpret_flog2(&mut s, seq);
                for _ in 0..o.repeat {
                    play(&s, seq, o, "xm5-play-flog2", play_s, 1.0, Tick::ZERO, &mut rows);
                }
            }
            if o.wants("play-dwg") {
                interpret_flog2(&mut s, seq);
                set_dwg(&mut s, seq);
                for _ in 0..o.repeat {
                    play(&s, seq, o, "xm5-play-dwg", play_s, 1.0, Tick::ZERO, &mut rows);
                }
            }
            if o.wants("play-graded") {
                grade(&mut s, seq);
                for _ in 0..o.repeat {
                    play(&s, seq, o, "xm5-play-graded", play_s, 1.0, Tick::ZERO, &mut rows);
                }
            }
        } else {
            eprintln!("xm5 play: skipped (clip unavailable)");
        }
    }
    if o.wants("scrub") {
        rows.extend(scrub(o));
    }
    if o.wants("multicut") {
        let mut s = Session::default();
        let ids: Option<Vec<ItemId>> = [PLAY, P2398, CLIP_B, LONG].iter().map(|n| import(&mut s, n)).collect();
        if let Some(ids) = ids {
            // 20 cuts of 3 s, alternating A B C D, each from a different place in its clip
            let cuts: Vec<(ItemId, f64, f64)> = (0..20).map(|i| (ids[i % 4], 1.0 + (i / 4) as f64 * 5.3, 3.0)).collect();
            let seq = cuts_sequence(&mut s, 3840, 2160, &cuts, false);
            for _ in 0..o.repeat {
                play(&s, seq, o, "xm5-multicut", if o.quick { 12.0 } else { 60.0 }, 1.0, Tick::ZERO, &mut rows);
            }
        }
    }
    if o.wants("export") {
        rows.extend(export(o));
    }
    // only when asked for by name: hours long
    if o.only.as_deref().is_some_and(|x| x.contains("soak")) {
        rows.extend(soak(o));
    }
    if o.wants("422-play") {
        let mut s = Session::default();
        if let Some(a) = import(&mut s, C422) {
            let seq = cuts_sequence(&mut s, 3840, 2160, &[(a, 0.0, 20.0)], true);
            for _ in 0..o.repeat {
                play(&s, seq, o, "xm5-422-play", if o.quick { 8.0 } else { 19.0 }, 1.0, Tick::ZERO, &mut rows);
            }
        }
    }
    rows
}

// ------------------------------------------------------------------------------------ ceilings

/// Frames in a clip according to ffprobe-free arithmetic on FilmCraft's own media info.
fn clip_frames(path: &Path) -> Option<i64> {
    let mut s = Session::default();
    let id = playback::import(&mut s, path);
    match s.project.item(id).map(|i| &i.kind) {
        Some(filmcraft_project::ItemKind::Media(m)) => {
            let v = m.info.video.as_ref()?;
            Some(v.frame_rate.frame_at(m.info.duration))
        }
        _ => None,
    }
}

/// Run ffmpeg and return (frames/s, wall s).
fn timed(ff: &Path, args: &[&str], frames: i64) -> Option<(f64, f64)> {
    let t0 = Instant::now();
    let out = Command::new(ff).args(["-hide_banner", "-v", "error", "-nostdin"]).args(args).output().ok()?;
    let dt = t0.elapsed().as_secs_f64();
    if !out.status.success() {
        eprintln!("ffmpeg {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
        return None;
    }
    Some((frames as f64 / dt, dt))
}

/// The hardware ceilings of FORK-PLAN §4, measured with the system ffmpeg (an oracle only).
fn ceilings(o: &Opts) -> Vec<Value> {
    let mut rows = Vec::new();
    let Some(ff) = crate::fixtures::ffmpeg() else {
        eprintln!("ceilings: no ffmpeg");
        return rows;
    };
    let dev = filmcraft_platform::ffmpeg::vaapi_device().unwrap_or("/dev/dri/renderD128").to_string();
    let (Some(clip420), Some(clip422)) = (clip(PLAY), clip(C422)) else {
        return rows;
    };
    let n420 = clip_frames(&clip420).unwrap_or(0);
    let n422 = clip_frames(&clip422).unwrap_or(0);
    let lim = if o.quick { "300" } else { "100000" };
    let n420 = if o.quick { n420.min(300) } else { n420 };
    let c420 = clip420.to_string_lossy().to_string();
    let c422 = clip422.to_string_lossy().to_string();
    let mut push = |what: &str, cmd: String, r: Option<(f64, f64)>| {
        if let Some((fps, wall)) = r {
            eprintln!("ceiling {what}: {fps:.1} fps");
            rows.push(json!({"case": format!("ceiling {what}"), "fps": fps, "wall_s": wall, "command": cmd, "load": load_avg()}));
        }
    };
    let hw = ["-hwaccel", "vaapi", "-hwaccel_device", &dev, "-hwaccel_output_format", "vaapi"];
    // decode, frames stay on the GPU
    let a: Vec<&str> = hw.iter().copied().chain(["-i", &c420, "-frames:v", lim, "-an", "-f", "null", "-"]).collect();
    push("decode 6.2K 4:2:0 (frames stay on GPU)", format!("ffmpeg {}", a.join(" ")), timed(&ff, &a, n420));
    // decode + download
    let a: Vec<&str> = hw.iter().copied().chain(["-i", &c420, "-frames:v", lim, "-an", "-vf", "hwdownload,format=p010", "-f", "null", "-"]).collect();
    push("decode 6.2K 4:2:0 + download P010", format!("ffmpeg {}", a.join(" ")), timed(&ff, &a, n420));
    // encode from frames already on the GPU (decode → scale_vaapi → hevc_vaapi), 4K and 6.2K
    for (label, vf) in
        [("encode 3840x2160 Main10 (GPU source)", "scale_vaapi=w=3840:h=2160:format=p010"), ("encode 6240x4160 Main10 (GPU source)", "scale_vaapi=format=p010")]
    {
        let a: Vec<&str> = hw
            .iter()
            .copied()
            .chain(["-i", &c420, "-frames:v", lim, "-an", "-vf", vf, "-c:v", "hevc_vaapi", "-profile:v", "main10", "-b:v", "100M", "-f", "null", "-"])
            .collect();
        push(label, format!("ffmpeg {}", a.join(" ")), timed(&ff, &a, n420));
    }
    // 4:2:2 software decode (the GPU can't)
    let a = ["-threads", "0", "-i", &c422, "-an", "-f", "null", "-"];
    push("CPU software decode 6.2K 4:2:2", format!("ffmpeg {}", a.join(" ")), timed(&ff, &a, n422));
    let a = ["-threads", "0", "-i", &c420, "-frames:v", lim, "-an", "-f", "null", "-"];
    push("CPU software decode 6.2K 4:2:0", format!("ffmpeg {}", a.join(" ")), timed(&ff, &a, n420));
    rows
}

// ------------------------------------------------------------------------------------ decode

/// Sequential decode of the 6.2K clip through FilmCraft's media stack (container source, GOP
/// cache, registered decoder, conversion), as playback reads it.
fn decode(o: &Opts) -> Vec<Value> {
    let mut rows = Vec::new();
    for name in [PLAY, C422] {
        let Some(path) = clip(name) else { continue };
        let mut s = Session::default();
        let id = playback::import(&mut s, &path);
        let Some(src) = s.media.source_for(&s.project, id, s.services.as_ref()) else {
            eprintln!("xm5 decode {name}: no source");
            continue;
        };
        let info = src.info();
        let rate = info.frame_rate();
        let n = rate.frame_at(info.duration).max(1);
        let n = if o.quick { n.min(120) } else { n.min(600) };
        let hw0 = filmcraft_codecs::hw::hw_stats().frames;
        let smp = Sampler::start();
        let (t0, c0) = (Instant::now(), filmcraft_ui_egui::frames::process_cpu_time().unwrap_or_default());
        let mut done = 0;
        for f in 0..n {
            if let Err(e) = src.video_frame(filmcraft_media::FrameRequest::full(rate.tick_of(f))) {
                eprintln!("xm5 decode {name} frame {f}: {e}");
                break;
            }
            done += 1;
        }
        let dt = t0.elapsed().as_secs_f64();
        let cpu = ms(filmcraft_ui_egui::frames::process_cpu_time().unwrap_or_default().saturating_sub(c0));
        let smp = smp.finish();
        let fps = done as f64 / dt;
        eprintln!("xm5 decode {name}: {fps:.1} fps, CPU {:.1} ms/frame", cpu / done.max(1) as f64);
        rows.push(json!({
            "case": format!("decode {name}"), "frames": done, "fps": fps, "cpu_ms_per_frame": cpu / done.max(1) as f64,
            "hw_frames": filmcraft_codecs::hw::hw_stats().frames - hw0, "sampled": smp.json(), "load": load_avg(),
        }));
    }
    rows
}

// ------------------------------------------------------------------------------------ scrub

/// 50 random seeks across the 5-minute clip: time until the exact full-resolution frame is on
/// screen.
fn scrub(o: &Opts) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut s = Session::default();
    let Some(a) = import(&mut s, LONG) else {
        eprintln!("xm5 scrub: skipped (clip unavailable)");
        return rows;
    };
    let seq = cuts_sequence(&mut s, 3840, 2160, &[(a, 0.0, 300.0)], false);
    let seeks = if o.quick { 10 } else { 50 };
    for _ in 0..o.repeat {
        let bench = new_bench(&s, 0.0);
        let mut display = Display::new(o.gpu);
        let smp = Sampler::start();
        let r = bench.scrub(&s, seq, &mut display, 1.0, seeks, 0, 0, Duration::from_secs(20));
        let smp = smp.finish();
        eprintln!("xm5 scrub: p50 {:.0} ms, p95 {:.0} ms, {} timeouts", pct(&r.seek_ms, 0.5), pct(&r.seek_ms, 0.95), r.timeouts);
        rows.push(json!({
            "case": "xm5-scrub", "seeks": seeks, "seek_p50_ms": pct(&r.seek_ms, 0.5), "seek_p95_ms": pct(&r.seek_ms, 0.95),
            "seek_max_ms": pct(&r.seek_ms, 1.0), "timeouts": r.timeouts, "decoded_per_seek": r.gop.decoded as f64 / seeks as f64,
            "process_cpu_ms": r.process_cpu_ms, "adapter": display.adapter, "sampled": smp.json(), "load": r.load,
        }));
    }
    rows
}

// ------------------------------------------------------------------------------------ export

/// 60 s of the 6.2K clip exported to HEVC Main10: 3840×2160 ungraded and graded, 6240×4160.
fn export(o: &Opts) -> Vec<Value> {
    let mut rows = Vec::new();
    let secs = if o.quick { 5.0 } else { 60.0 };
    let dir = std::env::temp_dir().join(format!("filmcraft-bench-xm5-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return rows;
    }
    let cases: [(&str, u32, u32, bool); 3] =
        [("xm5-export-4k", 3840, 2160, false), ("xm5-export-4k graded", 3840, 2160, true), ("xm5-export-6k", 6240, 4160, false)];
    for (name, w, h, graded) in cases {
        if !o.wants(name) && !o.wants("export") {
            continue;
        }
        let mut s = Session::default();
        let Some(a) = import(&mut s, PLAY) else { break };
        let seq = cuts_sequence(&mut s, w, h, &[(a, 0.0, secs)], true);
        if graded {
            grade(&mut s, seq);
        }
        let frames = s.project.sequence(seq).map(|q| q.settings.frame_rate.frame_at(q.duration())).unwrap_or(0);
        for _ in 0..o.repeat {
            let out = dir.join("out.mp4");
            let smp = Sampler::start();
            let (t0, c0) = (Instant::now(), filmcraft_ui_egui::frames::process_cpu_time().unwrap_or_default());
            let r = s.execute(
                "file.exportMedia",
                json!({"path": out.to_string_lossy(), "format": "hevc", "bitDepth": "10", "bitrateKbps": 100_000, "bitrateMode": "vbr1Pass", "wait": true}),
            );
            let dt = t0.elapsed().as_secs_f64();
            let cpu = ms(filmcraft_ui_egui::frames::process_cpu_time().unwrap_or_default().saturating_sub(c0));
            let smp = smp.finish();
            if let Err(e) = r {
                eprintln!("{name}: {e}");
                break;
            }
            if let Some(e) = s.jobs.last().and_then(|j| j.progress.error.lock().ok().and_then(|g| g.clone())) {
                eprintln!("{name}: {e}");
                break;
            }
            let fps = frames as f64 / dt;
            eprintln!("{name}: {frames} frames in {dt:.1} s = {fps:.1} fps");
            rows.push(json!({
                "case": name, "frames": frames, "seconds": dt, "fps": fps, "realtime": fps / FPS_29_97.as_f64(),
                "cpu_ms_per_frame": cpu / frames.max(1) as f64, "mbytes": std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) as f64 / 1e6,
                "sampled": smp.json(), "load": load_avg(),
            }));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    rows
}

// ------------------------------------------------------------------------------------ soak

/// Play the 6.2K sequence in a loop for `FILMCRAFT_SOAK_MINUTES` (default 120) and sample memory
/// every minute: RSS, GPU memory in use, the decode cache's GPU memory and live VA surfaces must
/// not grow (FORK-PLAN Phase 2 acceptance).
fn soak(o: &Opts) -> Vec<Value> {
    let minutes: u64 = std::env::var("FILMCRAFT_SOAK_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let mut s = Session::default();
    let Some(a) = import(&mut s, PLAY) else { return Vec::new() };
    let seq = cuts_sequence(&mut s, 3840, 2160, &[(a, 0.0, 60.0)], true);
    let bench = new_bench(&s, 55.0);
    let mut display = Display::new(o.gpu);
    let dev = amd_gpu_sysfs();
    let vram = || dev.as_ref().and_then(|d| read_num(&d.join("mem_info_vram_used"))).unwrap_or(0.0) / 1e6;
    let t0 = Instant::now();
    let mut samples = Vec::new();
    let (mut shown, mut dropped, mut plays) = (0u64, 0u64, 0u64);
    while t0.elapsed() < Duration::from_secs(minutes * 60) {
        let r = bench.play(&s, seq, &mut display, 1.0, Tick::ZERO, "soak");
        shown += r.shown;
        dropped += r.dropped;
        plays += 1;
        #[cfg(target_os = "linux")]
        let surfaces = filmcraft_platform::va::live_surfaces();
        #[cfg(not(target_os = "linux"))]
        let surfaces = 0;
        let row = json!({
            "minute": t0.elapsed().as_secs_f64() / 60.0, "rss_mb": rss_bytes().unwrap_or(0.0) / 1e6, "vram_mb": vram(),
            "cache_vram_mb": filmcraft_codecs::vram_usage().0 as f64 / 1e6, "va_surfaces": surfaces, "shown": r.shown, "dropped": r.dropped,
        });
        eprintln!("soak {row}");
        samples.push(row);
    }
    let first = samples.first().cloned().unwrap_or_default();
    let last = samples.last().cloned().unwrap_or_default();
    let grow = |k: &str| last[k].as_f64().unwrap_or(0.0) - first[k].as_f64().unwrap_or(0.0);
    vec![json!({
        "case": "xm5-soak", "minutes": minutes, "plays": plays, "shown": shown, "dropped": dropped,
        "rss_growth_mb": grow("rss_mb"), "vram_growth_mb": grow("vram_mb"), "cache_vram_growth_mb": grow("cache_vram_mb"),
        "va_surfaces_growth": grow("va_surfaces"), "samples": samples, "load": load_avg(),
    })]
}
