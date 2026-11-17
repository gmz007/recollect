//! Library facts the screens show (folders, counts), read in the background.

use super::App;
use crate::Library;
use crate::store::db::{Folder, Stats};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How old the cached library facts may get before a non-urgent refresh replaces them.
const SNAPSHOT_MAX_AGE: Duration = Duration::from_millis(500);

/// How often the folders are looked at again to see whether they can still be read, and the
/// longest it waits (each check that finds nothing new doubles the wait, up to this).
pub(super) const FOLDER_CHECK_EVERY: Duration = Duration::from_secs(30);
const FOLDER_CHECK_MAX: Duration = Duration::from_secs(300);

/// When the folders were last checked and how long to wait for the next check.
#[derive(Default)]
pub(super) struct FolderWatch {
    /// The folders the last check was started for, and when.
    checked: Option<(Vec<i64>, Instant)>,
    /// Checks in a row that found nothing new.
    quiet_checks: u32,
}

impl FolderWatch {
    /// The wait after the last check: [`FOLDER_CHECK_EVERY`], doubled for each quiet check.
    fn wait(&self) -> Duration {
        FOLDER_CHECK_EVERY
            .saturating_mul(2_u32.saturating_pow(self.quiet_checks.min(8)))
            .min(FOLDER_CHECK_MAX)
    }

    /// How long until the next check is due for `folders` (zero if it is due now).
    fn due_in(&mut self, folders: &[i64]) -> Duration {
        match &self.checked {
            Some((seen, at)) if seen == folders => self.wait().saturating_sub(at.elapsed()),
            Some(_) => {
                self.quiet_checks = 0;
                Duration::ZERO
            }
            None => Duration::ZERO,
        }
    }
}

/// The check run on a background thread: the unreadable folders, or nothing when they are the
/// same as the last time (so the screens are not woken for nothing).
pub(super) fn folder_check_job(lib: std::sync::Arc<Library>) -> impl Fn() -> Option<Vec<i64>> {
    let last = parking_lot::Mutex::new(None::<Vec<i64>>);
    move || {
        let now = missing_folders(&lib)?;
        let mut last = last.lock();
        if last.as_ref() == Some(&now) {
            return None;
        }
        *last = Some(now.clone());
        Some(now)
    }
}

/// Ids of the library folders that cannot be read now (an unplugged drive). Asking the file
/// system can take long on a network drive, so this only runs on a background thread.
pub(super) fn missing_folders(lib: &Library) -> Option<Vec<i64>> {
    let folders = lib.folders().ok()?;
    Some(
        folders
            .iter()
            .filter(|f| !f.path.is_dir())
            .map(|f| f.id)
            .collect(),
    )
}

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
        let folders = lib.folders().unwrap_or_default();
        Self {
            stats: lib.stats().unwrap_or_default(),
            folders,
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

    /// Whether no check was ever started (tests).
    #[cfg(test)]
    pub(super) fn folder_watch_idle(&self) -> bool {
        self.folder_watch.checked.is_none() && !self.folder_check.is_pending()
    }

    /// Looks for folders that cannot be read (in the background) when the list of folders has
    /// changed and then every [`FOLDER_CHECK_EVERY`], less often while nothing changes; picks
    /// up the answer. Nothing happens while the window is not in use, so idle disks are left
    /// alone.
    pub(super) fn poll_folder_check(&mut self, ctx: &egui::Context) {
        let was_pending = self.folder_check.is_pending();
        let answer = self.folder_check.poll();
        if let Some(missing) = answer {
            self.missing_folders = missing;
            self.folder_watch.quiet_checks = 0;
        } else if was_pending && !self.folder_check.is_pending() {
            self.folder_watch.quiet_checks += 1;
        }
        if !ctx.input(|i| i.focused) {
            return;
        }
        let ids: Vec<i64> = self.snapshot.folders.iter().map(|f| f.id).collect();
        let wait = self.folder_watch.due_in(&ids);
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
        } else if !self.folder_check.is_pending() {
            self.folder_watch.checked = Some((ids, Instant::now()));
            self.folder_check.request();
            ctx.request_repaint_after(self.folder_watch.wait());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_while_nothing_changes_up_to_five_minutes() {
        let mut w = FolderWatch::default();
        let secs = |w: &FolderWatch| w.wait().as_secs();
        assert_eq!(secs(&w), 30);
        let mut seen = Vec::new();
        for _ in 0..6 {
            w.quiet_checks += 1;
            seen.push(secs(&w));
        }
        assert_eq!(seen, [60, 120, 240, 300, 300, 300]);
    }

    #[test]
    fn a_change_of_folders_checks_at_once_and_starts_over() {
        let mut w = FolderWatch::default();
        assert!(w.due_in(&[1]).is_zero());
        w.checked = Some((vec![1], Instant::now()));
        w.quiet_checks = 3;
        // The same folders: wait out what is left of 240 seconds.
        assert!(w.due_in(&[1]) > Duration::from_secs(200));
        // Another folder: due now, and back to the short wait.
        assert!(w.due_in(&[1, 2]).is_zero());
        assert_eq!(w.wait(), FOLDER_CHECK_EVERY);
    }
}
