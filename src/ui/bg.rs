//! Small building blocks for work that runs off the UI thread.

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// A read that runs on a background thread and is collected by polling, once per frame.
///
/// Asking for a read while one is under way schedules exactly one more after it finishes, so
/// the answer is never older than the latest request.
pub(super) struct BgRead<T> {
    name: &'static str,
    ctx: egui::Context,
    /// Returns `None` when the read failed; nothing is delivered then.
    job: Arc<dyn Fn() -> Option<T> + Send + Sync>,
    rx: Option<Receiver<T>>,
    again: bool,
}

impl<T: Send + 'static> BgRead<T> {
    pub(super) fn new(
        ctx: &egui::Context,
        name: &'static str,
        job: impl Fn() -> Option<T> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            ctx: ctx.clone(),
            job: Arc::new(job),
            rx: None,
            again: false,
        }
    }

    /// Starts a read, or schedules one for when the running read ends.
    pub(super) fn request(&mut self) {
        if self.rx.is_some() {
            self.again = true;
            return;
        }
        let (tx, rx) = bounded(1);
        let job = Arc::clone(&self.job);
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("recollect-{}", self.name))
            .spawn(move || {
                if let Some(value) = job() {
                    let _ = tx.send(value);
                    ctx.request_repaint();
                }
            });
        match spawned {
            Ok(_) => self.rx = Some(rx),
            Err(e) => tracing::warn!("could not start the {} read: {e}", self.name),
        }
    }

    /// Whether a read is under way.
    pub(super) fn is_pending(&self) -> bool {
        self.rx.is_some()
    }

    /// Makes the running read be followed by another (without starting one when idle).
    pub(super) fn mark_again(&mut self) {
        self.again = true;
    }

    /// The finished value, if one arrived. A read whose thread died without an answer is
    /// forgotten, so the next request starts afresh; a queued request goes out either way.
    pub(super) fn poll(&mut self) -> Option<T> {
        let rx = self.rx.as_ref()?;
        let value = match rx.try_recv() {
            Ok(v) => Some(v),
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) => return None,
        };
        self.rx = None;
        if std::mem::take(&mut self.again) {
            self.request();
        }
        value
    }

    /// Installs a receiver in place of a started read.
    #[cfg(test)]
    pub(super) fn install(&mut self, rx: Receiver<T>) {
        self.rx = Some(rx);
    }

    #[cfg(test)]
    pub(super) fn again_queued(&self) -> bool {
        self.again
    }

    #[cfg(test)]
    pub(super) fn take_receiver(&mut self) -> Option<Receiver<T>> {
        self.rx.take()
    }
}

/// Runs requests on one dedicated thread. Only the newest queued request is executed, so
/// typing or clicking quickly never builds up stale work; replies carry the id of the request
/// they answer.
pub(super) struct LatestWorker<Q, Resp> {
    tx: Sender<(u64, Q)>,
    rx: Receiver<Resp>,
    next_id: u64,
    /// Lets tests deliver a reply of their own making.
    #[cfg(test)]
    inject: Sender<Resp>,
}

impl<Q: Send + 'static, Resp: Send + 'static> LatestWorker<Q, Resp> {
    /// `work` runs a request; `reply` builds the answer from the request's id, the request and
    /// the outcome (`None` when `work` panicked, which must not kill the thread for the rest of
    /// the session).
    pub(super) fn spawn<R>(
        name: &str,
        repaint: impl Fn() + Send + 'static,
        work: impl Fn(&Q) -> R + Send + 'static,
        reply: impl Fn(u64, Q, Option<R>) -> Resp + Send + 'static,
    ) -> Self {
        let (tx, req_rx) = unbounded::<(u64, Q)>();
        let (resp_tx, rx) = unbounded();
        #[cfg(test)]
        let inject = resp_tx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("recollect-{name}"))
            .spawn(move || {
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    let (id, request) = req;
                    let outcome = catch_unwind(AssertUnwindSafe(|| work(&request))).ok();
                    if resp_tx.send(reply(id, request, outcome)).is_err() {
                        break;
                    }
                    repaint();
                }
            });
        if let Err(e) = spawned {
            // Requests then go nowhere; the app keeps running without this feature.
            tracing::error!("could not start the {name} thread: {e}");
        }
        Self {
            tx,
            rx,
            next_id: 0,
            #[cfg(test)]
            inject,
        }
    }

    /// Submits a request; returns its id.
    pub(super) fn submit(&mut self, request: Q) -> u64 {
        self.next_id += 1;
        let _ = self.tx.send((self.next_id, request));
        self.next_id
    }

    /// The newest finished reply, if any arrived since the last call.
    pub(super) fn poll(&self) -> Option<Resp> {
        self.rx.try_iter().last()
    }

    #[cfg(test)]
    pub(super) fn inject(&self, reply: Resp) {
        let _ = self.inject.send(reply);
    }
}
