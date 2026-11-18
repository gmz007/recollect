//! Library folders and the files found in them: syncing with a scan, capture dates,
//! favorite marks and library counts.

use super::{
    Db, FileStatus, bump_epoch, count_of, now, path_str, signed, typed, typed_or_zero, unsigned,
};
use crate::media::metadata::{Capture, DateSource, effective_taken};
use crate::media::{MediaKind, ScannedFile};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub id: i64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub id: i64,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub size: u64,
    pub mtime: i64,
    pub status: FileStatus,
}

/// Facts about one file that list views do not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDetails {
    pub path: PathBuf,
    pub taken_at: i64,
    pub date_source: DateSource,
    pub camera: Option<String>,
    pub favorite: bool,
    /// Size on disk; the database does not know it, `Library::file_details` fills it in.
    pub size: Option<u64>,
    /// The albums the file is in (ids, in no particular order).
    pub album_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncStats {
    pub added: usize,
    pub changed: usize,
    pub unchanged: usize,
    /// Ids of files that disappeared (their items must leave the vector index).
    pub removed: Vec<i64>,
    /// Ids of files whose content changed; their old items were deleted.
    pub changed_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub folders: usize,
    pub images: usize,
    pub videos: usize,
    pub indexed: usize,
    pub pending: usize,
    pub errors: usize,
    pub items: usize,
}

pub(super) fn file_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        path: PathBuf::from(r.get::<_, String>(1)?),
        kind: typed(r, 2)?,
        size: unsigned(r.get::<_, i64>(3)?),
        mtime: r.get(4)?,
        status: typed(r, 5)?,
    })
}

/// The given files that have at least one item.
fn embedded_files(conn: &Connection, files: impl Iterator<Item = i64>) -> Result<Vec<i64>> {
    let mut st = conn.prepare_cached("SELECT 1 FROM items WHERE file_id = ?1 LIMIT 1")?;
    let mut out = Vec::new();
    for f in files {
        if st.exists([f])? {
            out.push(f);
        }
    }
    Ok(out)
}

/// All library folders, ordered by path, read through `conn` (which may be a transaction).
fn folders_in(conn: &rusqlite::Connection) -> Result<Vec<Folder>> {
    let mut st = conn.prepare("SELECT id, path FROM folders ORDER BY path")?;
    let rows = st.query_map([], |r| {
        Ok(Folder {
            id: r.get(0)?,
            path: PathBuf::from(r.get::<_, String>(1)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// What the database holds for a folder: path to (file id, size, mtime).
type ExistingFiles = HashMap<String, (i64, u64, i64)>;

fn existing_files(tx: &rusqlite::Transaction<'_>, folder_id: i64) -> Result<ExistingFiles> {
    let mut existing = ExistingFiles::new();
    let mut st = tx.prepare("SELECT path, id, size, mtime FROM files WHERE folder_id = ?1")?;
    let rows = st.query_map([folder_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            (r.get(1)?, unsigned(r.get::<_, i64>(2)?), r.get(3)?),
        ))
    })?;
    for row in rows {
        let (p, v) = row?;
        existing.insert(p, v);
    }
    Ok(existing)
}

/// Inserts the new files and resets the changed ones (dropping their items). Returns the counts
/// and the set of scanned paths.
fn apply_scan(
    tx: &rusqlite::Transaction<'_>,
    folder_id: i64,
    scanned: &[ScannedFile],
    existing: &ExistingFiles,
) -> Result<(SyncStats, HashSet<String>)> {
    let mut stats = SyncStats::default();
    let mut seen = HashSet::new();
    let mut ins = tx.prepare(
        "INSERT INTO files(folder_id, path, kind, size, mtime, status, added_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)
         ON CONFLICT(path) DO NOTHING",
    )?;
    let mut upd = tx.prepare(
        "UPDATE files SET size = ?2, mtime = ?3, status = 0, error = NULL, model = NULL,
         taken_at = NULL, date_source = NULL, camera = NULL WHERE id = ?1",
    )?;
    let mut del_items = tx.prepare("DELETE FROM items WHERE file_id = ?1")?;
    let added_at = now();
    for f in scanned {
        let p = path_str(&f.path)?;
        seen.insert(p.to_string());
        match existing.get(p) {
            None => {
                stats.added += ins.execute(params![
                    folder_id,
                    p,
                    i64::from(f.kind),
                    signed(f.size),
                    f.mtime,
                    added_at
                ])?;
            }
            Some(&(id, size, mtime)) if size != f.size || mtime != f.mtime => {
                upd.execute(params![id, signed(f.size), f.mtime])?;
                del_items.execute([id])?;
                stats.changed += 1;
                stats.changed_ids.push(id);
            }
            Some(_) => stats.unchanged += 1,
        }
    }
    Ok((stats, seen))
}

/// Deletes the files that were not seen in the scan, except those under `protected` paths.
fn delete_vanished(
    tx: &rusqlite::Transaction<'_>,
    existing: &ExistingFiles,
    seen: &HashSet<String>,
    protected: &[PathBuf],
    stats: &mut SyncStats,
) -> Result<()> {
    let mut del = tx.prepare("DELETE FROM files WHERE id = ?1")?;
    for (p, &(id, _, _)) in existing {
        if !seen.contains(p) && !protected.iter().any(|d| Path::new(p).starts_with(d)) {
            del.execute([id])?;
            stats.removed.push(id);
        }
    }
    Ok(())
}

impl Db {
    /// Adds a library folder. `path` should be absolute and canonical. A folder inside an
    /// existing one is rejected; adding a parent of existing folders absorbs them.
    pub fn add_folder(&mut self, path: &Path) -> Result<i64> {
        // Immediate: read the existing folders and change them under one write lock, so a
        // concurrent writer cannot add an overlapping folder in between.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing = folders_in(&tx)?;
        if let Some(f) = existing.iter().find(|f| path.starts_with(&f.path)) {
            if f.path == path {
                return Ok(f.id);
            }
            bail!(
                "{} is already part of the library (inside {})",
                path.display(),
                f.path.display()
            );
        }
        tx.execute(
            "INSERT INTO folders(path, added_at) VALUES (?1, ?2)",
            params![path_str(path)?, now()],
        )?;
        let id = tx.last_insert_rowid();
        for child in existing.iter().filter(|f| f.path.starts_with(path)) {
            tx.execute(
                "UPDATE files SET folder_id = ?1 WHERE folder_id = ?2",
                params![id, child.id],
            )?;
            tx.execute("DELETE FROM folders WHERE id = ?1", [child.id])?;
        }
        // No epoch bump: adding a folder (even one absorbing others) moves no items.
        tx.commit()?;
        Ok(id)
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        folders_in(&self.conn)
    }

    /// Removes a folder and everything indexed under it; returns the removed file ids.
    pub fn remove_folder(&mut self, folder_id: i64) -> Result<Vec<i64>> {
        // Immediate: this reads and then writes; a deferred transaction fails at once when
        // another process wrote in between, because the busy timeout does not apply there.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids: Vec<i64> = {
            let mut st = tx.prepare("SELECT id FROM files WHERE folder_id = ?1")?;
            st.query_map([folder_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        tx.execute("DELETE FROM files WHERE folder_id = ?1", [folder_id])?;
        tx.execute("DELETE FROM folders WHERE id = ?1", [folder_id])?;
        bump_epoch(&tx)?;
        tx.commit()?;
        Ok(ids)
    }

    /// Reconciles the files table with a fresh scan of one folder: inserts new files, marks
    /// changed ones (size or mtime differ) pending, deletes vanished ones.
    ///
    /// Files under `protected` paths (directories that could not be read during the scan) are
    /// kept even if they were not seen, so a temporary I/O error doesn't drop them.
    pub fn sync_folder(
        &mut self,
        folder_id: i64,
        scanned: &[ScannedFile],
        protected: &[PathBuf],
    ) -> Result<SyncStats> {
        // Immediate for the same reason as in `remove_folder`.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // The folder may have been removed (or absorbed by a parent) since the scan started.
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?1)",
            [folder_id],
            |r| r.get(0),
        )?;
        if !present {
            return Ok(SyncStats::default());
        }
        let existing = existing_files(&tx, folder_id)?;
        let (mut stats, seen) = apply_scan(&tx, folder_id, scanned, &existing)?;
        delete_vanished(&tx, &existing, &seen, protected, &mut stats)?;
        // A scan that found nothing new must not invalidate the index cache.
        if stats.added + stats.changed + stats.removed.len() > 0 {
            bump_epoch(&tx)?;
        }
        tx.commit()?;
        Ok(stats)
    }

    /// Files whose capture time has not been read yet.
    pub fn pending_dates(&self) -> Result<Vec<FileRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, path, kind, size, mtime, status FROM files WHERE taken_at IS NULL ORDER BY id",
        )?;
        let rows = st.query_map([], file_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Stores capture times in one transaction. Files that vanished meanwhile are skipped.
    /// Returns the ids of the files that already have searchable items: only those carry a
    /// capture time in the search index, which the caller patches in memory (once per date
    /// pass, see `indexer::patch_index_dates`). Their changed rows also move the epoch so the
    /// on-disk cache is rewritten rather than reused with old dates.
    pub fn set_captures(&mut self, batch: &[(i64, Capture)]) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        {
            let mut upd = tx.prepare(
                "UPDATE files SET taken_at = ?2, date_source = ?3, camera = ?4,
                 duration = COALESCE(duration, ?5) WHERE id = ?1",
            )?;
            for (id, c) in batch {
                upd.execute(params![
                    id,
                    c.taken_at,
                    i64::from(c.source),
                    c.camera,
                    c.duration
                ])?;
            }
        }
        let embedded = embedded_files(&tx, batch.iter().map(|b| b.0))?;
        if !embedded.is_empty() {
            bump_epoch(&tx)?;
        }
        tx.commit()?;
        Ok(embedded)
    }

    /// Capture times of files as the timeline shows them: the stored date, or the modification
    /// time converted to local wall-clock when the date is not known yet.
    pub fn taken_times(&self, file_ids: &[i64]) -> Result<HashMap<i64, i64>> {
        let rows = self.by_ids(
            "SELECT id, taken_at, mtime FROM files WHERE id IN (#)",
            file_ids,
            |r| {
                let taken: Option<i64> = r.get(1)?;
                let mtime: i64 = r.get(2)?;
                Ok((r.get::<_, i64>(0)?, effective_taken(taken, mtime)))
            },
        )?;
        Ok(rows.into_iter().collect())
    }

    /// Marks files as favorites (or not).
    pub fn set_favorite(&mut self, file_ids: &[i64], fav: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upd = tx.prepare("UPDATE files SET favorite = ?2 WHERE id = ?1")?;
            for id in file_ids {
                upd.execute(params![id, i64::from(fav)])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Date source, camera, favorite flag and albums of one file.
    pub fn file_details(&self, file_id: i64) -> Result<Option<FileDetails>> {
        let found = self
            .conn
            .query_row(
                "SELECT taken_at, mtime, date_source, camera, favorite, path FROM files WHERE id = ?1",
                [file_id],
                |r| {
                    let taken: Option<i64> = r.get(0)?;
                    let mtime: i64 = r.get(1)?;
                    Ok(FileDetails {
                        path: PathBuf::from(r.get::<_, String>(5)?),
                        taken_at: effective_taken(taken, mtime),
                        date_source: typed_or_zero(r, 2)?,
                        camera: r.get(3)?,
                        favorite: r.get::<_, i64>(4)? != 0,
                        size: None,
                        album_ids: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut details) = found else {
            return Ok(None);
        };
        let mut st = self
            .conn
            .prepare("SELECT album_id FROM album_items WHERE file_id = ?1")?;
        details.album_ids = st
            .query_map([file_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(details))
    }

    /// Library counts; `indexed`/`pending` are relative to `model`.
    pub fn stats(&self, model: &str) -> Result<Stats> {
        let folders: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM folders", [], |r| r.get(0))?;
        let (images, videos, indexed, pending, errors): (i64, i64, i64, i64, i64) =
            self.conn.query_row(
                "SELECT COALESCE(SUM(kind = 0), 0), COALESCE(SUM(kind = 1), 0),
                    COALESCE(SUM(status = 1 AND model = ?1), 0),
                    COALESCE(SUM(status = 0 OR (status = 1 AND (model IS NULL OR model != ?1))), 0),
                    COALESCE(SUM(status = 2), 0)
             FROM files",
                [model],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
        let items: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items JOIN files ON files.id = items.file_id WHERE files.model = ?1",
            [model],
            |r| r.get(0),
        )?;
        Ok(Stats {
            folders: count_of(folders),
            images: count_of(images),
            videos: count_of(videos),
            indexed: count_of(indexed),
            pending: count_of(pending),
            errors: count_of(errors),
            items: count_of(items),
        })
    }

    pub fn folder_file_count(&self, folder_id: i64) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?;
        Ok(count_of(n))
    }

    /// Makes every failed file pending again (e.g. after fixing a drive or updating Recollect).
    pub fn retry_errors(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE files SET status = 0, error = NULL WHERE status = 2",
            [],
        )?)
    }

    /// Most recent indexing errors: (path, message).
    pub fn errors(&self, limit: usize) -> Result<Vec<(PathBuf, String)>> {
        let mut st = self.conn.prepare(
            "SELECT path, COALESCE(error, '') FROM files WHERE status = 2 ORDER BY indexed_at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([signed(limit)], |r| {
            Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
