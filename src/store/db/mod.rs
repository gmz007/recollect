//! SQLite persistence: folders, files, embedded items. The database is the source of truth;
//! the in-memory vector index is rebuilt from it at startup (see `docs/design.md`).
//!
//! `Db` is one type; its methods are grouped by what they store: `files` (folders and the
//! files found in them), `items` (searchable vectors), `timeline` (date-ordered lists),
//! `albums`, and `schema` (creation and upgrades).

mod albums;
mod files;
mod items;
mod schema;
mod timeline;

pub use albums::{AlbumCover, AlbumError, AlbumInfo, MAX_ALBUM_NAME_CHARS, check_album_name};
pub use files::{FileDetails, FileRow, Folder, Stats, SyncStats};
pub use items::{FileMeta, ItemInfo, NewItem, StoredItem, blob_to_emb, emb_to_blob};
pub use timeline::{DateCounts, TimelineFilter, TimelineItem, TimelineOrder, TimelineRows};

use crate::media::UnknownValue;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use schema::SCHEMA_VERSION;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Pending,
    Indexed,
    Error,
}

impl TryFrom<i64> for FileStatus {
    type Error = UnknownValue;

    fn try_from(v: i64) -> Result<Self, UnknownValue> {
        match v {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Indexed),
            2 => Ok(Self::Error),
            value => Err(UnknownValue {
                what: "file status",
                value,
            }),
        }
    }
}

/// Reads column `idx` as `T`; a stored number that names no variant is an error.
fn typed<T: TryFrom<i64, Error = UnknownValue>>(
    r: &rusqlite::Row<'_>,
    idx: usize,
) -> rusqlite::Result<T> {
    let v: i64 = r.get(idx)?;
    T::try_from(v).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Integer, Box::new(e))
    })
}

/// [`typed`] for a column that may be NULL, which reads as the default (`0`).
fn typed_or_zero<T: TryFrom<i64, Error = UnknownValue>>(
    r: &rusqlite::Row<'_>,
    idx: usize,
) -> rusqlite::Result<T> {
    match r.get::<_, Option<i64>>(idx)? {
        Some(_) => typed(r, idx),
        None => T::try_from(0).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                idx,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        }),
    }
}

pub struct Db {
    conn: Connection,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

fn bump_epoch(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('items_epoch', '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [],
    )?;
    Ok(())
}

/// Random id tying an index cache to this database (a copied or recreated library must not
/// accept another library's cache even if the epoch happens to match).
fn new_library_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

fn path_str(p: &Path) -> Result<&str> {
    p.to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", p.display()))
}

/// A count read from the database; a corrupt negative value reads as zero.
fn count_of(v: i64) -> usize {
    usize::try_from(v).unwrap_or(0)
}

/// A size read from the database; a corrupt negative value reads as zero.
fn unsigned(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// A size or limit stored as a SQLite integer, saturating at its maximum.
fn signed<T: TryInto<i64>>(v: T) -> i64 {
    v.try_into().unwrap_or(i64::MAX)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| signed(d.as_secs()))
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // Another process (CLI next to the GUI) may hold a lock briefly; set this first.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let mut db = Self { conn };
        // Decide before touching any table: a newer database must not be modified by an older
        // build, and an older one needs its columns added before the new indexes exist.
        // Creation and migration run in one write transaction that re-reads the version, so two
        // processes opening the same new or old database at once cannot both change it.
        let version = db.meta("schema_version")?;
        let needs_work = match version.as_deref().map(str::parse::<i64>) {
            Some(Ok(SCHEMA_VERSION)) => false,
            Some(Ok(1)) | None => true,
            Some(other) => bail!(
                "library database has schema version {}; this build supports {SCHEMA_VERSION}",
                match other {
                    Ok(v) => v.to_string(),
                    Err(_) => "(unreadable)".into(),
                }
            ),
        };
        if needs_work {
            db.create_or_migrate()?;
        }
        // One statement, so two processes opening a new database end up with the same id.
        db.conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('library_id', ?1)",
            [new_library_id()],
        )?;
        Ok(db)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Identity of this library database (see [`new_library_id`]).
    pub fn library_id(&self) -> Result<String> {
        self.meta("library_id")?
            .context("library database has no library_id")
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Runs `sql` (which must contain `IN (#)`) for chunks of ids and collects rows.
    fn by_ids<T>(
        &self,
        sql: &str,
        ids: &[i64],
        mut row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            // Not cached: the statement text varies with the chunk length.
            let mut st = self.conn.prepare(&sql.replace('#', &marks))?;
            let rows = st.query_map(rusqlite::params_from_iter(chunk.iter()), &mut row)?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
