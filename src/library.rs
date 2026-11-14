//! `Library`: the thread-safe facade the GUI and CLI talk to. It owns the database, the vector
//! index, settings and the background worker (model download / indexing).

use crate::config::{Paths, Settings};
use crate::indexer;
use crate::model::{Device, ModelSpec, TextEncoder, download, registry};
use crate::search::{self, SearchHit, SearchOptions};
use crate::store::db::{
    AlbumError, AlbumInfo, DateCounts, FileDetails, Folder, Stats, TimelineFilter, TimelineItem,
};
use crate::store::{Db, VectorIndex};
use anyhow::{Context, Result, anyhow, bail};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Condvar, Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

/// What the background worker is doing right now.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Activity {
    #[default]
    Idle,
    Downloading {
        model: String,
        done: u64,
        total: u64,
    },
    Scanning {
        folder: PathBuf,
    },
    /// Reading capture dates (EXIF / video metadata) of new files.
    ReadingDates {
        done: usize,
        total: usize,
    },
    LoadingModel,
    Indexing(IndexProgress),
    SwitchingModel,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexProgress {
    pub done: usize,
    pub total: usize,
    pub errors: usize,
    pub current: Option<PathBuf>,
    /// Items (images or video frames) embedded per second.
    pub rate: f32,
    pub device: Option<Device>,
}

/// Summary of the last finished indexing run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunSummary {
    pub files: usize,
    pub errors: usize,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    /// Files whose capture date was read during this run.
    pub dates_read: usize,
    pub seconds: f32,
    pub cancelled: bool,
    pub device: Option<Device>,
    /// Folders that were unreachable or looked unmounted (kept as they were).
    pub skipped_folders: Vec<PathBuf>,
}

/// What a recorded failure was doing, so the interface can explain it in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorKind {
    #[default]
    Other,
    Download,
    AddFolder,
    RemoveFolder,
    Scan,
    SwitchModel,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub activity: Activity,
    pub paused: bool,
    /// The technical text of the last failure (shown only under "Details").
    pub last_error: Option<String>,
    /// What the last failure was doing.
    pub last_error_kind: ErrorKind,
    /// Counts every time an error was recorded, so a repeat of the same message is still
    /// noticed as a new failure.
    pub error_seq: u64,
    pub last_run: Option<RunSummary>,
}

impl Status {
    /// Records a failure (and counts it, see [`Status::error_seq`]).
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.set_error_of(ErrorKind::Other, message);
    }

    /// Like [`Status::set_error`], saying what was being done.
    pub fn set_error_of(&mut self, kind: ErrorKind, message: impl Into<String>) {
        self.last_error = Some(message.into());
        self.last_error_kind = kind;
        self.error_seq += 1;
    }
}

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

/// Work asked for while another job was running; it starts when that job ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pending {
    download: bool,
    scan: bool,
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

type Notifier = Box<dyn Fn() + Send + Sync>;

/// Work for the library writer thread.
enum WriteMsg {
    Favorite(i64, bool),
    Album(u64, AlbumOp),
    /// Answered once everything queued before it has been written.
    Flush(Sender<()>),
}

/// A change to the albums, applied by the writer in the order it was queued.
#[derive(Debug, Clone, PartialEq)]
pub enum AlbumOp {
    /// Creates an album and puts `add` in it, all in one transaction: if anything fails,
    /// nothing is created.
    Create {
        name: String,
        add: Vec<i64>,
    },
    Rename {
        id: i64,
        name: String,
    },
    Delete {
        id: i64,
    },
    Add {
        album: i64,
        files: Vec<i64>,
    },
    Remove {
        album: i64,
        files: Vec<i64>,
    },
}

/// The outcome of one favorites write, as reported by the writer thread.
#[derive(Debug, Clone, PartialEq)]
pub struct FavoriteWrite {
    pub file_id: i64,
    /// The value that was written (or that failed to be written).
    pub fav: bool,
    /// Why the write failed; `None` once it is committed.
    pub error: Option<String>,
}

/// The outcome of one album write.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumWrite {
    /// What [`Library::queue_album`] returned for this write.
    pub ticket: u64,
    pub op: AlbumOp,
    /// The album that was created, or the one the write was about.
    pub album: Option<i64>,
    /// How many items were added or removed (for `Create`: put into the new album).
    pub changed: usize,
    /// Why the write failed; `None` once it is committed.
    pub error: Option<String>,
    /// The reason when it is one the library refuses on purpose (a bad name, a removed
    /// album); `None` for any other failure.
    pub error_kind: Option<AlbumError>,
}

/// What [`Library::start_add_folders`] does once the folders are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterAdd {
    /// Only look through the folders.
    Nothing,
    /// Also download the smart-search model.
    DownloadModel,
}

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
    favorite_results: Receiver<FavoriteWrite>,
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

/// What an applied album change did.
struct Applied {
    album: Option<i64>,
    changed: usize,
    /// Whether anything in the database is different now.
    modified: bool,
}

/// The library writer: applies queued writes in order on one thread and reports each outcome.
/// Consecutive favorite marks collapse to the latest value per file; an album change ends the
/// run, so nothing is ever reordered. Exits when the library is gone.
fn library_writer(
    lib: &std::sync::Weak<Library>,
    rx: &Receiver<WriteMsg>,
    favorites: &Sender<FavoriteWrite>,
    albums: &Sender<AlbumWrite>,
) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        let Some(lib) = lib.upgrade() else {
            return;
        };
        let mut run: HashMap<i64, bool> = HashMap::new();
        let mut flushes = Vec::new();
        for m in batch {
            match m {
                WriteMsg::Favorite(id, fav) => {
                    run.insert(id, fav);
                }
                WriteMsg::Album(ticket, op) => {
                    write_favorites(&lib, &mut run, favorites);
                    let result = write_album(&lib, ticket, op);
                    let _ = albums.send(result);
                }
                WriteMsg::Flush(done) => flushes.push(done),
            }
        }
        write_favorites(&lib, &mut run, favorites);
        lib.notify();
        for done in flushes {
            let _ = done.send(());
        }
    }
}

fn write_favorites(lib: &Library, run: &mut HashMap<i64, bool>, results: &Sender<FavoriteWrite>) {
    let latest = std::mem::take(run);
    for fav in [true, false] {
        let ids: Vec<i64> = latest
            .iter()
            .filter(|(_, v)| **v == fav)
            .map(|(id, _)| *id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        #[cfg(test)]
        let outcome = if lib.fail_favorites.load(Ordering::Acquire) {
            Err(anyhow!("simulated failure"))
        } else {
            lib.db.lock().set_favorite(&ids, fav)
        };
        #[cfg(not(test))]
        let outcome = lib.db.lock().set_favorite(&ids, fav);
        let error = outcome.err().map(|e| format!("{e:#}"));
        if let Some(e) = &error {
            tracing::warn!("could not save the favorite mark: {e}");
        } else {
            lib.favorites_version.fetch_add(1, Ordering::AcqRel);
        }
        for file_id in ids {
            let _ = results.send(FavoriteWrite {
                file_id,
                fav,
                error: error.clone(),
            });
        }
    }
}

fn write_album(lib: &Library, ticket: u64, op: AlbumOp) -> AlbumWrite {
    #[cfg(test)]
    let outcome = if lib.fail_albums.load(Ordering::Acquire) {
        Err(anyhow!("simulated failure"))
    } else {
        lib.apply_album_op(&op)
    };
    #[cfg(not(test))]
    let outcome = lib.apply_album_op(&op);
    let (album, changed, error, error_kind) = match outcome {
        Ok(done) => {
            if done.modified {
                lib.albums_version.fetch_add(1, Ordering::AcqRel);
            }
            (done.album, done.changed, None, None)
        }
        Err(e) => {
            tracing::warn!("could not save the album change: {e:#}");
            let kind = e.downcast_ref::<AlbumError>().cloned();
            (None, 0, Some(format!("{e:#}")), kind)
        }
    };
    AlbumWrite {
        ticket,
        op,
        album,
        changed,
        error,
        error_kind,
    }
}

/// How often the hardware was asked about (tests check that screens don't do it every frame).
#[cfg(test)]
pub(crate) static HARDWARE_CHECKS: AtomicU64 = AtomicU64::new(0);

/// Picks the default model by hardware: a discrete GPU gets the quality model.
pub fn default_model_for_hardware() -> &'static str {
    #[cfg(test)]
    HARDWARE_CHECKS.fetch_add(1, Ordering::Relaxed);
    if crate::gpu::has_discrete_gpu() {
        registry::DEFAULT_GPU_MODEL
    } else {
        registry::DEFAULT_CPU_MODEL
    }
}

impl Library {
    /// Opens (or creates) the library under `paths`. `model_override` forces a model id.
    pub fn open(paths: Paths, model_override: Option<&str>) -> Result<Arc<Self>> {
        paths.create_all()?;
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
        let id = match model_override {
            Some(id) => id.to_string(),
            None => {
                if let Some(id) = &settings.model {
                    id.clone()
                } else {
                    let id = default_model_for_hardware().to_string();
                    settings.model = Some(id.clone());
                    settings.save(&paths.settings_path())?;
                    id
                }
            }
        };
        let spec = registry::get(&id).ok_or_else(|| anyhow!("unknown model '{id}'"))?;
        let db = Db::open(&paths.db_path())?;
        remove_stale_temp_files(&paths.data);
        let epoch = db.items_epoch()?;
        let library_id = db.library_id()?;
        let cache = index_cache_path(&paths, spec);
        let (index, from_cache) = match VectorIndex::load(
            &cache,
            spec.id,
            &library_id,
            spec.dim,
            spec.quant_scale,
            epoch,
        ) {
            Some(i) => (i, true),
            None => (load_index(&db, spec)?, false),
        };
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
        *lib.self_ref.write() = Arc::downgrade(&lib);
        let weak = Arc::downgrade(&lib);
        std::thread::Builder::new()
            .name("recollect-writer".into())
            .spawn(move || library_writer(&weak, &write_rx, &favorite_tx, &album_tx))
            .context("starting the library writer")?;
        Ok(lib)
    }

    /// True if startup reused the on-disk index cache instead of rebuilding from the database.
    pub fn loaded_from_cache(&self) -> bool {
        self.from_cache
    }

    /// Writes the in-memory index to disk so the next start can skip decoding every vector.
    /// Does nothing if a save is already running or the cache is already current.
    pub fn save_index_cache(&self) {
        let Some(_saving) = self.save_lock.try_lock() else {
            return;
        };
        // Lock order is db → index everywhere. Everything the header records (model, epoch,
        // library id, expected count) is read under the db lock, and the index read lock is
        // taken before the db lock is released, so the index we write is the one these describe.
        let db = self.db.lock();
        let spec = self.model();
        let (epoch, count, library_id) = match db.items_epoch().and_then(|e| {
            let n = db.item_count(spec.id)?;
            Ok((e, n, db.library_id()?))
        }) {
            Ok(v) => v,
            Err(e) => return tracing::warn!("index cache: {e:#}"),
        };
        if *self.saved.lock() == Some((spec.id, epoch)) {
            return;
        }
        let index = self.index.read();
        drop(db);
        if index.dim() != spec.dim || index.len() != count {
            // Never persist an index that doesn't mirror the database.
            return tracing::debug!(
                "index cache not written: {} vectors (dim {}) vs {count} items (dim {})",
                index.len(),
                index.dim(),
                spec.dim
            );
        }
        let path = index_cache_path(&self.paths, spec);
        // The read lock is held for the whole write: the file must describe exactly this index,
        // so writers (new items, date patches) wait until it is on disk. Searches are not
        // blocked, and the write is skipped when the cache is already current.
        match index.save(&path, spec.id, &library_id, epoch) {
            Ok(()) => *self.saved.lock() = Some((spec.id, epoch)),
            Err(e) => tracing::warn!("could not write index cache {}: {e}", path.display()),
        }
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

    /// Marks files as favorites (or removes the mark), waiting for the write. The GUI uses
    /// [`Self::queue_favorite`] instead.
    pub fn set_favorite(&self, file_ids: &[i64], fav: bool) -> Result<()> {
        self.db.lock().set_favorite(file_ids, fav)?;
        self.favorites_version.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Queues a favorite mark for the writer thread and returns at once. Writes are applied
    /// in order; consecutive writes for one file collapse into the latest value. The outcome
    /// arrives through [`Self::take_favorite_results`].
    pub fn queue_favorite(&self, file_id: i64, fav: bool) {
        let _ = self.write_tx.send(WriteMsg::Favorite(file_id, fav));
    }

    /// Blocks until every write queued so far (favorites and album changes) has been written
    /// (or has failed).
    pub fn wait_writes(&self) {
        self.wait_writes_for(None);
    }

    /// Like [`Self::wait_writes`] but gives up after `limit` (e.g. at exit, when the database
    /// may be busy). Returns true when everything queued has been written.
    pub fn wait_writes_for(&self, limit: impl Into<Option<std::time::Duration>>) -> bool {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if self.write_tx.send(WriteMsg::Flush(tx)).is_err() {
            return false;
        }
        match limit.into() {
            Some(l) => rx.recv_timeout(l).is_ok(),
            None => rx.recv().is_ok(),
        }
    }

    /// Queues an album change for the writer thread and returns its ticket at once. Changes
    /// are applied in the order queued, also relative to favorite marks; the outcome (with the
    /// ticket, and the new album's id for `Create`) arrives through
    /// [`Self::take_album_results`].
    pub fn queue_album(&self, op: AlbumOp) -> u64 {
        let ticket = self.next_ticket.fetch_add(1, Ordering::AcqRel);
        let _ = self.write_tx.send(WriteMsg::Album(ticket, op));
        ticket
    }

    /// Outcomes of the album writes finished since the last call, oldest first.
    pub fn take_album_results(&self) -> Vec<AlbumWrite> {
        self.album_results.try_iter().collect()
    }

    /// Changes after every committed album write: album lists and album views are stale once
    /// it moves.
    pub fn albums_version(&self) -> u64 {
        self.albums_version.load(Ordering::Acquire)
    }

    /// Applies one album change.
    fn apply_album_op(&self, op: &AlbumOp) -> Result<Applied> {
        let mut db = self.db.lock();
        Ok(match op {
            AlbumOp::Create { name, add } => {
                let (id, n) = db.create_album_with(name, add)?;
                Applied {
                    album: Some(id),
                    changed: n,
                    modified: true,
                }
            }
            AlbumOp::Rename { id, name } => {
                db.rename_album(*id, name)?;
                Applied {
                    album: Some(*id),
                    changed: 0,
                    modified: true,
                }
            }
            AlbumOp::Delete { id } => Applied {
                album: Some(*id),
                changed: 0,
                modified: db.delete_album(*id)?,
            },
            AlbumOp::Add { album, files } => {
                let n = db.add_to_album(*album, files)?;
                Applied {
                    album: Some(*album),
                    changed: n,
                    modified: n > 0,
                }
            }
            AlbumOp::Remove { album, files } => {
                let n = db.remove_from_album(*album, files)?;
                Applied {
                    album: Some(*album),
                    changed: n,
                    modified: n > 0,
                }
            }
        })
    }

    /// All albums with counts and covers.
    pub fn albums(&self) -> Result<Vec<AlbumInfo>> {
        self.db.lock().albums()
    }

    /// Creates an album straight away, bypassing the write queue (tests set up state with it).
    #[cfg(test)]
    pub(crate) fn create_album(&self, name: &str, add: &[i64]) -> Result<i64> {
        let done = self.apply_album_op(&AlbumOp::Create {
            name: name.to_string(),
            add: add.to_vec(),
        })?;
        self.albums_version.fetch_add(1, Ordering::AcqRel);
        Ok(done.album.expect("create returns an id"))
    }

    /// Adds files to an album straight away, bypassing the write queue (tests only).
    #[cfg(test)]
    pub(crate) fn add_to_album(&self, album: i64, files: &[i64]) -> Result<usize> {
        let done = self.apply_album_op(&AlbumOp::Add {
            album,
            files: files.to_vec(),
        })?;
        self.albums_version.fetch_add(1, Ordering::AcqRel);
        Ok(done.changed)
    }

    /// Outcomes of the favorites writes finished since the last call, oldest first.
    pub fn take_favorite_results(&self) -> Vec<FavoriteWrite> {
        self.favorite_results.try_iter().collect()
    }

    /// Changes after every committed favorites write: lists filtered by favorites are stale
    /// once it moves.
    pub fn favorites_version(&self) -> u64 {
        self.favorites_version.load(Ordering::Acquire)
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

    /// Runs `task` on a named background thread. If the thread cannot be started the failure
    /// is reported through the status as an error of `kind`.
    fn spawn_task(
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
                lib.save_index_cache();
            },
        );
    }

    pub(crate) fn remove_files_from_index(&self, ids: &HashSet<i64>) {
        self.index.write().remove_files(ids);
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

    /// Makes `id` the smart-search choice. Switching rebuilds the vector index, so it runs as a
    /// background job; the choice is stored when the switch completes and nothing else in the
    /// settings is touched.
    pub fn set_model(self: &Arc<Self>, id: &str) -> Result<()> {
        let spec = registry::get(id).ok_or_else(|| anyhow!("unknown model '{id}'"))?;
        if spec.id == self.model().id {
            return self.store_model(spec);
        }
        let lib = Arc::clone(self);
        self.spawn(move || {
            lib.update_status(|s| s.activity = Activity::SwitchingModel);
            let res = lib.switch_model(spec);
            let ok = res.is_ok();
            if !ok {
                // A download queued behind the switch would fetch the wrong model.
                lib.pending.lock().download = false;
            }
            lib.update_status(|s| {
                s.activity = Activity::Idle;
                s.last_run = None;
                if let Err(e) = res {
                    s.set_error_of(
                        ErrorKind::SwitchModel,
                        format!("Switching models failed: {e:#}"),
                    );
                }
            });
            if ok {
                // Re-embed with the new model (if it is installed) in the same job.
                lib.run_indexing_job();
            }
        })
    }

    /// Stores `spec` as the model field of the settings, leaving every other field as it is.
    fn store_model(&self, spec: &'static ModelSpec) -> Result<()> {
        let mut guard = self.settings.write();
        if guard.model.as_deref() == Some(spec.id) {
            return Ok(());
        }
        let mut new = guard.clone();
        new.model = Some(spec.id.to_string());
        new.save(&self.paths.settings_path())?;
        *guard = new;
        self.settings_version.fetch_add(1, Ordering::AcqRel);
        drop(guard);
        self.notify();
        Ok(())
    }

    fn switch_model(&self, spec: &'static ModelSpec) -> Result<()> {
        // Index and model change together under the db lock (order db → index), so a cache save
        // never sees one of them without the other.
        let db = self.db.lock();
        let index = load_index(&db, spec)?;
        self.store_model(spec)?;
        *self.index.write() = index;
        *self.model.write() = spec;
        drop(db);
        *self.text.lock() = None;
        Ok(())
    }

    /// Starts downloading the current model in the background (the caller has obtained the
    /// user's consent). When the download succeeds the same job goes on to scan, read dates and
    /// embed everything, so the library is searchable without another click. If another job
    /// (say, a dates-only scan) is running, it is cancelled first; waiting for it happens off the
    /// calling thread.
    pub fn start_download(self: &Arc<Self>) -> Result<()> {
        if matches!(self.status().activity, Activity::Downloading { .. }) {
            return Ok(());
        }
        self.update_status(|s| s.last_run = None);
        if self.spawn(Self::download_job(Arc::clone(self))).is_ok() {
            return Ok(());
        }
        self.cancel();
        self.spawn_task("recollect-download", ErrorKind::Download, move |lib| {
            for _ in 0..200 {
                lib.wait();
                if lib.spawn(Self::download_job(Arc::clone(lib))).is_ok() {
                    return;
                }
                // Something else won the race; cancel that one too and try again.
                lib.cancel();
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            lib.update_status(|s| {
                s.set_error_of(
                    ErrorKind::Download,
                    "Model download failed: another job would not stop",
                );
            });
        });
        Ok(())
    }

    /// True in tests that make downloads fail without the network.
    #[cfg_attr(
        not(test),
        expect(clippy::unused_self, reason = "only tests simulate download failures")
    )]
    fn simulated_download_failure(&self) -> bool {
        #[cfg(test)]
        return self.fail_download.load(Ordering::Acquire);
        #[cfg(not(test))]
        false
    }

    fn download_job(lib: Arc<Self>) -> impl FnOnce() + Send + 'static {
        move || {
            let spec = lib.model();
            #[cfg(test)]
            lib.job_log.lock().push("download");
            let res = if lib.simulated_download_failure() {
                Err(anyhow!("simulated failure"))
            } else {
                download::download(
                    &lib.paths.models_dir(),
                    spec,
                    lib.control.cancel_flag(),
                    &mut |done, total| {
                        lib.update_status(|s| {
                            s.activity = Activity::Downloading {
                                model: spec.id.to_string(),
                                done,
                                total,
                            }
                        });
                    },
                )
            };
            let ok = res.is_ok();
            lib.update_status(|s| {
                s.activity = Activity::Idle;
                if let Err(e) = res
                    && !lib.control.is_cancelled()
                {
                    s.set_error_of(ErrorKind::Download, format!("Model download failed: {e:#}"));
                }
            });
            if ok && !lib.control.is_cancelled() {
                lib.run_indexing_job();
            }
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
    fn run_indexing_job(&self) {
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

fn index_cache_path(paths: &Paths, spec: &ModelSpec) -> PathBuf {
    paths.data.join(format!("index-{}.bin", spec.id))
}

/// Removes temp files a crashed index-cache save left behind.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(600);

fn remove_stale_temp_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // Another process may be saving right now; its file keeps being written, so only
        // remove temp files nobody touched for a while (left by a killed save).
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_TEMP_AGE);
        if name.starts_with("index-") && name.ends_with(".tmp") && stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn load_index(db: &Db, spec: &ModelSpec) -> Result<VectorIndex> {
    let t = Instant::now();
    let mut idx = VectorIndex::new(spec.dim, spec.quant_scale);
    let mut bad = 0usize;
    db.for_each_item(spec.id, |it| {
        if it.embedding.len() == spec.dim {
            idx.add(it.id, it.file_id, it.kind, it.taken_at, &it.embedding);
        } else {
            bad += 1;
        }
    })?;
    if bad > 0 {
        tracing::warn!("{bad} stored vectors have the wrong dimension and were skipped");
    }
    tracing::info!(
        "loaded {} vectors for {} in {:?}",
        idx.len(),
        spec.id,
        t.elapsed()
    );
    Ok(idx)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::Embedded;
    use crate::media::{MediaKind, ScannedFile};
    use crate::search::SearchSort;
    use crate::store::db::{FileMeta, NewItem};

    fn open(dir: &Path) -> Arc<Library> {
        crate::gpu::force_cpu(true);
        Library::open(Paths::under(dir), Some(registry::CLIP_B32.id)).unwrap()
    }

    /// Stores one indexed image with a CLIP-sized vector; returns its item id.
    fn seed(lib: &Library) -> i64 {
        let mut db = lib.db.lock();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        let file = ScannedFile {
            path: "/lib/a.jpg".into(),
            kind: MediaKind::Image,
            size: 1,
            mtime: 1,
        };
        db.sync_folder(f, &[file], &[]).unwrap();
        let id = db.pending(registry::CLIP_B32.id).unwrap()[0].id;
        let mut v = vec![0.0; 512];
        v[0] = 1.0;
        let item = NewItem {
            t_start: None,
            t_end: None,
            embedding: v,
        };
        db.put_items(
            registry::CLIP_B32.id,
            &[(id, FileMeta::default(), vec![item])],
        )
        .unwrap()[0][0]
    }

    #[test]
    fn model_switch_rebuilds_index_and_old_vectors_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let item = seed(&open(d.path()));
        let lib = open(d.path()); // reload: the index now holds the seeded vector
        assert_eq!(lib.indexed_items(), 1);
        assert!(
            lib.similar(item, &SearchOptions::default())
                .unwrap()
                .is_empty()
        );

        lib.set_model(registry::SIGLIP_B16.id).unwrap();
        lib.wait();
        assert_eq!(lib.status().last_error, None);
        assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
        assert_eq!(lib.indexed_items(), 0);
        let saved = Settings::load(&lib.paths().settings_path()).unwrap();
        assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
        // The old 512-d vector must produce an error, not a panic.
        let err = lib.similar(item, &SearchOptions::default()).unwrap_err();
        assert_eq!(SearchError::of(&err), SearchError::NotReady, "{err}");
        assert!(search::by_vector(&lib, &[0.0; 512], &SearchOptions::default(), None).is_err());
    }

    #[test]
    fn a_settings_change_during_a_model_switch_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        {
            // The switch job waits for the database while the user changes the theme.
            let _db = lib.db.lock();
            lib.set_model(registry::SIGLIP_B16.id).unwrap();
            lib.update_settings(|s| {
                s.theme = crate::config::Theme::Dark;
                s.use_gpu = false;
            })
            .unwrap();
        }
        lib.wait();
        assert_eq!(lib.status().last_error, None);
        assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
        let saved = Settings::load(&lib.paths().settings_path()).unwrap();
        assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
        assert_eq!(saved.theme, crate::config::Theme::Dark);
        assert!(!saved.use_gpu);
        assert_eq!(lib.settings(), saved);
    }

    #[test]
    fn update_settings_leaves_the_model_choice_alone() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        let before = lib.settings().model;
        lib.update_settings(|s| {
            s.model = Some("something-else".into());
            s.search_prompt_dismissed = true;
        })
        .unwrap();
        assert_eq!(lib.settings().model, before);
        assert!(lib.settings().search_prompt_dismissed);
        let saved = Settings::load(&lib.paths().settings_path()).unwrap();
        assert!(saved.search_prompt_dismissed);
    }

    #[test]
    fn every_recorded_error_is_counted_even_when_the_text_repeats() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        assert_eq!(lib.status().error_seq, 0);
        lib.update_status(|s| s.set_error("disk full"));
        let first = lib.status().error_seq;
        lib.update_status(|s| s.set_error("disk full"));
        let st = lib.status();
        assert_eq!(st.last_error.as_deref(), Some("disk full"));
        assert_eq!(st.error_seq, first + 1);
    }

    #[test]
    fn long_videos_do_not_crowd_out_other_files() {
        let d = tempfile::tempdir().unwrap();
        {
            let lib = open(d.path());
            let mut db = lib.db.lock();
            let f = db.add_folder(Path::new("/lib")).unwrap();
            let mut files = vec![ScannedFile {
                path: "/lib/long.mp4".into(),
                kind: MediaKind::Video,
                size: 1,
                mtime: 1,
            }];
            for i in 0..30 {
                files.push(ScannedFile {
                    path: format!("/lib/{i:02}.jpg").into(),
                    kind: MediaKind::Image,
                    size: 1,
                    mtime: 1,
                });
            }
            db.sync_folder(f, &files, &[]).unwrap();
            // Spread over all dimensions like real embeddings (no saturating components):
            // q = constant direction, r = orthogonal alternating direction.
            let unit = |a: f32| {
                let c = 1.0 / (512f32).sqrt();
                (0..512)
                    .map(|i| c * a.cos() + if i % 2 == 0 { c } else { -c } * a.sin())
                    .collect::<Vec<f32>>()
            };
            let mut batch = Vec::new();
            for row in db.pending(registry::CLIP_B32.id).unwrap() {
                let items = if row.kind == MediaKind::Video {
                    // 1000 segments all closer to the query than any image.
                    (0..1000)
                        .map(|i| NewItem {
                            t_start: Some(f64::from(i) * 2.0),
                            t_end: Some(f64::from(i) * 2.0 + 2.0),
                            embedding: unit(0.01),
                        })
                        .collect()
                } else {
                    vec![NewItem {
                        t_start: None,
                        t_end: None,
                        embedding: unit(0.5),
                    }]
                };
                batch.push((row.id, FileMeta::default(), items));
            }
            db.put_items(registry::CLIP_B32.id, &batch).unwrap();
        }
        let lib = open(d.path());
        let q = vec![1.0 / (512f32).sqrt(); 512];
        let hits = search::by_vector(
            &lib,
            &q,
            &SearchOptions {
                limit: 20,
                kind: None,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(hits.len(), 20);
        assert_eq!(hits[0].kind, MediaKind::Video);
        assert_eq!(hits[0].moments.len(), 10);
        // "Find similar" from the video must skip the video's own segments.
        let hits = lib
            .similar(
                hits[0].item_id,
                &SearchOptions {
                    limit: 20,
                    kind: None,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(hits.len(), 20);
        assert!(hits.iter().all(|h| h.kind == MediaKind::Image));
    }

    #[test]
    fn empty_mount_point_keeps_the_index_and_missing_files_stay_pending() {
        let d = tempfile::tempdir().unwrap();
        let media = d.path().join("mnt");
        std::fs::create_dir_all(&media).unwrap();
        let lib = open(d.path());
        lib.add_folder(&media).unwrap();
        {
            // As if indexed earlier, then the drive was unmounted (empty mount point).
            let mut db = lib.db.lock();
            let f = db.folders().unwrap()[0].id;
            let file = ScannedFile {
                path: media.join("a.jpg"),
                kind: MediaKind::Image,
                size: 1,
                mtime: 1,
            };
            db.sync_folder(f, &[file], &[]).unwrap();
            let id = db.pending(registry::CLIP_B32.id).unwrap()[0].id;
            let item = NewItem {
                t_start: None,
                t_end: None,
                embedding: vec![0.1; 512],
            };
            db.put_items(
                registry::CLIP_B32.id,
                &[(id, FileMeta::default(), vec![item])],
            )
            .unwrap();
        }
        let sum = crate::indexer::run(&lib).unwrap();
        assert_eq!(
            sum.skipped_folders,
            vec![std::fs::canonicalize(&media).unwrap()]
        );
        assert_eq!(lib.stats().unwrap().indexed, 1, "index kept");

        // A file that vanishes between scan and decode is not marked as failed.
        let row = crate::store::db::FileRow {
            id: 1,
            path: media.join("gone.jpg"),
            kind: MediaKind::Image,
            size: 1,
            mtime: 1,
            status: crate::store::db::FileStatus::Pending,
        };
        let err = anyhow!("decode failed");
        assert_eq!(
            crate::indexer::file_failed(&lib, &row, &err).unwrap(),
            crate::indexer::Failure::Retry
        );
        // A file that is still there but cannot be decoded is recorded as an error.
        let bad = media.join("bad.jpg");
        std::fs::write(&bad, b"not an image").unwrap();
        let row = crate::store::db::FileRow { path: bad, ..row };
        assert_eq!(
            crate::indexer::file_failed(&lib, &row, &err).unwrap(),
            crate::indexer::Failure::Recorded
        );
    }

    #[test]
    fn retry_errors_and_sync_of_a_removed_folder() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        let file = ScannedFile {
            path: "/lib/a.jpg".into(),
            kind: MediaKind::Image,
            size: 1,
            mtime: 1,
        };
        db.sync_folder(f, std::slice::from_ref(&file), &[]).unwrap();
        let id = db.pending("m").unwrap()[0].id;
        db.set_error(id, "bad").unwrap();
        assert!(db.pending("m").unwrap().is_empty());
        assert_eq!(db.retry_errors().unwrap(), 1);
        assert_eq!(db.pending("m").unwrap().len(), 1);
        db.remove_folder(f).unwrap();
        // A scan that started before the removal must not resurrect rows (or hit FK errors).
        assert_eq!(
            db.sync_folder(f, &[file], &[]).unwrap(),
            crate::store::db::SyncStats::default()
        );
        assert_eq!(db.stats("m").unwrap().images, 0);
    }

    #[test]
    fn index_cache_is_written_and_reused() {
        let d = tempfile::tempdir().unwrap();
        seed(&open(d.path()));
        let lib = open(d.path()); // rebuilt from the database
        assert_eq!(lib.indexed_items(), 1);
        lib.save_index_cache();
        let cache = index_cache_path(lib.paths(), lib.model());
        assert!(cache.exists());
        drop(lib);
        let lib = open(d.path());
        assert_eq!(lib.indexed_items(), 1, "loaded from the cache");
        assert!(lib.loaded_from_cache());
        let id = lib.db.lock().library_id().unwrap();
        let load = |id: &str, epoch: u64| {
            VectorIndex::load(
                &cache,
                lib.model().id,
                id,
                512,
                lib.model().quant_scale,
                epoch,
            )
        };
        let epoch = lib.db.lock().items_epoch().unwrap();
        assert!(load(&id, epoch).is_some());
        assert!(load("another-library", epoch).is_none(), "library id");
        // Any database change makes the cache stale.
        lib.db.lock().set_meta("items_epoch", "999").unwrap();
        let epoch = lib.db.lock().items_epoch().unwrap();
        assert!(load(&id, epoch).is_none());
    }

    #[test]
    fn unchanged_index_is_not_rewritten_and_stale_temp_files_are_removed() {
        let d = tempfile::tempdir().unwrap();
        seed(&open(d.path()));
        let lib = open(d.path());
        assert!(!lib.loaded_from_cache());
        let cache = index_cache_path(lib.paths(), lib.model());
        assert!(!cache.exists(), "open must not write the cache");
        lib.save_index_cache();
        let first = std::fs::metadata(&cache).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        lib.save_index_cache();
        assert_eq!(
            std::fs::metadata(&cache).unwrap().modified().unwrap(),
            first
        );
        // A change makes the next save write again.
        lib.db.lock().set_meta("items_epoch", "999").unwrap();
        lib.save_index_cache();
        assert!(std::fs::metadata(&cache).unwrap().modified().unwrap() > first);
        // Leftovers of a crashed save are cleaned at open.
        let tmp = lib.paths().data.join("index-clip-vit-b32.1.2.tmp");
        let fresh = lib.paths().data.join("index-clip-vit-b32.3.4.tmp");
        std::fs::write(&tmp, b"x").unwrap();
        std::fs::write(&fresh, b"x").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&tmp)
            .unwrap()
            .set_modified(old)
            .unwrap();
        drop(lib);
        let _lib = open(d.path());
        assert!(!tmp.exists(), "abandoned temp file removed");
        assert!(
            fresh.exists(),
            "a save in progress (fresh temp file) is left alone"
        );
    }

    #[test]
    fn removing_a_folder_in_the_background_clears_the_index() {
        let d = tempfile::tempdir().unwrap();
        seed(&open(d.path()));
        let lib = open(d.path());
        let folder = lib.folders().unwrap()[0].id;
        lib.start_remove_folder(folder);
        for _ in 0..200 {
            if lib.folders().unwrap().is_empty() && !lib.is_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        lib.wait();
        assert!(lib.folders().unwrap().is_empty());
        assert_eq!(lib.indexed_items(), 0);
    }

    /// Writes every file of the current model with its expected size (sparse), so the download
    /// finds nothing to fetch.
    fn fake_install(lib: &Library) {
        let dir = lib.model_dir();
        std::fs::create_dir_all(&dir).unwrap();
        for f in lib.model().files {
            let file = std::fs::File::create(dir.join(f.local)).unwrap();
            file.set_len(f.size).unwrap();
        }
        assert!(lib.model_installed());
    }

    fn wait_idle(lib: &Library) {
        for _ in 0..500 {
            if !lib.is_busy() && lib.status().last_run.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        lib.wait();
    }

    #[test]
    fn a_finished_download_continues_with_an_indexing_run() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        let empty = d.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        lib.add_folder(&empty).unwrap();
        fake_install(&lib);
        lib.update_status(|s| {
            s.last_run = Some(RunSummary {
                files: 99,
                ..Default::default()
            });
        });
        lib.start_download().unwrap();
        wait_idle(&lib);
        let st = lib.status();
        assert_eq!(st.last_error, None);
        let run = st.last_run.expect("the download was followed by a run");
        assert_eq!(run.files, 0, "the stale summary was replaced");
        assert_eq!(st.activity, Activity::Idle);
    }

    #[test]
    fn downloading_while_another_job_runs_cancels_it_first() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        fake_install(&lib);
        let l = Arc::clone(&lib);
        // A job that only ends when cancelled (stands in for a long dates-only scan).
        lib.spawn(move || {
            while !l.control.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
        .unwrap();
        lib.start_download().unwrap();
        wait_idle(&lib);
        assert_eq!(lib.status().last_error, None);
        assert!(lib.status().last_run.is_some(), "download, then indexing");
    }

    /// A folder with one picture, for the tests of the job queue.
    fn one_photo_folder(d: &Path) -> PathBuf {
        let photos = d.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        image::RgbImage::new(4, 4)
            .save(photos.join("a.png"))
            .unwrap();
        photos
    }

    /// Waits until `cond` holds and no job runs (jobs may follow one another).
    fn wait_until(lib: &Library, mut cond: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if cond() && !lib.is_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        lib.wait();
        assert!(cond(), "condition not reached");
    }

    #[test]
    fn folders_are_scanned_before_a_download_that_was_asked_for_with_them() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(&d.path().join("home"));
        fake_install(&lib);
        // (Empty, so the placeholder model files never have to embed anything.)
        let empty = d.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        lib.start_add_folders(vec![empty], AfterAdd::DownloadModel);
        wait_until(&lib, || lib.job_log.lock().len() >= 3);
        // The scan, then the download, then the scan that follows a download.
        assert_eq!(*lib.job_log.lock(), ["scan", "download", "scan"]);
        assert_eq!(lib.folders().unwrap().len(), 1);
        assert_eq!(lib.status().last_error, None);
    }

    #[test]
    fn a_failed_download_still_leaves_the_folders_scanned() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(&d.path().join("home"));
        lib.fail_download.store(true, Ordering::Release);
        let photos = one_photo_folder(d.path());
        lib.start_add_folders(vec![photos], AfterAdd::DownloadModel);
        wait_until(&lib, || {
            lib.status().last_error.is_some() && lib.job_log.lock().len() >= 2
        });
        assert_eq!(*lib.job_log.lock(), ["scan", "download"]);
        assert_eq!(lib.status().last_error_kind, ErrorKind::Download);
        assert_eq!(lib.timeline(&TimelineFilter::default()).unwrap().len(), 1);
        assert_eq!(lib.date_counts().unwrap().file, 1, "dates were read");
    }

    #[test]
    fn a_scan_asked_for_while_busy_waits_instead_of_being_dropped() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(&d.path().join("home"));
        let release = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&release);
        lib.spawn(move || {
            while !r.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
        .unwrap();
        assert_eq!(lib.request_scan().unwrap(), Request::Queued);
        assert_eq!(lib.request_download().unwrap(), Request::Queued);
        assert!(lib.job_log.lock().is_empty(), "nothing started yet");
        release.store(true, Ordering::Release);
        lib.fail_download.store(true, Ordering::Release);
        wait_until(&lib, || lib.job_log.lock().len() >= 2);
        assert_eq!(*lib.job_log.lock(), ["scan", "download"]);
        // With nothing running, a request starts at once.
        assert_eq!(lib.request_scan().unwrap(), Request::Started);
        lib.wait();
    }

    #[test]
    fn indexing_without_a_model_reads_dates() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(&d.path().join("home"));
        assert!(!lib.model_installed());
        let photos = d.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        image::RgbImage::new(4, 4)
            .save(photos.join("a.png"))
            .unwrap();
        lib.add_folder(&photos).unwrap();
        let sum = lib.index_blocking().unwrap();
        assert_eq!((sum.added, sum.dates_read, sum.files), (1, 1, 0));
        let tl = lib.timeline(&TimelineFilter::default()).unwrap();
        assert_eq!(tl.len(), 1);
        assert_eq!(
            lib.stats().unwrap().pending,
            1,
            "embedding is still waiting"
        );
        assert_eq!(lib.date_counts().unwrap().file, 1);
        // A second run has nothing left to read.
        assert_eq!(lib.index_blocking().unwrap().dates_read, 0);
    }

    fn day(y: i32, m: u32, d: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    }

    /// A unit vector at `angle` radians from the first axis (the query used below).
    fn at_angle(angle: f32) -> Vec<f32> {
        let mut v = vec![0.0; 512];
        v[0] = angle.cos();
        v[1] = angle.sin();
        v
    }

    fn query() -> Vec<f32> {
        at_angle(0.0)
    }

    fn names(hits: &[search::SearchHit]) -> Vec<String> {
        hits.iter()
            .map(|h| h.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// Four images captured on known days. The better the match, the earlier its name: a, b, c, d.
    /// By date: b (2022) is newest, then d (2021), a (2020), c (2019).
    fn seed_dated(lib: &Library, undated: &[&str]) -> Vec<Embedded> {
        let mut db = lib.db.lock();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        let spec = [
            ("a", 0.1, day(2020, 6, 1)),
            ("b", 0.2, day(2022, 12, 31)),
            ("c", 0.3, day(2019, 1, 10)),
            ("d", 0.4, day(2021, 3, 5)),
        ];
        let files: Vec<_> = spec
            .iter()
            .map(|(n, _, _)| ScannedFile {
                path: format!("/lib/{n}.jpg").into(),
                kind: MediaKind::Image,
                size: 1,
                mtime: 1,
            })
            .collect();
        db.sync_folder(f, &files, &[]).unwrap();
        let rows = db.pending(registry::CLIP_B32.id).unwrap();
        let caps: Vec<_> = rows
            .iter()
            .filter(|r| !undated.iter().any(|u| r.path.ends_with(format!("{u}.jpg"))))
            .map(|r| {
                let t = spec
                    .iter()
                    .find(|s| r.path.ends_with(format!("{}.jpg", s.0)));
                let t = t.unwrap().2;
                (
                    r.id,
                    crate::media::metadata::Capture {
                        taken_at: t,
                        source: crate::media::metadata::DateSource::Exif,
                        camera: None,
                        duration: None,
                    },
                )
            })
            .collect();
        db.set_captures(&caps).unwrap();
        rows.into_iter()
            .map(|r| {
                let a = spec
                    .iter()
                    .find(|s| r.path.ends_with(format!("{}.jpg", s.0)))
                    .unwrap()
                    .1;
                let item = NewItem {
                    t_start: None,
                    t_end: None,
                    embedding: at_angle(a),
                };
                Embedded {
                    file: r,
                    meta: FileMeta::default(),
                    items: vec![item],
                }
            })
            .collect()
    }

    fn search_with(lib: &Library, range: Option<(i64, i64)>, sort: SearchSort) -> Vec<String> {
        let opts = SearchOptions {
            taken_range: range,
            sort,
            ..Default::default()
        };
        names(&search::by_vector(lib, &query(), &opts, None).unwrap())
    }

    fn check_date_search(lib: &Library) {
        let all = |r| search_with(lib, None, r);
        assert_eq!(
            all(SearchSort::Relevance),
            ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
        );
        assert_eq!(
            all(SearchSort::Newest),
            ["b.jpg", "d.jpg", "a.jpg", "c.jpg"]
        );
        assert_eq!(
            all(SearchSort::Oldest),
            ["c.jpg", "a.jpg", "d.jpg", "b.jpg"]
        );
        // 2020 through 2021, inclusive of both whole years.
        let range = Some((day(2020, 1, 1) - 43_200, day(2021, 12, 31) + 43_199));
        assert_eq!(
            search_with(lib, range, SearchSort::Relevance),
            ["a.jpg", "d.jpg"]
        );
        assert_eq!(
            search_with(lib, range, SearchSort::Newest),
            ["d.jpg", "a.jpg"]
        );
        // The bounds themselves are inside the range.
        let exact = Some((day(2019, 1, 10), day(2019, 1, 10)));
        assert_eq!(search_with(lib, exact, SearchSort::Relevance), ["c.jpg"]);
        assert!(
            search_with(
                lib,
                Some((day(2023, 1, 1), day(2024, 1, 1))),
                SearchSort::Newest
            )
            .is_empty()
        );
        // Hits carry their capture time.
        let hits = search::by_vector(lib, &query(), &SearchOptions::default(), None).unwrap();
        assert_eq!(hits[1].taken_at, day(2022, 12, 31));
    }

    #[test]
    fn search_by_date_range_and_sort_from_the_database_and_from_indexing() {
        // The index built from the database at startup.
        let d = tempfile::tempdir().unwrap();
        {
            let lib = open(d.path());
            let rows = seed_dated(&lib, &[]);
            let batch: Vec<_> = rows
                .iter()
                .map(|e| (e.file.id, e.meta, e.items.clone()))
                .collect();
            lib.db
                .lock()
                .put_items(registry::CLIP_B32.id, &batch)
                .unwrap();
        }
        let lib = open(d.path());
        assert_eq!(lib.indexed_items(), 4);
        check_date_search(&lib);
        // ...and through the cache.
        lib.save_index_cache();
        drop(lib);
        let lib = open(d.path());
        assert!(lib.loaded_from_cache());
        check_date_search(&lib);

        // The index filled while indexing (dates are known before embedding).
        let d2 = tempfile::tempdir().unwrap();
        let lib = open(d2.path());
        let rows = seed_dated(&lib, &[]);
        assert_eq!(lib.indexed_items(), 0);
        crate::indexer::write_items(&lib, rows).unwrap();
        assert_eq!(lib.indexed_items(), 4);
        check_date_search(&lib);
    }

    #[test]
    fn a_date_learned_later_updates_the_index_and_its_cache() {
        let d = tempfile::tempdir().unwrap();
        let id_b = {
            let lib = open(d.path());
            // File "b" is embedded before its date could be read: only its file time (1970)
            // is known.
            let rows = seed_dated(&lib, &["b"]);
            let b = rows
                .iter()
                .find(|r| r.file.path.ends_with("b.jpg"))
                .unwrap()
                .file
                .id;
            crate::indexer::write_items(&lib, rows).unwrap();
            b
        };
        let lib = open(d.path());
        assert_eq!(
            search_with(&lib, None, SearchSort::Newest),
            ["d.jpg", "a.jpg", "c.jpg", "b.jpg"]
        );
        lib.save_index_cache();
        let epoch = lib.db.lock().items_epoch().unwrap();
        let cap = |t| crate::media::metadata::Capture {
            taken_at: t,
            source: crate::media::metadata::DateSource::Exif,
            camera: None,
            duration: None,
        };
        let times =
            crate::indexer::apply_captures(&lib, &[(id_b, cap(day(2022, 12, 31)))]).unwrap();
        assert!(
            lib.db.lock().items_epoch().unwrap() > epoch,
            "the cache is stale now"
        );
        // The index is patched separately, once per date pass.
        assert_eq!(times, HashMap::from([(id_b, day(2022, 12, 31))]));
        assert_eq!(search_with(&lib, None, SearchSort::Newest)[3], "b.jpg");
        crate::indexer::patch_index_dates(&lib, &times);
        assert_eq!(
            search_with(&lib, None, SearchSort::Newest),
            ["b.jpg", "d.jpg", "a.jpg", "c.jpg"]
        );
        // The rewritten cache has the new date.
        lib.save_index_cache();
        drop(lib);
        let lib = open(d.path());
        assert!(lib.loaded_from_cache());
        assert_eq!(search_with(&lib, None, SearchSort::Newest)[0], "b.jpg");
        // A file without items leaves the epoch alone.
        let epoch = lib.db.lock().items_epoch().unwrap();
        let none = crate::indexer::apply_captures(&lib, &[(9999, cap(5))]).unwrap();
        assert!(none.is_empty());
        assert_eq!(lib.db.lock().items_epoch().unwrap(), epoch);
    }

    #[test]
    fn dates_of_files_without_items_never_touch_the_index() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        // Files are known (and undated) but nothing is embedded yet.
        let rows = seed_dated(&lib, &["a", "b", "c", "d"]);
        let cap = |t| crate::media::metadata::Capture {
            taken_at: t,
            source: crate::media::metadata::DateSource::Exif,
            camera: None,
            duration: None,
        };
        let caps: Vec<_> = rows
            .iter()
            .map(|r| (r.file.id, cap(day(2020, 1, 1))))
            .collect();
        // With a reader holding the index, any attempt to write to it would block for good.
        let guard = lib.index.read();
        let (tx, rx) = std::sync::mpsc::channel();
        let l = Arc::clone(&lib);
        std::thread::spawn(move || {
            let _ = tx.send(crate::indexer::apply_captures(&l, &caps).unwrap());
        });
        let times = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("apply_captures waited for the index");
        drop(guard);
        assert!(times.is_empty());
        crate::indexer::patch_index_dates(&lib, &times);
        assert_eq!(lib.indexed_items(), 0);
    }

    #[test]
    fn an_undated_file_has_the_same_date_everywhere() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        let rows = seed_dated(&lib, &["b"]);
        crate::indexer::write_items(&lib, rows).unwrap();
        let expected = crate::media::metadata::effective_taken(None, 1);
        let b = lib
            .timeline(&TimelineFilter::default())
            .unwrap()
            .into_iter()
            .find(|t| t.path.ends_with("b.jpg"))
            .unwrap();
        assert_eq!(b.taken_at, expected);
        let item = b.item_id.unwrap();
        assert_eq!(
            lib.db.lock().item_info(&[item]).unwrap()[0].taken_at,
            expected
        );
        assert_eq!(
            lib.db.lock().taken_times(&[b.file_id]).unwrap()[&b.file_id],
            expected
        );
        assert_eq!(
            lib.db
                .lock()
                .file_details(b.file_id)
                .unwrap()
                .unwrap()
                .taken_at,
            expected
        );
        let mut stored = Vec::new();
        lib.db
            .lock()
            .for_each_item(lib.model().id, |i| stored.push((i.file_id, i.taken_at)))
            .unwrap();
        assert!(stored.contains(&(b.file_id, expected)));
        let cands = lib.index.read().search(&query(), 10, None);
        assert_eq!(
            cands
                .iter()
                .find(|c| c.file_id == b.file_id)
                .unwrap()
                .taken_at,
            expected
        );
        let hits = search::by_vector(&lib, &query(), &SearchOptions::default(), None).unwrap();
        assert_eq!(
            hits.iter()
                .find(|h| h.file_id == b.file_id)
                .unwrap()
                .taken_at,
            expected
        );
        // Sorting by date uses that same value: b (1970) is the oldest.
        let opts = SearchOptions {
            sort: SearchSort::Oldest,
            ..Default::default()
        };
        let oldest = search::by_vector(&lib, &query(), &opts, None).unwrap();
        assert_eq!(names(&oldest)[0], "b.jpg");
    }

    #[test]
    fn a_file_removed_before_its_vectors_are_written_adds_none() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        let rows = seed_dated(&lib, &[]);
        let folder = lib.db.lock().folders().unwrap()[0].id;
        lib.db.lock().remove_folder(folder).unwrap();
        crate::indexer::write_items(&lib, rows).unwrap();
        assert_eq!(lib.indexed_items(), 0);
    }

    fn favorite_ids(lib: &Library) -> Vec<i64> {
        lib.timeline(&TimelineFilter {
            favorites_only: true,
            ..Default::default()
        })
        .unwrap()
        .iter()
        .map(|t| t.file_id)
        .collect()
    }

    #[test]
    fn rapid_favorite_toggles_end_as_the_last_one() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &[]);
        let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
        let v0 = lib.favorites_version();
        for fav in [true, false, true] {
            lib.queue_favorite(id, fav);
        }
        lib.wait_writes();
        assert_eq!(favorite_ids(&lib), [id]);
        assert!(lib.favorites_version() > v0);
        let results = lib.take_favorite_results();
        assert!(!results.is_empty());
        assert!(results.iter().all(|w| w.error.is_none() && w.file_id == id));
        assert!(results.last().unwrap().fav);
        // And off again.
        lib.queue_favorite(id, false);
        lib.wait_writes();
        assert!(favorite_ids(&lib).is_empty());
    }

    #[test]
    fn album_writes_keep_their_order_and_report_back() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &[]);
        let ids: Vec<i64> = lib
            .timeline(&TimelineFilter::default())
            .unwrap()
            .iter()
            .map(|t| t.file_id)
            .collect();
        let v0 = lib.albums_version();
        // Create-with-items and a duplicate name, in one burst with favorite marks between.
        let t1 = lib.queue_album(AlbumOp::Create {
            name: "Trip".into(),
            add: ids[..2].to_vec(),
        });
        lib.queue_favorite(ids[0], true);
        let t2 = lib.queue_album(AlbumOp::Create {
            name: "trip".into(),
            add: vec![],
        });
        lib.queue_favorite(ids[0], false);
        lib.wait_writes();
        let results = lib.take_album_results();
        assert_eq!(results.len(), 2);
        assert_eq!((results[0].ticket, results[1].ticket), (t1, t2));
        let album = results[0].album.unwrap();
        assert_eq!((results[0].changed, results[0].error.as_deref()), (2, None));
        assert!(
            results[1]
                .error
                .as_deref()
                .unwrap()
                .contains("already exists")
        );
        assert!(lib.albums_version() > v0);
        assert!(favorite_ids(&lib).is_empty(), "the later mark won");
        // Add, remove, rename, queued back to back: applied in that order.
        lib.queue_album(AlbumOp::Add {
            album,
            files: ids.clone(),
        });
        lib.queue_album(AlbumOp::Remove {
            album,
            files: ids[..1].to_vec(),
        });
        lib.queue_album(AlbumOp::Rename {
            id: album,
            name: "Holiday".into(),
        });
        lib.wait_writes();
        let r = lib.take_album_results();
        assert_eq!(
            r.iter().map(|w| w.changed).collect::<Vec<_>>(),
            [ids.len() - 2, 1, 0]
        );
        let info = lib.albums().unwrap();
        assert_eq!(
            (info[0].name.as_str(), info[0].count),
            ("Holiday", ids.len() - 1)
        );
        // Deleting the album leaves the files.
        lib.queue_album(AlbumOp::Delete { id: album });
        lib.wait_writes();
        assert!(lib.albums().unwrap().is_empty());
        assert_eq!(
            lib.timeline(&TimelineFilter::default()).unwrap().len(),
            ids.len()
        );
    }

    #[test]
    fn the_album_version_moves_only_when_something_changed() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &[]);
        let ids: Vec<i64> = lib
            .timeline(&TimelineFilter::default())
            .unwrap()
            .iter()
            .map(|t| t.file_id)
            .collect();
        lib.queue_album(AlbumOp::Create {
            name: "A".into(),
            add: ids[..1].to_vec(),
        });
        lib.wait_writes();
        let album = lib.take_album_results()[0].album.unwrap();
        let v = lib.albums_version();
        // Already in it, a missing file, removing what is not there, deleting what is gone.
        lib.queue_album(AlbumOp::Add {
            album,
            files: ids[..1].to_vec(),
        });
        lib.queue_album(AlbumOp::Remove {
            album,
            files: ids[1..2].to_vec(),
        });
        lib.queue_album(AlbumOp::Delete { id: album + 100 });
        lib.wait_writes();
        let r = lib.take_album_results();
        assert!(r.iter().all(|w| w.error.is_none()));
        assert_eq!(lib.albums_version(), v);
        // A real change moves it; so do rename and delete.
        lib.queue_album(AlbumOp::Add {
            album,
            files: ids[1..2].to_vec(),
        });
        lib.wait_writes();
        assert_eq!(lib.albums_version(), v + 1);
        lib.queue_album(AlbumOp::Rename {
            id: album,
            name: "B".into(),
        });
        lib.queue_album(AlbumOp::Delete { id: album });
        lib.wait_writes();
        assert_eq!(lib.albums_version(), v + 3);
        // The album is gone: adding to it is a plain message.
        lib.queue_album(AlbumOp::Add {
            album,
            files: ids[..1].to_vec(),
        });
        lib.wait_writes();
        let r = lib.take_album_results();
        let e = r.last().unwrap().error.as_deref().unwrap();
        assert_eq!(e, "That album no longer exists");
        assert_eq!(lib.albums_version(), v + 3);
    }

    #[test]
    fn a_failed_album_write_is_reported_and_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &[]);
        let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
        let album = lib.create_album("Keep", &[id]).unwrap();
        let v = lib.albums_version();
        lib.fail_albums.store(true, Ordering::Release);
        lib.queue_album(AlbumOp::Remove {
            album,
            files: vec![id],
        });
        lib.queue_album(AlbumOp::Delete { id: album });
        lib.queue_album(AlbumOp::Create {
            name: "New".into(),
            add: vec![id],
        });
        lib.wait_writes();
        let r = lib.take_album_results();
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|w| w.error.is_some() && w.changed == 0));
        assert_eq!(lib.albums_version(), v);
        let info = lib.albums().unwrap();
        assert_eq!((info.len(), info[0].count), (1, 1));
    }

    #[test]
    fn writes_queued_right_before_exit_are_committed() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &[]);
        let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
        lib.queue_album(AlbumOp::Create {
            name: "Last".into(),
            add: vec![id],
        });
        lib.queue_favorite(id, true);
        assert!(lib.wait_writes_for(std::time::Duration::from_secs(3)));
        assert_eq!(lib.albums().unwrap()[0].count, 1);
        assert_eq!(favorite_ids(&lib), [id]);
    }

    #[test]
    fn a_failed_favorite_write_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &[]);
        let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
        let v0 = lib.favorites_version();
        lib.fail_favorites.store(true, Ordering::Release);
        lib.queue_favorite(id, true);
        lib.wait_writes();
        let results = lib.take_favorite_results();
        assert_eq!(results.len(), 1);
        assert_eq!((results[0].file_id, results[0].fav), (id, true));
        assert!(results[0].error.is_some());
        assert!(favorite_ids(&lib).is_empty());
        assert_eq!(lib.favorites_version(), v0, "nothing was committed");
    }

    #[test]
    fn years_cover_the_whole_library() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        seed_dated(&lib, &["b"]);
        // b is undated: its file time (1970) counts.
        assert_eq!(lib.years().unwrap(), [2021, 2020, 2019, 1970]);
    }

    #[test]
    fn favorites_are_stored_and_found_in_search_hits_and_the_timeline() {
        let d = tempfile::tempdir().unwrap();
        let lib = open(d.path());
        let rows = seed_dated(&lib, &[]);
        crate::indexer::write_items(&lib, rows).unwrap();
        let tl = |fav| {
            lib.timeline(&TimelineFilter {
                favorites_only: fav,
                ..Default::default()
            })
            .unwrap()
        };
        assert!(tl(true).is_empty());
        let b = tl(false)
            .into_iter()
            .find(|t| t.path.ends_with("b.jpg"))
            .unwrap();
        lib.set_favorite(&[b.file_id], true).unwrap();
        assert_eq!(names_of(&tl(true)), ["b.jpg"]);
        let hits = search::by_vector(&lib, &query(), &SearchOptions::default(), None).unwrap();
        assert_eq!(
            hits.iter()
                .filter(|h| h.favorite)
                .map(|h| h.file_id)
                .collect::<Vec<_>>(),
            [b.file_id]
        );
        lib.set_favorite(&[b.file_id], false).unwrap();
        assert!(tl(true).is_empty());
    }

    fn names_of(items: &[TimelineItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }
}
