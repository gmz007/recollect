//! The vector index in memory and its on-disk cache, which lets a start skip decoding every
//! stored vector.

use super::Library;
use crate::config::Paths;
use crate::model::ModelSpec;
use crate::store::{Db, VectorIndex};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Instant;

impl Library {
    /// True if startup reused the on-disk index cache instead of rebuilding from the database.
    pub fn loaded_from_cache(&self) -> bool {
        self.from_cache
    }

    /// Writes the in-memory index to disk so the next start can skip decoding every vector.
    /// Does nothing if a save is already running, the cache is already current, or a job is
    /// running: the save holds the index read lock for the whole write, which would stall the
    /// job's next index write and, behind it, searches. A running job saves when it ends.
    pub fn save_index_cache(&self) {
        if self.is_busy() {
            return;
        }
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
        // so writers (new items, date patches) wait until it is on disk, and
        // searches queue behind a waiting writer. That is why a save never starts while a job
        // runs; the write is also skipped when the cache is already current.
        match index.save(&path, spec.id, &library_id, epoch) {
            Ok(()) => *self.saved.lock() = Some((spec.id, epoch)),
            Err(e) => tracing::warn!("could not write index cache {}: {e}", path.display()),
        }
    }
}

pub(super) fn index_cache_path(paths: &Paths, spec: &ModelSpec) -> PathBuf {
    paths.data.join(format!("index-{}.bin", spec.id))
}

/// Removes temp files a crashed index-cache save left behind.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(600);

pub(super) fn remove_stale_temp_files(dir: &Path) {
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

pub(super) fn load_index(db: &Db, spec: &ModelSpec) -> Result<VectorIndex> {
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
