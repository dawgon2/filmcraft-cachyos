//! One hardware decode at a time, most urgent first.
//!
//! A GPU has one video decode engine (VCN), shared by every stream decoding at once. The GOP
//! caches run every hardware `decode` call through [`run`], which serves waiting work in deadline
//! order ([`filmcraft_media::cancel::deadline`]: when the frame is due on screen), so the
//! pictures of frames due sooner are submitted first. (Submission order only: VA-API queues the
//! work and returns, so the engine may still interleave streams; making every call wait for its
//! picture to finish cost ~6 % of throughput at 6.2K without fewer dropped frames, measured.)
//! Software decoders are not gated (they use the CPU cores, not the engine).
//!
//! Re-entrant on one thread (a decode that decodes again inside, e.g. a nested request) and
//! released when the work unwinds.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};

struct Gate {
    busy: bool,
    /// (deadline, arrival) of the waiting work.
    waiting: BTreeSet<(u64, u64)>,
}

static GATE: Mutex<Gate> = Mutex::new(Gate { busy: false, waiting: BTreeSet::new() });
static FREED: Condvar = Condvar::new();
static ARRIVAL: AtomicU64 = AtomicU64::new(0);
/// Hardware decodes that waited for another one (`perf.stats`).
static WAITS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static HOLDING: Cell<bool> = const { Cell::new(false) };
}

/// Releases the gate (also when the work panics).
struct Held;

impl Drop for Held {
    fn drop(&mut self) {
        HOLDING.with(|h| h.set(false));
        GATE.lock().unwrap_or_else(PoisonError::into_inner).busy = false;
        FREED.notify_all();
    }
}

/// Run `f` (one hardware decode call) when no other hardware decode runs and no more urgent one
/// waits.
pub fn run<R>(f: impl FnOnce() -> R) -> R {
    if HOLDING.with(Cell::get) {
        return f();
    }
    let me = (filmcraft_media::cancel::deadline(), ARRIVAL.fetch_add(1, Ordering::Relaxed));
    {
        let mut g = GATE.lock().unwrap_or_else(PoisonError::into_inner);
        g.waiting.insert(me);
        let mut waited = false;
        while g.busy || g.waiting.first() != Some(&me) {
            waited = true;
            g = FREED.wait(g).unwrap_or_else(PoisonError::into_inner);
        }
        g.waiting.remove(&me);
        g.busy = true;
        if waited {
            WAITS.fetch_add(1, Ordering::Relaxed);
        }
    }
    HOLDING.with(|h| h.set(true));
    let _held = Held;
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
}
