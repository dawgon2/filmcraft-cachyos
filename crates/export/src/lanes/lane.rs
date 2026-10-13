//! A lane of a two-GPU export (fork 6E.3, DESIGN-6E3 §2.2), and the decision to run lanes.
//!
//! A lane owns everything that touches its GPU: a rayon pool whose threads (and the lane thread
//! itself) bind hardware decoders to the lane's render node, a copy of the sources (decoders of
//! its own, [`SourceProvider::lane_copy`]), a [`Pipeline`] rendering on its GPU, and a fresh
//! encoder per chunk on its GPU. The primary lane (node `None`) uses the process's displays and
//! the registered renderer; it too gets a source copy, so its threads can outlive any one
//! [`crate::job::Exporter::step`] (the caller's provider keeps serving the audio).
//!
//! Choices (DESIGN-6E3 §4 took the recommended answers):
//! * 4:2:2 sources: a chunk showing a 4:2:2 clip is left to a lane whose GPU decodes 4:2:2 (Intel
//!   here) while one is alive; the other lane takes such a chunk only when nothing else waits.
//!   (`display_for` would send its decode to the 4:2:2 GPU anyway: this keeps the AMD lane off
//!   the Intel decoder.)
//! * One `hvc1` entry only when the lanes' drivers share a stream **and** a probe encoder on each
//!   lane reports the very same sample entry; otherwise in-band parameter sets (`hev1`).
//! * A lane whose GPU has no export renderer fails at start (the other lane takes the chunks);
//!   its CPU-fallback frames (HDR, overlays) render through the default GPU's frame renderers (Q3).

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use filmcraft_project::{ItemId, Project};
use filmcraft_render::SourceProvider;
use filmcraft_time::{FrameRate, Tick};

use super::chunker::{self, Chunk};
use super::coordinator::{ChunkDone, Coordinator, LaneSpec, LaneWork};
use super::spill::{SpillPacket, SpillWriter};
use super::{LaneDevices, LaneEncoders, entries_agree, lane_devices, mem_available};
use crate::job::{Frame, IN_FLIGHT_BUDGET, RenderMode, frame_bytes, overlap_batch, render_batch};
use crate::pipeline::Pipeline;
use crate::settings::{ExportEngines, HevcEncoder};
use crate::{EncoderFrame, ExportSettings, Format, GpuFrames, Stage, VideoEncoder, timed};

/// Nominal chunk length in seconds (DESIGN-6E3 Q1).
pub const CHUNK_SECONDS: i64 = 8;
/// Least `MemAvailable` for two lanes (DESIGN-6E3 Q8): the Intel lane's surfaces live in RAM.
pub const MIN_MEM_AVAILABLE: u64 = 6 << 30;
/// A lane holding a chunk with no frame encoded for this long is given up on (a hung GPU).
pub const WATCHDOG: Duration = Duration::from_secs(60);

/// Two lanes running for an export.
pub(crate) struct Dual {
    pub coord: Coordinator,
    /// Parameter sets travel in band (`hev1`).
    pub in_band: bool,
}

/// What [`start`] needs of the exporter.
pub(crate) struct DualInputs<'a> {
    pub settings: &'a ExportSettings,
    pub project: &'a Arc<Project>,
    pub seq: ItemId,
    pub hdr_out: bool,
    pub w: u32,
    pub h: u32,
    pub rate: FrameRate,
    pub f0: i64,
    pub f1: i64,
    pub two_pass: bool,
}

/// Start two lanes for this export when [`ExportSettings::export_engines`] and the machine allow
/// it; `Err` says why the export stays on one GPU (not an error: the caller runs single).
pub(crate) fn start(i: &DualInputs<'_>, sources: &dyn SourceProvider) -> std::result::Result<Dual, String> {
    let s = i.settings;
    if s.export_engines == ExportEngines::Single {
        return Err("Export engines: one GPU".into());
    }
    if s.video_format() != Format::Hevc || s.hevc_encoder == HevcEncoder::X265 {
        return Err("two GPUs encode HEVC through VA-API only".into());
    }
    if s.hevc_chroma_422 {
        return Err("4:2:2 is encoded by one GPU".into());
    }
    if s.part_of_batch || s.sink.is_some() || i.two_pass {
        return Err("not for render-cache parts, in-memory or two-pass exports".into());
    }
    if cfg!(target_arch = "wasm32") || rayon::current_num_threads() < 4 {
        return Err("fewer than 4 threads".into());
    }
    let devices = lane_devices().ok_or("no second GPU support on this platform")?;
    let second = devices.second_node().ok_or("no second GPU with an HEVC encoder")?;
    match mem_available() {
        Some(m) if m >= MIN_MEM_AVAILABLE => {}
        Some(m) => return Err(format!("{} MB of memory available, two GPUs need {} MB", m >> 20, MIN_MEM_AVAILABLE >> 20)),
        None => return Err("available memory unknown".into()),
    }
    let keyint = i64::from(s.keyframe_distance.unwrap_or(0).max(1));
    let nominal = CHUNK_SECONDS.saturating_mul(i.rate.num) / i.rate.den.max(1);
    let (cuts, transitions) = cut_points(i.project, i.seq, i.rate);
    let chunks = chunker::plan(i.f0, i.f1, keyint, nominal, 2, &cuts, &transitions);
    if chunks.len() < 2 {
        return Err("one chunk: nothing to share".into());
    }
    let nodes = vec![None, Some(second)];
    let encs = devices.encoders(i.w, i.h, i.rate, s, &nodes).ok_or("the GPUs can't encode this export")?;
    let in_band = !encs.shared_stream() || {
        // FFI review (S7): one hvc1 entry only when every lane's encoder writes the very same one
        let mut entries = Vec::with_capacity(nodes.len());
        for lane in 0..nodes.len() {
            entries.push(encs.make(lane, false).map_err(|e| format!("lane {lane} encoder: {e}"))?.sample_entry());
        }
        !entries_agree(&entries)
    };
    let decodes: Vec<bool> = nodes.iter().map(|n| devices.decodes_422(n.as_deref())).collect();
    let needs_422 = if decodes.iter().all(|&d| d == decodes.first().copied().unwrap_or(false)) {
        HashSet::new()
    } else {
        chunks_with_422(i.project, i.seq, i.rate, &chunks, sources)
    };
    let total = rayon::current_num_threads();
    let mut specs = Vec::with_capacity(nodes.len());
    for (lane, node) in nodes.iter().enumerate() {
        let src = sources.lane_copy().ok_or("the sources can't be opened twice")?;
        let threads = if lane == 0 { total.saturating_sub(4).max(2) } else { 4 };
        let budget = if lane == 0 { IN_FLIGHT_BUDGET } else { IN_FLIGHT_BUDGET / 2 };
        let ctx = LaneCtx {
            devices,
            node: node.clone(),
            lane,
            threads,
            budget,
            encs: encs.clone(),
            in_band,
            settings: s.clone(),
            project: i.project.clone(),
            seq: i.seq,
            hdr_out: i.hdr_out,
            sources: src,
        };
        specs.push(LaneSpec {
            name: devices.name(node.as_deref()),
            node: node.clone(),
            decodes_422: decodes.get(lane).copied().unwrap_or(false),
            make: Box::new(move || ctx.build()),
        });
    }
    let coord = Coordinator::start(chunks, specs, &chunk_dir(&s.path), needs_422, !in_band, WATCHDOG).map_err(|e| e.to_string())?;
    Ok(Dual { coord, in_band })
}

/// The spill directory of an export to `path`: next to it (same filesystem, DESIGN-6E3 Q5), with
/// a name of this run's own (process id and a counter), so it is never a directory the user owns
/// nor one a lane abandoned by an earlier run still writes into. Removed on every exit path.
pub(crate) fn chunk_dir(path: &str) -> PathBuf {
    static RUN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    PathBuf::from(format!("{path}.{}-{run}-{nanos:08x}.chunks", std::process::id()))
}

/// The output frames where an enabled clip on an enabled video track starts or ends, and the
/// frame ranges of the transitions on those tracks.
fn cut_points(project: &Project, seq: ItemId, rate: FrameRate) -> (Vec<i64>, Vec<(i64, i64)>) {
    let (mut cuts, mut transitions) = (Vec::new(), Vec::new());
    let Some(q) = project.sequence(seq) else { return (cuts, transitions) };
    let end_frame = |t: Tick| rate.frame_at(t - Tick(1)).saturating_add(1);
    for track in q.video_tracks.iter().filter(|t| t.enabled) {
        for item in track.items.iter().filter(|it| it.enabled) {
            cuts.push(rate.frame_at(item.start));
            cuts.push(rate.frame_at(item.end()));
        }
        for t in &track.transitions {
            transitions.push((rate.frame_at(t.start), end_frame(t.end())));
        }
    }
    (cuts, transitions)
}

/// The chunks showing a clip whose source is 4:2:2.
fn chunks_with_422(project: &Project, seq: ItemId, rate: FrameRate, chunks: &[Chunk], sources: &dyn SourceProvider) -> HashSet<usize> {
    let mut out = HashSet::new();
    let Some(q) = project.sequence(seq) else { return out };
    for track in q.video_tracks.iter().filter(|t| t.enabled) {
        for item in track.items.iter().filter(|it| it.enabled) {
            let is_422 = sources.source(item.item).is_some_and(|src| src.info().video.as_ref().is_some_and(|v| v.pixel_format.contains("422")));
            if !is_422 {
                continue;
            }
            let (a, b) = (rate.frame_at(item.start), rate.frame_at(item.end()));
            out.extend(chunks.iter().filter(|c| c.start < b && a < c.end).map(|c| c.index));
        }
    }
    out
}

/// Everything a lane thread builds its work from.
struct LaneCtx {
    devices: &'static dyn LaneDevices,
    node: Option<String>,
    lane: usize,
    threads: usize,
    budget: u64,
    encs: Arc<dyn LaneEncoders>,
    in_band: bool,
    settings: ExportSettings,
    project: Arc<Project>,
    seq: ItemId,
    hdr_out: bool,
    sources: Arc<dyn SourceProvider + Send + Sync>,
}

impl LaneCtx {
    /// On the lane thread: bind its decoders, build its pool and pipeline.
    fn build(self) -> std::result::Result<Box<dyn LaneWork>, String> {
        let devices = self.devices;
        devices.bind_thread(self.node.as_deref());
        let (lane, node) = (self.lane, self.node.clone());
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads.max(1))
            .thread_name(move |k| format!("export-lane{lane}-{k}"))
            // every worker makes decoders (frames decode lazily where they are drawn)
            .start_handler(move |_| devices.bind_thread(node.as_deref()))
            .build()
            .map_err(|e| format!("thread pool: {e}"))?;
        let gpu: Option<Arc<dyn GpuFrames>> = match &self.node {
            Some(n) => Some(Arc::from(devices.frames_on(n).ok_or_else(|| format!("no export renderer on {n}"))?)),
            None => None,
        };
        let pipe = pool.install(|| Pipeline::new(self.project.clone(), self.seq, &self.settings, self.hdr_out)).map_err(|e| e.to_string())?.for_lane(gpu);
        let batch = overlap_batch(i64::try_from(self.threads.clamp(2, 16)).unwrap_or(2), frame_bytes(pipe.w, pipe.h, pipe.hdr_out), self.budget);
        Ok(Box::new(Lane { pipe, sources: self.sources, pool, encs: self.encs, lane: self.lane, in_band: self.in_band, settings: self.settings, batch }))
    }
}

struct Lane {
    pipe: Pipeline,
    sources: Arc<dyn SourceProvider + Send + Sync>,
    pool: rayon::ThreadPool,
    encs: Arc<dyn LaneEncoders>,
    lane: usize,
    in_band: bool,
    settings: ExportSettings,
    batch: i64,
}

impl LaneWork for Lane {
    fn run_chunk(&mut self, chunk: Chunk, out: &Path, keep_going: &dyn Fn(u64) -> bool) -> std::result::Result<ChunkDone, String> {
        let mut enc = self.encs.make(self.lane, self.in_band).map_err(|e| e.to_string())?;
        let mode = RenderMode::of(&self.settings, &self.pipe, enc.as_ref());
        let mut w = SpillWriter::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let (pipe, sources, pool, batch) = (&self.pipe, &*self.sources, &self.pool, self.batch.max(1));
        let not_ready = || "media data not ready".to_string();
        let mut count = 0u64;
        let mut f = chunk.start;
        let mut ahead = None;
        while f < chunk.end {
            let end = f.saturating_add(batch).min(chunk.end);
            let frames = match ahead.take() {
                Some(frames) => frames,
                None => timed(Stage::Render, || pool.install(|| render_batch(pipe, f, end, sources, &mode))).ok_or_else(not_ready)?,
            };
            // the next batch renders on the lane's pool while this one is encoded here
            let next = (end < chunk.end).then(|| (end, end.saturating_add(batch).min(chunk.end)));
            let mut slot = None;
            let encoded = pool.in_place_scope(|s| {
                if let Some((a, b)) = next {
                    let slot = &mut slot;
                    let mode = &mode;
                    s.spawn(move |_| *slot = Some(catch_unwind(AssertUnwindSafe(|| timed(Stage::Render, || render_batch(pipe, a, b, sources, mode))))));
                }
                encode_into(enc.as_mut(), &mut w, pipe, frames, u64::try_from(f.saturating_sub(chunk.start)).unwrap_or(0))
            });
            let n = encoded?;
            count = count.saturating_add(n);
            ahead = match slot {
                Some(Ok(Some(frames))) => Some(frames),
                Some(Ok(None)) => return Err(not_ready()),
                Some(Err(_)) => return Err("internal error while rendering a frame (the panic was logged by the panic hook)".into()),
                None => None,
            };
            if !keep_going(u64::try_from(end.saturating_sub(f)).unwrap_or(0)) {
                return Err("cancelled".into());
            }
            f = end;
        }
        for p in timed(Stage::Encode, || enc.flush()).map_err(|e| e.to_string())? {
            w.push(&SpillPacket { data: p.data, key: p.key, duration: p.duration, cts: p.composition_offset }).map_err(|e| e.to_string())?;
            count = count.saturating_add(1);
        }
        w.finish().map_err(|e| e.to_string())?;
        if Some(count) != u64::try_from(chunk.len()).ok() {
            return Err(format!("the encoder gave {count} pictures for {} frames", chunk.len()));
        }
        Ok(ChunkDone { entry: enc.sample_entry(), timescale: enc.timescale(), media_start: enc.media_start() })
    }
}

/// Encode `frames` (chunk-relative index `first` on) and spill their packets; the packet count.
fn encode_into(enc: &mut dyn VideoEncoder, w: &mut SpillWriter, pipe: &Pipeline, frames: Vec<Frame>, first: u64) -> std::result::Result<u64, String> {
    let mut n = 0u64;
    for (k, (rgba, hdr, yuv, gpu)) in frames.into_iter().enumerate() {
        let fr = EncoderFrame {
            width: pipe.w,
            height: pipe.h,
            rgba: &rgba,
            hdr: pipe.hdr_out.then_some(hdr.as_slice()),
            yuv: yuv.as_deref(),
            gpu,
            index: first.saturating_add(k as u64),
        };
        for p in timed(Stage::Encode, || enc.encode(&fr)).map_err(|e| e.to_string())? {
            w.push(&SpillPacket { data: p.data, key: p.key, duration: p.duration, cts: p.composition_offset }).map_err(|e| e.to_string())?;
            n = n.saturating_add(1);
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chunk_directory_sits_next_to_the_output() {
        let (a, b) = (chunk_dir("/x/out.mp4"), chunk_dir("/x/out.mp4"));
        assert_ne!(a, b, "a name of each run's own");
        for d in [a, b] {
            let name = d.to_string_lossy().into_owned();
            assert!(name.starts_with(&format!("/x/out.mp4.{}-", std::process::id())) && name.ends_with(".chunks"), "{name}");
        }
    }
}
