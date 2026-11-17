//! `Library`: the thread-safe facade the GUI and CLI talk to. It owns the database, the vector
//! index, settings and the background worker (model download / indexing).
//!
//! Its methods are grouped by concern: `jobs` (the background worker and what waits for it),
//! `folders`, `model` (the smart-search choice and its download), `writes` (favorite marks and
//! album changes, applied in order by one writer thread) and `index_cache`.

mod folders;
mod index_cache;
mod jobs;
mod model;
mod status;
mod writes;

pub use folders::AfterAdd;
pub use jobs::{Control, Request};
#[cfg(test)]
pub(crate) use model::HARDWARE_CHECKS;
pub use model::default_model_for_hardware;
pub use status::{Activity, ErrorKind, IndexProgress, RunSummary, Status};
pub use writes::{AlbumOp, AlbumWrite, FavoriteWrite};

use crate::config::{Paths, Settings};
use crate::model::{ModelSpec, TextEncoder, download, registry};
use crate::search::{self, SearchHit, SearchOptions};
use crate::store::db::{DateCounts, FileDetails, Folder, Stats, TimelineFilter, TimelineItem};
use crate::store::{Db, VectorIndex};
use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use index_cache::{index_cache_path, load_index, remove_stale_temp_files};
use jobs::Pending;
use parking_lot::{Mutex, RwLock};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use writes::{WriteMsg, library_writer};

/// Why a search could not run, for callers that word the reason themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// The photos are still being indexed with another model, or the query is out of date.
    #[error("the library is still being prepared for this model; wait for re-indexing to finish")]
    NotReady,
    /// Smart search has not been downloaded.
    #[error("the model is not downloaded yet")]
    ModelMissing,
    /// The photo asked about is no longer in the library.
    #[error("that item is no longer in the library")]
    ItemGone,
    #[error("search failed")]
    Other,
}

impl SearchError {
    /// The kind of a failure returned by a search (`Other` when it carries none).
    pub fn of(e: &anyhow::Error) -> Self {
        e.downcast_ref::<Self>().copied().unwrap_or(Self::Other)
    }
}

type Notifier = Box<dyn Fn() + Send + Sync>;

pub struct Library {
    paths: Paths,
    settings: RwLock<Settings>,
    /// Incremented whenever the settings change (a cheap way to notice without copying them).
    settings_version: AtomicU64,
    model: RwLock<&'static ModelSpec>,
    pub(crate) db: Mutex<Db>,
    pub(crate) index: RwLock<VectorIndex>,
    status: Mutex<Status>,
    pub(crate) control: Control,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// True while a background job runs (lock-free, polled by the UI every frame).
    busy: Arc<AtomicBool>,
    /// Incremented each time a background job (or a folder removal) finishes.
    jobs_finished: Arc<AtomicU64>,
    /// Set when folders change while a job runs; the indexer scans again before finishing.
    pub(crate) rescan_requested: AtomicBool,
    /// Requests that arrived while a job was running (see [`Library::request_scan`]).
    pending: Mutex<Pending>,
    /// Weak self-reference so jobs can persist the index cache when they finish.
    self_ref: RwLock<std::sync::Weak<Library>>,
    text: Mutex<Option<TextEncoder>>,
    notifier: RwLock<Option<Notifier>>,
    /// Held while an index cache is being written; a second saver simply skips.
    save_lock: Mutex<()>,
    /// (model id, epoch) the on-disk cache is known to match: set when the index was loaded
    /// from the cache or after a successful save. Avoids rewriting hundreds of MB for nothing.
    saved: Mutex<Option<(&'static str, u64)>>,
    /// Whether `open` found a usable cache (instead of rebuilding from the database).
    from_cache: bool,
    /// Favorite marks and album changes go to one writer thread in order (see
    /// [`Library::queue_favorite`] and [`Library::queue_album`]).
    write_tx: Sender<WriteMsg>,
    favorite_results: Receiver<Vec<FavoriteWrite>>,
    album_results: Receiver<AlbumWrite>,
    next_ticket: AtomicU64,
    /// Incremented after each committed favorites write.
    favorites_version: AtomicU64,
    /// Incremented after each album write that changed something.
    albums_version: AtomicU64,
    /// Makes the next favorites writes fail (tests).
    #[cfg(test)]
    pub(crate) fail_favorites: AtomicBool,
    /// Makes the next album writes fail (tests).
    #[cfg(test)]
    pub(crate) fail_albums: AtomicBool,
    /// Makes the next downloads fail without touching the network (tests).
    #[cfg(test)]
    pub(crate) fail_download: AtomicBool,
    /// Which jobs started, in order (tests).
    #[cfg(test)]
    pub(crate) job_log: Mutex<Vec<&'static str>>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("paths", &self.paths)
            .field("model", &self.model().id)
            .field("busy", &self.busy.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// The receiving and sending ends the writer thread works with.
struct WriterEnds {
    requests: Receiver<WriteMsg>,
    favorites: Sender<Vec<FavoriteWrite>>,
    albums: Sender<AlbumWrite>,
}

/// Loads the settings; a file that cannot be read is kept aside and replaced by
/// defaults. A model id the app does not know is dropped.
fn load_settings_or_default(paths: &Paths) -> Settings {
    let mut settings = match Settings::load(&paths.settings_path()) {
        Ok(s) => s,
        Err(e) => {
            // Keep the broken file for inspection and start with defaults.
            tracing::warn!("{e:#}; using default settings");
            let bad = paths.settings_path().with_extension("toml.broken");
            let _ = std::fs::rename(paths.settings_path(), bad);
            Settings::default()
        }
    };
    if settings
        .model
        .as_deref()
        .is_some_and(|id| registry::get(id).is_none())
    {
        tracing::warn!("unknown model in settings; choosing a default");
        settings.model = None;
    }
    settings
}

/// The override if given, else the model in the settings, else a default for
/// this hardware (which is saved into the settings).
fn choose_model_id(
    paths: &Paths,
    settings: &mut Settings,
    model_override: Option<&str>,
) -> Result<String> {
    if let Some(id) = model_override {
        return Ok(id.to_string());
    }
    if let Some(id) = &settings.model {
        return Ok(id.clone());
    }
    let id = default_model_for_hardware().to_string();
    settings.model = Some(id.clone());
    settings.save(&paths.settings_path())?;
    Ok(id)
}

/// The saved index if it is still current, else one rebuilt from the database.
/// The flag says whether it came from the saved copy.
fn load_or_rebuild_index(
    paths: &Paths,
    db: &Db,
    spec: &ModelSpec,
    epoch: u64,
) -> Result<(VectorIndex, bool)> {
    let library_id = db.library_id()?;
    let cache = index_cache_path(paths, spec);
    match VectorIndex::load(
        &cache,
        spec.id,
        &library_id,
        spec.dim,
        spec.quant_scale,
        epoch,
    ) {
        Some(i) => Ok((i, true)),
        None => Ok((load_index(db, spec)?, false)),
    }
}

impl Library {
    /// Opens (or creates) the library under `paths`. `model_override` forces a model id.
    pub fn open(paths: Paths, model_override: Option<&str>) -> Result<Arc<Self>> {
        paths.create_all()?;
        let mut settings = load_settings_or_default(&paths);
        let id = choose_model_id(&paths, &mut settings, model_override)?;
        let spec = registry::get(&id).ok_or_else(|| anyhow!("unknown model '{id}'"))?;
        let db = Db::open(&paths.db_path())?;
        remove_stale_temp_files(&paths.data);
        let epoch = db.items_epoch()?;
        let (index, from_cache) = load_or_rebuild_index(&paths, &db, spec, epoch)?;
        let (lib, writer_ends) = Self::new(paths, settings, spec, db, (index, from_cache), epoch);
        *lib.self_ref.write() = Arc::downgrade(&lib);
        let weak = Arc::downgrade(&lib);
        std::thread::Builder::new()
            .name("recollect-writer".into())
            .spawn(move || {
                library_writer(
                    &weak,
                    &writer_ends.requests,
                    &writer_ends.favorites,
                    &writer_ends.albums,
                );
            })
            .context("starting the library writer")?;
        Ok(lib)
    }

    /// Assembles the library around an opened database and index, with the
    /// channels to its writer thread (whose ends are returned).
    fn new(
        paths: Paths,
        settings: Settings,
        spec: &'static ModelSpec,
        db: Db,
        (index, from_cache): (VectorIndex, bool),
        epoch: u64,
    ) -> (Arc<Self>, WriterEnds) {
        let (write_tx, write_rx) = crossbeam_channel::unbounded();
        let (favorite_tx, favorite_results) = crossbeam_channel::unbounded();
        let (album_tx, album_results) = crossbeam_channel::unbounded();
        let lib = Arc::new(Self {
            paths,
            settings: RwLock::new(settings),
            settings_version: AtomicU64::new(0),
            model: RwLock::new(spec),
            db: Mutex::new(db),
            index: RwLock::new(index),
            status: Mutex::new(Status::default()),
            control: Control::default(),
            worker: Mutex::new(None),
            busy: Arc::new(AtomicBool::new(false)),
            jobs_finished: Arc::new(AtomicU64::new(0)),
            rescan_requested: AtomicBool::new(false),
            pending: Mutex::new(Pending::default()),
            self_ref: RwLock::new(std::sync::Weak::new()),
            text: Mutex::new(None),
            notifier: RwLock::new(None),
            save_lock: Mutex::new(()),
            saved: Mutex::new(from_cache.then_some((spec.id, epoch))),
            from_cache,
            write_tx,
            favorite_results,
            album_results,
            next_ticket: AtomicU64::new(1),
            favorites_version: AtomicU64::new(0),
            albums_version: AtomicU64::new(0),
            #[cfg(test)]
            fail_favorites: AtomicBool::new(false),
            #[cfg(test)]
            fail_albums: AtomicBool::new(false),
            #[cfg(test)]
            fail_download: AtomicBool::new(false),
            #[cfg(test)]
            job_log: Mutex::new(Vec::new()),
        });
        let ends = WriterEnds {
            requests: write_rx,
            favorites: favorite_tx,
            albums: album_tx,
        };
        (lib, ends)
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    /// Changes whenever the settings do.
    pub fn settings_version(&self) -> u64 {
        self.settings_version.load(Ordering::Acquire)
    }

    pub fn model(&self) -> &'static ModelSpec {
        *self.model.read()
    }

    pub fn model_dir(&self) -> PathBuf {
        download::model_dir(&self.paths.models_dir(), self.model())
    }

    pub fn model_installed(&self) -> bool {
        download::is_installed(&self.paths.models_dir(), self.model())
    }

    /// Called whenever status changes (the GUI uses it to request a repaint).
    pub fn set_notifier(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.notifier.write() = Some(Box::new(f));
    }

    fn notify(&self) {
        if let Some(n) = &*self.notifier.read() {
            n();
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }

    pub(crate) fn update_status(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock());
        self.notify();
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Number of background jobs finished so far; changes whenever one ends (UI refresh cue).
    pub fn jobs_finished(&self) -> u64 {
        self.jobs_finished.load(Ordering::Acquire)
    }

    pub fn stats(&self) -> Result<Stats> {
        self.db.lock().stats(self.model().id)
    }

    pub fn errors(&self, limit: usize) -> Result<Vec<(PathBuf, String)>> {
        self.db.lock().errors(limit)
    }

    /// Marks failed files pending again; the next indexing run retries them.
    pub fn retry_errors(&self) -> Result<usize> {
        self.db.lock().retry_errors()
    }

    /// Every file matching `filter` (indexed or not), in display order.
    pub fn timeline(&self, filter: &TimelineFilter) -> Result<Vec<TimelineItem>> {
        // Only the query holds the lock; conversion and sorting of a big library take longer.
        let rows = self.db.lock().timeline_rows(filter)?;
        Ok(rows.into_sorted(filter.order))
    }

    /// Date source, camera and favorite flag of a file, and its size on disk if readable.
    pub fn file_details(&self, file_id: i64) -> Result<Option<FileDetails>> {
        let Some(mut d) = self.db.lock().file_details(file_id)? else {
            return Ok(None);
        };
        d.size = std::fs::metadata(&d.path).ok().map(|m| m.len());
        Ok(Some(d))
    }

    /// Years (newest first) with any file, over the whole library.
    pub fn years(&self) -> Result<Vec<i32>> {
        self.db.lock().years()
    }

    /// Number of files per capture-date source.
    pub fn date_counts(&self) -> Result<DateCounts> {
        self.db.lock().date_counts()
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        self.db.lock().folders()
    }

    pub fn indexed_items(&self) -> usize {
        self.index.read().len()
    }

    /// Changes only the settings `f` touches and saves them, atomically with respect to other
    /// callers: a change made meanwhile (say, while a smart-search switch is running) is kept.
    /// The smart-search choice itself is not changed here; use [`Self::set_model`].
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut guard = self.settings.write();
        let mut new = guard.clone();
        f(&mut new);
        new.model.clone_from(&guard.model);
        if new == *guard {
            return Ok(());
        }
        new.save(&self.paths.settings_path())?;
        *guard = new;
        self.settings_version.fetch_add(1, Ordering::AcqRel);
        drop(guard);
        self.notify();
        Ok(())
    }

    /// Text → vector with the current model (loads the text tower on first use).
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        // One model for the whole call, even if the choice changes meanwhile.
        let spec = self.model();
        let mut guard = self.text.lock();
        let encoder = match &mut *guard {
            Some(t) if t.spec().id == spec.id => t,
            // Missing, or loaded for another model: (re)load it.
            slot => {
                let dir = download::model_dir(&self.paths.models_dir(), spec);
                if !download::is_installed(&self.paths.models_dir(), spec) {
                    return Err(SearchError::ModelMissing.into());
                }
                let threads = std::thread::available_parallelism()
                    .map_or(4, std::num::NonZero::get)
                    .min(8);
                slot.insert(TextEncoder::load(spec, &dir, threads)?)
            }
        };
        encoder.embed(text)
    }

    /// Text search.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> Result<Vec<SearchHit>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let v = self.embed_text(q)?;
        search::by_vector(self, &v, opts, None)
    }

    /// Media similar to an indexed item (image→image / frame→frame).
    pub fn similar(&self, item_id: i64, opts: &SearchOptions) -> Result<Vec<SearchHit>> {
        let (v, file_id) = {
            let db = self.db.lock();
            let (file_id, model) = db.item_owner(item_id)?.ok_or(SearchError::ItemGone)?;
            if model.as_deref() != Some(self.model().id) {
                return Err(SearchError::NotReady.into());
            }
            let v = db
                .embeddings(&[item_id])?
                .remove(&item_id)
                .ok_or(SearchError::ItemGone)?;
            (v, file_id)
        };
        search::by_vector(self, &v, opts, Some(file_id))
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.cancel();
        if let Some(h) = self.worker.get_mut().take()
            && h.thread().id() != std::thread::current().id()
        {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests;
