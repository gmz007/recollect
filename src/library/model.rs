//! The smart-search choice: picking a default, switching (which rebuilds the vector index)
//! and downloading.

use super::index_cache::load_index;
use super::{Activity, ErrorKind, Library};
use crate::model::{ModelSpec, download, registry};
use anyhow::{Result, anyhow};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

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

    pub(super) fn download_job(lib: Arc<Self>) -> impl FnOnce() + Send + 'static {
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
}
