//! Searchable items: the vectors of each indexed file (one per image, several per video).

use super::files::file_row;
use super::{Db, FileRow, bump_epoch, count_of, now, typed};
use crate::media::MediaKind;
use crate::media::metadata::effective_taken;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;
use std::path::PathBuf;

/// Facts learned about a file while indexing it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FileMeta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
}

/// One searchable unit: a whole image, or a time range of a video.
#[derive(Debug, Clone, PartialEq)]
pub struct NewItem {
    pub t_start: Option<f64>,
    pub t_end: Option<f64>,
    pub embedding: Vec<f32>,
}

/// An indexed item as loaded at startup.
#[derive(Debug, Clone)]
pub struct StoredItem {
    pub id: i64,
    pub file_id: i64,
    pub kind: MediaKind,
    /// Capture time of the file as the timeline shows it (see [`Db::taken_times`]).
    pub taken_at: i64,
    pub embedding: Vec<f32>,
}

/// Display information for a search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    pub item_id: i64,
    pub file_id: i64,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub mtime: i64,
    pub t_start: Option<f64>,
    pub t_end: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub favorite: bool,
    /// Local wall-clock capture time; the file time if unknown.
    pub taken_at: i64,
}

pub fn emb_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn blob_to_emb(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

impl Db {
    /// Files that need (re-)embedding with `model`: never indexed, changed, or indexed with
    /// another model. Images first, then videos (cheap wins first).
    pub fn pending(&self, model: &str) -> Result<Vec<FileRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, path, kind, size, mtime, status FROM files
             WHERE status = 0 OR (status = 1 AND (model IS NULL OR model != ?1))
             ORDER BY kind, path",
        )?;
        let rows = st.query_map([model], file_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Stores the items of several files in one transaction, replacing earlier items, and
    /// marks the files indexed. Returns the new item ids per file (same order as input).
    pub fn put_items(
        &mut self,
        model: &str,
        batch: &[(i64, FileMeta, Vec<NewItem>)],
    ) -> Result<Vec<Vec<i64>>> {
        let tx = self.conn.transaction()?;
        let mut all = Vec::with_capacity(batch.len());
        {
            let mut del = tx.prepare("DELETE FROM items WHERE file_id = ?1")?;
            let mut ins = tx.prepare(
                "INSERT INTO items(file_id, t_start, t_end, embedding) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut upd = tx.prepare(
                "UPDATE files SET status = 1, model = ?2, error = NULL, width = ?3, height = ?4, duration = ?5,
                 indexed_at = ?6 WHERE id = ?1",
            )?;
            let ts = now();
            for (file_id, meta, items) in batch {
                let changed = upd.execute(params![
                    file_id,
                    model,
                    meta.width,
                    meta.height,
                    meta.duration,
                    ts
                ])?;
                if changed == 0 {
                    // The file was removed meanwhile (e.g. its folder was removed).
                    all.push(Vec::new());
                    continue;
                }
                del.execute([file_id])?;
                let mut ids = Vec::with_capacity(items.len());
                for it in items {
                    ins.execute(params![
                        file_id,
                        it.t_start,
                        it.t_end,
                        emb_to_blob(&it.embedding)
                    ])?;
                    ids.push(tx.last_insert_rowid());
                }
                all.push(ids);
            }
        }
        bump_epoch(&tx)?;
        tx.commit()?;
        Ok(all)
    }

    pub fn set_error(&self, file_id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET status = 2, error = ?2, indexed_at = ?3 WHERE id = ?1",
            params![file_id, error, now()],
        )?;
        if self
            .conn
            .execute("DELETE FROM items WHERE file_id = ?1", [file_id])?
            > 0
        {
            bump_epoch(&self.conn)?;
        }
        Ok(())
    }

    /// Number of items embedded with `model`.
    pub fn item_count(&self, model: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items JOIN files ON files.id = items.file_id WHERE files.status = 1 AND files.model = ?1",
            [model],
            |r| r.get(0),
        )?;
        Ok(count_of(n))
    }

    /// Changes whenever the set of stored items may have changed (guards the index cache).
    pub fn items_epoch(&self) -> Result<u64> {
        Ok(self
            .meta("items_epoch")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    /// Streams all items embedded with `model` (for building the in-memory index).
    pub fn for_each_item(&self, model: &str, mut f: impl FnMut(StoredItem)) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT items.id, items.file_id, files.kind, items.embedding, files.taken_at, files.mtime FROM items
             JOIN files ON files.id = items.file_id
             WHERE files.status = 1 AND files.model = ?1 ORDER BY items.id",
        )?;
        let mut rows = st.query([model])?;
        while let Some(r) = rows.next()? {
            f(StoredItem {
                id: r.get(0)?,
                file_id: r.get(1)?,
                kind: typed(r, 2)?,
                taken_at: effective_taken(r.get(4)?, r.get(5)?),
                embedding: blob_to_emb(r.get_ref(3)?.as_blob()?),
            });
        }
        Ok(())
    }

    pub fn embeddings(&self, item_ids: &[i64]) -> Result<HashMap<i64, Vec<f32>>> {
        let rows = self.by_ids(
            "SELECT id, embedding FROM items WHERE id IN (#)",
            item_ids,
            |r| Ok((r.get::<_, i64>(0)?, blob_to_emb(r.get_ref(1)?.as_blob()?))),
        )?;
        Ok(rows.into_iter().collect())
    }

    /// Display information for items, in the order of `item_ids`. Missing items are skipped
    /// and an id listed twice yields one row, at its first position.
    pub fn item_info(&self, item_ids: &[i64]) -> Result<Vec<ItemInfo>> {
        let rows = self.by_ids(
            "SELECT items.id, items.file_id, files.path, files.kind, files.mtime, items.t_start, items.t_end,
                    files.width, files.height, files.duration, files.favorite, files.taken_at
             FROM items JOIN files ON files.id = items.file_id WHERE items.id IN (#)",
            item_ids,
            |r| {
                Ok(ItemInfo {
                    item_id: r.get(0)?,
                    file_id: r.get(1)?,
                    path: PathBuf::from(r.get::<_, String>(2)?),
                    kind: typed(r, 3)?,
                    mtime: r.get(4)?,
                    t_start: r.get(5)?,
                    t_end: r.get(6)?,
                    width: r.get(7)?,
                    height: r.get(8)?,
                    duration: r.get(9)?,
                    favorite: r.get::<_, i64>(10)? != 0,
                    taken_at: effective_taken(r.get(11)?, r.get(4)?),
                })
            },
        )?;
        let mut by_id: HashMap<i64, ItemInfo> = rows.into_iter().map(|i| (i.item_id, i)).collect();
        Ok(item_ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    /// The file an item belongs to and the model that file was indexed with.
    pub fn item_owner(&self, item_id: i64) -> Result<Option<(i64, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT files.id, files.model FROM items JOIN files ON files.id = items.file_id WHERE items.id = ?1",
                [item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}
