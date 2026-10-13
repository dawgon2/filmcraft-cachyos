//! The shared chunk queue the lanes pull from (fork 6E.3, DESIGN-6E3 §2.1 / §2.5).
//!
//! A lane pulls the front chunk, so chunks finish roughly in file order and the joiner waits
//! little. A chunk whose lane failed goes back to the front for another lane. A lane asking for
//! work while the queue is empty but chunks are still out waits: one of them may come back.

use super::chunker::Chunk;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Debug, Default)]
struct State {
    queue: VecDeque<Chunk>,
    /// Chunks pulled and neither finished nor re-queued.
    out: usize,
    cancelled: bool,
}

/// Chunks waiting for a lane.
#[derive(Debug, Default)]
pub struct Scheduler {
    state: Mutex<State>,
    changed: Condvar,
}

impl Scheduler {
    /// A queue holding `chunks` in order.
    pub fn new(chunks: impl IntoIterator<Item = Chunk>) -> Scheduler {
        Scheduler { state: Mutex::new(State { queue: chunks.into_iter().collect(), out: 0, cancelled: false }), changed: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next chunk for a lane; waits while other lanes still hold chunks that may come back.
    /// `None`: every chunk is done, or the export was cancelled.
    pub fn pull(&self) -> Option<Chunk> {
        self.pull_preferring(|_| false)
    }

    /// As [`pull`](Self::pull), but the first waiting chunk `avoid` does not refuse comes first;
    /// a chunk it refuses is still taken when nothing else waits (a preference, never a stall:
    /// fork 6E.3 sends 4:2:2-source chunks to the lane whose GPU decodes 4:2:2).
    pub fn pull_preferring(&self, avoid: impl Fn(&Chunk) -> bool) -> Option<Chunk> {
        let mut s = self.lock();
        loop {
            if s.cancelled {
                return None;
            }
            let pick = s.queue.iter().position(|c| !avoid(c)).unwrap_or(0);
            if let Some(c) = s.queue.remove(pick) {
                s.out = s.out.saturating_add(1);
                return Some(c);
            }
            if s.out == 0 {
                return None;
            }
            s = self.changed.wait(s).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The lane finished a chunk it pulled.
    pub fn done(&self) {
        let mut s = self.lock();
        s.out = s.out.saturating_sub(1);
        drop(s);
        self.changed.notify_all();
    }

    /// The lane failed on `chunk`: it goes back to the front for another lane.
    pub fn requeue(&self, chunk: Chunk) {
        let mut s = self.lock();
        s.out = s.out.saturating_sub(1);
        s.queue.push_front(chunk);
        drop(s);
        self.changed.notify_all();
    }

    /// Stop handing out chunks; waiting lanes return `None`.
    pub fn cancel(&self) {
        self.lock().cancelled = true;
        self.changed.notify_all();
    }

    /// Chunks still waiting in the queue.
    pub fn waiting(&self) -> usize {
        self.lock().queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn chunks(n: usize) -> Vec<Chunk> {
        (0..n).map(|i| Chunk { index: i, start: i as i64 * 10, end: i as i64 * 10 + 10 }).collect()
    }

    #[test]
    fn front_first_and_requeue_to_the_front() {
        let s = Scheduler::new(chunks(3));
        let a = s.pull().expect("0");
        assert_eq!(a.index, 0);
        let b = s.pull().expect("1");
        s.requeue(a);
        assert_eq!(s.pull().map(|c| c.index), Some(0));
        s.done();
        s.done();
        assert_eq!(s.pull().map(|c| c.index), Some(2));
        let _ = b;
        s.done();
        assert_eq!(s.pull(), None);
        // extra done() calls (a buggy lane) do not underflow or wake anyone wrongly
        s.done();
        assert_eq!(s.pull(), None);
        assert!(Scheduler::new([]).pull().is_none());
    }

    #[test]
    fn a_preference_reorders_but_never_stalls() {
        let s = Scheduler::new(chunks(3));
        let odd = |c: &Chunk| c.index % 2 == 1;
        assert_eq!(s.pull_preferring(|c| !odd(c)).map(|c| c.index), Some(1));
        assert_eq!(s.pull_preferring(odd).map(|c| c.index), Some(0));
        // only refused chunks are left: taken anyway
        assert_eq!(s.pull_preferring(|_| true).map(|c| c.index), Some(2));
        s.done();
        s.done();
        s.done();
        assert_eq!(s.pull_preferring(|_| true), None);
    }

    #[test]
    fn a_waiting_lane_takes_a_failed_chunk() {
        let s = Arc::new(Scheduler::new(chunks(1)));
        let c = s.pull().expect("0");
        let s2 = Arc::clone(&s);
        let other = std::thread::spawn(move || s2.pull());
        std::thread::sleep(Duration::from_millis(50));
        s.requeue(c);
        let got = other.join().unwrap_or(None);
        assert_eq!(got.map(|c| c.index), Some(0));
    }

    #[test]
    fn cancel_and_completion_wake_waiting_lanes() {
        for cancel in [true, false] {
            let s = Arc::new(Scheduler::new(chunks(1)));
            let _c = s.pull();
            let waiters: Vec<_> = (0..3)
                .map(|_| {
                    let s = Arc::clone(&s);
                    std::thread::spawn(move || s.pull())
                })
                .collect();
            std::thread::sleep(Duration::from_millis(50));
            if cancel {
                s.cancel();
            } else {
                s.done();
            }
            for w in waiters {
                assert_eq!(w.join().unwrap_or(Some(chunks(1)[0])), None);
            }
        }
    }

    #[test]
    fn many_lanes_take_every_chunk_once() {
        let s = Arc::new(Scheduler::new(chunks(200)));
        let lanes: Vec<_> = (0..4)
            .map(|_| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    let mut got = Vec::new();
                    while let Some(c) = s.pull() {
                        got.push(c.index);
                        s.done();
                    }
                    got
                })
            })
            .collect();
        let mut all: Vec<usize> = lanes.into_iter().flat_map(|l| l.join().unwrap_or_default()).collect();
        all.sort_unstable();
        assert_eq!(all, (0..200).collect::<Vec<_>>());
        assert_eq!(s.waiting(), 0);
    }
}
