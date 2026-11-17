//! Albums: named collections of files. Deleting an album never touches its files.

use super::{Db, count_of, now, typed};
use crate::media::MediaKind;
use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::PathBuf;

/// The longest album name, in characters.
pub const MAX_ALBUM_NAME_CHARS: usize = 100;

/// Why an album change was refused, for callers that word the reason themselves.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AlbumError {
    #[error("An album needs a name")]
    NameEmpty,
    #[error("Album names can have at most {MAX_ALBUM_NAME_CHARS} characters")]
    TooLong,
    #[error("An album named “{0}” already exists")]
    NameTaken(String),
    #[error("That album no longer exists")]
    Gone,
}

/// The one place album names are validated (the UI pre-checks with it too): trimmed, not empty,
/// at most [`MAX_ALBUM_NAME_CHARS`] characters, and not used by another album of `others`
/// (`(id, name)` pairs; `except` is the album being renamed) ignoring case. Returns the
/// trimmed name.
pub fn check_album_name<'a>(
    name: &str,
    others: impl IntoIterator<Item = (i64, &'a str)>,
    except: Option<i64>,
) -> std::result::Result<String, AlbumError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AlbumError::NameEmpty);
    }
    if name.chars().count() > MAX_ALBUM_NAME_CHARS {
        return Err(AlbumError::TooLong);
    }
    let wanted = name.to_lowercase();
    for (id, other) in others {
        if Some(id) != except && other.to_lowercase() == wanted {
            return Err(AlbumError::NameTaken(other.to_string()));
        }
    }
    Ok(name.to_string())
}

/// Puts files in an album inside a transaction; returns how many were new to it.
fn add_items(tx: &Connection, album: i64, file_ids: &[i64]) -> Result<usize> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM albums WHERE id = ?1)",
        [album],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(AlbumError::Gone.into());
    }
    let mut ins = tx.prepare(
        "INSERT OR IGNORE INTO album_items(album_id, file_id, added_at)
         SELECT ?1, id, ?3 FROM files WHERE id = ?2",
    )?;
    let at = now();
    let mut added = 0;
    for id in file_ids {
        added += ins.execute(params![album, id, at])?;
    }
    Ok(added)
}

/// The photo or video that stands for an album (its most recently taken item).
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumCover {
    pub file_id: i64,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub mtime: i64,
}

/// An album with its size and cover, for lists.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumInfo {
    pub id: i64,
    pub name: String,
    pub count: usize,
    /// `None` while the album is empty.
    pub cover: Option<AlbumCover>,
    pub created_at: i64,
}

impl Db {
    fn existing_album_names(conn: &Connection) -> Result<Vec<(i64, String)>> {
        let mut st = conn.prepare("SELECT id, name FROM albums")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Creates an album and returns its id. Names are trimmed, must not be empty, at most
    /// [`MAX_ALBUM_NAME_CHARS`] long and unique ignoring case.
    pub fn create_album(&mut self, name: &str) -> Result<i64> {
        Ok(self.create_album_with(name, &[])?.0)
    }

    /// Creates an album and puts `files` in it in one transaction: on any error nothing is
    /// left behind. Returns the id and how many files were added.
    pub fn create_album_with(&mut self, name: &str, files: &[i64]) -> Result<(i64, usize)> {
        // Immediate: the name check and the insert must see the same albums.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let name = check_album_name(
            name,
            Self::existing_album_names(&tx)?
                .iter()
                .map(|(i, n)| (*i, n.as_str())),
            None,
        )?;
        tx.execute(
            "INSERT INTO albums(name, created_at) VALUES (?1, ?2)",
            params![name, now()],
        )?;
        let id = tx.last_insert_rowid();
        let added = add_items(&tx, id, files)?;
        tx.commit()?;
        Ok((id, added))
    }

    /// Renames an album (same rules as [`Self::create_album`]; keeping its own name in another
    /// case is allowed).
    pub fn rename_album(&mut self, id: i64, name: &str) -> Result<()> {
        // Immediate: the name check and the update must see the same albums.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let name = check_album_name(
            name,
            Self::existing_album_names(&tx)?
                .iter()
                .map(|(i, n)| (*i, n.as_str())),
            Some(id),
        )?;
        let n = tx.execute(
            "UPDATE albums SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        if n == 0 {
            return Err(AlbumError::Gone.into());
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes an album and its entries. The files themselves are never touched. Returns
    /// whether the album existed.
    pub fn delete_album(&mut self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM albums WHERE id = ?1", [id])?
            > 0)
    }

    /// All albums by name (ignoring case), with item counts and covers: two queries however
    /// many albums there are.
    pub fn albums(&self) -> Result<Vec<AlbumInfo>> {
        let mut st = self.conn.prepare(
            "SELECT a.id, a.name, a.created_at, COUNT(i.file_id)
             FROM albums a LEFT JOIN album_items i ON i.album_id = a.id
             GROUP BY a.id",
        )?;
        let mut out: Vec<AlbumInfo> = st
            .query_map([], |r| {
                Ok(AlbumInfo {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    count: count_of(r.get::<_, i64>(3)?),
                    cover: None,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        // The newest item of each album stands for it. Taken time is approximated by the
        // stored value or the file time (no time zone conversion), which is plenty here.
        let mut st = self.conn.prepare(
            "SELECT album_id, id, path, kind, mtime FROM (
                 SELECT i.album_id, f.id, f.path, f.kind, f.mtime,
                        ROW_NUMBER() OVER (
                            PARTITION BY i.album_id
                            ORDER BY COALESCE(f.taken_at, f.mtime) DESC, f.path) AS rn
                 FROM album_items i JOIN files f ON f.id = i.file_id)
             WHERE rn = 1",
        )?;
        let mut covers: HashMap<i64, AlbumCover> = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    AlbumCover {
                        file_id: r.get(1)?,
                        path: PathBuf::from(r.get::<_, String>(2)?),
                        kind: typed(r, 3)?,
                        mtime: r.get(4)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for a in &mut out {
            a.cover = covers.remove(&a.id);
        }
        out.sort_by_cached_key(|a| (a.name.to_lowercase(), a.id));
        Ok(out)
    }

    /// Adds files to an album; files already in it are ignored. Returns how many were added.
    /// An album that no longer exists is a friendly error.
    pub fn add_to_album(&mut self, album: i64, file_ids: &[i64]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let added = add_items(&tx, album, file_ids)?;
        tx.commit()?;
        Ok(added)
    }

    /// Which of `file_ids` are in the album, each once, in the order given.
    pub fn album_members_among(&self, album: i64, file_ids: &[i64]) -> Result<Vec<i64>> {
        /// Ids per query; SQLite limits the number of bound parameters.
        const CHUNK: usize = 500;
        let mut members = std::collections::HashSet::new();
        for chunk in file_ids.chunks(CHUNK) {
            let marks = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT file_id FROM album_items WHERE album_id = ?1 AND file_id IN ({marks})"
            );
            let mut q = self.conn.prepare(&sql)?;
            let args = std::iter::once(&album).chain(chunk.iter());
            let rows = q.query_map(rusqlite::params_from_iter(args), |r| r.get::<_, i64>(0))?;
            for row in rows {
                members.insert(row?);
            }
        }
        let mut seen = std::collections::HashSet::new();
        Ok(file_ids
            .iter()
            .copied()
            .filter(|id| members.contains(id) && seen.insert(*id))
            .collect())
    }

    /// Takes files out of an album (they stay in the library). Returns how many were removed.
    pub fn remove_from_album(&mut self, album: i64, file_ids: &[i64]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut removed = 0;
        {
            let mut del =
                tx.prepare("DELETE FROM album_items WHERE album_id = ?1 AND file_id = ?2")?;
            for id in file_ids {
                removed += del.execute(params![album, id])?;
            }
        }
        tx.commit()?;
        Ok(removed)
    }
}
