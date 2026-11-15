//! Adding and removing library folders.

use super::{ErrorKind, Library};
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// What [`Library::start_add_folders`] does once the folders are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterAdd {
    /// Only look through the folders.
    Nothing,
    /// Also download the smart-search model.
    DownloadModel,
}

impl Library {
    /// Adds a folder to the library (does not start indexing).
    pub fn add_folder(&self, path: &Path) -> Result<()> {
        let canon = std::fs::canonicalize(path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        if !canon.is_dir() {
            bail!("{} is not a folder", canon.display());
        }
        let canon = strip_verbatim(canon);
        self.db.lock().add_folder(&canon)?;
        self.rescan_requested.store(true, Ordering::Release);
        self.notify();
        Ok(())
    }

    /// Adds a folder off the calling thread (canonicalizing a dead network path or waiting for
    /// the database must not freeze the UI), then makes sure it gets indexed.
    pub fn start_add_folder(self: &Arc<Self>, path: PathBuf) {
        self.spawn_task("recollect-add-folder", ErrorKind::AddFolder, move |lib| {
            match lib.add_folder(&path) {
                Ok(()) => {
                    // If a job is running it picks the folder up (rescan_requested).
                    // Without a model the run still reads capture dates for the album.
                    if !lib.is_busy() {
                        let _ = lib.start_indexing();
                    }
                }
                Err(e) => {
                    lib.update_status(|s| s.set_error_of(ErrorKind::AddFolder, format!("{e:#}")));
                }
            }
            lib.jobs_finished.fetch_add(1, Ordering::AcqRel);
            lib.notify();
            // Saves the index cache unless a job is running; a running job (the scan this
            // started, or an earlier one) saves when it ends instead.
            lib.save_index_cache();
        });
    }

    /// Adds several folders one after the other off the calling thread, then looks through them
    /// (dates, thumbnails) so they can be browsed. With [`AfterAdd::DownloadModel`], smart search is
    /// downloaded after that, without cutting the scan short; if the download fails the folders
    /// are still scanned.
    pub fn start_add_folders(self: &Arc<Self>, paths: Vec<PathBuf>, then: AfterAdd) {
        self.spawn_task("recollect-add-folders", ErrorKind::AddFolder, move |lib| {
            let mut added = false;
            let mut failure = None;
            for p in &paths {
                match lib.add_folder(p) {
                    Ok(()) => added = true,
                    Err(e) => failure = failure.or(Some(format!("{e:#}"))),
                }
            }
            if added {
                let _ = lib.request_scan();
            }
            if then == AfterAdd::DownloadModel
                && let Err(e) = lib.request_download()
            {
                lib.update_status(|s| {
                    s.set_error_of(ErrorKind::Download, format!("Model download failed: {e:#}"));
                });
            }
            // After the jobs started: starting one clears the last error.
            if let Some(e) = failure {
                lib.update_status(|s| s.set_error_of(ErrorKind::AddFolder, e));
            }
            lib.jobs_finished.fetch_add(1, Ordering::AcqRel);
            lib.notify();
            // Saves the index cache unless a job is running; a running job (the scan this
            // started, or an earlier one) saves when it ends instead.
            lib.save_index_cache();
        });
    }

    /// Removes a folder and its index entries. Media files are never touched.
    /// Blocks while the database is busy; GUI code should use [`Self::start_remove_folder`].
    pub fn remove_folder(&self, folder_id: i64) -> Result<()> {
        {
            // Lock order everywhere: db, then index.
            let mut db = self.db.lock();
            let removed = db.remove_folder(folder_id)?;
            self.index
                .write()
                .remove_files(&removed.into_iter().collect());
        }
        self.notify();
        Ok(())
    }

    /// Removes a folder on a background thread. It never cancels the running job: removal is
    /// a database transaction plus an index update under the usual lock order, and a running
    /// indexer simply skips files that disappeared.
    pub fn start_remove_folder(self: &Arc<Self>, folder_id: i64) {
        self.spawn_task(
            "recollect-remove-folder",
            ErrorKind::RemoveFolder,
            move |lib| {
                if let Err(e) = lib.remove_folder(folder_id) {
                    lib.update_status(|s| {
                        s.set_error_of(
                            ErrorKind::RemoveFolder,
                            format!("Removing the folder failed: {e:#}"),
                        );
                    });
                }
                lib.jobs_finished.fetch_add(1, Ordering::AcqRel);
                lib.notify();
                // Saves the index cache unless a job is running; a running job (the scan this
                // started, or an earlier one) saves when it ends instead.
                lib.save_index_cache();
            },
        );
    }

    pub(crate) fn remove_files_from_index(&self, ids: &HashSet<i64>) {
        self.index.write().remove_files(ids);
    }
}

/// On Windows `canonicalize` returns `\\?\C:\...`; keep paths in their familiar form.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            if !rest.starts_with("UNC\\") {
                return PathBuf::from(rest.to_string());
            }
        }
    }
    p
}
