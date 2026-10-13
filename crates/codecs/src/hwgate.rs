//! One hardware decode at a time per decode engine, most urgent first.
//!
//! A GPU has one video decode engine (VCN), shared by every stream decoding at once. The GOP
//! caches run every hardware `decode` call through [`run`], which serves waiting work in deadline
//! order ([`filmcraft_media::cancel::deadline`]: when the frame is due on screen), so the
//! pictures of frames due sooner are submitted first. (Submission order only: VA-API queues the
//! work and returns, so the engine may still interleave streams; making every call wait for its
//! picture to finish cost ~6 % of throughput at 6.2K without fewer dropped frames, measured.)
//! Software decoders are not gated (they use the CPU cores, not the engine).
//!
//! Each GPU has its own engine, so the gate is per decode node ([`crate::hw::decode_node`] of the
//! calling thread; fork 6E.3: the lanes of a two-GPU export decode at the same time instead of
//! taking turns). Threads with no node (playback, the default GPU) share one gate as before.
//!
//! Re-entrant on one thread (a decode that decodes again inside, e.g. a nested request) and
//! released when the work unwinds.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

struct Gate {
    busy: bool,
    /// (deadline, arrival) of the waiting work.
    waiting: BTreeSet<(u64, u64)>,
}

/// One decode engine's gate.
struct Engine {
    gate: Mutex<Gate>,
    freed: Condvar,
}

impl Engine {
    fn new() -> Engine {
        Engine { gate: Mutex::new(Gate { busy: false, waiting: BTreeSet::new() }), freed: Condvar::new() }
    }
}

/// Most engines kept apart; nodes beyond these share the default engine's gate.
const MAX_ENGINES: usize = 8;

/// The gate of each decode node seen so far (`None`: the default GPU).
static ENGINES: Mutex<Vec<(Option<String>, Arc<Engine>)>> = Mutex::new(Vec::new());
static ARRIVAL: AtomicU64 = AtomicU64::new(0);
/// Hardware decodes that waited for another one (`perf.stats`).
static WAITS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static HOLDING: Cell<bool> = const { Cell::new(false) };
}

/// The gate of decode node `node`.
fn engine(node: Option<String>) -> Arc<Engine> {
    let mut g = ENGINES.lock().unwrap_or_else(PoisonError::into_inner);
    // the default engine always has the first slot, so the cap counts it
    if g.is_empty() {
        g.push((None, Arc::new(Engine::new())));
    }
    if let Some((_, e)) = g.iter().find(|(n, _)| *n == node) {
        return e.clone();
    }
    if g.len() >= MAX_ENGINES {
        return g.first().map_or_else(|| Arc::new(Engine::new()), |(_, e)| e.clone());
    }
    let e = Arc::new(Engine::new());
    g.push((node, e.clone()));
    e
}

/// Releases the gate (also when the work panics).
struct Held(Arc<Engine>);

impl Drop for Held {
    fn drop(&mut self) {
        HOLDING.with(|h| h.set(false));
        self.0.gate.lock().unwrap_or_else(PoisonError::into_inner).busy = false;
        self.0.freed.notify_all();
    }
}

/// Run `f` (one hardware decode call) when no other hardware decode runs on this thread's decode
/// engine and no more urgent one waits for it.
pub fn run<R>(f: impl FnOnce() -> R) -> R {
    if HOLDING.with(Cell::get) {
        return f();
    }
    let e = engine(crate::hw::decode_node());
    let me = (filmcraft_media::cancel::deadline(), ARRIVAL.fetch_add(1, Ordering::Relaxed));
    {
        let mut g = e.gate.lock().unwrap_or_else(PoisonError::into_inner);
        g.waiting.insert(me);
        let mut waited = false;
        while g.busy || g.waiting.first() != Some(&me) {
            waited = true;
            g = e.freed.wait(g).unwrap_or_else(PoisonError::into_inner);
        }
        g.waiting.remove(&me);
        g.busy = true;
        if waited {
            WAITS.fetch_add(1, Ordering::Relaxed);
        }
    }
    HOLDING.with(|h| h.set(true));
    let _held = Held(e);
    f()
}

/// Hardware decodes that had to wait for another one so far.
pub fn waits() -> u64 {
    WAITS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn the_most_urgent_waiting_work_goes_first() {
        let order = Arc::new(Mutex::new(Vec::new()));
        // hold the gate while three workers with different deadlines queue up
        let (tx, rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            run(|| {
                let _ = rx.recv_timeout(Duration::from_secs(5));
            })
        });
        std::thread::sleep(Duration::from_millis(50));
        let mut ts = Vec::new();
        for due in [300u64, 100, 200] {
            let o = order.clone();
            ts.push(std::thread::spawn(move || filmcraft_media::cancel::with_deadline(due, || run(|| o.lock().unwrap().push(due)))));
            std::thread::sleep(Duration::from_millis(30));
        }
        tx.send(()).unwrap();
        holder.join().unwrap();
        for t in ts {
            t.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![100, 200, 300]);
        // re-entrant, and released after a panic
        assert_eq!(run(|| run(|| 7)), 7);
        assert!(std::panic::catch_unwind(|| run(|| panic!("decoder bug"))).is_err());
        assert_eq!(run(|| 1), 1, "the gate is free again");
    }

    /// Fork 6E.3: two decode nodes run at the same time; one node stays one at a time.
    #[test]
    fn engines_are_gated_separately() {
        let node = |n: Option<&str>| {
            let n = n.map(str::to_string);
            move || crate::hw::set_thread_decode_node(n)
        };
        // two nodes: both inside the gate at once (each waits, bounded, to see the other inside)
        let both = Arc::new(AtomicU64::new(0));
        let ts: Vec<_> = [Some("/dev/dri/renderD900"), Some("/dev/dri/renderD901")]
            .into_iter()
            .map(|n| {
                let (b, set) = (both.clone(), node(n));
                std::thread::spawn(move || {
                    set();
                    run(|| {
                        b.fetch_add(1, Ordering::SeqCst);
                        let t = std::time::Instant::now();
                        while b.load(Ordering::SeqCst) < 2 && t.elapsed() < Duration::from_secs(3) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        b.load(Ordering::SeqCst)
                    })
                })
            })
            .collect();
        for t in ts {
            assert_eq!(t.join().unwrap(), 2, "the other node's decode ran at the same time");
        }
        // one node: never two inside at once
        let inside = Arc::new(AtomicU64::new(0));
        let most = Arc::new(AtomicU64::new(0));
        let ts: Vec<_> = (0..4)
            .map(|_| {
                let (i, m, set) = (inside.clone(), most.clone(), node(Some("/dev/dri/renderD902")));
                std::thread::spawn(move || {
                    set();
                    for _ in 0..5 {
                        run(|| {
                            let now = i.fetch_add(1, Ordering::SeqCst) + 1;
                            m.fetch_max(now, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(2));
                            i.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                })
            })
            .collect();
        for t in ts {
            t.join().unwrap();
        }
        assert_eq!(most.load(Ordering::SeqCst), 1);
        // hostile: many nodes share the default gate past the cap instead of growing the table
        for k in 0..64 {
            node(Some(&format!("/dev/dri/renderD{}", 1000 + k)))();
            assert_eq!(run(|| k), k);
        }
        node(None)();
        assert!(ENGINES.lock().unwrap().len() <= MAX_ENGINES);
    }
}
