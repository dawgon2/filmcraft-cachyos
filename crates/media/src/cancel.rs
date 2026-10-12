//! Cooperative cancellation of frame requests.
//!
//! A frame worker runs a job inside [`with_cancel`]; sources that do long work for a request (a
//! decoder seeking back to a keyframe and decoding forward) poll [`cancelled`] and give up with
//! [`MediaError::Cancelled`](crate::MediaError::Cancelled) once the job is no longer wanted, e.g.
//! playback has moved past its frame. The flag travels through a thread-local, so the
//! [`MediaSource`](crate::MediaSource) API is unchanged.
//!
//! The same way, [`with_catch_up`] tells sources which frames before the requested one are late
//! (playback has passed them): a decoder that has to decode forward to the frame may skip
//! pictures that only produce late frames and that nothing else references.
//!
//! [`with_skim`] says playback steps over frames (fast forward or reverse): a nearby picture will
//! do, and pictures only unrequested frames need can be left out.
//!
//! [`with_deadline`] says when the frame being worked on is due (playback): shared hardware decoders
//! serve the most urgent work first (`filmcraft_codecs::hwgate`).
//!
//! [`with_draft`] marks reduced-resolution playback (opt-in): decoders may take spec-safe
//! shortcuts that only change pictures nothing references (H.264: no deblocking of non-reference
//! pictures). Never set for exports, renders or a frame shown while paused.
//!
//! [`with_background`] marks work nobody waits on interactively (thumbnails of the Project panel,
//! the Media Browser and the timeline): sources should not keep expensive state, such as a
//! frame-threaded 4K decoder, for it once it is done.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use filmcraft_time::Tick;

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static CATCH_UP: Cell<Option<Tick>> = const { Cell::new(None) };
    static DRAFT: Cell<bool> = const { Cell::new(false) };
    static DEADLINE: Cell<u64> = const { Cell::new(u64::MAX) };
    static SKIM: Cell<Option<Tick>> = const { Cell::new(None) };
    static PACE: Cell<Option<Duration>> = const { Cell::new(None) };
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` with the hardware-decode pace `interval` (restoring the previous one after): a source
/// decoding many pictures for the frame (reverse playback decodes a GOP at a time) submits
/// pictures to a hardware decoder no faster than one per `interval`, where it has time to spare.
/// A video engine running flat out made the GPU's compositing wait 40–100 ms per frame on an
/// RX 6700 XT (Mesa radeonsi, VCN 3); paced, it did not.
pub fn with_pace<R>(interval: Option<Duration>, f: impl FnOnce() -> R) -> R {
    /// Restores the previous pace however `f` ends (a panic caught further up must not leave the
    /// thread paced).
    struct Restore(Option<Duration>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PACE.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(PACE.with(|c| c.replace(interval)));
    f()
}

/// The hardware-decode pace of the work running on this thread (None: as fast as it goes).
pub fn pace() -> Option<Duration> {
    PACE.with(Cell::get)
}

/// Run `f` with the skimming hint `tolerance` (restoring the previous one after): playback steps
/// over frames (fast forward, fast reverse), so any frame within `tolerance` of the requested
/// time will do, and a decoder may leave out every picture that only produces frames nobody
/// asked for. Measured in media time at normal speed, like [`with_catch_up`].
pub fn with_skim<R>(tolerance: Option<Tick>, f: impl FnOnce() -> R) -> R {
    let prev = SKIM.with(|c| c.replace(tolerance));
    let r = f();
    SKIM.with(|c| c.set(prev));
    r
}

/// The skimming tolerance of the work running on this thread (None: the exact frame).
pub fn skim() -> Option<Tick> {
    SKIM.with(Cell::get)
}

/// Run `f` with the deadline `due` (restoring the previous one after): when the frame it works for
/// is due, in microseconds of the caller's clock (smaller = more urgent; the same clock for all
/// work that competes for a decoder). Work without a deadline comes last.
pub fn with_deadline<R>(due: u64, f: impl FnOnce() -> R) -> R {
    let prev = DEADLINE.with(|c| c.replace(due));
    let r = f();
    DEADLINE.with(|c| c.set(prev));
    r
}

/// The deadline of the work running on this thread (`u64::MAX`: none).
pub fn deadline() -> u64 {
    DEADLINE.with(Cell::get)
}

/// Run `f` as background work when `on` (restoring the previous hint after): a thumbnail, not a
/// frame a monitor or playback waits for.
pub fn with_background<R>(on: bool, f: impl FnOnce() -> R) -> R {
    /// Restores the previous hint however `f` ends (a panic caught further up must not leave the
    /// worker thread marked as background).
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            BACKGROUND.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(BACKGROUND.with(|c| c.replace(on)));
    f()
}

/// Whether the work running on this thread is background work (off unless inside
/// [`with_background`]`(true, …)`).
pub fn background() -> bool {
    BACKGROUND.with(Cell::get)
}

/// Run `f` with the draft-decoding hint `on` (restoring the previous one after). Frames decoded
/// in draft mode are only handed to requests made in draft mode.
pub fn with_draft<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let prev = DRAFT.with(|c| c.replace(on));
    let r = f();
    DRAFT.with(|c| c.set(prev));
    r
}

/// Whether the work running on this thread accepts draft-quality decoding (off unless inside
/// [`with_draft`]`(true, …)`).
pub fn draft() -> bool {
    DRAFT.with(Cell::get)
}

/// Run `f` with the catch-up hint `margin` (restoring the previous one after): frames shown more
/// than `margin` before the requested time are late (`Some(Tick::ZERO)`: everything before it).
/// The margin is measured in media time at normal speed; a wrong guess (a sped-up or reversed
/// clip) only costs a later re-seek, never a wrong frame.
pub fn with_catch_up<R>(margin: Option<Tick>, f: impl FnOnce() -> R) -> R {
    let prev = CATCH_UP.with(|c| c.replace(margin));
    let r = f();
    CATCH_UP.with(|c| c.set(prev));
    r
}

/// The catch-up margin of the work running on this thread (None: no frame is late).
pub fn catch_up() -> Option<Tick> {
    CATCH_UP.with(Cell::get)
}

/// Run `f` with `flag` as this thread's cancellation flag (restoring the previous one after).
pub fn with_cancel<R>(flag: &Arc<AtomicBool>, f: impl FnOnce() -> R) -> R {
    /// Restores the previous flag however `f` ends (a panic caught further up must not leave the
    /// thread with a stale flag).
    struct Restore(Option<Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = prev);
        }
    }
    let _restore = Restore(CURRENT.with(|c| c.replace(Some(flag.clone()))));
    f()
}

/// Whether the work running on this thread has been cancelled.
pub fn cancelled() -> bool {
    CURRENT.with(|c| c.borrow().as_ref().is_some_and(|f| f.load(Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_is_scoped_to_the_closure() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(!cancelled());
        with_cancel(&flag, || {
            assert!(!cancelled());
            flag.store(true, Ordering::Relaxed);
            assert!(cancelled());
        });
        assert!(!cancelled());
    }

    #[test]
    fn draft_is_off_by_default_and_scoped() {
        assert!(!draft());
        with_draft(true, || {
            assert!(draft());
            with_draft(false, || assert!(!draft()));
            assert!(draft());
        });
        assert!(!draft());
    }

    #[test]
    fn background_is_off_by_default_and_scoped() {
        assert!(!background());
        with_background(true, || {
            assert!(background());
            with_background(false, || assert!(!background()));
            assert!(background());
        });
        assert!(!background());
    }

    #[test]
    fn a_panic_inside_background_work_does_not_leave_the_thread_marked() {
        let r = std::panic::catch_unwind(|| with_background(true, || panic!("boom")));
        assert!(r.is_err());
        assert!(!background());
    }

    #[test]
    fn a_panic_inside_cancel_or_pace_restores_the_outer_values() {
        let outer = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(AtomicBool::new(true));
        with_cancel(&outer, || {
            with_pace(Some(Duration::from_millis(3)), || {
                let r = std::panic::catch_unwind(|| with_cancel(&inner, || with_pace(Some(Duration::from_millis(9)), || panic!("boom"))));
                assert!(r.is_err());
                assert!(!cancelled(), "the outer flag again, not the inner (set) one");
                assert_eq!(pace(), Some(Duration::from_millis(3)));
            });
        });
        let r = std::panic::catch_unwind(|| with_cancel(&inner, || with_pace(Some(Duration::from_millis(9)), || panic!("boom"))));
        assert!(r.is_err());
        assert!(!cancelled(), "no flag left on the thread");
        assert_eq!(pace(), None);
    }

    #[test]
    fn catch_up_is_scoped_to_the_closure() {
        assert_eq!(catch_up(), None);
        with_catch_up(Some(Tick(5)), || {
            assert_eq!(catch_up(), Some(Tick(5)));
            with_catch_up(None, || assert_eq!(catch_up(), None));
            assert_eq!(catch_up(), Some(Tick(5)));
        });
        assert_eq!(catch_up(), None);
    }

    #[test]
    fn pace_is_scoped_to_the_closure() {
        assert_eq!(pace(), None);
        with_pace(Some(Duration::from_millis(5)), || {
            assert_eq!(pace(), Some(Duration::from_millis(5)));
            with_pace(None, || assert_eq!(pace(), None));
        });
        assert_eq!(pace(), None);
    }

    #[test]
    fn skim_is_scoped_to_the_closure() {
        assert_eq!(skim(), None);
        with_skim(Some(Tick(7)), || {
            assert_eq!(skim(), Some(Tick(7)));
            with_skim(None, || assert_eq!(skim(), None));
            assert_eq!(skim(), Some(Tick(7)));
        });
        assert_eq!(skim(), None);
    }
}
