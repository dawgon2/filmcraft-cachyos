//! The lanes' threads and the joiner's side of them (fork 6E.3, DESIGN-6E3 §2.4 / §2.5).
//!
//! Each lane runs on a thread of its own: it pulls a chunk from the [`Scheduler`], encodes it
//! into a spill file (`<dir>/<chunk>.<lane>.pkts`) and reports it. The coordinator hands the
//! chunks back **in file order** ([`Coordinator::next_chunk`]) to the exporter, which muxes them.
//!
//! Failure rules (never crash, never hang):
//! * A lane that fails a chunk (an error, a panic, or no progress for the watchdog time) is dead:
//!   the coordinator puts its chunk back at the front of the queue for the other lanes and
//!   ignores whatever the lane reports afterwards (a hung thread is left behind, detached; its
//!   file name is its own, so it can't corrupt another lane's attempt).
//! * Only the coordinator settles a pulled chunk (`done` / `requeue`), so a zombie lane can't
//!   unbalance the queue's count. A lane killed between pulling and announcing a chunk puts it
//!   back itself; one announced by a lane already dead is put back by the coordinator.
//! * Every lane dead: the queue is cancelled (a lane waiting on it returns) and the export ends
//!   with an error. Cancel: lanes stop at their next batch, the queue is cancelled.

use std::collections::{BTreeMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use filmcraft_isobmff::SampleEntry;

use super::chunker::Chunk;
use super::scheduler::Scheduler;
use super::spill::{self, SpillPacket};
use super::{LaneReport, entries_agree};
use crate::{ExportError, Result};

/// A lane's work: encode one chunk into a spill file.
pub trait LaneWork {
    /// Encode `chunk` into the spill file `out` (one packet per frame, in order, an IDR first).
    /// Call `keep_going(frames)` after every batch with the frames just encoded; `false` means
    /// stop now (cancelled, or the lane was given up on).
    fn run_chunk(&mut self, chunk: Chunk, out: &Path, keep_going: &dyn Fn(u64) -> bool) -> std::result::Result<ChunkDone, String>;
}

/// What the encoder of a finished chunk said about its stream.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkDone {
    pub entry: SampleEntry,
    pub timescale: u32,
    pub media_start: Option<i64>,
}

/// Builds a lane's work on the lane's own thread (its GPU, pool and decoders live there).
pub type MakeLane = Box<dyn FnOnce() -> std::result::Result<Box<dyn LaneWork>, String> + Send>;

/// One lane to start.
pub struct LaneSpec {
    pub name: String,
    pub node: Option<String>,
    /// Its GPU decodes 4:2:2: chunks of 4:2:2 sources prefer it.
    pub decodes_422: bool,
    pub make: MakeLane,
}

enum Event {
    /// The lane's work was built (`make` returned): from now on it waits for or holds chunks.
    Ready {
        lane: usize,
    },
    Started {
        lane: usize,
        chunk: Chunk,
    },
    Frames {
        lane: usize,
        n: u64,
    },
    Done {
        lane: usize,
        chunk: usize,
        path: PathBuf,
        done: Box<ChunkDone>,
    },
    Failed {
        lane: usize,
        why: String,
    },
    Exited {
        lane: usize,
    },
}

struct LaneState {
    name: String,
    node: Option<String>,
    decodes_422: bool,
    /// Set when the coordinator gave up on the lane: it stops and settles nothing more.
    killed: Arc<AtomicBool>,
    alive: bool,
    exited: bool,
    /// Its `make` returned (a lane stuck building is given up on by the watchdog too).
    ready: bool,
    held: Option<Chunk>,
    /// Frames of `held` already counted in the progress.
    held_frames: u64,
    /// When it took `held`, and its time spent on finished chunks (its speed, for the slow-lane rule).
    held_since: Option<Instant>,
    busy: Duration,
    last: Instant,
    chunks: u64,
    frames: u64,
    failed: Option<String>,
    /// Set instead of `failed` when the slow-lane rule gave up on it.
    retired: Option<String>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// A finished chunk waiting for its turn in file order.
struct Ready {
    path: PathBuf,
    done: ChunkDone,
}

/// Runs the lanes and hands their chunks back in order.
pub struct Coordinator {
    chunks: Vec<Chunk>,
    sched: Arc<Scheduler>,
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<Event>,
    lanes: Vec<LaneState>,
    /// Lanes alive whose GPU decodes 4:2:2 (the others leave 4:2:2 chunks to them meanwhile).
    lanes_422: Arc<AtomicUsize>,
    needs_422: Arc<HashSet<usize>>,
    /// The slow-lane rule's settings (see [`Coordinator::retire_slow_lanes`]; tests change them).
    slow_after: Duration,
    slow_ratio: f64,
    tail_margin: f64,
    ready: BTreeMap<usize, Ready>,
    next: usize,
    dir: PathBuf,
    watchdog: Duration,
    /// Every chunk must carry the same sample entry (one `hvc1` for the file).
    same_entry: bool,
    first_entry: Option<SampleEntry>,
    /// Timescale and media start of the first finished chunk: the muxer's for the whole track,
    /// so every chunk must have the same (in band too: each driver's SPS sets the reorder delay).
    first_timing: Option<(u32, Option<i64>)>,
    /// Frames of chunks finished by live lanes (the progress never goes below what it showed).
    finished_frames: u64,
    /// The progress counter when the lanes started (frames before them).
    base: Option<u64>,
    t0: Instant,
    finished: bool,
}

/// How long the coordinator waits for an event before it checks cancel and the watchdog.
const POLL: Duration = Duration::from_millis(200);

/// Slow-lane rule ([`Coordinator::retire_slow_lanes`]): a lane's speed counts after this much work,
const SLOW_AFTER: Duration = Duration::from_secs(4);
/// a lane below this fraction of another's speed is given up on,
const SLOW_RATIO: f64 = 0.25;
/// and an idle lane takes over a chunk it would finish this many times sooner.
const TAIL_MARGIN: f64 = 1.5;

impl Coordinator {
    /// Start one thread per lane over `chunks` (in file order, as the chunker made them), spilling
    /// into `dir` (created; removed when the coordinator is dropped). `needs_422`: indices of
    /// chunks with 4:2:2 sources. `same_entry`: the file has one `hvc1` entry, so every chunk's
    /// must be equal to it. `watchdog`: a lane still building its work, or holding a chunk with no
    /// progress, for this long is given up on. `dir` must not exist yet (a name of this run's own,
    /// [`super::lane::chunk_dir`]): nothing the user owns, or another run's lane wrote, is touched.
    pub fn start(chunks: Vec<Chunk>, lanes: Vec<LaneSpec>, dir: &Path, needs_422: HashSet<usize>, same_entry: bool, watchdog: Duration) -> Result<Coordinator> {
        std::fs::create_dir(dir).map_err(|e| ExportError::Io(format!("{}: {e}", dir.display())))?;
        let sched = Arc::new(Scheduler::new(chunks.iter().copied()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let lanes_422 = Arc::new(AtomicUsize::new(lanes.iter().filter(|l| l.decodes_422).count()));
        let needs_422 = Arc::new(needs_422);
        let now = Instant::now();
        let mut states = Vec::with_capacity(lanes.len());
        for (i, spec) in lanes.into_iter().enumerate() {
            let killed = Arc::new(AtomicBool::new(false));
            let ctx = LaneCtx {
                lane: i,
                sched: sched.clone(),
                stop: stop.clone(),
                killed: killed.clone(),
                tx: tx.clone(),
                dir: dir.to_path_buf(),
                avoid_422: (!spec.decodes_422).then(|| (needs_422.clone(), lanes_422.clone())),
            };
            let make = spec.make;
            let spawned = std::thread::Builder::new().name(format!("export-lane-{i}")).spawn(move || lane_main(ctx, make));
            let (alive, handle, failed) = match spawned {
                Ok(h) => (true, Some(h), None),
                Err(e) => (false, None, Some(format!("thread: {e}"))),
            };
            if !alive && spec.decodes_422 {
                lanes_422.fetch_sub(1, Ordering::Relaxed);
            }
            states.push(LaneState {
                name: spec.name,
                node: spec.node,
                decodes_422: spec.decodes_422,
                killed,
                alive,
                exited: !alive,
                ready: false,
                held: None,
                held_frames: 0,
                held_since: None,
                busy: Duration::ZERO,
                last: now,
                chunks: 0,
                frames: 0,
                failed,
                retired: None,
                handle,
            });
        }
        Ok(Coordinator {
            chunks,
            sched,
            stop,
            rx,
            lanes: states,
            lanes_422,
            needs_422,
            slow_after: SLOW_AFTER,
            slow_ratio: SLOW_RATIO,
            tail_margin: TAIL_MARGIN,
            ready: BTreeMap::new(),
            next: 0,
            dir: dir.to_path_buf(),
            watchdog,
            same_entry,
            first_entry: None,
            first_timing: None,
            finished_frames: 0,
            base: None,
            t0: now,
            finished: false,
        })
    }

    /// Chunks in the export.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// True when there are no chunks.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The next chunk in file order with its packets, waiting for a lane to finish it; `None`
    /// after the last. `cancel` is polled; `done` is raised to the frames the lanes encoded (never
    /// lowered: a failed attempt's frames are encoded again before it moves on). Errors:
    /// cancelled, every lane dead, a damaged spill file, chunks whose timescale / media start
    /// differ, chunks whose entries differ in a one-entry file.
    pub fn next_chunk(&mut self, cancel: &AtomicBool, done: &AtomicU64) -> Result<Option<(Chunk, Vec<SpillPacket>, ChunkDone)>> {
        if self.base.is_none() {
            self.base = Some(done.load(Ordering::Relaxed));
        }
        loop {
            let Some(&chunk) = self.chunks.get(self.next) else {
                self.finish();
                return Ok(None);
            };
            if let Some(r) = self.ready.remove(&self.next) {
                self.next = self.next.saturating_add(1);
                let packets = spill::read_all(&r.path).map_err(|e| ExportError::Io(e.to_string()));
                let _ = std::fs::remove_file(&r.path);
                let packets = packets?;
                if u64::try_from(packets.len()).ok() != u64::try_from(chunk.len()).ok() {
                    self.shut_down();
                    return Err(ExportError::Encode(format!("chunk {} has {} packets for {} frames", chunk.index, packets.len(), chunk.len())));
                }
                return Ok(Some((chunk, packets, r.done)));
            }
            if cancel.load(Ordering::Relaxed) {
                self.shut_down();
                return Err(ExportError::Cancelled);
            }
            if self.lanes.iter().all(|l| !l.alive || l.exited) {
                self.shut_down();
                let why: Vec<String> = self.lanes.iter().filter_map(|l| l.failed.as_ref().map(|f| format!("{}: {f}", l.name))).collect();
                return Err(ExportError::Encode(format!("every export lane stopped before chunk {} ({})", chunk.index, why.join("; "))));
            }
            for i in 0..self.lanes.len() {
                let stuck = self.lanes.get(i).is_some_and(|l| l.alive && (l.held.is_some() || !l.ready) && l.last.elapsed() > self.watchdog);
                if stuck {
                    self.kill(i, format!("no progress for {} s", self.watchdog.as_secs()));
                }
            }
            // judge the lanes on what they have reported so far (a lane that took the last chunk
            // must not look idle because its `Started` is still unread)
            while let Ok(ev) = self.rx.try_recv() {
                self.handle(ev)?;
            }
            self.retire_slow_lanes();
            match self.rx.recv_timeout(POLL) {
                Ok(ev) => {
                    self.handle(ev)?;
                    self.show_progress(done);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                // every lane thread is gone (each sends `Exited` last, so this is after those)
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    for l in &mut self.lanes {
                        l.exited = true;
                    }
                }
            }
        }
    }

    /// Raise the progress to the frames encoded so far (finished chunks and live lanes' current
    /// ones); never lower it: frames of a failed attempt are encoded again before it moves on.
    fn show_progress(&self, done: &AtomicU64) {
        let held: u64 = self.lanes.iter().filter(|l| l.alive).map(|l| l.held_frames).fold(0, u64::saturating_add);
        let total = self.base.unwrap_or(0).saturating_add(self.finished_frames).saturating_add(held);
        done.fetch_max(total, Ordering::Relaxed);
    }

    fn handle(&mut self, ev: Event) -> Result<()> {
        match ev {
            Event::Ready { lane } => {
                if let Some(l) = self.lanes.get_mut(lane).filter(|l| l.alive) {
                    l.ready = true;
                    l.last = Instant::now();
                }
            }
            Event::Started { lane, chunk } => match self.lanes.get_mut(lane) {
                Some(l) if l.alive => {
                    l.held = Some(chunk);
                    l.held_frames = 0;
                    l.held_since = Some(Instant::now());
                    l.last = Instant::now();
                }
                // pulled by a lane given up on meanwhile: back for the others
                _ => self.sched.requeue(chunk),
            },
            Event::Frames { lane, n } => {
                // a report without frames is no progress (the watchdog keeps counting)
                if let Some(l) = self.lanes.get_mut(lane).filter(|l| l.alive && n > 0) {
                    l.last = Instant::now();
                    l.held_frames = l.held_frames.saturating_add(n);
                }
            }
            Event::Done { lane, chunk, path, done: d } => {
                let Some(l) = self.lanes.get_mut(lane).filter(|l| l.alive && l.held.is_some_and(|c| c.index == chunk)) else {
                    let _ = std::fs::remove_file(&path);
                    return Ok(());
                };
                let frames = l.held.map_or(0, |c| u64::try_from(c.len()).unwrap_or(0));
                l.held = None;
                l.held_frames = 0;
                if let Some(t) = l.held_since.take() {
                    l.busy = l.busy.saturating_add(t.elapsed());
                }
                l.last = Instant::now();
                l.chunks = l.chunks.saturating_add(1);
                l.frames = l.frames.saturating_add(frames);
                self.sched.done();
                self.finished_frames = self.finished_frames.saturating_add(frames);
                let timing = (d.timescale, d.media_start);
                match self.first_timing {
                    None => self.first_timing = Some(timing),
                    Some(first) if first != timing => {
                        self.shut_down();
                        return Err(ExportError::Encode(format!(
                            "lane {} wrote chunk {chunk} with timing {timing:?}, the file's is {first:?}",
                            self.lane_name(lane)
                        )));
                    }
                    Some(_) => {}
                }
                if self.same_entry {
                    match &self.first_entry {
                        None => self.first_entry = Some(d.entry.clone()),
                        Some(first) if !entries_agree(&[first.clone(), d.entry.clone()]) => {
                            self.shut_down();
                            return Err(ExportError::Encode(format!(
                                "lane {} wrote chunk {chunk} with other parameter sets than the file's",
                                self.lane_name(lane)
                            )));
                        }
                        Some(_) => {}
                    }
                }
                self.ready.insert(chunk, Ready { path, done: *d });
            }
            Event::Failed { lane, why } => self.kill(lane, why),
            Event::Exited { lane } => {
                if let Some(l) = self.lanes.get_mut(lane) {
                    l.exited = true;
                }
            }
        }
        Ok(())
    }

    fn lane_name(&self, lane: usize) -> String {
        self.lanes.get(lane).map(|l| l.name.clone()).unwrap_or_default()
    }

    /// Give up on a lane that went wrong: its chunk goes back to the front of the queue for the
    /// others.
    fn kill(&mut self, lane: usize, why: String) {
        self.stop_lane(lane, why, false);
    }

    /// Stop a lane (`retired`: the slow-lane rule, reported apart from failures).
    fn stop_lane(&mut self, lane: usize, why: String, retired: bool) {
        let Some(l) = self.lanes.get_mut(lane).filter(|l| l.alive) else { return };
        l.alive = false;
        l.killed.store(true, Ordering::Relaxed);
        if retired {
            l.retired = Some(why);
        } else {
            l.failed = Some(why);
        }
        l.held_frames = 0;
        l.held_since = None;
        if l.decodes_422 {
            self.lanes_422.fetch_sub(1, Ordering::Relaxed);
        }
        if let Some(c) = l.held.take() {
            let _ = std::fs::remove_file(self.dir.join(attempt_name(c.index, lane)));
            self.sched.requeue(c);
        }
        if self.lanes.iter().all(|l| !l.alive) {
            // nobody is left to pull what was put back: lanes waiting on the queue must return
            self.sched.cancel();
        }
    }

    /// Frames per second of lane `i` while it held chunks, once it has worked `slow_after`.
    fn speed(&self, i: usize) -> Option<f64> {
        let l = self.lanes.get(i).filter(|l| l.alive && l.ready)?;
        let busy = l.busy.saturating_add(l.held_since.map_or(Duration::ZERO, |t| t.elapsed()));
        let frames = l.frames.saturating_add(l.held_frames);
        (busy >= self.slow_after && frames > 0).then(|| frames as f64 / busy.as_secs_f64().max(1e-6))
    }

    /// The slow-lane rule (6E.3 S11: a graded 4K export on the integrated GPU ran at a tenth of
    /// the other lane's speed, so two GPUs were slower than one). A lane is given up on, its chunk
    /// back to the front of the queue, when another live lane is measured and
    /// * it runs at less than [`SLOW_RATIO`] of that lane's speed (it costs the fast lane more
    ///   than it adds: they share memory bandwidth and CPU), or
    /// * the queue is empty, that lane is idle, and it would finish this lane's whole chunk from
    ///   the start [`TAIL_MARGIN`] times sooner than this lane finishes what is left of it.
    ///
    /// Never the last live lane, never a 4:2:2 chunk's lane in favour of one that can't decode it,
    /// and never the last lane that decodes 4:2:2 while 4:2:2 chunks are still to be encoded.
    fn retire_slow_lanes(&mut self) {
        let queue_empty = self.sched.waiting() == 0;
        let open_422 = self.needs_422.iter().any(|&c| c >= self.next && !self.ready.contains_key(&c));
        for i in 0..self.lanes.len() {
            let Some(mine) = self.speed(i) else { continue };
            let Some(held) = self.lanes.get(i).and_then(|l| l.held) else { continue };
            if open_422 && self.lanes.get(i).is_some_and(|l| l.decodes_422) && self.lanes_422.load(Ordering::Relaxed) <= 1 {
                continue;
            }
            let held_422 = self.needs_422.contains(&held.index) && self.lanes.get(i).is_some_and(|l| l.decodes_422);
            let left = u64::try_from(held.len()).unwrap_or(0).saturating_sub(self.lanes.get(i).map_or(0, |l| l.held_frames));
            let mut why = None;
            for j in (0..self.lanes.len()).filter(|&j| j != i) {
                let Some(other) = self.speed(j) else { continue };
                let Some(o) = self.lanes.get(j) else { continue };
                if held_422 && !o.decodes_422 {
                    continue;
                }
                if mine < other * self.slow_ratio {
                    why = Some(format!("too slow: {mine:.1} fps vs {other:.1} on {}", o.name));
                } else if queue_empty && o.held.is_none() && left as f64 / mine > self.tail_margin * held.len() as f64 / other {
                    why = Some(format!("its last chunk is faster on {} ({mine:.1} vs {other:.1} fps)", o.name));
                }
                if why.is_some() {
                    break;
                }
            }
            if let Some(why) = why {
                self.stop_lane(i, why, true);
            }
        }
    }

    /// Per lane: name, node, chunks, frames, fps, why it failed or was retired.
    pub fn report(&self) -> Vec<LaneReport> {
        let secs = self.t0.elapsed().as_secs_f64().max(1e-6);
        self.lanes
            .iter()
            .map(|l| LaneReport {
                name: l.name.clone(),
                node: l.node.clone(),
                chunks: l.chunks,
                frames: l.frames,
                fps: l.frames as f64 / secs,
                failed: l.failed.clone(),
                retired: l.retired.clone(),
            })
            .collect()
    }

    /// Stop every lane (cancel, error): the queue is cancelled and lanes stop at their next batch.
    fn shut_down(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.sched.cancel();
    }

    /// All chunks were handed out: lanes are finishing on their own.
    fn finish(&mut self) {
        self.finished = true;
        self.sched.cancel();
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        if !self.finished {
            self.shut_down();
        }
        // join the lanes that stop within a moment; a hung one (a stuck GPU) is left detached
        let until = Instant::now() + Duration::from_secs(5);
        for l in &mut self.lanes {
            let Some(h) = l.handle.take() else { continue };
            while !h.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The spill file name of `lane`'s attempt at chunk `chunk` (a lane tries a chunk once).
fn attempt_name(chunk: usize, lane: usize) -> String {
    format!("{chunk}.{lane}.pkts")
}

struct LaneCtx {
    lane: usize,
    sched: Arc<Scheduler>,
    stop: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
    tx: mpsc::Sender<Event>,
    dir: PathBuf,
    /// For a lane that can't decode 4:2:2: the 4:2:2 chunks and how many lanes that can are alive.
    avoid_422: Option<(Arc<HashSet<usize>>, Arc<AtomicUsize>)>,
}

fn lane_main(ctx: LaneCtx, make: MakeLane) {
    let lane = ctx.lane;
    let send = |ev: Event| {
        // the coordinator gone (dropped) is fine: the lane stops at its next check
        let _ = ctx.tx.send(ev);
    };
    let run = catch_unwind(AssertUnwindSafe(|| {
        let mut work = match make() {
            Ok(w) => w,
            Err(why) => return Err(why),
        };
        send(Event::Ready { lane });
        loop {
            if ctx.stop.load(Ordering::Relaxed) || ctx.killed.load(Ordering::Relaxed) {
                return Ok(());
            }
            let avoid = |c: &Chunk| ctx.avoid_422.as_ref().is_some_and(|(set, alive)| alive.load(Ordering::Relaxed) > 0 && set.contains(&c.index));
            let Some(chunk) = ctx.sched.pull_preferring(avoid) else { return Ok(()) };
            if ctx.killed.load(Ordering::Relaxed) {
                ctx.sched.requeue(chunk);
                return Ok(());
            }
            send(Event::Started { lane, chunk });
            let path = ctx.dir.join(attempt_name(chunk.index, lane));
            let keep_going = |n: u64| {
                send(Event::Frames { lane, n });
                !ctx.stop.load(Ordering::Relaxed) && !ctx.killed.load(Ordering::Relaxed)
            };
            match work.run_chunk(chunk, &path, &keep_going) {
                Ok(done) => send(Event::Done { lane, chunk: chunk.index, path, done: Box::new(done) }),
                Err(why) => return Err(why),
            }
        }
    }));
    match run {
        Ok(Ok(())) => {}
        Ok(Err(why)) => send(Event::Failed { lane, why }),
        Err(_) => send(Event::Failed { lane, why: "internal error (the panic was logged by the panic hook)".into() }),
    }
    send(Event::Exited { lane });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::chunker;
    use crate::lanes::spill::SpillWriter;
    use filmcraft_isobmff::PcmConfig;

    fn entry(bits: u16) -> SampleEntry {
        SampleEntry::pcm(PcmConfig { bits, float: false, big_endian: false, signed: true, channels: 2, sample_rate: 48_000.0 })
    }

    /// A lane that "encodes" a chunk as one packet per frame holding the frame number.
    struct Fake {
        delay: Duration,
        /// Fail at this chunk index (the lane dies).
        fail_at: Option<usize>,
        /// Panic instead of failing.
        panic: bool,
        /// Hang (stop reporting) at this chunk index, until told to stop.
        hang_at: Option<usize>,
        bits: u16,
        media_start: Option<i64>,
    }

    impl LaneWork for Fake {
        fn run_chunk(&mut self, chunk: Chunk, out: &Path, keep_going: &dyn Fn(u64) -> bool) -> std::result::Result<ChunkDone, String> {
            if self.fail_at == Some(chunk.index) {
                // half the chunk is encoded (and shown in the progress) before it fails
                for _ in chunk.start..chunk.start + chunk.len() / 2 {
                    std::thread::sleep(self.delay);
                    keep_going(1);
                }
                if self.panic {
                    panic!("fake lane panics");
                }
                return Err("fake failure".into());
            }
            if self.hang_at == Some(chunk.index) {
                // no progress reported; returns only when stopped (a real hang would never)
                while keep_going(0) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                return Err("cancelled".into());
            }
            let mut w = SpillWriter::create(out).map_err(|e| e.to_string())?;
            for f in chunk.start..chunk.end {
                std::thread::sleep(self.delay);
                w.push(&SpillPacket { data: f.to_le_bytes().to_vec(), key: f == chunk.start, duration: 1, cts: 0 }).map_err(|e| e.to_string())?;
                if !keep_going(1) {
                    return Err("cancelled".into());
                }
            }
            w.finish().map_err(|e| e.to_string())?;
            Ok(ChunkDone { entry: entry(self.bits), timescale: 24, media_start: self.media_start })
        }
    }

    fn fake(delay_ms: u64) -> Fake {
        Fake { delay: Duration::from_millis(delay_ms), fail_at: None, panic: false, hang_at: None, bits: 16, media_start: None }
    }

    fn spec(name: &str, f: Fake) -> LaneSpec {
        LaneSpec { name: name.into(), node: None, decodes_422: false, make: Box::new(move || Ok(Box::new(f) as Box<dyn LaneWork>)) }
    }

    fn dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fc-lanes-{}-{name}.chunks", std::process::id()))
    }

    /// Runs `body` on a thread and fails the test if it takes longer than `secs` (a hang).
    fn within<T: Send + 'static>(secs: u64, body: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(body());
        });
        match rx.recv_timeout(Duration::from_secs(secs)) {
            Ok(v) => v,
            Err(_) => panic!("timed out after {secs} s (hang)"),
        }
    }

    /// Joins every chunk; the frame numbers of all packets, in order.
    fn join_all(c: &mut Coordinator, cancel: &AtomicBool, done: &AtomicU64) -> Result<Vec<i64>> {
        let mut frames = Vec::new();
        while let Some((chunk, packets, _)) = c.next_chunk(cancel, done)? {
            assert_eq!(chunk.len() as usize, packets.len());
            for (k, p) in packets.iter().enumerate() {
                assert_eq!(p.key, k == 0, "an IDR at each chunk start only");
                let b: [u8; 8] = p.data.as_slice().try_into().expect("8 bytes");
                frames.push(i64::from_le_bytes(b));
            }
        }
        Ok(frames)
    }

    #[test]
    fn chunks_join_in_file_order_from_lanes_of_different_speeds() {
        within(30, || {
            let chunks = chunker::plan(0, 300, 10, 40, 2, &[], &[]);
            let n = chunks.len();
            let d = dir("order");
            let mut c = Coordinator::start(chunks, vec![spec("fast", fake(0)), spec("slow", fake(2))], &d, HashSet::new(), true, Duration::from_secs(20))
                .expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            let frames = join_all(&mut c, &cancel, &done).expect("join");
            assert_eq!(frames, (0..300).collect::<Vec<_>>());
            assert_eq!(done.load(Ordering::Relaxed), 300);
            let r = c.report();
            assert_eq!(r.iter().map(|l| l.chunks).sum::<u64>(), n as u64);
            assert!(r.iter().all(|l| l.failed.is_none()), "{r:?}");
            drop(c);
            assert!(!d.exists(), "the chunk directory is removed");
        });
    }

    /// Runs two lanes over `chunks` with the slow-lane rule measuring after `after`; the report.
    /// `rule`: which half of the rule may act (`"ratio"`, `"tail"`, or both with anything else).
    fn run_pair(name: &str, chunks: Vec<Chunk>, a: Fake, b: Fake, after: Duration, rule: &str) -> Vec<LaneReport> {
        let total = chunks.last().map_or(0, |c| c.end);
        let d = dir(name);
        let mut c = Coordinator::start(chunks, vec![spec("fast", a), spec("slow", b)], &d, HashSet::new(), true, Duration::from_secs(20)).expect("start");
        c.slow_after = after;
        match rule {
            "ratio" => c.tail_margin = f64::INFINITY,
            "tail" => c.slow_ratio = 0.0,
            _ => {}
        }
        let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
        let frames = join_all(&mut c, &cancel, &done).expect("join");
        assert_eq!(frames, (0..total).collect::<Vec<_>>(), "every frame once, in order");
        c.report()
    }

    #[test]
    fn a_much_slower_lane_is_given_up_on_and_its_chunk_redone() {
        within(30, || {
            let r = run_pair("slow-ratio", chunker::plan(0, 400, 10, 40, 2, &[], &[]), fake(1), fake(20), Duration::from_millis(200), "ratio");
            let slow = r.get(1).expect("slow lane");
            assert!(slow.retired.as_deref().is_some_and(|f| f.contains("too slow")), "{r:?}");
            assert!(slow.failed.is_none(), "retiring a working lane is not a failure: {r:?}");
            assert!(r.first().is_some_and(|l| l.failed.is_none()), "{r:?}");
        });
    }

    #[test]
    fn an_idle_lane_takes_over_a_last_chunk_it_finishes_sooner() {
        within(30, || {
            // two equal chunks (the chunker's shrinking tail would hide the rule), the ratio half
            // off: the slow lane's chunk would end long after the idle fast lane could redo it
            let r = run_pair(
                "slow-tail",
                vec![Chunk { index: 0, start: 0, end: 60 }, Chunk { index: 1, start: 60, end: 120 }],
                fake(4),
                fake(13),
                Duration::from_millis(100),
                "tail",
            );
            let slow = r.get(1).expect("slow lane");
            assert!(slow.retired.as_deref().is_some_and(|f| f.contains("last chunk")), "{r:?}");
            assert!(slow.failed.is_none(), "{r:?}");
            assert_eq!(r.first().map(|l| l.chunks), Some(2), "{r:?}");
        });
    }

    #[test]
    fn the_only_422_lane_is_kept_while_422_chunks_remain() {
        within(30, || {
            let chunks: Vec<Chunk> = (0..6).map(|k| Chunk { index: k, start: k as i64 * 40, end: k as i64 * 40 + 40 }).collect();
            let d = dir("slow-422");
            let slow = LaneSpec { decodes_422: true, ..spec("slow", fake(20)) };
            let mut c = Coordinator::start(
                chunks,
                vec![spec("fast", Fake { fail_at: Some(5), ..fake(2) }), slow],
                &d,
                HashSet::from([5]),
                true,
                Duration::from_secs(20),
            )
            .expect("start");
            c.slow_after = Duration::from_millis(100);
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            let frames = join_all(&mut c, &cancel, &done).expect("join");
            assert_eq!(frames, (0..240).collect::<Vec<_>>());
            let r = c.report();
            // far below the ratio, but the 4:2:2 chunk is still open and only it can encode it (the
            // fast lane fails it): retired early, nobody would be left for that chunk
            assert!(r.get(1).is_some_and(|l| l.failed.is_none() && l.retired.is_none() && l.chunks >= 1), "{r:?}");
        });
    }

    #[test]
    fn lanes_of_equal_speed_are_never_given_up_on() {
        within(30, || {
            let r = run_pair("slow-equal", chunker::plan(0, 300, 10, 30, 2, &[], &[]), fake(3), fake(3), Duration::from_millis(50), "both");
            assert!(r.iter().all(|l| l.failed.is_none() && l.retired.is_none()), "{r:?}");
        });
    }

    #[test]
    fn a_failing_lane_gives_its_chunks_to_the_survivor() {
        for panic in [false, true] {
            within(30, move || {
                let chunks = chunker::plan(0, 200, 10, 20, 2, &[], &[]);
                let d = dir(&format!("fail-{panic}"));
                let bad = Fake { fail_at: Some(1), panic, ..fake(1) };
                let mut c =
                    Coordinator::start(chunks, vec![spec("ok", fake(1)), spec("bad", bad)], &d, HashSet::new(), true, Duration::from_secs(20)).expect("start");
                let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
                let frames = join_all(&mut c, &cancel, &done).expect("join");
                assert_eq!(frames, (0..200).collect::<Vec<_>>());
                assert_eq!(done.load(Ordering::Relaxed), 200, "failed attempts are not counted twice");
                let r = c.report();
                // the bad lane may never have reached chunk 1 (the good one took it): then it's alive
                if let Some(why) = &r[1].failed {
                    assert!(why.contains("fake") || why.contains("internal"), "{why}");
                }
            });
        }
    }

    #[test]
    fn every_lane_dead_is_an_error_not_a_hang() {
        within(30, || {
            let chunks = chunker::plan(0, 100, 10, 20, 2, &[], &[]);
            let d = dir("dead");
            let a = Fake { fail_at: Some(0), ..fake(0) };
            let b = Fake { fail_at: Some(0), ..fake(0) };
            let broken = LaneSpec { name: "broken".into(), node: None, decodes_422: false, make: Box::new(|| Err("no GPU".to_string())) };
            let mut c = Coordinator::start(chunks, vec![spec("a", a), spec("b", b), broken], &d, HashSet::new(), true, Duration::from_secs(20)).expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            // every lane fails chunk 0 (it is put back each time): no lane is left
            let r = join_all(&mut c, &cancel, &done);
            let Err(ExportError::Encode(why)) = r else { panic!("expected an error, got {r:?}") };
            assert!(why.contains("every export lane stopped"), "{why}");
            assert!(c.report().iter().all(|l| l.failed.is_some()));
        });
    }

    #[test]
    fn a_hung_lane_is_given_up_on_by_the_watchdog() {
        within(30, || {
            let chunks = chunker::plan(0, 60, 10, 20, 2, &[], &[]);
            let d = dir("hang");
            let hung = Fake { hang_at: Some(0), ..fake(0) };
            // the hung lane starts first and takes chunk 0; the good one starts a moment later
            let slow_start = LaneSpec {
                name: "late".into(),
                node: None,
                decodes_422: false,
                make: Box::new(|| {
                    std::thread::sleep(Duration::from_millis(100));
                    Ok(Box::new(fake(0)) as Box<dyn LaneWork>)
                }),
            };
            let mut c = Coordinator::start(chunks, vec![spec("hung", hung), slow_start], &d, HashSet::new(), true, Duration::from_millis(500)).expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            let frames = join_all(&mut c, &cancel, &done).expect("join");
            assert_eq!(frames, (0..60).collect::<Vec<_>>());
            let r = c.report();
            assert!(r[0].failed.as_deref().is_some_and(|w| w.contains("no progress")), "{r:?}");
        });
    }

    #[test]
    fn cancel_stops_every_lane_promptly() {
        within(30, || {
            let chunks = chunker::plan(0, 100_000, 10, 1000, 2, &[], &[]);
            let d = dir("cancel");
            let mut c =
                Coordinator::start(chunks, vec![spec("a", fake(5)), spec("b", fake(5))], &d, HashSet::new(), true, Duration::from_secs(20)).expect("start");
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                flag.store(true, Ordering::Relaxed);
            });
            let done = AtomicU64::new(0);
            let t = Instant::now();
            let r = join_all(&mut c, &cancel, &done);
            assert!(matches!(r, Err(ExportError::Cancelled)), "{r:?}");
            drop(c);
            assert!(t.elapsed() < Duration::from_secs(5), "lanes stopped and joined promptly");
            assert!(!d.exists());
        });
    }

    #[test]
    fn chunks_with_other_entries_fail_a_one_entry_file() {
        within(30, || {
            let chunks = chunker::plan(0, 100, 10, 10, 2, &[], &[]);
            let d = dir("entries");
            let other = Fake { bits: 24, ..fake(1) };
            let mut c = Coordinator::start(chunks.clone(), vec![spec("a", fake(3)), spec("b", other)], &d, HashSet::new(), true, Duration::from_secs(20))
                .expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            let r = join_all(&mut c, &cancel, &done);
            assert!(matches!(&r, Err(ExportError::Encode(w)) if w.contains("parameter sets")), "{r:?}");
            drop(c);
            // in band (hev1), different entries are expected and fine
            let other = Fake { bits: 24, ..fake(1) };
            let mut c =
                Coordinator::start(chunks, vec![spec("a", fake(1)), spec("b", other)], &d, HashSet::new(), false, Duration::from_secs(20)).expect("start");
            assert_eq!(join_all(&mut c, &cancel, &done).expect("join").len(), 100);
        });
    }

    /// Review fix 1: a lane stuck building its work (driver init) holds no chunk; the watchdog
    /// gives up on it too, so the other lane failing ends the export instead of a hang.
    #[test]
    fn a_lane_stuck_in_setup_does_not_hang_the_export() {
        within(30, || {
            let chunks = chunker::plan(0, 60, 10, 20, 2, &[], &[]);
            let stuck = LaneSpec {
                name: "stuck".into(),
                node: None,
                decodes_422: false,
                make: Box::new(|| {
                    std::thread::sleep(Duration::from_secs(3600));
                    Ok(Box::new(fake(0)) as Box<dyn LaneWork>)
                }),
            };
            let bad = Fake { fail_at: Some(0), ..fake(0) };
            let mut c =
                Coordinator::start(chunks, vec![stuck, spec("bad", bad)], &dir("stuck"), HashSet::new(), false, Duration::from_millis(500)).expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            let r = join_all(&mut c, &cancel, &done);
            assert!(matches!(&r, Err(ExportError::Encode(w)) if w.contains("every export lane stopped")), "{r:?}");
            let rep = c.report();
            assert!(rep[0].failed.as_deref().is_some_and(|w| w.contains("no progress")), "{rep:?}");
        });
    }

    /// Review fix 2: the muxer takes the track's timescale and media start from the first chunk,
    /// so a chunk with others is an error in every mode (hev1 too).
    #[test]
    fn chunks_with_other_timing_are_an_error_even_in_band() {
        within(30, || {
            let chunks = chunker::plan(0, 100, 10, 10, 2, &[], &[]);
            let other = Fake { media_start: Some(2), ..fake(1) };
            let mut c = Coordinator::start(chunks, vec![spec("a", fake(3)), spec("b", other)], &dir("timing"), HashSet::new(), false, Duration::from_secs(20))
                .expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            let r = join_all(&mut c, &cancel, &done);
            assert!(matches!(&r, Err(ExportError::Encode(w)) if w.contains("timing")), "{r:?}");
        });
    }

    /// Review fix 3: a failed attempt's frames are not taken back from the progress.
    #[test]
    fn the_progress_never_goes_backwards() {
        within(30, || {
            let chunks = chunker::plan(0, 200, 10, 20, 2, &[], &[]);
            let bad = Fake { fail_at: Some(1), ..fake(2) };
            let mut c =
                Coordinator::start(chunks, vec![spec("ok", fake(1)), spec("bad", bad)], &dir("progress"), HashSet::new(), false, Duration::from_secs(20))
                    .expect("start");
            let (cancel, done) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
            let stop = Arc::new(AtomicBool::new(false));
            let watch = {
                let (done, stop) = (done.clone(), stop.clone());
                std::thread::spawn(move || {
                    let mut last = 0;
                    while !stop.load(Ordering::Relaxed) {
                        let now = done.load(Ordering::Relaxed);
                        if now < last {
                            return Err(format!("went back from {last} to {now}"));
                        }
                        last = now;
                        std::thread::sleep(Duration::from_micros(200));
                    }
                    Ok(())
                })
            };
            let frames = join_all(&mut c, &cancel, &done).expect("join");
            stop.store(true, Ordering::Relaxed);
            assert_eq!(frames.len(), 200);
            assert_eq!(done.load(Ordering::Relaxed), 200);
            assert_eq!(watch.join().unwrap_or_else(|_| Err("watcher panicked".into())), Ok(()));
        });
    }

    #[test]
    fn hostile_shapes_one_chunk_no_chunks_more_lanes_than_chunks() {
        within(30, || {
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            // zero-length range: no chunks, nothing to wait for
            let mut c = Coordinator::start(
                chunker::plan(5, 5, 10, 20, 2, &[], &[]),
                vec![spec("a", fake(0))],
                &dir("zero"),
                HashSet::new(),
                true,
                Duration::from_secs(5),
            )
            .expect("start");
            assert!(c.is_empty());
            assert_eq!(join_all(&mut c, &cancel, &done).expect("join"), Vec::<i64>::new());
            // one frame, one chunk, four lanes
            let lanes = (0..4).map(|i| spec(&format!("l{i}"), fake(0))).collect();
            let mut c =
                Coordinator::start(chunker::plan(7, 8, 10, 20, 4, &[], &[]), lanes, &dir("one"), HashSet::new(), true, Duration::from_secs(5)).expect("start");
            assert_eq!(join_all(&mut c, &cancel, &done).expect("join"), vec![7]);
            // no lanes at all: an error, not a hang
            let mut c = Coordinator::start(chunker::plan(0, 50, 10, 20, 2, &[], &[]), Vec::new(), &dir("none"), HashSet::new(), true, Duration::from_secs(5))
                .expect("start");
            assert!(join_all(&mut c, &cancel, &done).is_err());
        });
    }

    #[test]
    fn chunks_of_422_sources_go_to_the_lane_that_decodes_them() {
        within(30, || {
            let chunks = chunker::plan(0, 200, 10, 10, 2, &[], &[]);
            let needs: HashSet<usize> = chunks.iter().filter(|c| c.index % 2 == 0).map(|c| c.index).collect();
            let d = dir("422");
            // the 4:2:2 lane starts first so it is there for chunk 0
            let mut intel = spec("intel", fake(1));
            intel.decodes_422 = true;
            let amd = LaneSpec {
                name: "amd".into(),
                node: None,
                decodes_422: false,
                make: Box::new(|| {
                    std::thread::sleep(Duration::from_millis(50));
                    Ok(Box::new(fake(1)) as Box<dyn LaneWork>)
                }),
            };
            let mut c = Coordinator::start(chunks, vec![intel, amd], &d, needs, true, Duration::from_secs(20)).expect("start");
            let (cancel, done) = (AtomicBool::new(false), AtomicU64::new(0));
            assert_eq!(join_all(&mut c, &cancel, &done).expect("join"), (0..200).collect::<Vec<_>>());
            let r = c.report();
            // the AMD lane only took 4:2:2 chunks when nothing else was waiting: at most the tail
            assert!(r[0].chunks >= 8, "{r:?}");
        });
    }
}
