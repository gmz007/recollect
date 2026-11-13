//! `Library`: the thread-safe facade the GUI and CLI talk to. It owns the database, the vector
//! index, settings and the background worker (model download / indexing).

use crate::config::{Paths, Settings};
use crate::indexer;
use crate::model::{Device, ModelSpec, TextEncoder, download, registry};
use crate::search::{self, SearchHit, SearchOptions};
use crate::store::db::{Folder, Stats};
use crate::store::{Db, VectorIndex};
use anyhow::{Context, Result, anyhow, bail};
use parking_lot::{Condvar, Mutex, RwLock};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    LoadingModel,
    Indexing(IndexProgress),
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
    pub seconds: f32,
    pub cancelled: bool,
    pub device: Option<Device>,
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
    text: Mutex<Option<TextEncoder>>,
    notifier: RwLock<Option<Notifier>>,
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
        let mut settings = Settings::load(&paths.settings_path())?;
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
        let index = load_index(&db, spec)?;
        Ok(Arc::new(Self {
            paths,
            settings: RwLock::new(settings),
            model: RwLock::new(spec),
            db: Mutex::new(db),
            index: RwLock::new(index),
            status: Mutex::new(Status::default()),
            control: Control::default(),
            worker: Mutex::new(None),
            text: Mutex::new(None),
            notifier: RwLock::new(None),
        }))
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
        self.worker
            .lock()
            .as_ref()
            .is_some_and(|h| !h.is_finished())
    }

    pub fn stats(&self) -> Result<Stats> {
        self.db.lock().stats()
    }

    pub fn errors(&self, limit: usize) -> Result<Vec<(PathBuf, String)>> {
        self.db.lock().errors(limit)
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
        self.notify();
        Ok(())
    }

    /// Removes a folder and its index entries. Media files are never touched.
    pub fn remove_folder(&self, folder_id: i64) -> Result<()> {
        let removed = self.db.lock().remove_folder(folder_id)?;
        self.index
            .write()
            .remove_files(&removed.into_iter().collect());
        self.notify();
        Ok(())
    }

    pub(crate) fn remove_files_from_index(&self, ids: &HashSet<i64>) {
        self.index.write().remove_files(ids);
    }

    pub fn set_settings(&self, new: Settings) -> Result<()> {
        new.save(&self.paths.settings_path())?;
        let model_changed = new.model != self.settings.read().model;
        *self.settings.write() = new.clone();
        if model_changed && let Some(id) = &new.model {
            self.switch_model(id)?;
        }
        self.notify();
        Ok(())
    }

    fn switch_model(&self, id: &str) -> Result<()> {
        if self.is_busy() {
            bail!("wait for the current job to finish before switching models");
        }
        let spec = registry::get(id).ok_or_else(|| anyhow!("unknown model '{id}'"))?;
        let index = load_index(&self.db.lock(), spec)?;
        *self.model.write() = spec;
        *self.index.write() = index;
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
                if let Err(e) = res {
                    s.last_error = Some(format!("Model download failed: {e:#}"));
                }
            });
        })
    }

    /// Starts a scan + indexing run in the background.
    pub fn start_indexing(self: &Arc<Self>) -> Result<()> {
        if !self.model_installed() {
            bail!("the model is not downloaded yet");
        }
        let lib = Arc::clone(self);
        self.spawn(move || {
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
        if w.as_ref().is_some_and(|h| !h.is_finished()) {
            bail!("another job is already running");
        }
        if let Some(h) = w.take() {
            let _ = h.join();
        }
        self.control.reset();
        self.update_status(|s| s.last_error = None);
        *w = Some(
            std::thread::Builder::new()
                .name("recollect-worker".into())
                .spawn(f)?,
        );
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
        if let Some(h) = self.worker.lock().take() {
            let _ = h.join();
        }
    }

    /// Runs indexing on the calling thread (CLI).
    pub fn index_blocking(&self) -> Result<RunSummary> {
        if !self.model_installed() {
            bail!("the model is not downloaded yet (run `recollect models download`)");
        }
        self.control.reset();
        let r = indexer::run(self);
        self.update_status(|s| s.activity = Activity::Idle);
        r
    }

    /// Text → vector with the current model (loads the text tower on first use).
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let mut guard = self.text.lock();
        if guard.is_none() {
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
            let v = db
                .embeddings(&[item_id])?
                .remove(&item_id)
                .ok_or_else(|| anyhow!("item {item_id} not found"))?;
            let info = db.item_info(&[item_id])?;
            (v, info.first().map(|i| i.file_id))
        };
        search::by_vector(self, &v, opts, file_id)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.cancel();
        if let Some(h) = self.worker.get_mut().take() {
            let _ = h.join();
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
