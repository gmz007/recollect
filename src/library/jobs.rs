//! The background worker: one job at a time (scan and index, download, model switch), with
//! pause and cancel, and requests that wait for the running job to end.

use super::{Activity, ErrorKind, Library, RunSummary};
use crate::indexer;
use anyhow::{Result, anyhow, bail};
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Work asked for while another job was running; it starts when that job ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Pending {
    pub(super) download: bool,
    pub(super) scan: bool,
}

/// What became of a request that may have to wait for the running job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// It is running now.
    Started,
    /// Another job is running; this starts when it ends.
    Queued,
}

/// Pause / cancel signalling shared with worker threads.
#[derive(Default)]
pub struct Control {
    cancel: AtomicBool,
    paused: Mutex<bool>,
    cv: Condvar,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control")
            .field("cancelled", &self.cancel.load(Ordering::Relaxed))
            .field("paused", &*self.paused.lock())
            .finish_non_exhaustive()
    }
}

impl Control {
    /// Blocks while paused; returns `true` if the current job should stop.
    pub fn checkpoint(&self) -> bool {
        let mut p = self.paused.lock();
        while *p && !self.cancel.load(Ordering::Relaxed) {
            self.cv.wait(&mut p);
        }
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    fn set_paused(&self, v: bool) {
        *self.paused.lock() = v;
        self.cv.notify_all();
    }

    fn reset(&self) {
        self.cancel.store(false, Ordering::Relaxed);
        self.set_paused(false);
    }
}

impl Library {
    /// Runs `task` on a named background thread. If the thread cannot be started the failure
    /// is reported through the status as an error of `kind`.
    pub(super) fn spawn_task(
        self: &Arc<Self>,
        name: &str,
        kind: ErrorKind,
        task: impl FnOnce(&Arc<Self>) + Send + 'static,
    ) {
        let lib = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || task(&lib));
        if let Err(e) = spawned {
            self.update_status(|s| {
                s.set_error_of(kind, format!("Could not start a background task: {e}"));
            });
            self.notify();
        }
    }

    /// Looks for new photos now, or as soon as the running job ends. Never cuts a job short.
    pub fn request_scan(self: &Arc<Self>) -> Result<Request> {
        self.request(|p| p.scan = true, Library::start_indexing)
    }

    /// Downloads smart search now, or as soon as the running job ends (the job goes on to scan
    /// and index). Unlike [`Self::start_download`] it never cancels the running job.
    pub fn request_download(self: &Arc<Self>) -> Result<Request> {
        self.request(
            |p| p.download = true,
            |lib| {
                lib.update_status(|s| s.last_run = None);
                lib.spawn(Self::download_job(Arc::clone(lib)))
            },
        )
    }

    /// Runs `start` unless a job is running, in which case `mark` records the request for
    /// [`Self::run_pending`]. The check and the record happen under the lock the finishing job
    /// takes, so a request is never lost between the two.
    fn request(
        self: &Arc<Self>,
        mark: impl Fn(&mut Pending),
        start: impl Fn(&Arc<Self>) -> Result<()>,
    ) -> Result<Request> {
        let mut last = None;
        for _ in 0..3 {
            {
                let mut p = self.pending.lock();
                if self.busy.load(Ordering::Acquire) {
                    mark(&mut p);
                    return Ok(Request::Queued);
                }
            }
            match start(self) {
                Ok(()) => return Ok(Request::Started),
                // Lost a race with another job: look again.
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("another job is already running")))
    }

    /// Starts what was asked for while the last job ran (called when a job ends).
    fn run_pending(self: &Arc<Self>) {
        let p = std::mem::take(&mut *self.pending.lock());
        // The scan first: a download requested meanwhile then waits for it.
        if p.scan {
            let _ = self.request_scan();
        }
        if p.download {
            let _ = self.request_download();
        }
    }

    /// Starts a scan + indexing run in the background. Without an installed model it only
    /// scans and reads capture dates (the album works before any model is downloaded).
    pub fn start_indexing(self: &Arc<Self>) -> Result<()> {
        let lib = Arc::clone(self);
        self.spawn(move || lib.run_indexing_job())
    }

    /// Body of an indexing job: runs on a worker thread and records the outcome in the status.
    pub(super) fn run_indexing_job(&self) {
        #[cfg(test)]
        self.job_log.lock().push("scan");
        self.update_status(|s| s.last_run = None);
        let res = indexer::run(self);
        self.update_status(|s| {
            s.activity = Activity::Idle;
            s.paused = false;
            match res {
                Ok(sum) => s.last_run = Some(sum),
                Err(e) => s.set_error_of(ErrorKind::Scan, format!("Indexing failed: {e:#}")),
            }
        });
    }

    pub(crate) fn spawn(&self, f: impl FnOnce() + Send + 'static) -> Result<()> {
        let mut w = self.worker.lock();
        if self.busy.load(Ordering::Acquire) {
            bail!("another job is already running");
        }
        // A previous job may still be writing the index cache (busy is already false then);
        // detach it rather than refusing or waiting.
        if let Some(h) = w.take()
            && h.is_finished()
        {
            let _ = h.join();
        }
        self.control.reset();
        self.update_status(|s| {
            s.last_error = None;
            s.paused = false;
        });
        self.busy.store(true, Ordering::Release);
        let busy = Arc::clone(&self.busy);
        let finished = Arc::clone(&self.jobs_finished);
        let weak = self.self_ref.read().clone();
        let job = move || {
            // Reset even if the job panics.
            struct Done(Arc<AtomicBool>, Arc<AtomicU64>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.1.fetch_add(1, Ordering::AcqRel);
                    self.0.store(false, Ordering::Release);
                }
            }
            let done = Done(busy, finished);
            f();
            // Release `busy` before the (slow) cache write so new jobs aren't refused meanwhile.
            drop(done);
            if let Some(lib) = weak.upgrade() {
                lib.run_pending();
                lib.save_index_cache();
            }
        };
        match std::thread::Builder::new()
            .name("recollect-worker".into())
            .spawn(job)
        {
            Ok(h) => *w = Some(h),
            Err(e) => {
                self.busy.store(false, Ordering::Release);
                return Err(e.into());
            }
        }
        Ok(())
    }

    pub fn pause(&self, paused: bool) {
        self.control.set_paused(paused);
        self.update_status(|s| s.paused = paused);
    }

    pub fn cancel(&self) {
        *self.pending.lock() = Pending::default();
        self.control.cancel.store(true, Ordering::Relaxed);
        self.control.set_paused(false);
    }

    /// Waits for the background job to finish (CLI, tests, shutdown).
    pub fn wait(&self) {
        // Take the handle first: joining while holding the lock would block `spawn` callers.
        let handle = self.worker.lock().take();
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    /// Like [`Self::wait`] but gives up after `limit` (e.g. at exit, when a download may be
    /// stuck in a network read). Returns true if the job finished.
    pub fn wait_for(&self, limit: std::time::Duration) -> bool {
        let start = Instant::now();
        while self.is_busy() {
            if start.elapsed() >= limit {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.wait();
        true
    }

    /// Runs indexing on the calling thread (CLI); see [`Self::start_indexing`] for the case
    /// of a missing model.
    pub fn index_blocking(&self) -> Result<RunSummary> {
        self.control.reset();
        let r = indexer::run(self);
        self.update_status(|s| s.activity = Activity::Idle);
        self.save_index_cache();
        r
    }
}
