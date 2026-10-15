//! GOP-aware random access shared by the container sources (MP4/MOV, Matroska/WebM).
//!
//! Seeking: find the sample whose presentation interval covers the requested time, decode forward
//! from the preceding sync sample, and cache every decoded frame (keyed by pts). Playback requests
//! for the next frames therefore hit the cache or continue the running decoder without re-seeking.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError, Weak};
use std::time::Duration;

use web_time::Instant;

use filmcraft_color::ColorInfo;
use filmcraft_frame::{Region, VideoFrame};

use crate::CodecError;
use crate::video::VideoDecoder;

/// Process-wide decode counters of every [`GopCache`] (benchmarks and diagnostics).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GopStats {
    /// Requests answered from the decoded-frame cache.
    pub hits: u64,
    /// Requests that had to decode.
    pub misses: u64,
    /// Decoder restarts at a sync sample (seeks).
    pub seeks: u64,
    /// Samples fed to decoders.
    pub decoded: u64,
    /// Decoded frames evicted from the cache.
    pub evicted: u64,
    /// Wall time spent inside decoders (`decode` / `flush`), nanoseconds.
    pub decode_ns: u64,
    /// Non-reference samples left out while catching up (frames already late).
    pub skipped: u64,
    /// Frames decoded in draft mode (reduced-resolution playback, [`VideoDecoder::set_draft`]).
    pub draft: u64,
    /// Pictures decoders output (hardware and software; [`crate::hw::hw_stats`] counts the
    /// hardware ones).
    pub frames: u64,
}

impl GopStats {
    /// Share of requests answered from the decoded-frame cache (0 when there were none).
    pub fn hit_rate(&self) -> f64 {
        let n = self.hits + self.misses;
        if n == 0 { 0.0 } else { self.hits as f64 / n as f64 }
    }

    /// Mean decoder wall time per sample fed (ms).
    pub fn decode_ms_per_sample(&self) -> f64 {
        if self.decoded == 0 { 0.0 } else { self.decode_ns as f64 / 1e6 / self.decoded as f64 }
    }
}

static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static SEEKS: AtomicU64 = AtomicU64::new(0);
static DECODED: AtomicU64 = AtomicU64::new(0);
static EVICTED: AtomicU64 = AtomicU64::new(0);
static DECODE_NS: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
static DRAFT: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);

/// Count pictures a decoder call output.
fn count_frames<T>(out: &[T]) {
    FRAMES.fetch_add(out.len() as u64, Ordering::Relaxed);
}

/// Run a decoder call, adding its wall time to the process-wide counter.
fn timed<R>(f: impl FnOnce() -> R) -> R {
    let t0 = Instant::now();
    let r = f();
    DECODE_NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    r
}

/// The counters so far (they only grow; subtract two snapshots to measure an interval).
pub fn gop_stats() -> GopStats {
    GopStats {
        hits: HITS.load(Ordering::Relaxed),
        misses: MISSES.load(Ordering::Relaxed),
        seeks: SEEKS.load(Ordering::Relaxed),
        decoded: DECODED.load(Ordering::Relaxed),
        evicted: EVICTED.load(Ordering::Relaxed),
        decode_ns: DECODE_NS.load(Ordering::Relaxed),
        skipped: SKIPPED.load(Ordering::Relaxed),
        draft: DRAFT.load(Ordering::Relaxed),
        frames: FRAMES.load(Ordering::Relaxed),
    }
}

impl std::ops::Sub for GopStats {
    type Output = GopStats;
    fn sub(self, o: GopStats) -> GopStats {
        GopStats {
            hits: self.hits - o.hits,
            misses: self.misses - o.misses,
            seeks: self.seeks - o.seeks,
            decoded: self.decoded - o.decoded,
            evicted: self.evicted - o.evicted,
            decode_ns: self.decode_ns - o.decode_ns,
            skipped: self.skipped - o.skipped,
            draft: self.draft - o.draft,
            frames: self.frames - o.frames,
        }
    }
}

/// A container's video sample table, in decode (file) order.
pub trait VideoSamples {
    fn count(&self) -> usize;
    /// Presentation timestamp of sample `i` (track units).
    fn pts(&self, i: usize) -> i64;
    /// Nearest sync sample at or before `i`.
    fn sync_before(&self, i: usize) -> usize;
    /// The sample presented at `t` (track units), if any.
    fn sample_at(&self, t: i64) -> Option<usize>;
    fn read(&self, i: usize) -> crate::Result<Vec<u8>>;
    fn make_decoder(&self) -> crate::Result<Box<dyn VideoDecoder>>;
}

/// The sync sample to start decoding at for sample `i`: its sync sample, or an earlier one when
/// `i` is a leading picture of an open GOP (stored after its random-access picture, shown before
/// it, like the RASL pictures after every HEVC CRA of iPhone video). Those are predicted from the
/// GOP before, so a decoder starting at their random-access picture leaves them out.
fn decode_start(s: &dyn VideoSamples, i: usize) -> usize {
    let mut key = s.sync_before(i);
    while key > 0 && s.pts(i) < s.pts(key) {
        key = s.sync_before(key - 1).min(key - 1);
    }
    key
}

struct State {
    decoder: Option<Box<dyn VideoDecoder>>,
    /// The decoder only produces intra pictures: frames decode independently and in parallel.
    intra: bool,
    /// Idle decoders for parallel intra decoding.
    spare: Vec<Box<dyn VideoDecoder>>,
    /// Next sample (decode order) to feed, and the sync sample the current run started from.
    next: usize,
    start: usize,
    /// Highest pts the running decoder has output since it started (pictures leave in pts order).
    out_max: i64,
    /// The decoder is a hardware one (counted against the hardware cap).
    hw_decoder: bool,
    /// Whether samples are non-reference pictures ([`VideoDecoder::is_disposable`]), by sample
    /// index, as far as looked up (bounded).
    disposable: HashMap<usize, bool>,
    /// Only background requests (thumbnails, [`filmcraft_media::cancel::with_background`]) have
    /// used the decoder since it was made: it is given up as soon as another background request
    /// is done, whatever the caches' recency.
    background: bool,
}

/// A cache's decoded frames, under a lock of their own: a request for a cached frame never waits
/// for the decoder (which may be busy decoding a GOP for another request for a second or more:
/// reverse playback asks for the GOP before while it shows the cached one).
struct Store {
    /// Decoded frames by presentation pts (bounded).
    frames: BTreeMap<i64, Arc<VideoFrame>>,
    /// The cached frames that were decoded in draft mode: served to draft requests only.
    drafts: std::collections::BTreeSet<i64>,
    bytes: usize,
    /// GPU memory of the frames held (hardware decoders' surfaces).
    vram: usize,
    /// When a frame was last asked of this cache.
    last_used: Instant,
    /// Presentation times recently asked for (newest last): the frames between their minimum
    /// and maximum are wanted, so eviction takes others first.
    recent: std::collections::VecDeque<i64>,
    /// Presentation times of frames recently handed out (newest last).
    served: std::collections::VecDeque<i64>,
    /// Requests move backwards (reverse playback), and the samples from a sync sample to the
    /// frame last asked for: a whole GOP is decoded forward and then shown backwards, so the
    /// cache keeps at least that many frames ([`GopCache::store`]).
    reverse: bool,
    reverse_span: usize,
}

/// Requests remembered for eviction (about a second of playback with its prefetch).
const RECENT_REQUESTS: usize = 48;
/// Recent requests that decide whether playback runs backwards.
const REVERSE_WINDOW: usize = 12;
/// Frames kept beyond the GOP being played backwards (the playhead's frame and pictures still in
/// flight in the decoder).
const REVERSE_MARGIN: usize = 8;

/// Disposable flags remembered per cache before the map is cleared.
const DISPOSABLE_FLAGS: usize = 8192;

impl Store {
    fn new() -> Self {
        Self {
            frames: BTreeMap::new(),
            drafts: Default::default(),
            bytes: 0,
            vram: 0,
            last_used: Instant::now(),
            recent: std::collections::VecDeque::new(),
            served: std::collections::VecDeque::new(),
            reverse: false,
            reverse_span: 0,
        }
    }

    /// The cached frame nearest to `pts` within `tol` either side (exact first), for a skimming
    /// request.
    fn near(&self, pts: i64, tol: i64, draft_ok: bool) -> Option<i64> {
        let lo = pts.saturating_sub(tol);
        let hi = pts.saturating_add(tol);
        self.frames.range(lo..=hi).map(|(p, _)| *p).filter(|p| draft_ok || !self.drafts.contains(p)).min_by_key(|p| p.abs_diff(pts))
    }

    /// A cached frame that answers a request for `pts`: that exact one, or while skimming one
    /// within the tolerance.
    fn hit(&mut self, pts: i64, skim: Option<i64>, draft_ok: bool) -> Option<Arc<VideoFrame>> {
        let at = if self.cached(pts, draft_ok).is_some() { pts } else { skim.and_then(|tol| self.near(pts, tol, draft_ok))? };
        let f = self.frames.get(&at).cloned()?;
        self.note_served(at);
        Some(f)
    }

    /// A frame at `pts` was handed out.
    fn note_served(&mut self, pts: i64) {
        self.served.push_back(pts);
        if self.served.len() > RECENT_REQUESTS {
            self.served.pop_front();
        }
    }

    /// Record a request for `pts` and update [`Self::reverse`]: most recent requests are earlier
    /// than the one before them.
    fn note_request(&mut self, pts: i64) {
        self.recent.push_back(pts);
        if self.recent.len() > RECENT_REQUESTS {
            self.recent.pop_front();
        }
        let tail: Vec<i64> = self.recent.iter().rev().take(REVERSE_WINDOW + 1).copied().collect();
        let back = tail.windows(2).filter(|w| matches!(w, [newer, older] if newer < older)).count();
        let fwd = tail.windows(2).filter(|w| matches!(w, [newer, older] if newer > older)).count();
        self.reverse = tail.len() > REVERSE_WINDOW / 2 && back > fwd;
        if !self.reverse {
            self.reverse_span = 0;
        }
    }

    /// Evict frames, earliest first, while `more` says so.
    fn release_frames(&mut self, pool: &Pool, more: impl Fn() -> bool) {
        while more() {
            let Some((pts, f)) = self.frames.pop_first() else { break };
            self.drafts.remove(&pts);
            self.bytes = self.bytes.saturating_sub(f.byte_size());
            self.vram = self.vram.saturating_sub(f.gpu_bytes());
            pool.bytes.fetch_sub(f.byte_size(), Ordering::Relaxed);
            pool.vram.fetch_sub(f.gpu_bytes(), Ordering::Relaxed);
            EVICTED.fetch_add(1, Ordering::Relaxed);
            filmcraft_frame::pool::recycle(f);
        }
    }

    /// The cached frame at `pts`, unless it is a draft frame and the request wants the exact one.
    fn cached(&self, pts: i64, draft_ok: bool) -> Option<&Arc<VideoFrame>> {
        self.frames.get(&pts).filter(|_| draft_ok || !self.drafts.contains(&pts))
    }

    /// The nearest usable frame at or before `pts` (robust to decoder pts quirks).
    fn at_or_before(&self, pts: i64, draft_ok: bool) -> Option<Arc<VideoFrame>> {
        self.at_or_before_keyed(pts, draft_ok).map(|(_, f)| f)
    }

    /// [`Self::at_or_before`] with the frame's pts.
    fn at_or_before_keyed(&self, pts: i64, draft_ok: bool) -> Option<(i64, Arc<VideoFrame>)> {
        self.frames.range(..=pts).rev().find(|(p, _)| draft_ok || !self.drafts.contains(p)).map(|(p, f)| (*p, f.clone()))
    }
}

impl State {
    /// Whether sample `k` is a non-reference picture (looked up once per sample).
    fn is_disposable(&mut self, s: &dyn VideoSamples, k: usize) -> crate::Result<bool> {
        if let Some(d) = self.disposable.get(&k) {
            return Ok(*d);
        }
        let data = s.read(k)?;
        Ok(self.flag_disposable(k, Some(&data)))
    }

    /// [`Self::is_disposable`] with the sample's data at hand (`None`: the flag is known).
    fn flag_disposable(&mut self, k: usize, data: Option<&[u8]>) -> bool {
        if let Some(d) = self.disposable.get(&k) {
            return *d;
        }
        let d = data.is_some_and(|data| self.decoder.as_ref().is_some_and(|dec| dec.is_disposable(data)));
        if self.disposable.len() >= DISPOSABLE_FLAGS {
            self.disposable.clear();
        }
        self.disposable.insert(k, d);
        d
    }

    /// Give up the decoder (and the spare intra decoders): the next request that has to decode
    /// makes a new one and restarts at a sync sample. Returned so they are dropped off the lock.
    fn release_decoder(&mut self, pool: &Pool) -> Vec<Box<dyn VideoDecoder>> {
        let mut out = std::mem::take(&mut self.spare);
        if let Some(d) = self.decoder.take() {
            if self.hw_decoder {
                pool.hw_decoders.fetch_sub(1, Ordering::Relaxed);
            } else {
                pool.decoders.fetch_sub(1, Ordering::Relaxed);
            }
            out.push(d);
        }
        self.next = usize::MAX;
        out
    }
}

thread_local! {
    /// Caches whose lock this thread holds while decoding (see [`GopCache::frame`]).
    static DECODING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Marks a cache as decoding on this thread until dropped (also on early return / `?`).
struct DecodingGuard(usize);

impl DecodingGuard {
    fn enter(cache: usize) -> Self {
        DECODING.with(|d| d.borrow_mut().push(cache));
        Self(cache)
    }
    fn active(cache: usize) -> bool {
        DECODING.with(|d| d.borrow().contains(&cache))
    }
}

impl Drop for DecodingGuard {
    fn drop(&mut self) {
        DECODING.with(|d| {
            let mut d = d.borrow_mut();
            if let Some(k) = d.iter().rposition(|&c| c == self.0) {
                d.remove(k);
            }
        });
    }
}

/// A decoder keeps its reference and in-flight pictures (hundreds of MB for a frame-threaded
/// 4K decoder), so only this many caches keep theirs once idle: a timeline's other clips make a
/// new one when the playhead reaches them, which costs what a seek costs.
const MAX_LIVE_DECODERS: usize = 4;

/// Hardware decoders kept live (see `Pool::hw_decoders`).
const MAX_LIVE_HW_DECODERS: usize = 8;

/// A cache asked for a frame this recently may be needed again soon (the next clip being
/// prefetched, a clip the user scrubs back to): it keeps its decoder up to [`HARD_DECODERS`], and
/// beyond [`MAX_LIVE_DECODERS`] only until it has been idle this long. Its frames are only kept
/// within [`FRAME_BUDGET`] once it has been idle for [`RECENT`]: going back to a clip left (or
/// paused) more than that long ago may cost a re-decode from its keyframe when other clips have
/// filled the budget meanwhile. Frames go least recently used cache first, earliest frames first.
const IDLE: Duration = Duration::from_secs(3);

/// A cache asked for a frame this recently is in use (a layer of the frame being composited): it
/// keeps its decoder and frames whatever the count. Caches idle for less than [`IDLE`] but longer
/// than this are a burst of navigation (thumbnails of every clip, the playhead jumping from clip
/// to clip) that the playhead has already left: they give up decoders beyond [`HARD_DECODERS`]
/// and frames beyond [`FRAME_BUDGET`]. Without this, everything touched within [`IDLE`] counted
/// as in use, so a burst held every clip's decoder (42 live hardware sessions, 6.5 GB of VRAM)
/// and hundreds of MB of frames per clip.
const RECENT: Duration = Duration::from_millis(500);

/// Most decoders caches idle for [`RECENT`] to [`IDLE`] keep together (a composite of a few
/// layers plus the clips just left behind).
const HARD_DECODERS: usize = 8;

/// Decoded frames all caches may hold together before idle ones give theirs up, until
/// [`configure`] sets the process's own (Settings ▸ Memory ▸ Decoded frame cache, or
/// [`auto_limits`] from the installed RAM). Each cache also has its own budget, which alone let
/// every clip on a timeline keep hundreds of MB of frames nobody was looking at.
pub const FRAME_BUDGET: usize = 1 << 30;

/// The shared budget and live-decoder cap for a machine with `ram` bytes of memory (None:
/// unknown): a fifth of the RAM for decoded frames (1–16 GiB: 6.2K 10-bit frames are 78 MB, so
/// 1 GiB holds only 13 of them), and a decoder per 8 GiB (2–8).
pub fn auto_limits(ram: Option<u64>) -> (usize, usize) {
    let Some(ram) = ram.filter(|r| *r > 0) else { return (FRAME_BUDGET, MAX_LIVE_DECODERS) };
    let budget = (ram / 5).clamp(1 << 30, 16 << 30);
    let decoders = (ram / (8 << 30)).clamp(2, 8);
    (usize::try_from(budget).unwrap_or(FRAME_BUDGET), usize::try_from(decoders).unwrap_or(MAX_LIVE_DECODERS))
}

/// Physical memory in bytes (Linux `/proc/meminfo`; None elsewhere or when unreadable).
pub fn installed_ram() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}

/// Set the process-wide decoded-frame budget (bytes, at least 64 MiB) and live-decoder cap (1–64).
pub fn configure(budget: usize, max_decoders: usize) {
    let p = Pool::global();
    p.budget.store(budget.max(64 << 20), Ordering::Relaxed);
    p.max_decoders.store(max_decoders.clamp(1, 64), Ordering::Relaxed);
}

/// The process-wide decoded-frame budget and live-decoder cap in force (`perf.stats`).
pub fn limits() -> (usize, usize) {
    let p = Pool::global();
    (p.budget.load(Ordering::Relaxed), p.max_decoders.load(Ordering::Relaxed))
}

/// Dedicated video memory of the largest GPU (Linux amdgpu `mem_info_vram_total`; None elsewhere).
pub fn installed_vram() -> Option<u64> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    cards.flatten().filter_map(|c| std::fs::read_to_string(c.path().join("device/mem_info_vram_total")).ok()).filter_map(|v| v.trim().parse::<u64>().ok()).max()
}

/// The budget for decoded frames held in GPU memory: a quarter of the VRAM (512 MiB–32 GiB), 2 GiB
/// when unknown. Decoders' reference pictures, spare surfaces, clips in use (each keeps at least
/// [`MIN_FRAMES_LARGE`] frames) and the compositor need the rest: measured on a 12 GB card with
/// four 6.2K clips playing alternately, half the VRAM for the cache ran the card out of memory.
pub fn auto_vram_budget(vram: Option<u64>) -> usize {
    vram.filter(|v| *v > 0).map_or(2 << 30, |v| usize::try_from((v / 4).clamp(512 << 20, 32 << 30)).unwrap_or(2 << 30))
}

/// Set the process-wide budget for decoded frames in GPU memory (bytes, at least 128 MiB).
pub fn configure_vram(budget: usize) {
    Pool::global().vram_budget.store(budget.max(128 << 20), Ordering::Relaxed);
}

/// GPU memory of the decoded frames held, and its budget (`perf.stats`).
pub fn vram_usage() -> (usize, usize) {
    let p = Pool::global();
    (p.vram.load(Ordering::Relaxed), p.vram_budget.load(Ordering::Relaxed))
}

/// What the caches of a process share: the cap on live decoders and the frame budget.
struct Pool {
    caches: Mutex<Vec<Weak<Shared>>>,
    /// Caches that hold a (software) decoder.
    decoders: AtomicUsize,
    max_decoders: AtomicUsize,
    /// Caches that hold a hardware decoder (its memory is GPU surfaces under the VRAM budget, so
    /// it has its own, larger cap: restarting one at every cut of a multi-clip timeline costs a
    /// context and a GOP of decoding).
    hw_decoders: AtomicUsize,
    idle: Duration,
    /// Caches idle at least this long are no longer in use (see [`RECENT`]).
    recent: Duration,
    /// Decoders the caches idle for [`Self::recent`] but not [`Self::idle`] may keep.
    hard_decoders: usize,
    /// Bytes of the frames the caches hold.
    bytes: AtomicUsize,
    budget: AtomicUsize,
    /// GPU memory of the frames the caches hold, and its budget.
    vram: AtomicUsize,
    vram_budget: AtomicUsize,
}

/// The part of a [`GopCache`] its pool reaches.
struct Shared {
    /// The decoder side; lock order: `state`, then `store` (`store` alone for cache hits).
    state: Mutex<State>,
    store: Mutex<Store>,
    pool: Arc<Pool>,
    /// The name of the decoder that decoded last (fork H3.2b; a leaf lock, held only to copy
    /// the name, so the Properties dialog never waits on a decode).
    decoder_name: Mutex<Option<String>>,
}

impl Shared {
    /// Remember the name of the decoder that just decoded (a hybrid decoder's name changes
    /// when it falls back to software, so this runs after decoding, not only when it is made).
    fn note_decoder(&self, d: Option<&dyn VideoDecoder>) {
        let Some(d) = d else { return };
        let mut g = self.decoder_name.lock().unwrap_or_else(PoisonError::into_inner);
        if g.as_deref() != Some(d.name()) {
            *g = Some(d.name().to_string());
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let st = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        drop(st.release_decoder(&self.pool));
        let store = self.store.get_mut().unwrap_or_else(PoisonError::into_inner);
        self.pool.bytes.fetch_sub(store.bytes, Ordering::Relaxed);
        self.pool.vram.fetch_sub(store.vram, Ordering::Relaxed);
    }
}

/// A cache's state unless another thread is using it (it is then decoding: not idle).
fn try_state(c: &Shared) -> Option<MutexGuard<'_, State>> {
    match c.state.try_lock() {
        Ok(g) => Some(g),
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

impl Pool {
    fn new(max_decoders: usize, idle: Duration, budget: usize) -> Self {
        Self {
            caches: Mutex::new(Vec::new()),
            decoders: AtomicUsize::new(0),
            max_decoders: AtomicUsize::new(max_decoders),
            hw_decoders: AtomicUsize::new(0),
            idle,
            recent: idle,
            hard_decoders: max_decoders,
            bytes: AtomicUsize::new(0),
            budget: AtomicUsize::new(budget),
            vram: AtomicUsize::new(0),
            // FILMCRAFT_VRAM_CACHE_MB overrides the automatic budget (diagnostics, benchmarks)
            vram_budget: AtomicUsize::new(
                std::env::var("FILMCRAFT_VRAM_CACHE_MB")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .map_or_else(|| auto_vram_budget(installed_vram()), |mb| mb.saturating_mul(1 << 20)),
            ),
        }
    }

    /// Caches idle for `recent` (< the idle time) are still trimmed, down to `hard_decoders`.
    fn tiered(mut self, recent: Duration, hard_decoders: usize) -> Self {
        self.recent = recent.min(self.idle);
        self.hard_decoders = hard_decoders.max(self.max_decoders.load(Ordering::Relaxed));
        self
    }

    /// The pool of every cache in the process.
    fn global() -> Arc<Pool> {
        static POOL: std::sync::OnceLock<Arc<Pool>> = std::sync::OnceLock::new();
        POOL.get_or_init(|| {
            let (budget, decoders) = auto_limits(installed_ram());
            Arc::new(Pool::new(decoders, IDLE, budget).tiered(RECENT, HARD_DECODERS))
        })
        .clone()
    }

    fn register(&self, c: &Arc<Shared>) {
        let mut g = self.caches.lock().unwrap_or_else(PoisonError::into_inner);
        g.retain(|w| w.strong_count() > 0);
        g.push(Arc::downgrade(c));
    }

    /// The caches other than `me` idle for at least `idle` that `keep` selects, least recently used first. Other
    /// caches are only ever try-locked, so two caches trimming each other cannot deadlock.
    fn idle_others(&self, me: &Shared, idle: Duration, keep: impl Fn(&State, &Store) -> bool) -> Vec<Arc<Shared>> {
        let all: Vec<Arc<Shared>> = self.caches.lock().unwrap_or_else(PoisonError::into_inner).iter().filter_map(Weak::upgrade).collect();
        let mut idle: Vec<(Instant, Arc<Shared>)> = all
            .into_iter()
            .filter(|c| !std::ptr::eq(Arc::as_ptr(c), me))
            .filter_map(|c| {
                let st = try_state(&c)?;
                let store = c.store.lock().unwrap_or_else(PoisonError::into_inner);
                let used = (keep(&st, &store) && store.last_used.elapsed() >= idle).then_some(store.last_used)?;
                drop((store, st));
                Some((used, c))
            })
            .collect();
        idle.sort_by_key(|(used, _)| *used);
        idle.into_iter().map(|(_, c)| c).collect()
    }

    /// Take the decoders of idle caches, least recently used first, down to the cap. The caller
    /// drops them with no cache lock held: dropping a decoder can block (a hardware session waits
    /// for its frames in flight).
    #[must_use = "drop the released decoders with no cache lock held"]
    fn trim_decoders(&self, me: &Shared) -> Vec<Box<dyn VideoDecoder>> {
        let mut released = Vec::new();
        let max = self.max_decoders.load(Ordering::Relaxed);
        // first the caches idle for long, down to the caps; then those a burst has left behind.
        // Hardware decoders have a cap of their own (`hw_decoders`), the same in both tiers.
        for (idle, cap) in [(self.idle, max), (self.recent, self.hard_decoders.max(max))] {
            let sw_over = || self.decoders.load(Ordering::Relaxed) > cap;
            let hw_over = || self.hw_decoders.load(Ordering::Relaxed) > MAX_LIVE_HW_DECODERS;
            if !sw_over() && !hw_over() {
                continue;
            }
            for c in self.idle_others(me, idle, |st, _| st.decoder.is_some()) {
                if !sw_over() && !hw_over() {
                    break;
                }
                if let Some(mut st) = try_state(&c)
                    && ((st.hw_decoder && hw_over()) || (!st.hw_decoder && sw_over()))
                {
                    released.extend(st.release_decoder(self));
                }
            }
        }
        released
    }

    /// Take the decoders of the caches other than `me` that only background requests have used
    /// (see [`State::background`]) and that are not decoding now. A burst of thumbnails (an import
    /// of many clips) touches every clip within moments, so the recency rules treat them all as
    /// in use; each software 4K decoder holds ~0.5 GB. The cache `me` just served keeps its
    /// decoder (hovering a clip asks for more of its frames). Drop the result with no cache lock
    /// held.
    #[must_use = "drop the released decoders with no cache lock held"]
    fn release_background(&self, me: &Shared) -> Vec<Box<dyn VideoDecoder>> {
        let all: Vec<Arc<Shared>> = self.caches.lock().unwrap_or_else(PoisonError::into_inner).iter().filter_map(Weak::upgrade).collect();
        let mut released = Vec::new();
        for c in all.iter().filter(|c| !std::ptr::eq(Arc::as_ptr(c), me)) {
            if let Some(mut st) = try_state(c)
                && st.background
                && st.decoder.is_some()
            {
                released.extend(st.release_decoder(self));
            }
        }
        released
    }

    /// Evict the frames of caches idle for [`RECENT`], least recently used first, down to the
    /// budget. Caches in use keep theirs (each within its own budget): a frame evicted before it is shown costs a
    /// re-decode from the keyframe.
    fn trim_frames(&self, me: &Shared) {
        let (budget, vram_budget) = (self.budget.load(Ordering::Relaxed), self.vram_budget.load(Ordering::Relaxed));
        let over = || self.bytes.load(Ordering::Relaxed) > budget || self.vram.load(Ordering::Relaxed) > vram_budget;
        if !over() {
            return;
        }
        for c in self.idle_others(me, self.recent, |_, store| !store.frames.is_empty()) {
            if !over() {
                break;
            }
            if let Some(st) = try_state(&c) {
                c.store.lock().unwrap_or_else(PoisonError::into_inner).release_frames(self, over);
                drop(st);
            }
        }
    }
}

/// Caches that hold a decoder right now, process-wide (`perf.stats`).
pub fn live_decoders() -> usize {
    let p = Pool::global();
    p.decoders.load(Ordering::Relaxed) + p.hw_decoders.load(Ordering::Relaxed)
}

/// Bytes of decoded frames the caches hold right now, process-wide (`perf.stats`).
pub fn cached_bytes() -> usize {
    Pool::global().bytes.load(Ordering::Relaxed)
}

/// Decoder + decoded-frame cache for one video track.
pub struct GopCache {
    shared: Arc<Shared>,
    /// Colour signalled by the container, which wins over the bitstream's.
    explicit_color: Option<ColorInfo>,
    /// Clockwise quarter turns from the container's display matrix, applied to every frame.
    rotation: u8,
    /// The container's clean aperture, cut from every frame before the rotation.
    crop: Option<Region>,
    budget: usize,
}

/// The cache always has room for this many frames of up to [`MIN_FRAMES_BYTES`] together: a
/// frame-threaded decoder runs up to ~2x its thread count ahead of the frame it returns, and
/// playback prefetches ahead of the playhead (14–24 frames), so a byte budget alone would evict
/// 4K frames before they are shown and force a re-decode from the keyframe.
const MIN_FRAMES: usize = 64;
/// ... but large frames get fewer: 64 6.2K 10-bit frames would be 5 GB per clip in use (and with
/// several clips of a timeline in use at once, GPU memory runs out).
const MIN_FRAMES_BYTES: usize = 1 << 30;
/// Never fewer than this (playback prefetches 14–24 frames ahead; measured: 16 frames per 6.2K
/// clip in use made a timeline cutting between four 6.2K clips drop ~85 of 360 frames, 24 ~23).
const MIN_FRAMES_LARGE: usize = 24;

/// Frames of `frame_bytes` each a cache in use always keeps (see [`MIN_FRAMES`]).
fn min_frames(frame_bytes: usize) -> usize {
    (MIN_FRAMES_BYTES / frame_bytes.max(1)).clamp(MIN_FRAMES_LARGE, MIN_FRAMES)
}

/// Samples looked through for the next sync sample or its leading pictures (longer GOPs are
/// decoded per request, as forwards).
const MAX_GOP_SCAN: usize = 600;

/// Samples either side of a skimming request's target searched for a reference picture to show
/// instead (B-picture runs are shorter than this).
const SKIM_SEARCH: usize = 8;

/// A running decoder up to this many samples before the wanted sample's sync sample keeps going
/// rather than restarting at the sync sample.
const CONTINUE_THROUGH: usize = 48;

impl GopCache {
    pub fn new(explicit_color: Option<ColorInfo>) -> Self {
        // unit tests count decoder restarts: each of their caches gets a pool of its own
        let pool = if cfg!(test) { Arc::new(Pool::new(MAX_LIVE_DECODERS, IDLE, FRAME_BUDGET).tiered(RECENT, HARD_DECODERS)) } else { Pool::global() };
        Self::in_pool(explicit_color, pool)
    }

    fn in_pool(explicit_color: Option<ColorInfo>, pool: Arc<Pool>) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                decoder: None,
                intra: false,
                spare: Vec::new(),
                next: usize::MAX,
                start: 0,
                out_max: i64::MIN,
                hw_decoder: false,
                disposable: HashMap::new(),
                background: false,
            }),
            store: Mutex::new(Store::new()),
            pool: pool.clone(),
            decoder_name: Mutex::new(None),
        });
        pool.register(&shared);
        Self { shared, explicit_color, rotation: 0, crop: None, budget: 384 << 20 }
    }

    /// The name of the decoder that decoded this track's last frames ("VA-API H.264 (in
    /// process)", "FilmCraft H.264", …), `None` before the first frame (fork H3.2b: the
    /// Properties dialog's Decoder line). Never waits for a decode.
    pub fn decoder_name(&self) -> Option<String> {
        self.shared.decoder_name.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Turn every decoded frame clockwise by `quarter_turns` × 90° (the container's display
    /// rotation, e.g. portrait phone video stored landscape).
    pub fn with_rotation(mut self, quarter_turns: u8) -> Self {
        self.rotation = quarter_turns % 4;
        self
    }

    /// Cut every decoded frame to `region` (the container's clean aperture, in stored picture
    /// coordinates) before the rotation, as cropping and the display matrix compose.
    /// Frames are cut to a region (`with_crop`): given in full-size coordinates, so a scaled
    /// hand-off would cut the wrong part.
    pub fn crops(&self) -> bool {
        self.crop.is_some()
    }

    pub fn with_crop(mut self, region: Option<Region>) -> Self {
        self.crop = region;
        self
    }

    /// Colour signalled by the container wins over the bitstream's (YUV frames only); the
    /// container's clean aperture and then its display rotation are applied.
    fn finish(&self, mut f: VideoFrame) -> VideoFrame {
        if let Some(c) = self.explicit_color
            && !matches!(f.data, filmcraft_frame::PixelData::Rgba8(_) | filmcraft_frame::PixelData::RgbaF32(_))
        {
            f.color = c;
        }
        if let Some(r) = self.crop {
            f = f.cropped(r);
        }
        if self.rotation != 0 { f.rotated(self.rotation) } else { f }
    }

    /// Cache a decoded frame; returns it (the cache may evict it again right away, see below).
    fn store(&self, pts: i64, f: VideoFrame, draft: bool) -> Arc<VideoFrame> {
        let f = Arc::new(self.finish(f));
        let mut st = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        st.last_used = Instant::now();
        if draft {
            st.drafts.insert(pts);
            DRAFT.fetch_add(1, Ordering::Relaxed);
        } else {
            st.drafts.remove(&pts);
        }
        // a frame costs its memory wherever it is (RAM, or GPU memory for hardware surfaces)
        let cost = f.byte_size() + f.gpu_bytes();
        // Played backwards, a whole GOP is decoded forward before its last frame can be shown
        // (frames already handed out go first, see below), within the GPU-memory budget (or
        // half the shared RAM budget).
        let reverse = if st.reverse { st.reverse_span } else { 0 };
        let pool = &self.shared.pool;
        let cap = if f.gpu_bytes() > 0 { pool.vram_budget.load(Ordering::Relaxed) } else { pool.budget.load(Ordering::Relaxed) / 2 };
        let floor = min_frames(cost).max(reverse.min(cap / cost.max(1)));
        let budget = self.budget.max(floor.saturating_mul(cost));
        let (before, vram_before) = (st.bytes, st.vram);
        st.bytes += f.byte_size();
        st.vram += f.gpu_bytes();
        if let Some(old) = st.frames.insert(pts, f.clone()) {
            st.bytes -= old.byte_size();
            st.vram -= old.gpu_bytes();
            filmcraft_frame::pool::recycle(old);
        }
        // Evict frames outside the span recently asked for first: before it (already used), then
        // beyond it (decoded ahead of every request); only then, the farther end from the newest
        // decoded picture. Evicting a frame inside the span — one a queued job still wants —
        // costs a seek back to its keyframe and a GOP of decoding.
        let wanted = st.recent.iter().min().copied().zip(st.recent.iter().max().copied());
        while st.bytes + st.vram > budget && st.frames.len() > 2 {
            let (Some(&first), Some(&last)) = (st.frames.keys().next(), st.frames.keys().next_back()) else { break };
            // Backwards, frames already handed out go first (their requests are done: the
            // caller keeps what it rendered from them), then those after the span (shown), and
            // only then the earliest (shown last).
            let served = if st.reverse { st.frames.keys().rev().find(|p| **p != pts && st.served.contains(p)).copied() } else { None };
            let victim = match wanted {
                _ if served.is_some() => served.unwrap_or(first),
                Some((_, hi)) if st.reverse && last > hi && last != pts => last,
                _ if st.reverse => first,
                Some((lo, _)) if first < lo => first,
                Some((_, hi)) if last > hi && last != pts => last,
                _ if pts - first > last - pts => first,
                _ => last,
            };
            if let Some(v) = st.frames.remove(&victim) {
                st.drafts.remove(&victim);
                st.bytes -= v.byte_size();
                st.vram -= v.gpu_bytes();
                EVICTED.fetch_add(1, Ordering::Relaxed);
                // its planes serve the next decoded pictures instead of going back to the allocator
                filmcraft_frame::pool::recycle(v);
            }
        }
        let pool = &self.shared.pool;
        if st.bytes >= before {
            pool.bytes.fetch_add(st.bytes - before, Ordering::Relaxed);
        } else {
            pool.bytes.fetch_sub(before - st.bytes, Ordering::Relaxed);
        }
        if st.vram >= vram_before {
            pool.vram.fetch_add(st.vram - vram_before, Ordering::Relaxed);
        } else {
            pool.vram.fetch_sub(vram_before - st.vram, Ordering::Relaxed);
        }
        drop(st);
        pool.trim_frames(&self.shared);
        f
    }

    /// Store decoder output (in presentation order) and advance `out_max`. `keep` collects the
    /// latest frame at or before `want` (pts, whether drafts serve): the request that decoded it
    /// returns it even if the cache evicts it at once (another worker's requests may have moved
    /// the span the cache keeps past it).
    fn store_output(&self, st: &mut State, out: Vec<crate::video::DecodedFrame>, want: (i64, bool), keep: &mut Option<(i64, Arc<VideoFrame>)>) {
        count_frames(&out);
        for d in out {
            st.out_max = st.out_max.max(d.pts);
            let (pts, draft) = (d.pts, d.draft);
            let f = self.store(pts, d.frame, draft);
            if pts <= want.0 && (want.1 || !draft) && keep.as_ref().is_none_or(|(p, _)| *p <= pts) {
                *keep = Some((pts, f));
            }
        }
    }

    /// The frame presented at `target` (track units, clamped to the stream).
    ///
    /// Decoders may run slices on rayon, and a rayon thread waiting inside a decode can pick up
    /// another render job that asks this same source for a frame — on the thread that holds this
    /// cache's lock. Such a nested request must not lock again (deadlock): it decodes the frame
    /// with a private decoder instead ([`Self::private_frame`]).
    pub fn frame(&self, s: &dyn VideoSamples, target: i64) -> crate::Result<Arc<VideoFrame>> {
        self.frame_late(s, target, None)
    }

    /// [`Self::frame`] while catching up: frames shown before `late_before` (track units) are
    /// late. Non-reference samples of late frames are left out on the way to the wanted frame
    /// (no other picture depends on them, so the wanted frame decodes exactly as it would
    /// otherwise; a later request for a skipped frame re-seeks).
    ///
    /// Inside [`filmcraft_media::cancel::with_draft`]`(true, …)` the decoder runs in draft mode
    /// ([`VideoDecoder::set_draft`]); its draft frames are cached for draft requests only, and an
    /// exact request for one re-decodes it.
    pub fn frame_late(&self, s: &dyn VideoSamples, target: i64, late_before: Option<i64>) -> crate::Result<Arc<VideoFrame>> {
        self.frame_skim(s, target, late_before, None)
    }

    /// [`Self::frame_late`] while skimming (playback faster than real time or backwards at a
    /// speed that steps over frames, see [`filmcraft_media::cancel::with_skim`]): any picture
    /// within `tolerance` (track units) of `target` will do. A cached one is returned; a
    /// non-reference target with a reference picture within the tolerance is replaced by it (it
    /// is decoded anyway); every other non-reference picture on the way is left out; and a
    /// target in a later GOP is reached by a seek rather than by decoding on through the GOP.
    pub fn frame_skim(&self, s: &dyn VideoSamples, target: i64, late_before: Option<i64>, tolerance: Option<i64>) -> crate::Result<Arc<VideoFrame>> {
        let r = self.frame_skim_locked(s, target, late_before, tolerance);
        if filmcraft_media::cancel::background() {
            // Outside the cache's lock (see `release_background`). A thumbnail leaves the decoders
            // of earlier thumbnails no reason to stay: each costs hundreds of MB at 4K.
            drop(self.shared.pool.release_background(&self.shared));
        }
        r
    }

    fn frame_skim_locked(&self, s: &dyn VideoSamples, target: i64, late_before: Option<i64>, tolerance: Option<i64>) -> crate::Result<Arc<VideoFrame>> {
        let n = s.count();
        let mut i = s.sample_at(target.max(0)).or_else(|| (n > 0).then(|| n - 1)).ok_or_else(|| CodecError::Decode("empty track".into()))?;
        let mut want_pts = s.pts(i);
        let skim = tolerance.filter(|t| *t > 0);
        let me = self as *const Self as usize;
        if DecodingGuard::active(me) {
            return self.private_frame(s, i, want_pts, n);
        }
        let draft = filmcraft_media::cancel::draft();
        // Idle caches' decoders released below. Declared before the lock guard, so it is dropped
        // after the guard on every return path: dropping a decoder can block (a hardware session
        // waits for its frames in flight) and must not hold this cache's lock.
        let mut released: Vec<Box<dyn VideoDecoder>> = Vec::new();
        {
            let mut store = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
            store.last_used = Instant::now();
            store.note_request(want_pts);
            if let Some(f) = store.hit(want_pts, skim, draft) {
                HITS.fetch_add(1, Ordering::Relaxed);
                drop(store);
                // a monitor asking for a frame of a thumbnail's clip makes its decoder one in use
                // (unless it is decoding right now: the request that follows marks it)
                if !filmcraft_media::cancel::background()
                    && let Some(mut st) = try_state(&self.shared)
                {
                    st.background = false;
                }
                return Ok(f);
            }
        }
        let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let _decoding = DecodingGuard::enter(me);
        let background = filmcraft_media::cancel::background();
        // decoded meanwhile by the request this one waited for
        if let Some(f) = self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).hit(want_pts, skim, draft) {
            HITS.fetch_add(1, Ordering::Relaxed);
            // a monitor asking for a frame of a thumbnail's clip makes its decoder one in use
            st.background &= background;
            return Ok(f);
        }
        MISSES.fetch_add(1, Ordering::Relaxed);
        if !background {
            st.background = false;
        } else if st.decoder.is_none() {
            st.background = true;
        }
        if !background && st.decoder.as_ref().is_some_and(|d| d.thread_limited()) {
            // made for a thumbnail (few threads): a monitor gets a full one
            released.extend(st.release_decoder(&self.shared.pool));
        }
        if st.decoder.is_none() {
            let d = s.make_decoder()?;
            st.intra = d.intra_only();
            st.hw_decoder = d.is_hardware();
            if st.hw_decoder {
                self.shared.pool.hw_decoders.fetch_add(1, Ordering::Relaxed);
            } else {
                self.shared.pool.decoders.fetch_add(1, Ordering::Relaxed);
            }
            st.decoder = Some(d);
            st.next = usize::MAX;
        }
        released.extend(self.shared.pool.trim_decoders(&self.shared));
        // Nothing cached: the work ahead (possibly a seek and a GOP of decoding) is only worth it
        // while someone still wants the frame.
        if filmcraft_media::cancel::cancelled() {
            return Err(CodecError::Cancelled);
        }
        if st.intra {
            return self.intra_frame(st, s, i, want_pts);
        }
        if let Some(tol) = skim {
            // A sync sample shown within the tolerance costs one picture after a seek, where a
            // leading picture of an open GOP costs the whole GOP before it (unless the decoder
            // is running through that GOP anyway) and a picture deep in a GOP all the
            // references before it.
            let here = s.sync_before(i);
            let after = s.sync_before(i.saturating_add(SKIM_SEARCH).min(n.saturating_sub(1)));
            let leading = want_pts < s.pts(here);
            let through_gop_before = st.next != usize::MAX && st.next <= here && here > 0 && st.next >= s.sync_before(here - 1);
            let sync = [here, after]
                .into_iter()
                .filter(|k| *k != i && s.pts(*k).abs_diff(want_pts) <= tol.unsigned_abs())
                .min_by_key(|k| s.pts(*k).abs_diff(want_pts));
            if let Some(k) = sync
                && (leading && !through_gop_before || st.next == usize::MAX || st.next > i || k > i)
            {
                i = k;
                want_pts = s.pts(k);
                if let Some(f) = self.cached(want_pts, draft) {
                    HITS.fetch_add(1, Ordering::Relaxed);
                    return Ok(f);
                }
            }
        }
        if let Some(tol) = skim
            && st.is_disposable(s, i)?
        {
            // a reference picture near the target is decoded on the way anyway: show that
            let lo = i.saturating_sub(SKIM_SEARCH);
            let hi = i.saturating_add(SKIM_SEARCH).min(n.saturating_sub(1));
            let mut best: Option<usize> = None;
            for j in lo..=hi {
                let d = s.pts(j).abs_diff(want_pts);
                if j == i || d > tol.unsigned_abs() || best.is_some_and(|b| s.pts(b).abs_diff(want_pts) <= d) {
                    continue;
                }
                if !st.is_disposable(s, j)? {
                    best = Some(j);
                }
            }
            if let Some(j) = best {
                i = j;
                want_pts = s.pts(j);
                if let Some(f) = self.cached(want_pts, draft) {
                    HITS.fetch_add(1, Ordering::Relaxed);
                    return Ok(f);
                }
            }
        }
        let mut key = decode_start(s, i);
        // Continue the running decoder when it has passed the wanted sample's sync sample and
        // either has not reached the sample yet or has been fed it without outputting it yet
        // (a frame-threaded decoder holds many pictures in flight). Otherwise the frame was
        // evicted or lies in another GOP: restart at the sync sample. A run that started after
        // `key` (at the random-access picture of a wanted leading picture) left the picture out.
        let live = st.next != usize::MAX && st.next <= n;
        let running = live && st.start <= key;
        // Decoding on through a short stretch into the next GOP is cheaper than a restart when
        // playback wants those frames. Not when they are all late (`late_before` at or past the
        // sync sample: a cut back into this clip, or playback a GOP behind): nothing between
        // would be shown and a restart costs nothing, so seek (fork 6.5: continuing through
        // ~40 unwanted samples at re-entered cuts overloaded the decoder and dropped frames).
        // (Skimming steps over most of the GOP: a seek is cheaper.)
        let wanted_between = late_before.is_none_or(|l| l < s.pts(key));
        let near = skim.is_none() && wanted_between && st.next <= key && key - st.next <= CONTINUE_THROUGH;
        let continuing = running && (st.next > key || near) && (i >= st.next || (st.start <= i && want_pts > st.out_max));
        // Skimming forward, frames are asked for by several workers at once and the decoder may
        // have passed this one (for a later frame, leaving out pictures nobody else wanted):
        // rather than seek back a GOP, show the nearest frame decoded within a grid step
        // (four tolerances). (Any live run counts here, also one that started after `key`.)
        let forward = !self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).reverse;
        if let Some(tol) = skim
            && live
            && !continuing
            && forward
            && i < st.next
            && let Some(f) = self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).hit(want_pts, Some(tol.saturating_mul(4)), draft)
        {
            HITS.fetch_add(1, Ordering::Relaxed);
            return Ok(f);
        }
        if !continuing {
            SEEKS.fetch_add(1, Ordering::Relaxed);
            // The container's sync flags may be wrong for the codec (an MP4 without `stss` marks
            // every sample): step back to a sample the decoder can start from.
            while key > 0 {
                let data = s.read(key)?;
                if st.decoder.as_ref().and_then(|d| d.is_random_access(&data)) != Some(false) {
                    break;
                }
                key = s.sync_before(key - 1);
            }
            if let Some(d) = st.decoder.as_mut() {
                d.reset();
            }
            st.next = key;
            st.start = key;
            st.out_max = i64::MIN;
        }
        // Backwards (exact frames), one run decodes every frame shown between this GOP's sync
        // sample and the next one: through the next sync sample and its leading pictures, which
        // are shown before it. Requests for the range arrive in any order (several workers), and
        // a run that stopped at an earlier one would have to decode the GOP again for a later.
        let reverse = skim.is_none() && self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).reverse;
        let run_to = if reverse { Self::display_range_end(s, key, i, n) } else { i };
        {
            let mut store = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
            if store.reverse && !continuing {
                store.reverse_span = run_to.saturating_sub(st.start).saturating_add(REVERSE_MARGIN);
            }
        }
        let limit = (run_to.max(st.next) + 64).min(n);
        // Skimming, the next shown frame (a grid step, two tolerances, on) in another GOP means a
        // seek: flush the target out right after feeding it instead of decoding on until it
        // leaves the reorder buffer.
        let flush_at_target = skim.is_some_and(|tol| {
            let backwards = self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).reverse;
            let next = s.sample_at(want_pts.saturating_add(tol.saturating_mul(2)).max(0));
            backwards || next.is_none_or(|j| s.sync_before(j) != s.sync_before(i))
        });
        // skimming wants no picture but the target: every non-reference one is left out
        let late = if skim.is_some() { i64::MAX } else { late_before.unwrap_or(i64::MIN).min(want_pts) };
        let Some(decoder) = st.decoder.as_mut() else {
            return Err(CodecError::Decode("no video decoder".into()));
        };
        decoder.set_draft(draft);
        let mut kept: Option<(i64, Arc<VideoFrame>)> = None;
        while st.next < limit {
            if filmcraft_media::cancel::cancelled() {
                // The decoder state stays consistent (`next`, `out_max`): a later request continues.
                return Err(CodecError::Cancelled);
            }
            let k = st.next;
            let skippable = k != i && s.pts(k) < late;
            // a sample known to be skipped is not read at all
            let data = if skippable && st.disposable.get(&k) == Some(&true) { None } else { Some(s.read(k)?) };
            if skippable && st.flag_disposable(k, data.as_deref()) {
                st.next += 1;
                SKIPPED.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let data = match data {
                Some(d) => d,
                None => s.read(k)?,
            };
            let hw = st.hw_decoder;
            let Some(decoder) = st.decoder.as_mut() else {
                return Err(CodecError::Decode("no video decoder".into()));
            };
            // a hardware decoder shares the GPU's decode engine: most urgent work first
            let submitted = Instant::now();
            let out = if hw { crate::hwgate::run(|| timed(|| decoder.decode(&data, s.pts(k)))) } else { timed(|| decoder.decode(&data, s.pts(k))) }?;
            // a hardware decoder with time to spare is paced (see `cancel::with_pace`)
            if let Some(rest) = filmcraft_media::cancel::pace().filter(|_| hw).and_then(|p| p.checked_sub(submitted.elapsed())) {
                std::thread::sleep(rest);
            }
            st.next += 1;
            DECODED.fetch_add(1, Ordering::Relaxed);
            self.store_output(&mut st, out, (want_pts, draft), &mut kept);
            if (self.cached(want_pts, draft).is_some() && k >= run_to) || (flush_at_target && k == i) {
                break;
            }
            // Backwards, the next request seeks back anyway: once the range is decoded, flush it
            // out (below) rather than decode the following pictures until it leaves the reorder
            // buffer.
            if k >= run_to && self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).reverse {
                break;
            }
        }
        if self.cached(want_pts, draft).is_none() {
            let hw = st.hw_decoder;
            let out = st.decoder.as_mut().map(|d| if hw { crate::hwgate::run(|| timed(|| d.flush())) } else { timed(|| d.flush()) }).unwrap_or_default();
            self.store_output(&mut st, out, (want_pts, draft), &mut kept);
            st.next = usize::MAX;
        }
        self.shared.note_decoder(st.decoder.as_deref());
        // nearest decoded frame at or before the wanted pts (robust to decoder pts quirks)
        let mut store = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        store.note_served(want_pts);
        // the cache's frame, or the one this call decoded when it is nearer (the cache may have
        // evicted it already)
        match (store.at_or_before_keyed(want_pts, draft), kept) {
            (Some((cp, c)), Some((kp, k))) => Ok(if kp > cp { k } else { c }),
            (Some((_, c)), None) | (None, Some((_, c))) => Ok(c),
            (None, None) => Err(CodecError::Decode("frame not produced".into())),
        }
    }

    /// The last sample (decode order) of the frames shown from sync sample `key` to the next one:
    /// that sync sample's leading pictures (at least `i`).
    fn display_range_end(s: &dyn VideoSamples, key: usize, i: usize, n: usize) -> usize {
        let mut next_sync = key.saturating_add(1);
        while next_sync < n && s.sync_before(next_sync) != next_sync && next_sync - key < MAX_GOP_SCAN {
            next_sync += 1;
        }
        if next_sync >= n || s.sync_before(next_sync) != next_sync {
            return i;
        }
        let shown_at = s.pts(next_sync);
        let mut end = next_sync;
        while end + 1 < n && s.pts(end + 1) < shown_at && end - next_sync < MAX_GOP_SCAN {
            end += 1;
        }
        end.max(i)
    }

    /// The cached frame at `pts` (see [`Store::cached`]).
    fn cached(&self, pts: i64, draft_ok: bool) -> Option<Arc<VideoFrame>> {
        self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).cached(pts, draft_ok).cloned()
    }

    /// The nearest usable cached frame at or before `pts`.
    fn at_or_before(&self, pts: i64, draft_ok: bool) -> Option<Arc<VideoFrame>> {
        self.shared.store.lock().unwrap_or_else(|e| e.into_inner()).at_or_before(pts, draft_ok)
    }

    /// A nested request (see [`Self::frame`]): decode from the sync sample with a fresh decoder,
    /// without touching the shared state.
    fn private_frame(&self, s: &dyn VideoSamples, i: usize, want_pts: i64, n: usize) -> crate::Result<Arc<VideoFrame>> {
        let mut d = s.make_decoder()?;
        let mut next = usize::MAX;
        let out = Self::decode_to(s, d.as_mut(), &mut next, i, want_pts, n)?;
        let f = out
            .into_iter()
            .filter(|p| p.pts <= want_pts)
            .max_by_key(|p| p.pts)
            .map(|p| p.frame)
            .ok_or_else(|| CodecError::Decode("frame not produced".into()))?;
        Ok(Arc::new(self.finish(f)))
    }

    /// Decode with `d` (positioned at `next`) until the picture with `want_pts` (sample `i`) comes out.
    fn decode_to(
        s: &dyn VideoSamples,
        d: &mut dyn VideoDecoder,
        next: &mut usize,
        i: usize,
        want_pts: i64,
        n: usize,
    ) -> crate::Result<Vec<crate::video::DecodedFrame>> {
        let mut key = decode_start(s, i);
        // Continue the running decoder when the wanted sample is ahead within this GOP run.
        let continuing = *next != usize::MAX && *next > key && *next <= i + 16 && *next <= n;
        if !continuing {
            // The container's sync flags may be wrong for the codec (an MP4 without `stss` marks
            // every sample): step back to a sample the decoder can start from.
            while key > 0 {
                let data = s.read(key)?;
                if d.is_random_access(&data) != Some(false) {
                    break;
                }
                key = s.sync_before(key - 1);
            }
            d.reset();
            *next = key;
        }
        let limit = (i + 64).min(n);
        let mut out = Vec::new();
        let mut found = false;
        while *next < limit {
            let k = *next;
            let data = s.read(k)?;
            let pics = timed(|| d.decode(&data, s.pts(k)))?;
            count_frames(&pics);
            *next += 1;
            found |= pics.iter().any(|p| p.pts == want_pts);
            out.extend(pics);
            if found {
                break;
            }
        }
        if !found {
            let pics = timed(|| d.flush());
            count_frames(&pics);
            out.extend(pics);
            *next = usize::MAX;
        }
        Ok(out)
    }

    /// Intra-only streams: decode the one sample outside the lock, so several frame workers
    /// decode different frames of the same source at once.
    fn intra_frame(&self, st: std::sync::MutexGuard<'_, State>, s: &dyn VideoSamples, i: usize, want_pts: i64) -> crate::Result<Arc<VideoFrame>> {
        let mut st = st;
        let spare = st.spare.pop();
        drop(st);
        let mut dec = match spare {
            Some(d) => d,
            None => {
                let d = s.make_decoder()?;
                // (intra decoders are recorded when made: the spares' names are known already)
                self.shared.note_decoder(Some(d.as_ref()));
                d
            }
        };
        let res = s.read(i).and_then(|data| {
            DECODED.fetch_add(1, Ordering::Relaxed);
            timed(|| dec.decode(&data, want_pts))
        });
        let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.spare.len() < 16 {
            st.spare.push(dec);
        }
        let res = res?;
        count_frames(&res);
        drop(st);
        for d in res {
            self.store(d.pts, d.frame, d.draft);
        }
        self.at_or_before(want_pts, true).ok_or_else(|| CodecError::Decode("frame not produced".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::DecodedFrame;
    use std::collections::VecDeque;

    /// `n` samples, a sync sample every `gop`, pts = 1000 · index (no reordering).
    struct Samples {
        n: usize,
        gop: usize,
        delay: usize,
        intra: bool,
        /// Its decoders say they are hardware ones.
        hw: bool,
        resets: Arc<AtomicUsize>,
        decodes: Arc<AtomicUsize>,
        /// Called with each sample index read (tests acting as other workers mid-request).
        on_read: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    }

    /// Outputs each picture `delay` samples late, like a frame-threaded decoder.
    struct Dec {
        delay: usize,
        intra: bool,
        /// (pts, decoded as a draft picture)
        held: VecDeque<(i64, bool)>,
        draft: bool,
        hw: bool,
        /// made for background work (few threads, like the H.264 decoder's)
        limited: bool,
        resets: Arc<AtomicUsize>,
        decodes: Arc<AtomicUsize>,
    }

    /// A picture whose pixel encodes its index (and, in blue, whether it is a draft picture).
    fn picture((pts, draft): (i64, bool)) -> DecodedFrame {
        let i = (pts / 1000) as u32;
        DecodedFrame { pts, frame: VideoFrame::rgba8(1, 1, vec![i as u8, (i >> 8) as u8, draft as u8, 255]), draft }
    }

    impl VideoDecoder for Dec {
        fn decode(&mut self, sample: &[u8], pts: i64) -> crate::Result<Vec<DecodedFrame>> {
            self.decodes.fetch_add(1, Ordering::Relaxed);
            // like H.264 draft mode: only non-reference (odd) pictures are approximate
            self.held.push_back((pts, self.draft && sample[0] % 2 == 1));
            let mut out = Vec::new();
            while self.held.len() > self.delay {
                out.push(picture(self.held.pop_front().expect("held")));
            }
            Ok(out)
        }
        fn flush(&mut self) -> Vec<DecodedFrame> {
            self.held.drain(..).map(picture).collect()
        }
        fn reset(&mut self) {
            self.resets.fetch_add(1, Ordering::Relaxed);
            self.held.clear();
        }
        fn set_draft(&mut self, on: bool) {
            self.draft = on;
        }
        fn name(&self) -> &str {
            "test"
        }
        fn thread_limited(&self) -> bool {
            self.limited
        }
        fn intra_only(&self) -> bool {
            self.intra
        }
        fn is_hardware(&self) -> bool {
            self.hw
        }
        // odd samples are non-reference pictures
        fn is_disposable(&self, sample: &[u8]) -> bool {
            sample[0] % 2 == 1
        }
    }

    impl VideoSamples for Samples {
        fn count(&self) -> usize {
            self.n
        }
        fn pts(&self, i: usize) -> i64 {
            i as i64 * 1000
        }
        fn sync_before(&self, i: usize) -> usize {
            if self.intra { i } else { i / self.gop * self.gop }
        }
        fn sample_at(&self, t: i64) -> Option<usize> {
            let i = (t / 1000) as usize;
            (i < self.n).then_some(i)
        }
        fn read(&self, i: usize) -> crate::Result<Vec<u8>> {
            if let Some(f) = &self.on_read {
                f(i);
            }
            Ok(vec![i as u8])
        }
        fn make_decoder(&self) -> crate::Result<Box<dyn VideoDecoder>> {
            Ok(Box::new(Dec {
                delay: self.delay,
                intra: self.intra,
                held: VecDeque::new(),
                draft: false,
                hw: self.hw,
                limited: filmcraft_media::cancel::background(),
                resets: self.resets.clone(),
                decodes: self.decodes.clone(),
            }))
        }
    }

    fn samples(n: usize, gop: usize, delay: usize, intra: bool) -> Samples {
        Samples { n, gop, delay, intra, hw: false, resets: Default::default(), decodes: Default::default(), on_read: None }
    }

    /// Fork Phase 5.2c: parallel export workers asked for later frames while one worker decoded
    /// an earlier one; the cache's span of recent requests moved past it, the cache evicted the
    /// frame as it was stored and the request failed ("frame not produced": a black frame in the
    /// export). The request that decoded a frame returns it whatever the cache keeps.
    #[test]
    fn a_frame_evicted_as_it_is_stored_still_serves_its_request() {
        let mut s = samples(300, 300, 0, false);
        let mut c = GopCache::new(None);
        c.budget = 0;
        // one GOP decoded to its end: the cache keeps its floor of frames (the last ones)
        assert_eq!(index_of(&c.frame(&s, 299_000).expect("frame")), 299);
        let shared = c.shared.clone();
        s.on_read = Some(Arc::new(move |k| {
            if k == 100 {
                // meanwhile, other workers asked for frames far ahead
                let mut st = shared.store.lock().unwrap_or_else(|e| e.into_inner());
                for p in 0..RECENT_REQUESTS as i64 {
                    st.recent.push_back(250_000 + p * 1000);
                    if st.recent.len() > RECENT_REQUESTS {
                        st.recent.pop_front();
                    }
                }
            }
        }));
        assert_eq!(index_of(&c.frame(&s, 100_000).expect("frame 100 is served")), 100);
    }

    /// Fork H3.2b: the cache reports the decoder that decoded (the Properties dialog's Decoder
    /// line), nothing before the first frame.
    #[test]
    fn decoder_name_is_known_after_the_first_frame() {
        let s = samples(30, 10, 0, false);
        let c = GopCache::new(None);
        assert_eq!(c.decoder_name(), None);
        assert_eq!(index_of(&c.frame(&s, 5_000).expect("frame")), 5);
        assert_eq!(c.decoder_name().as_deref(), Some("test"));
        // intra-only streams decode outside the lock, with the same record
        let s = samples(30, 1, 0, true);
        let c = GopCache::new(None);
        assert_eq!(index_of(&c.frame(&s, 7_000).expect("frame")), 7);
        assert_eq!(c.decoder_name().as_deref(), Some("test"));
    }

    fn index_of(f: &VideoFrame) -> usize {
        match &f.data {
            filmcraft_frame::PixelData::Rgba8(d) => d[0] as usize | (d[1] as usize) << 8,
            _ => unreachable!(),
        }
    }

    #[test]
    fn frames_held_by_a_threaded_decoder_do_not_restart_it() {
        // 24 pictures in flight (more than any fixed look-ahead margin), one long GOP.
        let s = samples(300, 250, 24, false);
        let c = GopCache::new(None);
        // Playback order with prefetch running ahead: a later frame first, then earlier ones the
        // decoder has been fed but not output yet.
        for i in [10usize, 11, 40, 12, 30, 13, 60, 14, 15] {
            assert_eq!(index_of(&c.frame(&s, i as i64 * 1000).expect("frame")), i);
        }
        assert_eq!(s.resets.load(Ordering::Relaxed), 1, "only the first request seeks");
        assert!(s.decodes.load(Ordering::Relaxed) <= 60 + 24 + 1);
    }

    #[test]
    fn running_decoder_continues_through_a_nearby_sync_sample() {
        let s = samples(200, 30, 0, false);
        let c = GopCache::new(None);
        assert_eq!(index_of(&c.frame(&s, 25_000).expect("frame")), 25);
        // frame 40 lies in the next GOP (sync sample 30): decode 26..40 instead of restarting at 30
        assert_eq!(index_of(&c.frame(&s, 40_000).expect("frame")), 40);
        assert_eq!(index_of(&c.frame(&s, 28_000).expect("frame")), 28);
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
        // far ahead: restart at the sync sample rather than decode everything in between
        assert_eq!(index_of(&c.frame(&s, 150_000).expect("frame")), 150);
        assert_eq!(s.resets.load(Ordering::Relaxed), 2);
        assert!(s.decodes.load(Ordering::Relaxed) <= 41 + 1);
    }

    #[test]
    fn a_cut_back_into_a_clip_seeks_rather_than_decoding_unwanted_samples() {
        // (fork 6.5) the decoder stopped at 25; a cut re-enters at 70 (sync sample 60) with
        // everything before 70 late: restart at 60 instead of feeding 26..70
        let s = samples(200, 30, 0, false);
        let c = GopCache::new(None);
        assert_eq!(index_of(&c.frame(&s, 25_000).expect("frame")), 25);
        let before = s.decodes.load(Ordering::Relaxed);
        assert_eq!(index_of(&c.frame_late(&s, 70_000, Some(70_000)).expect("frame")), 70);
        assert_eq!(s.resets.load(Ordering::Relaxed), 2, "seeked to the sync sample");
        let fed = s.decodes.load(Ordering::Relaxed) - before;
        assert!(fed <= 11 + 1, "fed {fed}");
        // the same jump with frames before the sync sample still wanted continues
        let s2 = samples(200, 30, 0, false);
        let c2 = GopCache::new(None);
        assert_eq!(index_of(&c2.frame(&s2, 25_000).expect("frame")), 25);
        assert_eq!(index_of(&c2.frame_late(&s2, 40_000, Some(27_000)).expect("frame")), 40);
        assert_eq!(s2.resets.load(Ordering::Relaxed), 1, "continued through");
        // late up to exactly the sync sample: nothing before it is wanted either
        let s3 = samples(200, 30, 0, false);
        let c3 = GopCache::new(None);
        assert_eq!(index_of(&c3.frame(&s3, 25_000).expect("frame")), 25);
        assert_eq!(index_of(&c3.frame_late(&s3, 62_000, Some(60_000)).expect("frame")), 62);
        assert_eq!(s3.resets.load(Ordering::Relaxed), 2, "seeked at the boundary");
    }

    #[test]
    fn intra_frames_decode_in_parallel_once_each() {
        let s = Arc::new(samples(64, 1, 0, true));
        let c = Arc::new(GopCache::new(None));
        std::thread::scope(|scope| {
            for t in 0..4usize {
                let (s, c) = (s.clone(), c.clone());
                scope.spawn(move || {
                    for k in 0..64usize {
                        let i = (k * 7 + t * 16) % 64;
                        assert_eq!(index_of(&c.frame(&*s, i as i64 * 1000).expect("frame")), i);
                    }
                });
            }
        });
        // a frame two threads miss at the same moment may decode twice; nothing more
        assert!(s.decodes.load(Ordering::Relaxed) <= 64 + 4 * 4);
        assert_eq!(s.resets.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn catching_up_skips_late_non_reference_samples_only() {
        let s = samples(300, 250, 4, false);
        let c = GopCache::new(None);
        assert_eq!(index_of(&c.frame(&s, 2_000).expect("frame")), 2);
        let before = s.decodes.load(Ordering::Relaxed);
        // playback is behind: frame 41 is due, 7..=40 are late
        let f = c.frame_late(&s, 41_000, Some(41_000)).expect("frame");
        assert_eq!(index_of(&f), 41, "the wanted (odd, disposable) frame itself is decoded");
        let fed = s.decodes.load(Ordering::Relaxed) - before;
        // samples 7..41 minus the odd ones before 41, plus the decoder's output delay
        assert!(fed <= 18 + 4 + 1, "fed {fed}");
        // the decoder continues from there: the next frames are decoded, not skipped
        for i in 42..48usize {
            assert_eq!(index_of(&c.frame(&s, i as i64 * 1000).expect("frame")), i);
        }
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
        // without the hint nothing is skipped
        let s2 = samples(300, 250, 4, false);
        let c2 = GopCache::new(None);
        assert_eq!(index_of(&c2.frame(&s2, 41_000).expect("frame")), 41);
        assert!(s2.decodes.load(Ordering::Relaxed) >= 42);
        // a prefetch job for frame 41 while frame 30 is due: only frames before 30 are late
        let s3 = samples(300, 250, 0, false);
        let c3 = GopCache::new(None);
        assert_eq!(index_of(&c3.frame_late(&s3, 41_000, Some(30_000)).expect("frame")), 41);
        assert_eq!(s3.decodes.load(Ordering::Relaxed), 42 - 15, "odd samples 1..=29 skipped");
        for i in 30..41usize {
            assert_eq!(index_of(&c3.frame(&s3, i as i64 * 1000).expect("cached")), i);
        }
        assert_eq!(s3.resets.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn skimming_shows_reference_pictures_and_leaves_the_others_out() {
        let s = samples(300, 250, 2, false);
        let c = GopCache::new(None);
        // 41 is a non-reference picture: its reference neighbour within a frame is shown, and
        // no non-reference picture on the way is decoded
        let f = c.frame_skim(&s, 41_000, None, Some(1000)).expect("frame");
        assert_eq!(index_of(&f), 40);
        assert!(s.decodes.load(Ordering::Relaxed) <= 21 + 2 + 1, "fed {}", s.decodes.load(Ordering::Relaxed));
        // the next frames of the grid continue the decoder
        for t in [43_000, 45_000, 47_000] {
            assert!(index_of(&c.frame_skim(&s, t, None, Some(1000)).expect("frame")).abs_diff((t / 1000) as usize) <= 1);
        }
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn skimming_prefers_a_sync_sample_and_never_seeks_back() {
        let s = samples(300, 30, 0, false);
        let c = GopCache::new(None);
        // a sync sample within the tolerance costs one picture
        assert_eq!(index_of(&c.frame_skim(&s, 59_000, None, Some(2000)).expect("frame")), 60);
        assert_eq!(s.decodes.load(Ordering::Relaxed), 1);
        // a frame the decoder has passed (another worker asked for a later one first): the
        // nearest decoded frame within a grid step rather than a seek back to its GOP
        let resets = s.resets.load(Ordering::Relaxed);
        assert_eq!(index_of(&c.frame_skim(&s, 57_000, None, Some(2000)).expect("frame")), 60);
        assert_eq!(s.resets.load(Ordering::Relaxed), resets);
        // the next GOP is reached by a seek, not by decoding through the rest of this one
        assert_eq!(index_of(&c.frame_skim(&s, 90_000, None, Some(2000)).expect("frame")), 90);
        assert!(s.decodes.load(Ordering::Relaxed) <= 3, "fed {}", s.decodes.load(Ordering::Relaxed));
        // exact requests are unchanged
        assert_eq!(index_of(&c.frame(&s, 57_000).expect("frame")), 57);
    }

    #[test]
    fn playing_backwards_decodes_each_gop_once() {
        // GOPs of 100 pictures, more than the cache keeps of them when not playing backwards
        let s = samples(400, 100, 2, false);
        let mut c = GopCache::new(None);
        c.budget = 0;
        assert!(min_frames(4) < 100);
        for i in (0..400usize).rev() {
            assert_eq!(index_of(&c.frame(&s, i as i64 * 1000).expect("frame")), i);
        }
        // one seek per GOP (the first request comes before the direction is known)
        assert!(s.resets.load(Ordering::Relaxed) <= 5, "{} seeks", s.resets.load(Ordering::Relaxed));
        // (plus part of the first GOP again: decoded before the direction was known, it was
        // held to the usual floor)
        assert!(s.decodes.load(Ordering::Relaxed) <= 400 + 5 * 4 + 40, "fed {}", s.decodes.load(Ordering::Relaxed));
        // and holds about a GOP, not everything played
        assert!(c.shared.store.lock().unwrap().frames.len() <= 100 + REVERSE_MARGIN + 2);
    }

    fn is_draft(f: &VideoFrame) -> bool {
        match &f.data {
            filmcraft_frame::PixelData::Rgba8(d) => d[2] == 1,
            _ => unreachable!(),
        }
    }

    #[test]
    fn draft_frames_are_served_to_draft_requests_only() {
        let s = samples(300, 250, 3, false);
        let c = GopCache::new(None);
        let before = gop_stats().draft;
        // reduced-resolution playback: odd (non-reference) pictures come out as drafts
        for i in 0..12usize {
            let f = filmcraft_media::cancel::with_draft(true, || c.frame(&s, i as i64 * 1000)).expect("frame");
            assert_eq!(index_of(&f), i);
            assert_eq!(is_draft(&f), i % 2 == 1, "frame {i}");
        }
        assert!(gop_stats().draft - before >= 6);
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
        // paused / export (no draft hint): an even frame is exact and cached
        let f = c.frame(&s, 4_000).expect("frame");
        assert!(!is_draft(&f) && index_of(&f) == 4);
        assert_eq!(s.resets.load(Ordering::Relaxed), 1, "exact frames are served from the cache");
        // a draft frame is decoded again, exactly
        let f = c.frame(&s, 5_000).expect("frame");
        assert!(!is_draft(&f) && index_of(&f) == 5);
        assert_eq!(s.resets.load(Ordering::Relaxed), 2, "re-decoded from the sync sample");
        // and a draft request takes the exact frame now cached
        let f = filmcraft_media::cancel::with_draft(true, || c.frame(&s, 5_000)).expect("frame");
        assert!(!is_draft(&f));
        // without the hint nothing is ever a draft
        let s2 = samples(300, 250, 3, false);
        let c2 = GopCache::new(None);
        assert!((0..12).all(|i| !is_draft(&c2.frame(&s2, i * 1000).expect("frame"))));
    }

    #[test]
    fn cancelled_request_stops_decoding_and_keeps_state() {
        let s = samples(300, 250, 0, false);
        let c = GopCache::new(None);
        assert_eq!(index_of(&c.frame(&s, 5_000).expect("frame")), 5);
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let r = filmcraft_media::cancel::with_cancel(&flag, || c.frame(&s, 200_000));
        assert!(matches!(r, Err(CodecError::Cancelled)));
        assert_eq!(s.decodes.load(Ordering::Relaxed), 6);
        // cached frames are still served, and the decoder carries on from where it was
        let r = filmcraft_media::cancel::with_cancel(&flag, || c.frame(&s, 3_000));
        assert_eq!(index_of(&r.expect("cached")), 3);
        assert_eq!(index_of(&c.frame(&s, 7_000).expect("frame")), 7);
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
    }

    /// An open-GOP stream: every 10 samples a CRA, then two leading pictures (shown before it)
    /// that reference the GOP before; a decoder started at a CRA drops its leading pictures, as
    /// HEVC decoders drop RASL pictures.
    struct OpenGop {
        n: usize,
        resets: Arc<AtomicUsize>,
    }

    impl OpenGop {
        /// Decode-order sample → presentation time (in frames): CRA at 10k shown at 10k + 2,
        /// leading samples 10k + 1, 10k + 2 shown at 10k, 10k + 1; others in order.
        fn shown_at(k: usize) -> i64 {
            match k % 10 {
                0 if k > 0 => k as i64 + 2,
                1 | 2 if k > 10 => k as i64 - 1,
                _ => k as i64,
            }
        }
    }

    struct OpenGopDec {
        start: Option<i64>,
        resets: Arc<AtomicUsize>,
    }

    impl VideoDecoder for OpenGopDec {
        fn decode(&mut self, _sample: &[u8], pts: i64) -> crate::Result<Vec<DecodedFrame>> {
            let start = *self.start.get_or_insert(pts);
            // leading pictures of the CRA decoding started at are dropped
            if pts < start {
                return Ok(Vec::new());
            }
            Ok(vec![picture((pts, false))])
        }
        fn flush(&mut self) -> Vec<DecodedFrame> {
            Vec::new()
        }
        fn reset(&mut self) {
            self.resets.fetch_add(1, Ordering::Relaxed);
            self.start = None;
        }
        fn name(&self) -> &str {
            "open gop"
        }
    }

    impl VideoSamples for OpenGop {
        fn count(&self) -> usize {
            self.n
        }
        fn sample_at(&self, pts: i64) -> Option<usize> {
            (0..self.n).find(|&k| self.pts(k) == pts).or_else(|| (0..self.n).filter(|&k| self.pts(k) <= pts).max_by_key(|&k| self.pts(k)))
        }
        fn pts(&self, k: usize) -> i64 {
            Self::shown_at(k) * 1000
        }
        fn sync_before(&self, i: usize) -> usize {
            i - i % 10
        }
        fn read(&self, k: usize) -> crate::Result<Vec<u8>> {
            Ok(vec![k as u8])
        }
        fn make_decoder(&self) -> crate::Result<Box<dyn VideoDecoder>> {
            Ok(Box::new(OpenGopDec { start: None, resets: self.resets.clone() }))
        }
    }

    #[test]
    fn leading_pictures_of_an_open_gop_seek_from_the_gop_before() {
        // sample 21 (shown at frame 20, before the CRA at sample 20 shown at 22) used to be
        // "not produced": the seek started at that CRA, whose decoder drops it
        let s = OpenGop { n: 40, resets: Default::default() };
        let c = GopCache::in_pool(None, Arc::new(Pool::new(4, Duration::from_secs(3600), FRAME_BUDGET)));
        for frame in [20, 21, 30, 5, 11, 22] {
            assert_eq!(index_of(&c.frame(&s, frame * 1000).expect("frame")), frame as usize, "frame {frame}");
        }
    }

    #[test]
    fn hardware_decoders_have_their_own_cap() {
        // software cap 2, hardware cap MAX_LIVE_HW_DECODERS: ten hardware clips keep 8 decoders,
        // three software clips 2, independently
        let pool = Arc::new(Pool::new(2, Duration::ZERO, FRAME_BUDGET));
        let hw: Vec<Samples> = (0..10).map(|_| Samples { hw: true, ..samples(300, 250, 3, false) }).collect();
        let sw: Vec<Samples> = (0..3).map(|_| samples(300, 250, 3, false)).collect();
        let ch: Vec<GopCache> = (0..10).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        let cs: Vec<GopCache> = (0..3).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        for k in 0..10 {
            ch[k].frame(&hw[k], 10_000).expect("frame");
            if k < 3 {
                cs[k].frame(&sw[k], 10_000).expect("frame");
            }
        }
        assert_eq!(pool.hw_decoders.load(Ordering::Relaxed), MAX_LIVE_HW_DECODERS);
        assert_eq!(pool.decoders.load(Ordering::Relaxed), 2);
        drop((ch, cs));
        assert_eq!((pool.hw_decoders.load(Ordering::Relaxed), pool.decoders.load(Ordering::Relaxed)), (0, 0));
    }

    #[test]
    fn idle_caches_give_up_their_decoders_beyond_the_cap() {
        let pool = Arc::new(Pool::new(2, Duration::ZERO, FRAME_BUDGET));
        let live = || pool.decoders.load(Ordering::Relaxed);
        let s: Vec<Samples> = (0..5).map(|_| samples(300, 250, 3, false)).collect();
        let c: Vec<GopCache> = (0..5).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        // a clip after another, as when clips are dropped on a timeline
        for k in 0..5 {
            assert_eq!(index_of(&c[k].frame(&s[k], 10_000).expect("frame")), 10);
            assert!(live() <= 2, "{} live decoders after clip {k}", live());
        }
        assert_eq!(live(), 2);
        // the most recently used keep theirs and carry on without a seek
        assert_eq!(index_of(&c[4].frame(&s[4], 20_000).expect("frame")), 20);
        assert_eq!(s[4].resets.load(Ordering::Relaxed), 1);
        // a cache that lost its decoder still serves what it cached…
        let decodes = s[0].decodes.load(Ordering::Relaxed);
        assert_eq!(index_of(&c[0].frame(&s[0], 10_000).expect("cached")), 10);
        assert_eq!(s[0].decodes.load(Ordering::Relaxed), decodes);
        // …and decodes again with a new decoder, from the sync sample
        assert_eq!(index_of(&c[0].frame(&s[0], 20_000).expect("frame")), 20);
        assert_eq!(s[0].resets.load(Ordering::Relaxed), 2);
        assert_eq!(live(), 2);
        // a dropped cache no longer counts
        drop(c);
        assert_eq!(live(), 0);
    }

    #[test]
    fn caches_in_use_keep_their_decoders_whatever_the_cap() {
        // every layer of one composited frame was asked moments ago: none is idle
        let pool = Arc::new(Pool::new(2, Duration::from_secs(3600), FRAME_BUDGET));
        let s: Vec<Samples> = (0..5).map(|_| samples(300, 250, 3, false)).collect();
        let c: Vec<GopCache> = (0..5).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        for t in [10_000, 11_000, 12_000] {
            for k in 0..5 {
                assert_eq!(index_of(&c[k].frame(&s[k], t).expect("frame")) as i64, t / 1000);
            }
        }
        assert_eq!(pool.decoders.load(Ordering::Relaxed), 5);
        assert!(s.iter().all(|s| s.resets.load(Ordering::Relaxed) == 1), "no decoder restarted");
    }

    /// A burst of navigation (every clip of a timeline asked once within moments, as when
    /// thumbnails are made or the playhead jumps clip to clip) must not keep every clip's decoder
    /// and frames: only the caches of the frame being composited are in use.
    #[test]
    fn a_burst_of_single_requests_does_not_keep_every_decoder_and_frame() {
        // the real decoder caps and idle time; RECENT scaled down from 500 ms to the test's pace
        // (3 ms per clip). With the old rule (recent = idle) all 24 decoders stay live.
        let pool = Arc::new(Pool::new(MAX_LIVE_DECODERS, IDLE, FRAME_BUDGET).tiered(Duration::from_millis(2), HARD_DECODERS));
        let n = 24;
        let s: Vec<Samples> = (0..n).map(|_| samples(300, 250, 3, false)).collect();
        let c: Vec<GopCache> = (0..n).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        for k in 0..n {
            assert_eq!(index_of(&c[k].frame(&s[k], 10_000).expect("frame")), 10);
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(
            pool.decoders.load(Ordering::Relaxed) <= HARD_DECODERS + 1,
            "{} live decoders after a burst over {n} clips",
            pool.decoders.load(Ordering::Relaxed)
        );
        // the clip asked last, and a clip in use together with it, keep theirs and carry on
        let resets = s[n - 1].resets.load(Ordering::Relaxed);
        assert_eq!(index_of(&c[n - 1].frame(&s[n - 1], 11_000).expect("frame")), 11);
        assert_eq!(s[n - 1].resets.load(Ordering::Relaxed), resets);
    }

    /// The thumbnails an import queues: every clip asked once, quickly (the real caps and recency,
    /// no sleeping: all of them are "recent"). Each 4K software decoder costs ~0.5 GB, and 42
    /// clips used to hold 6+ GB at once.
    #[test]
    fn background_requests_leave_no_decoders_behind() {
        use filmcraft_media::cancel::with_background;
        let pool = Arc::new(Pool::new(MAX_LIVE_DECODERS, IDLE, FRAME_BUDGET).tiered(RECENT, HARD_DECODERS));
        let n = 24;
        let s: Vec<Samples> = (0..n).map(|_| samples(300, 250, 3, false)).collect();
        let c: Vec<GopCache> = (0..n).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        // a clip in use by a monitor
        assert_eq!(index_of(&c[0].frame(&s[0], 10_000).expect("frame")), 10);
        for k in 1..n {
            assert_eq!(index_of(&with_background(true, || c[k].frame(&s[k], 10_000)).expect("frame")), 10);
            assert!(pool.decoders.load(Ordering::Relaxed) <= 2, "{} live decoders after thumbnail {k}", pool.decoders.load(Ordering::Relaxed));
        }
        // the monitor's clip and the last thumbnail keep theirs and carry on without a seek
        assert_eq!(pool.decoders.load(Ordering::Relaxed), 2);
        assert_eq!(index_of(&c[0].frame(&s[0], 11_000).expect("frame")), 11);
        assert_eq!(s[0].resets.load(Ordering::Relaxed), 1);
        assert_eq!(index_of(&with_background(true, || c[n - 1].frame(&s[n - 1], 12_000)).expect("frame")), 12);
        assert_eq!(s[n - 1].resets.load(Ordering::Relaxed), 1);
        // a monitor asking for a frame of a thumbnail's clip makes its decoder one in use
        assert_eq!(index_of(&c[n - 1].frame(&s[n - 1], 13_000).expect("frame")), 13);
        for k in 1..4 {
            with_background(true, || c[k].frame(&s[k], 20_000)).expect("frame");
        }
        assert_eq!(index_of(&c[n - 1].frame(&s[n - 1], 14_000).expect("frame")), 14);
        assert!(c[n - 1].shared.state.lock().unwrap().decoder.is_some(), "the thumbnails left its decoder");
        // a cache that lost its decoder still decodes (from the sync sample)
        assert_eq!(index_of(&with_background(true, || c[1].frame(&s[1], 30_000)).expect("frame")), 30);
    }

    #[test]
    fn a_monitor_replaces_a_decoder_made_for_a_thumbnail() {
        use filmcraft_media::cancel::with_background;
        let pool = Arc::new(Pool::new(MAX_LIVE_DECODERS, IDLE, FRAME_BUDGET).tiered(RECENT, HARD_DECODERS));
        let (s, c) = (samples(300, 250, 3, false), GopCache::in_pool(None, pool.clone()));
        assert_eq!(index_of(&with_background(true, || c.frame(&s, 10_000)).expect("frame")), 10);
        // another thumbnail of the same clip carries on with the thumbnail decoder
        assert_eq!(index_of(&with_background(true, || c.frame(&s, 12_000)).expect("frame")), 12);
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
        // a frame a monitor waits for (not cached: far ahead) is decoded by a full-thread decoder
        assert_eq!(index_of(&c.frame(&s, 100_000).expect("frame")), 100);
        assert!(!c.shared.state.lock().unwrap().decoder.as_ref().is_some_and(|d| d.thread_limited()));
        assert_eq!(pool.decoders.load(Ordering::Relaxed), 1);
        // and keeps it for the next frames
        let resets = s.resets.load(Ordering::Relaxed);
        assert_eq!(index_of(&c.frame(&s, 101_000).expect("frame")), 101);
        assert_eq!(s.resets.load(Ordering::Relaxed), resets);
    }

    #[test]
    fn a_burst_of_single_requests_does_not_keep_every_clips_frames() {
        // 1×1 RGBA pictures (4 bytes): a shared budget of ten of them
        let pool = Arc::new(Pool::new(usize::MAX, Duration::from_secs(3600), 40).tiered(Duration::from_millis(2), usize::MAX));
        let n = 12;
        let s: Vec<Samples> = (0..n).map(|_| samples(300, 250, 0, false)).collect();
        let c: Vec<GopCache> = (0..n).map(|_| GopCache::in_pool(None, pool.clone())).collect();
        for k in 0..n {
            assert_eq!(index_of(&c[k].frame(&s[k], 10_000).expect("frame")), 10);
            std::thread::sleep(Duration::from_millis(5));
        }
        let held = pool.bytes.load(Ordering::Relaxed);
        assert!(held <= 40 + 11 * 4, "{held} bytes held after a burst over {n} clips");
    }

    #[test]
    fn intra_caches_give_up_their_spare_decoders_too() {
        let pool = Arc::new(Pool::new(1, Duration::ZERO, FRAME_BUDGET));
        let (a, b) = (samples(50, 1, 0, true), samples(50, 1, 0, true));
        let (ca, cb) = (GopCache::in_pool(None, pool.clone()), GopCache::in_pool(None, pool.clone()));
        assert_eq!(index_of(&ca.frame(&a, 3_000).expect("frame")), 3);
        assert_eq!(index_of(&cb.frame(&b, 4_000).expect("frame")), 4);
        let st = ca.shared.state.lock().unwrap();
        assert!(st.decoder.is_none() && st.spare.is_empty());
        drop(st);
        assert_eq!(pool.decoders.load(Ordering::Relaxed), 1);
        assert_eq!(index_of(&ca.frame(&a, 5_000).expect("frame")), 5);
    }

    #[test]
    fn limits_follow_the_installed_ram_and_frame_size() {
        let gib = 1u64 << 30;
        assert_eq!(auto_limits(None), (FRAME_BUDGET, MAX_LIVE_DECODERS));
        assert_eq!(auto_limits(Some(0)), (FRAME_BUDGET, MAX_LIVE_DECODERS));
        assert_eq!(auto_limits(Some(32 * gib)), ((32 * gib / 5) as usize, 4));
        assert_eq!(auto_limits(Some(4 * gib)), (1 << 30, 2), "small machines: 1 GiB, 2 decoders");
        assert_eq!(auto_limits(Some(512 * gib)), (16 << 30, 8), "capped");
        assert_eq!(auto_limits(Some(u64::MAX)).1, 8);
        // 1080p 8-bit keeps 64 frames, 4K 10-bit 43, 6.2K 10-bit 24 (not 5 GB), absurd sizes 24
        assert_eq!(min_frames(1920 * 1080 * 3 / 2), 64);
        assert_eq!(min_frames(3840 * 2160 * 3), 43);
        assert_eq!(min_frames(6240 * 4160 * 3), 24);
        assert_eq!((min_frames(0), min_frames(usize::MAX)), (64, 24));
        assert_eq!(auto_vram_budget(Some(12 << 30)), 3 << 30);
        assert_eq!(auto_vram_budget(None), 2 << 30);
        assert_eq!(auto_vram_budget(Some(1 << 30)), 512 << 20);
    }

    #[test]
    fn idle_caches_give_up_their_frames_beyond_the_shared_budget() {
        // 1×1 RGBA pictures: 4 bytes each, a shared budget of ten of them
        let pool = Arc::new(Pool::new(usize::MAX, Duration::ZERO, 40));
        let held = || pool.bytes.load(Ordering::Relaxed);
        let (a, b) = (samples(300, 250, 0, false), samples(300, 250, 0, false));
        let (ca, cb) = (GopCache::in_pool(None, pool.clone()), GopCache::in_pool(None, pool.clone()));
        // the only cache keeps what its own budget allows
        assert_eq!(index_of(&ca.frame(&a, 10_000).expect("frame")), 10);
        assert_eq!(held(), 44);
        // a second clip takes the room from the idle first one, earliest frames first
        assert_eq!(index_of(&cb.frame(&b, 5_000).expect("frame")), 5);
        assert_eq!(held(), 40);
        let decodes = b.decodes.load(Ordering::Relaxed);
        assert!((0..=5).all(|i| index_of(&cb.frame(&b, i * 1000).expect("cached")) as i64 == i));
        assert_eq!(b.decodes.load(Ordering::Relaxed), decodes, "the clip in use lost nothing");
        let st = ca.shared.store.lock().unwrap();
        assert_eq!((st.frames.len(), st.bytes), (4, 16));
        assert_eq!(st.frames.keys().next(), Some(&7_000));
        drop(st);
        // an evicted frame decodes again
        assert_eq!(index_of(&ca.frame(&a, 2_000).expect("frame")), 2);
        assert!(held() <= 40 + 4 * 3, "{} bytes held", held());
        drop((ca, cb));
        assert_eq!(held(), 0);
    }

    #[test]
    fn caches_in_use_keep_their_frames_whatever_the_shared_budget() {
        let pool = Arc::new(Pool::new(usize::MAX, Duration::from_secs(3600), 40));
        let (a, b) = (samples(300, 250, 0, false), samples(300, 250, 0, false));
        let (ca, cb) = (GopCache::in_pool(None, pool.clone()), GopCache::in_pool(None, pool.clone()));
        assert_eq!(index_of(&ca.frame(&a, 10_000).expect("frame")), 10);
        assert_eq!(index_of(&cb.frame(&b, 10_000).expect("frame")), 10);
        assert_eq!(pool.bytes.load(Ordering::Relaxed), 88);
        let decodes = a.decodes.load(Ordering::Relaxed);
        assert_eq!(index_of(&ca.frame(&a, 0).expect("cached")), 0);
        assert_eq!(a.decodes.load(Ordering::Relaxed), decodes);
    }

    /// Pictures big enough for the plane pool (its own size, so that parallel tests cannot take
    /// the buffers), no reordering, one long GOP.
    struct Big(usize);
    const BIG: (u32, u32) = (200, 101);

    impl VideoDecoder for Big {
        fn decode(&mut self, _sample: &[u8], pts: i64) -> crate::Result<Vec<DecodedFrame>> {
            let mut px = filmcraft_frame::pool::take_u8((BIG.0 * BIG.1 * 4) as usize);
            px.resize((BIG.0 * BIG.1 * 4) as usize, (pts / 1000) as u8);
            Ok(vec![DecodedFrame { pts, frame: VideoFrame::rgba8(BIG.0, BIG.1, px), draft: false }])
        }
        fn flush(&mut self) -> Vec<DecodedFrame> {
            Vec::new()
        }
        fn reset(&mut self) {}
        fn name(&self) -> &str {
            "big"
        }
    }

    impl VideoSamples for Big {
        fn count(&self) -> usize {
            self.0
        }
        fn pts(&self, i: usize) -> i64 {
            i as i64 * 1000
        }
        fn sync_before(&self, _i: usize) -> usize {
            0
        }
        fn sample_at(&self, t: i64) -> Option<usize> {
            let i = (t / 1000) as usize;
            (i < self.0).then_some(i)
        }
        fn read(&self, i: usize) -> crate::Result<Vec<u8>> {
            Ok(vec![i as u8])
        }
        fn make_decoder(&self) -> crate::Result<Box<dyn VideoDecoder>> {
            Ok(Box::new(Big(self.0)))
        }
    }

    #[test]
    fn evicted_frames_are_decoded_into_again() {
        let s = Big(200);
        // room for MIN_FRAMES pictures only
        let mut c = GopCache::new(None);
        c.budget = 0;
        let reused = || filmcraft_frame::pool::stats().reused;
        let before = reused();
        // the cache holds MIN_FRAMES pictures: none is evicted yet, every plane is new
        for i in 0..MIN_FRAMES as i64 {
            assert_eq!(c.frame(&s, i * 1000).expect("frame").width, BIG.0);
        }
        assert_eq!(reused(), before);
        // from here on each picture evicts one, whose buffer the next picture is decoded into
        let more = 40;
        for i in MIN_FRAMES as i64..MIN_FRAMES as i64 + more {
            let f = c.frame(&s, i * 1000).expect("frame");
            assert!(matches!(&f.data, filmcraft_frame::PixelData::Rgba8(d) if d.iter().all(|&b| b == i as u8)), "frame {i} has its own pixels");
        }
        assert!(reused() - before >= more as u64 - 2, "{} of {more} planes recycled", reused() - before);
    }

    /// An open-GOP stream like iPhone HEVC: 8 pictures a GOP; after the first, each GOP stores
    /// its random-access picture (display 8·g) first, then two leading pictures shown before it
    /// (8·g − 2, 8·g − 1, predicted from the GOP before), then the rest. pts = 1000 · display.
    struct LeadingGop {
        /// (display index, sync) in decode order
        order: Vec<(i64, bool)>,
        resets: Arc<AtomicUsize>,
    }

    impl LeadingGop {
        fn new(gops: i64) -> Self {
            let mut order: Vec<(i64, bool)> = (0..6).map(|d| (d, d == 0)).collect();
            for g in 1..gops {
                order.extend([(8 * g, true), (8 * g - 2, false), (8 * g - 1, false)]);
                order.extend((8 * g + 1..8 * g + 6).map(|d| (d, false)));
            }
            Self { order, resets: Default::default() }
        }
    }

    /// Leaves out the leading pictures of the random-access picture a run starts at (RASL
    /// pictures, as the HEVC decoders do), outputs the others at once.
    struct LeadingGopDec {
        run_start: Option<i64>,
        resets: Arc<AtomicUsize>,
    }

    impl VideoDecoder for LeadingGopDec {
        fn decode(&mut self, _sample: &[u8], pts: i64) -> crate::Result<Vec<DecodedFrame>> {
            let start = *self.run_start.get_or_insert(pts);
            Ok(if pts < start { Vec::new() } else { vec![picture((pts, false))] })
        }
        fn flush(&mut self) -> Vec<DecodedFrame> {
            Vec::new()
        }
        fn reset(&mut self) {
            self.resets.fetch_add(1, Ordering::Relaxed);
            self.run_start = None;
        }
        fn name(&self) -> &str {
            "open gop"
        }
    }

    impl VideoSamples for LeadingGop {
        fn count(&self) -> usize {
            self.order.len()
        }
        fn pts(&self, i: usize) -> i64 {
            self.order[i].0 * 1000
        }
        fn sync_before(&self, i: usize) -> usize {
            (0..=i).rev().find(|&k| self.order[k].1).unwrap_or(0)
        }
        fn sample_at(&self, t: i64) -> Option<usize> {
            self.order.iter().position(|&(d, _)| d == t / 1000)
        }
        fn read(&self, i: usize) -> crate::Result<Vec<u8>> {
            Ok(vec![i as u8])
        }
        fn make_decoder(&self) -> crate::Result<Box<dyn VideoDecoder>> {
            Ok(Box::new(LeadingGopDec { run_start: None, resets: self.resets.clone() }))
        }
    }

    /// A leading picture of an open GOP, asked for first (a clip trimmed to start on it, the
    /// playhead parked on it), came out black: decoding started at the random-access picture
    /// after it, which leaves it out.
    #[test]
    fn leading_pictures_of_an_open_gop_decode_from_the_gop_before() {
        let s = LeadingGop::new(6);
        for d in [6usize, 7, 14, 15, 39] {
            let c = GopCache::new(None);
            assert_eq!(index_of(&c.frame(&s, d as i64 * 1000).expect("frame")), d);
        }
        // the random-access picture first, then its leading pictures: the run that started at
        // it left them out, so they restart a GOP earlier
        let c = GopCache::new(None);
        assert_eq!(index_of(&c.frame(&s, 16_000).expect("frame")), 16);
        assert_eq!(index_of(&c.frame(&s, 15_000).expect("frame")), 15);
        assert_eq!(index_of(&c.frame(&s, 14_000).expect("frame")), 14);
        // playing through: one seek, every picture in turn
        let s = LeadingGop::new(6);
        let c = GopCache::new(None);
        for d in 0..46usize {
            assert_eq!(index_of(&c.frame(&s, d as i64 * 1000).expect("frame")), d);
        }
        assert_eq!(s.resets.load(Ordering::Relaxed), 1);
    }
}
