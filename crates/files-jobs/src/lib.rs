//! Runs work off the caller's thread with progress, cancel and a stall watchdog.

pub mod paste;
pub mod sizes;

pub use paste::{paste, paste_from, Summary};

use files_core::{Problem, Progress, RelPath};
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// With the 100 ms tick in `wait`, cancel returns within 1 s (G1).
pub const CANCEL_GRACE: Duration = Duration::from_millis(900);
pub const PROGRESS_CAP: usize = 16;
const TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub stall: Duration,
    pub cancel_grace: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            stall: STALL_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
        }
    }
}

/// Set once; remembers when cancel was first requested.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<OnceLock<Instant>>);

impl CancelToken {
    pub fn cancel(&self) {
        let _ = self.0.set(Instant::now());
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.get().is_some()
    }
    pub fn cancelled_at(&self) -> Option<Instant> {
        self.0.get().copied()
    }
}

pub struct JobCtx {
    cancel: CancelToken,
    progress: SyncSender<Progress>,
}

impl JobCtx {
    /// Best effort: dropped when the channel is full.
    pub fn report(&self, p: Progress) {
        let _ = self.progress.try_send(p);
    }
    /// Always delivered while `wait` runs (used after each finished file).
    pub fn report_done(&self, p: Progress) {
        let _ = self.progress.send(p);
    }
    pub fn check_cancel(&self) -> Result<(), JobError> {
        if self.cancel.is_cancelled() {
            Err(JobError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub enum JobError {
    Cancelled,
    Panicked,
    Win(files_win::Error),
    Io(std::io::Error),
    Problems(Vec<Problem>),
    SizeExceeded(RelPath),
    SizeShort {
        path: RelPath,
        declared: u64,
        got: u64,
    },
    TargetNotDir(PathBuf),
    /// An error while copying one entry, naming it.
    Entry {
        path: RelPath,
        source: Box<JobError>,
    },
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobError::Cancelled => write!(f, "cancelled"),
            JobError::Panicked => write!(f, "internal error (job thread panicked)"),
            JobError::Win(e) => write!(f, "{e}"),
            JobError::Io(e) => write!(f, "{e}"),
            JobError::Problems(p) => write!(f, "{} problem(s) found before writing", p.len()),
            JobError::SizeExceeded(p) => write!(f, "{p}: more data than declared"),
            JobError::SizeShort {
                path,
                declared,
                got,
            } => {
                write!(f, "{path}: stream ended at {got} of {declared} bytes")
            }
            JobError::TargetNotDir(p) => write!(f, "not a folder: {}", p.display()),
            JobError::Entry { path, source } => write!(f, "{path}: {source}"),
        }
    }
}

impl std::error::Error for JobError {}

impl From<files_win::Error> for JobError {
    fn from(e: files_win::Error) -> Self {
        JobError::Win(e)
    }
}

impl From<std::io::Error> for JobError {
    fn from(e: std::io::Error) -> Self {
        JobError::Io(e)
    }
}

#[derive(Debug)]
pub enum Outcome<T> {
    Done(T),
    Failed(JobError),
    Cancelled,
    Stalled,
}

pub struct JobHandle<T> {
    cancel: CancelToken,
    rx: Receiver<Progress>,
    thread: JoinHandle<Result<T, JobError>>,
    limits: Limits,
}

pub fn start<T: Send + 'static>(
    name: &str,
    limits: Limits,
    f: impl FnOnce(JobCtx) -> Result<T, JobError> + Send + 'static,
) -> JobHandle<T> {
    let (tx, rx) = sync_channel(PROGRESS_CAP);
    let cancel = CancelToken::default();
    let ctx = JobCtx {
        cancel: cancel.clone(),
        progress: tx,
    };
    let thread = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || f(ctx))
        .expect("spawn job thread");
    JobHandle {
        cancel,
        rx,
        thread,
        limits,
    }
}

impl<T> JobHandle<T> {
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Blocks until the job ends, cancel's grace runs out, or it stalls.
    /// On `Cancelled` (grace) and `Stalled` the thread is abandoned.
    pub fn wait(self, mut on_progress: impl FnMut(&Progress)) -> Outcome<T> {
        let mut last = Instant::now();
        loop {
            match self.rx.recv_timeout(TICK) {
                Ok(p) => {
                    on_progress(&p);
                    last = Instant::now();
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return match self.thread.join() {
                        Ok(Ok(t)) => Outcome::Done(t),
                        Ok(Err(JobError::Cancelled)) => Outcome::Cancelled,
                        Ok(Err(e)) => Outcome::Failed(e),
                        Err(_) => Outcome::Failed(JobError::Panicked),
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            match self.cancel.cancelled_at() {
                Some(at) if at.elapsed() >= self.limits.cancel_grace => return Outcome::Cancelled,
                Some(_) => {} // cancel wins over the stall check
                None if last.elapsed() >= self.limits.stall => {
                    self.cancel.cancel();
                    return Outcome::Stalled;
                }
                None => {}
            }
        }
    }
}
