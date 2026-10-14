//! Running model work without ever taking the app down: a panic inside a model step becomes an
//! error, and a [`Cancel`] token lets the caller stop long work between steps.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A shared stop flag: the owner calls [`Cancel::cancel`], the worker checks
/// [`Cancel::is_cancelled`] between steps (frames, audio blocks, tokens).
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Why guarded work did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobError {
    /// The work returned an error.
    Failed(String),
    /// The work panicked (the message, if it had one).
    Panicked(String),
    /// [`Cancel::cancel`] was called and the work noticed.
    Cancelled,
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::Failed(e) => write!(f, "{e}"),
            JobError::Panicked(m) => write!(f, "the model stopped unexpectedly ({m})"),
            JobError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for JobError {}

/// The text of a caught panic payload.
pub fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_string()
    }
}

/// Run `work` (which gets the cancel token to check between its steps) under `catch_unwind`.
/// State the work mutates must be safe to drop or reset after a panic (the caller's
/// responsibility, as with every `AssertUnwindSafe`).
pub fn guarded<T, E: std::fmt::Display>(cancel: &Cancel, work: impl FnOnce(&Cancel) -> Result<T, E>) -> Result<T, JobError> {
    if cancel.is_cancelled() {
        return Err(JobError::Cancelled);
    }
    match catch_unwind(AssertUnwindSafe(|| work(cancel))) {
        Ok(Ok(v)) if cancel.is_cancelled() => {
            drop(v);
            Err(JobError::Cancelled)
        }
        Ok(Ok(v)) => Ok(v),
        Ok(Err(_)) if cancel.is_cancelled() => Err(JobError::Cancelled),
        Ok(Err(e)) => Err(JobError::Failed(e.to_string())),
        Err(p) => Err(JobError::Panicked(panic_message(p.as_ref()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_errors_panics_and_cancels() {
        let c = Cancel::new();
        assert_eq!(guarded(&c, |_| Ok::<_, String>(3)), Ok(3));
        assert_eq!(guarded(&c, |_| Err::<u8, _>("bad input")), Err(JobError::Failed("bad input".into())));
        let r = guarded(&c, |_| -> Result<u8, String> { panic!("boom") });
        assert_eq!(r, Err(JobError::Panicked("boom".into())));
        // cancelled while running: the result is dropped
        let r = guarded(&c, |c| {
            c.cancel();
            Ok::<_, String>(1)
        });
        assert_eq!(r, Err(JobError::Cancelled));
        // cancelled before: the work never runs
        assert_eq!(guarded(&c, |_| -> Result<u8, String> { panic!("must not run") }), Err(JobError::Cancelled));
    }
}
