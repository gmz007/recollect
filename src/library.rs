//! `Library`: the thread-safe facade the GUI and CLI talk to. It owns the database, the vector
//! index, settings and the background worker (model download / indexing).

use crate::config::{Paths, Settings};
use crate::indexer;
use crate::model::{Device, ModelSpec, TextEncoder, download, registry};
use crate::search::{self, SearchHit, SearchOptions};
use crate::store::db::{DateCounts, Folder, Stats, TimelineFilter, TimelineItem};
use crate::store::{Db, VectorIndex};
use anyhow::{Context, Result, anyhow, bail};
use parking_lot::{Condvar, Mutex, RwLock};
use std::collections::HashSet;
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub activity: Activity,
    pub paused: bool,
    pub last_error: Option<String>,
    pub last_run: Option<RunSummary>,
}

/// Pause / cancel signalling shared with worker threads.
#[derive(Default)]
pub struct Control {
    cancel: AtomicBool,
    paused: Mutex<bool>,
    cv: Condvar,
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

pub struct Library {
    paths: Paths,
    settings: RwLock<Settings>,
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
}

/// Picks the default model by hardware: a discrete GPU gets the quality model.
pub fn default_model_for_hardware() -> &'static str {
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
            None => match &settings.model {
                Some(id) => id.clone(),
                None => {
                    let id = default_model_for_hardware().to_string();
                    settings.model = Some(id.clone());
                    settings.save(&paths.settings_path())?;
                    id
                }
            },
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
        let lib = Arc::new(Self {
            paths,
            settings: RwLock::new(settings),
            model: RwLock::new(spec),
            db: Mutex::new(db),
            index: RwLock::new(index),
            status: Mutex::new(Status::default()),
            control: Control::default(),
            worker: Mutex::new(None),
            busy: Arc::new(AtomicBool::new(false)),
            jobs_finished: Arc::new(AtomicU64::new(0)),
            rescan_requested: AtomicBool::new(false),
            self_ref: RwLock::new(std::sync::Weak::new()),
            text: Mutex::new(None),
            notifier: RwLock::new(None),
            save_lock: Mutex::new(()),
            saved: Mutex::new(from_cache.then_some((spec.id, epoch))),
            from_cache,
        });
        *lib.self_ref.write() = Arc::downgrade(&lib);
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
        self.db.lock().timeline(filter)
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

    /// Adds a folder off the calling thread (canonicalizing a dead network path or waiting for
    /// the database must not freeze the UI), then makes sure it gets indexed.
    pub fn start_add_folder(self: &Arc<Self>, path: PathBuf) {
        let lib = Arc::clone(self);
        std::thread::spawn(move || {
            match lib.add_folder(&path) {
                Ok(()) => {
                    // If a job is running it picks the folder up (rescan_requested).
                    // Without a model the run still reads capture dates for the album.
                    if !lib.is_busy() {
                        let _ = lib.start_indexing();
                    }
                }
                Err(e) => lib.update_status(|s| s.last_error = Some(format!("{e:#}"))),
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
        let lib = Arc::clone(self);
        std::thread::spawn(move || {
            if let Err(e) = lib.remove_folder(folder_id) {
                lib.update_status(|s| {
                    s.last_error = Some(format!("Removing the folder failed: {e:#}"))
                });
            }
            lib.jobs_finished.fetch_add(1, Ordering::AcqRel);
            lib.notify();
            lib.save_index_cache();
        });
    }

    pub(crate) fn remove_files_from_index(&self, ids: &HashSet<i64>) {
        self.index.write().remove_files(ids);
    }

    /// Applies new settings. A model change runs as a background job (it rebuilds the vector
    /// index); nothing is persisted unless the change is applied.
    pub fn apply_settings(self: &Arc<Self>, new: Settings) -> Result<()> {
        let target = new.model.as_deref().unwrap_or(self.model().id);
        let spec = registry::get(target).ok_or_else(|| anyhow!("unknown model '{target}'"))?;
        if spec.id == self.model().id {
            new.save(&self.paths.settings_path())?;
            *self.settings.write() = new;
            self.notify();
            return Ok(());
        }
        let lib = Arc::clone(self);
        self.spawn(move || {
            lib.update_status(|s| s.activity = Activity::SwitchingModel);
            let res = lib.switch_model(spec, new);
            lib.update_status(|s| {
                s.activity = Activity::Idle;
                s.last_run = None;
                if let Err(e) = res {
                    s.last_error = Some(format!("Switching models failed: {e:#}"));
                }
            });
        })
    }

    fn switch_model(&self, spec: &'static ModelSpec, new: Settings) -> Result<()> {
        // Index and model change together under the db lock (order db → index), so a cache save
        // never sees one of them without the other.
        let db = self.db.lock();
        let index = load_index(&db, spec)?;
        new.save(&self.paths.settings_path())?;
        *self.settings.write() = new;
        *self.index.write() = index;
        *self.model.write() = spec;
        drop(db);
        *self.text.lock() = None;
        Ok(())
    }

    /// Starts downloading the current model in the background (the caller has obtained the
    /// user's consent).
    pub fn start_download(self: &Arc<Self>) -> Result<()> {
        let lib = Arc::clone(self);
        self.spawn(move || {
            let spec = lib.model();
            let res = download::download(
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
                    })
                },
            );
            lib.update_status(|s| {
                s.activity = Activity::Idle;
                if let Err(e) = res
                    && !lib.control.is_cancelled()
                {
                    s.last_error = Some(format!("Model download failed: {e:#}"));
                }
            });
        })
    }

    /// Starts a scan + indexing run in the background. Without an installed model it only
    /// scans and reads capture dates (the album works before any model is downloaded).
    pub fn start_indexing(self: &Arc<Self>) -> Result<()> {
        let lib = Arc::clone(self);
        self.spawn(move || {
            lib.update_status(|s| s.last_run = None);
            let res = indexer::run(&lib);
            lib.update_status(|s| {
                s.activity = Activity::Idle;
                s.paused = false;
                match res {
                    Ok(sum) => s.last_run = Some(sum),
                    Err(e) => s.last_error = Some(format!("Indexing failed: {e:#}")),
                }
            });
        })
    }

    fn spawn(&self, f: impl FnOnce() + Send + 'static) -> Result<()> {
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
        let mut guard = self.text.lock();
        // Reload if missing or if the model changed since it was loaded.
        if guard
            .as_ref()
            .is_none_or(|t| t.spec().id != self.model().id)
        {
            if !self.model_installed() {
                bail!("the model is not downloaded yet");
            }
            let threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8);
            *guard = Some(TextEncoder::load(self.model(), &self.model_dir(), threads)?);
        }
        guard
            .as_mut()
            .map(|t| t.embed(text))
            .unwrap_or_else(|| Err(anyhow!("text model unavailable")))
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
            let (file_id, model) = db
                .item_owner(item_id)?
                .ok_or_else(|| anyhow!("that item is no longer in the library"))?;
            if model.as_deref() != Some(self.model().id) {
                bail!("that item was indexed with another model; wait for re-indexing to finish");
            }
            let v = db
                .embeddings(&[item_id])?
                .remove(&item_id)
                .ok_or_else(|| anyhow!("item {item_id} not found"))?;
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
            idx.add(it.id, it.file_id, it.kind, &it.embedding);
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
    use crate::media::{MediaKind, ScannedFile};
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

        let new = Settings {
            model: Some(registry::SIGLIP_B16.id.into()),
            ..lib.settings()
        };
        lib.apply_settings(new).unwrap();
        lib.wait();
        assert_eq!(lib.status().last_error, None);
        assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
        assert_eq!(lib.indexed_items(), 0);
        let saved = Settings::load(&lib.paths().settings_path()).unwrap();
        assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
        // The old 512-d vector must produce an error, not a panic.
        let err = lib.similar(item, &SearchOptions::default()).unwrap_err();
        assert!(err.to_string().contains("another model"), "{err}");
        assert!(search::by_vector(&lib, &[0.0; 512], &SearchOptions::default(), None).is_err());
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
                            t_start: Some(i as f64 * 2.0),
                            t_end: Some(i as f64 * 2.0 + 2.0),
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
        assert!(!crate::indexer::file_failed(&lib, &row, &err).unwrap());
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
}
