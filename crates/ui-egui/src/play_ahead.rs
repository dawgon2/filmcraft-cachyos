//! Audio played ahead: playback mixes on its own thread into a ring buffer that stays
//! [`LEAD_S`] ahead of the device, and the device callback only copies samples out of it.
//!
//! Mixing (decoding AAC, effects, opening a clip's file at a cut) used to run inside the device
//! callback, which has one I/O buffer (~10 ms) to return. On a busy Mac any stall there missed the
//! device's deadline, heard as static and dropouts that were different on every play. Now a stall
//! only drains part of the lead; the callback never decodes, allocates or takes a lock.
//!
//! When the ring does run dry (an underrun), the callback plays silence for the missing frames and
//! counts them, and the mixer skips the same span, so sound stays locked to the picture.
//!
//! Desktop only: the web build has no threads and keeps mixing in its callback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// How far ahead of the device the mixer keeps the ring (s). Edits made while playing are heard
/// this much later.
pub const LEAD_S: f64 = 0.2;
/// Frames mixed per block on the mixing thread.
pub const BLOCK_FRAMES: usize = 1024;
/// Ceiling on mixed channels (large interfaces and aggregate devices stay well under it).
const MAX_CHANNELS: usize = 64;
/// Fork A1.2: the most the lead grows to (s): after underruns, or for a device with large buffers.
pub const MAX_LEAD_S: f64 = 0.5;
/// Fork A1.2: the lead grows by this much (s) after each underrun.
const LEAD_STEP_S: f64 = 0.05;
/// Fork A1.2: the lead is at least this many device buffers.
const LEAD_BUFFERS: usize = 4;
/// Fork A1.2: highest sample rate the ring is sized for (a hostile device rate can't allocate
/// gigabytes), and the most samples the ring holds.
const MAX_RATE: u32 = 768_000;
const MAX_RING_SAMPLES: usize = 1 << 22;
/// Fork A1.2: a mixer that panics this many blocks in a row stops (playback goes on silent).
const MAX_PANICS_IN_A_ROW: u32 = 4;

/// Underrun counters for the current (or last) play, readable from any thread (`perf.stats`).
#[derive(Debug, Default)]
pub struct AudioStats {
    /// Device callbacks served.
    pub callbacks: AtomicU64,
    /// Callbacks the ring could not fill completely.
    pub underruns: AtomicU64,
    /// Frames played as silence because the ring was empty.
    pub missing_frames: AtomicU64,
    /// Fewest frames left in the ring after a callback, once it first filled to the lead
    /// (`u64::MAX` until then).
    pub min_lead_frames: AtomicU64,
    /// Fork A1.2: blocks whose mixing panicked (played as silence).
    pub mixer_panics: AtomicU64,
    /// Fork A1.2: the mixer stopped after [`MAX_PANICS_IN_A_ROW`] panics; playback is silent.
    pub mixer_stopped: AtomicBool,
    /// Fork A1.2: the lead the mixer keeps now (frames), grown from [`LEAD_S`] by the device's
    /// buffer size and by underruns.
    pub lead_frames: AtomicU64,
}

impl AudioStats {
    pub fn reset(&self) {
        self.callbacks.store(0, Ordering::Relaxed);
        self.underruns.store(0, Ordering::Relaxed);
        self.missing_frames.store(0, Ordering::Relaxed);
        self.min_lead_frames.store(u64::MAX, Ordering::Relaxed);
        self.mixer_panics.store(0, Ordering::Relaxed);
        self.mixer_stopped.store(false, Ordering::Relaxed);
        self.lead_frames.store(0, Ordering::Relaxed);
    }

    pub fn to_json(&self, sample_rate: u32) -> serde_json::Value {
        let min = self.min_lead_frames.load(Ordering::Relaxed);
        serde_json::json!({
            "callbacks": self.callbacks.load(Ordering::Relaxed),
            "underruns": self.underruns.load(Ordering::Relaxed),
            "missingFrames": self.missing_frames.load(Ordering::Relaxed),
            "minLeadMs": if min == u64::MAX || sample_rate == 0 { serde_json::Value::Null } else { serde_json::json!(min as f64 * 1e3 / sample_rate as f64) },
            "leadMs": if sample_rate == 0 { serde_json::Value::Null } else { serde_json::json!(self.lead_frames.load(Ordering::Relaxed) as f64 * 1e3 / sample_rate as f64) },
            "mixerPanics": self.mixer_panics.load(Ordering::Relaxed),
            "mixerStopped": self.mixer_stopped.load(Ordering::Relaxed),
        })
    }
}

/// Single-producer, single-consumer ring of interleaved f32 samples (stored as bits, so it needs
/// no `unsafe`). Indices count samples written / read since the start and only grow.
struct Ring {
    slots: Vec<AtomicU32>,
    write: AtomicU64,
    read: AtomicU64,
}

impl Ring {
    fn new(samples: usize) -> Self {
        Self { slots: (0..samples.max(1)).map(|_| AtomicU32::new(0)).collect(), write: AtomicU64::new(0), read: AtomicU64::new(0) }
    }

    fn len(&self) -> usize {
        self.write.load(Ordering::Acquire).saturating_sub(self.read.load(Ordering::Acquire)) as usize
    }

    /// Producer: append as many samples as fit; returns how many were written.
    fn push(&self, src: &[f32]) -> usize {
        let cap = self.slots.len();
        let w = self.write.load(Ordering::Relaxed);
        let free = cap.saturating_sub(w.saturating_sub(self.read.load(Ordering::Acquire)) as usize);
        let n = src.len().min(free);
        for (i, x) in src.iter().take(n).enumerate() {
            if let Some(s) = self.slots.get(((w + i as u64) % cap as u64) as usize) {
                s.store(x.to_bits(), Ordering::Relaxed);
            }
        }
        self.write.store(w + n as u64, Ordering::Release);
        n
    }

    /// Consumer: fill `dst` (`dev_ch` interleaved channels) with whole frames of `ch` channels from
    /// the ring: the first `min(ch, dev_ch)` channels copied, any further device channels zeroed.
    /// Returns the frames copied; the rest of `dst` is left for the caller.
    fn pop(&self, dst: &mut [f32], dev_ch: usize, ch: usize) -> usize {
        let cap = self.slots.len();
        let r = self.read.load(Ordering::Relaxed);
        let avail = self.write.load(Ordering::Acquire).saturating_sub(r) as usize / ch.max(1);
        let n = (dst.len() / dev_ch.max(1)).min(avail);
        for (f, frame) in dst.chunks_mut(dev_ch.max(1)).take(n).enumerate() {
            for (c, x) in frame.iter_mut().enumerate() {
                *x = if c < ch {
                    self.slots.get(((r + (f * ch + c) as u64) % cap as u64) as usize).map_or(0.0, |s| f32::from_bits(s.load(Ordering::Relaxed)))
                } else {
                    0.0
                };
            }
        }
        self.read.store(r + (n * ch) as u64, Ordering::Release);
        n
    }
}

/// Stops the mixing thread when the device callback that owns it is dropped (stream stopped).
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Mixes `mix(cursor, buf, channels)` (interleaved frames from device frame `cursor`) on its own
/// thread, from `start`, and returns the device callback that plays it (`fill(buf, channels)`).
/// The first block is mixed before this returns, so playback starts with sound. The thread runs
/// until the returned callback is dropped.
pub fn spawn<M>(mut mix: M, start: i64, sample_rate: u32, channels: usize, stats: Arc<AudioStats>) -> Box<dyn FnMut(&mut [f32], usize) + Send>
where
    M: FnMut(i64, &mut [f32], usize) + Send + 'static,
{
    stats.reset();
    let ch = channels.clamp(1, MAX_CHANNELS);
    let rate = sample_rate.clamp(1, MAX_RATE) as f64;
    // the ring holds the largest lead; the mixer keeps it filled to `lead` (fork A1.2)
    let max_lead = ((rate * MAX_LEAD_S) as usize).max(BLOCK_FRAMES * 4).min(MAX_RING_SAMPLES / ch);
    let base_lead = ((rate * LEAD_S) as usize).max(BLOCK_FRAMES * 2).min(max_lead);
    let step = ((rate * LEAD_STEP_S) as usize).max(1);
    let lead = Arc::new(AtomicUsize::new(base_lead));
    stats.lead_frames.store(base_lead as u64, Ordering::Relaxed);
    let ring = Arc::new(Ring::new(max_lead * ch));
    let mut block = vec![0.0; BLOCK_FRAMES * ch];
    // (a panicking mix plays as silence, here and on the mixing thread)
    let mut mix_block = move |cursor: i64, block: &mut [f32]| -> bool {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mix(cursor, block, ch))).is_ok();
        if !ok {
            block.fill(0.0);
        }
        // a mixer left half-updated by an earlier panic (or a bad effect) must not send NaN or
        // infinity to the device
        for x in block.iter_mut().filter(|x| !x.is_finite()) {
            *x = 0.0;
        }
        ok
    };
    if !mix_block(start, &mut block) {
        stats.mixer_panics.fetch_add(1, Ordering::Relaxed);
    }
    ring.push(&block);
    // Frames the callback played as silence that the mixer has not skipped yet.
    let skip = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (ring, skip, stop, lead, stats) = (ring.clone(), skip.clone(), stop.clone(), lead.clone(), stats.clone());
        let body = move || {
            let mut cursor = start + BLOCK_FRAMES as i64;
            let mut panics_in_a_row = 0;
            while !stop.load(Ordering::Acquire) {
                if ring.len() + block.len() > lead.load(Ordering::Relaxed) * ch {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                cursor += skip.swap(0, Ordering::AcqRel) as i64;
                block.fill(0.0);
                if mix_block(cursor, &mut block) {
                    panics_in_a_row = 0;
                } else if !stop.load(Ordering::Acquire) {
                    // (a stopped play's mixer finishing its block must not mark the next play,
                    // which shares `stats`)
                    stats.mixer_panics.fetch_add(1, Ordering::Relaxed);
                    panics_in_a_row += 1;
                    if panics_in_a_row >= MAX_PANICS_IN_A_ROW {
                        stats.mixer_stopped.store(true, Ordering::Relaxed);
                        log::error!("audio mixing stopped after {MAX_PANICS_IN_A_ROW} panics in a row; playback continues silent");
                        return;
                    }
                }
                cursor += BLOCK_FRAMES as i64;
                // silence played while this block was mixing: its first frames are already late
                let late = skip.swap(0, Ordering::AcqRel) as usize;
                let drop = late.min(BLOCK_FRAMES);
                cursor += (late - drop) as i64;
                ring.push(block.get(drop * ch..).unwrap_or_default());
            }
        };
        let spawned = std::thread::Builder::new().name("filmcraft-audio-mix".into()).spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
                log::error!("audio mixing stopped after a panic; playback continues silent");
            }
        });
        if let Err(e) = spawned {
            log::error!("audio mixing thread: {e}");
        }
    }
    let guard = StopOnDrop(stop);
    let mut primed = false;
    // the previous callback ran short: one underrun episode grows the lead once (fork A1.2)
    let mut short = false;
    Box::new(move |buf: &mut [f32], dev_ch: usize| {
        let _keep = &guard;
        stats.callbacks.fetch_add(1, Ordering::Relaxed);
        // a device wider or narrower than the mix (it changed since `channels` was read) still
        // plays the channels both have
        let dev_ch = dev_ch.max(1);
        let have = ring.pop(buf, dev_ch, ch);
        if let Some(rest) = buf.get_mut(have * dev_ch..) {
            rest.fill(0.0);
        }
        let n = buf.len() / dev_ch;
        // the lead covers a few of the device's buffers, and grows after an underrun (fork A1.2;
        // atomics only: this is the device's real-time thread)
        let current = lead.load(Ordering::Relaxed);
        let mut want = current.max(n.saturating_mul(LEAD_BUFFERS));
        if have < n {
            stats.underruns.fetch_add(1, Ordering::Relaxed);
            stats.missing_frames.fetch_add((n - have) as u64, Ordering::Relaxed);
            skip.fetch_add((n - have) as u64, Ordering::AcqRel);
            if primed && !short {
                want = want.saturating_add(step);
            }
        }
        short = have < n;
        let want = want.min(max_lead);
        if want != current {
            lead.store(want, Ordering::Relaxed);
            stats.lead_frames.store(want as u64, Ordering::Relaxed);
        }
        let left = ring.len();
        primed |= left + 2 * BLOCK_FRAMES * ch > want * ch;
        if primed {
            stats.min_lead_frames.fetch_min((left / ch) as u64, Ordering::Relaxed);
        }
    })
}
