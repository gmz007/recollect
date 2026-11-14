//! Library facts the screens show (folders, counts), read in the background.

use super::App;
use crate::Library;
use crate::store::db::{Folder, Stats};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How old the cached library facts may get before a non-urgent refresh replaces them.
pub(super) const SNAPSHOT_MAX_AGE: Duration = Duration::from_millis(500);

/// Cached library facts, refreshed a few times per second rather than every frame.
#[derive(Default)]
pub(super) struct Snapshot {
    pub(super) stats: Stats,
    pub(super) folders: Vec<Folder>,
    pub(super) errors: Vec<(PathBuf, String)>,
    pub(super) model_installed: bool,
    /// Ids of the smart-search choices whose files are on this computer.
    pub(super) installed_models: Vec<&'static str>,
    pub(super) taken: Option<Instant>,
}

impl Snapshot {
    /// Database queries: run off the UI thread (the database may be busy for a while).
    pub(super) fn fetch(lib: &Library) -> Self {
        Self {
            stats: lib.stats().unwrap_or_default(),
            folders: lib.folders().unwrap_or_default(),
            errors: lib.errors(50).unwrap_or_default(),
            model_installed: lib.model_installed(),
            installed_models: crate::model::registry::MODELS
                .iter()
                .filter(|m| crate::model::download::is_installed(&lib.paths().models_dir(), m))
                .map(|m| m.id)
                .collect(),
            taken: Some(Instant::now()),
        }
    }
}

impl App {
    /// Requests fresh library facts from a background thread (never blocks the UI).
    pub(super) fn refresh_snapshot(&mut self, force: bool) {
        if self.snapshot_read.is_pending() {
            if force {
                self.snapshot_read.mark_again();
            }
            return;
        }
        if !force
            && self
                .snapshot
                .taken
                .is_some_and(|t| t.elapsed() < SNAPSHOT_MAX_AGE)
        {
            return;
        }
        self.snapshot_read.request();
    }

    pub(super) fn poll_snapshot(&mut self) {
        if let Some(s) = self.snapshot_read.poll() {
            self.snapshot = s;
        }
    }
}
