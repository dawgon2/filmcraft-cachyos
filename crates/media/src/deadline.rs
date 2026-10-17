//! File access with a time limit, for files on network shares (STORAGE-PLAN S1 item 3).
//!
//! When the server behind an SMB / NFS share goes away, a `stat`, `open` or `read` on it can
//! block for minutes. [`with_deadline`] runs such a call on a helper thread and gives up after a
//! time limit with [`io::ErrorKind::TimedOut`], so the caller (a decoder, the save path, a scan)
//! gets an error it can show as "offline" instead of hanging. The helper thread is left to finish
//! on its own once the kernel gives up; at most [`MAX_STUCK`] can be outstanding, after which
//! calls fail at once rather than piling up threads.
//!
//! [`TimeoutReader`] wraps a [`ByteReader`] so every read has the limit; after one read times out
//! the reader fails every later read at once (the share is gone; re-opening the clip when it comes
//! back is the media pool's job, STORAGE-PLAN S1 item 3b).

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::reader::ByteReader;

/// How long one file call on a network share may take before it counts as a dropped share.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(8);

/// The most helper threads that may be stuck in the kernel at once.
pub const MAX_STUCK: usize = 32;

static STUCK: AtomicUsize = AtomicUsize::new(0);

fn timed_out(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("{what}: the network share didn't answer in time"))
}

/// Helper threads still running after their caller gave up.
pub fn stuck_threads() -> usize {
    STUCK.load(Ordering::Relaxed)
}

/// Runs `f` on a helper thread and waits at most `limit` for it. A panic in `f` becomes an
/// error. Without threads (the web build) `f` runs inline.
pub fn with_deadline<T, F>(limit: Duration, what: &str, f: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    if cfg!(target_arch = "wasm32") {
        return f();
    }
    if STUCK.load(Ordering::Relaxed) >= MAX_STUCK {
        // not a timeout of this call: a reader that gets this isn't marked dead
        return Err(io::Error::new(io::ErrorKind::ResourceBusy, format!("{what}: too many network file calls are still waiting")));
    }
    let (tx, rx) = std::sync::mpsc::sync_channel::<io::Result<T>>(1);
    let gave_up = Arc::new(AtomicBool::new(false));
    let gave_up_t = gave_up.clone();
    let spawned = std::thread::Builder::new().name("fc-deadline".into()).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(io::Error::other("file access failed")));
        let _ = tx.send(r);
        // the caller stopped waiting and counted this thread as stuck: it isn't any more
        if gave_up_t.swap(false, Ordering::AcqRel) {
            STUCK.fetch_sub(1, Ordering::AcqRel);
        }
    });
    if let Err(e) = spawned {
        return Err(io::Error::other(format!("{what}: couldn't start a helper thread: {e}")));
    }
    match rx.recv_timeout(limit) {
        Ok(r) => r,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            STUCK.fetch_add(1, Ordering::AcqRel);
            gave_up.store(true, Ordering::Release);
            // the thread may have finished between the timeout and the flag: take its result
            if let Ok(r) = rx.try_recv() {
                if gave_up.swap(false, Ordering::AcqRel) {
                    STUCK.fetch_sub(1, Ordering::AcqRel);
                }
                return r;
            }
            Err(timed_out(what))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other(format!("{what}: file access failed"))),
    }
}

/// The largest single read handed to a helper thread (bigger reads are split, so a slow but
/// working share, ≥ 4 MB per [`NETWORK_TIMEOUT`], never times out, and a stuck helper holds at
/// most this much memory).
const MAX_CHUNK: usize = 4 << 20;

/// A [`ByteReader`] whose reads each have a time limit; once one times out, every later read
/// fails at once with `TimedOut`.
pub struct TimeoutReader {
    inner: Arc<dyn ByteReader>,
    limit: Duration,
    dead: AtomicBool,
}

impl TimeoutReader {
    pub fn new(inner: Arc<dyn ByteReader>, limit: Duration) -> Self {
        Self { inner, limit, dead: AtomicBool::new(false) }
    }

    /// A read has timed out: the share is treated as gone.
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }
}

impl ByteReader for TimeoutReader {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut at = offset;
        for chunk in buf.chunks_mut(MAX_CHUNK) {
            if self.is_dead() {
                return Err(timed_out("read"));
            }
            let inner = self.inner.clone();
            let n = chunk.len();
            let r = with_deadline(self.limit, "read", move || {
                let mut v = vec![0u8; n];
                inner.read_at(at, &mut v).map(|()| v)
            });
            match r {
                Ok(v) if v.len() == n => chunk.copy_from_slice(&v),
                Ok(_) => return Err(io::Error::other("short read")),
                Err(e) => {
                    if e.kind() == io::ErrorKind::TimedOut {
                        self.dead.store(true, Ordering::Release);
                    }
                    return Err(e);
                }
            }
            at = at.saturating_add(n as u64);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::MemReader;

    /// A reader that blocks until released (a share that stopped answering).
    struct Hung(Arc<AtomicBool>);
    impl ByteReader for Hung {
        fn len(&self) -> u64 {
            100
        }
        fn read_at(&self, _: u64, buf: &mut [u8]) -> io::Result<()> {
            while !self.0.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            buf.fill(7);
            Ok(())
        }
    }

    #[test]
    fn quick_calls_return_their_result() {
        assert_eq!(with_deadline(Duration::from_secs(5), "t", || Ok(3)).ok(), Some(3));
        let e = with_deadline::<(), _>(Duration::from_secs(5), "t", || Err(io::Error::new(io::ErrorKind::NotFound, "x"))).err();
        assert_eq!(e.map(|e| e.kind()), Some(io::ErrorKind::NotFound));
    }

    #[test]
    fn panics_become_errors() {
        let r = with_deadline::<(), _>(Duration::from_secs(5), "t", || panic!("boom"));
        assert!(r.is_err());
    }

    #[test]
    fn a_hung_reader_times_out_then_fails_fast_and_the_thread_is_released() {
        let release = Arc::new(AtomicBool::new(false));
        let r = TimeoutReader::new(Arc::new(Hung(release.clone())), Duration::from_millis(50));
        let mut buf = [0u8; 10];
        let t0 = std::time::Instant::now();
        let e = r.read_at(0, &mut buf).err().map(|e| e.kind());
        assert_eq!(e, Some(io::ErrorKind::TimedOut));
        assert!(t0.elapsed() < Duration::from_secs(3));
        assert!(r.is_dead());
        let t1 = std::time::Instant::now();
        assert_eq!(r.read_at(0, &mut buf).err().map(|e| e.kind()), Some(io::ErrorKind::TimedOut));
        assert!(t1.elapsed() < Duration::from_millis(40), "a dead reader fails at once");
        // fill the cap with stuck calls: a new reader then gets "busy", not a timeout, and stays alive
        for _ in 0..MAX_STUCK {
            let hung = Hung(release.clone());
            let _ = with_deadline(Duration::from_millis(1), "t", move || {
                let mut b = [0u8; 1];
                hung.read_at(0, &mut b)
            });
        }
        let fresh = TimeoutReader::new(Arc::new(MemReader(vec![1u8; 4].into())), Duration::from_secs(5));
        let mut one = [0u8; 1];
        assert_eq!(fresh.read_at(0, &mut one).err().map(|e| e.kind()), Some(io::ErrorKind::ResourceBusy));
        assert!(!fresh.is_dead());
        // the share answers again: the stuck helpers finish and stop counting
        release.store(true, Ordering::Release);
        let t2 = std::time::Instant::now();
        while stuck_threads() > 0 && t2.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(stuck_threads(), 0);
    }

    #[test]
    fn reads_pass_through_and_split() {
        let data: Arc<[u8]> = (0..=255u8).cycle().take(1000).collect::<Vec<_>>().into();
        let r = TimeoutReader::new(Arc::new(MemReader(data.clone())), Duration::from_secs(5));
        let mut buf = vec![0u8; 300];
        assert!(r.read_at(100, &mut buf).is_ok());
        assert_eq!(&buf[..], &data[100..400]);
        assert_eq!(r.len(), 1000);
        let mut past = vec![0u8; 10];
        assert_eq!(r.read_at(995, &mut past).err().map(|e| e.kind()), Some(io::ErrorKind::UnexpectedEof));
        assert!(!r.is_dead());
    }
}
