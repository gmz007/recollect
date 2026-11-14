//! The database layout, and upgrading a database written by an older version.

use super::{Db, now};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::PathBuf;

pub(super) const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS folders (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    kind INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    status INTEGER NOT NULL DEFAULT 0,
    model TEXT,
    error TEXT,
    width INTEGER,
    height INTEGER,
    duration REAL,
    indexed_at INTEGER,
    taken_at INTEGER,
    date_source INTEGER,
    camera TEXT,
    favorite INTEGER NOT NULL DEFAULT 0,
    added_at INTEGER
);
CREATE INDEX IF NOT EXISTS files_folder ON files(folder_id);
CREATE INDEX IF NOT EXISTS files_status ON files(status);
CREATE TABLE IF NOT EXISTS items (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    t_start REAL,
    t_end REAL,
    embedding BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS items_file ON items(file_id);
";

/// Tables and indexes added after the first schema version; also part of a fresh database.
const SCHEMA_ALBUMS: &str = r"
CREATE TABLE IF NOT EXISTS albums (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS album_items (
    album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (album_id, file_id)
);
CREATE INDEX IF NOT EXISTS album_items_file ON album_items(file_id);
CREATE INDEX IF NOT EXISTS files_taken ON files(taken_at);
";

/// Upgrades a version 1 database in place (the caller runs this inside a transaction).
const MIGRATE_V1_TO_V2: &str = r"
ALTER TABLE files ADD COLUMN taken_at INTEGER;
ALTER TABLE files ADD COLUMN date_source INTEGER;
ALTER TABLE files ADD COLUMN camera TEXT;
ALTER TABLE files ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE files ADD COLUMN added_at INTEGER;
";

/// Upgrades a version 1 database inside the caller's transaction. Columns that already exist
/// (an earlier upgrade that was cut short) are left alone.
fn migrate_v1(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let have: HashSet<String> = tx
        .prepare("SELECT name FROM pragma_table_info('files')")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for stmt in MIGRATE_V1_TO_V2
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let col = stmt.split_whitespace().nth(5).unwrap_or_default();
        if !have.contains(col) {
            tx.execute_batch(stmt)?;
        }
    }
    // Existing files count as added when they were indexed (best guess), else now.
    tx.execute(
        "UPDATE files SET added_at = COALESCE(added_at, indexed_at, ?1)",
        [now()],
    )?;
    tx.execute_batch(SCHEMA_ALBUMS)?;
    Ok(())
}

impl Db {
    pub(super) fn create_or_migrate(&mut self) -> Result<()> {
        use rusqlite::TransactionBehavior;
        // A first start of an old version may have been interrupted after creating its tables
        // but before recording a version: such a database is version 1, not empty.
        let has_files = |c: &Connection| -> Result<bool> {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'files')",
                [],
                |r| r.get(0),
            )?)
        };
        // Back up a database that will be upgraded (once; never overwrite an earlier backup).
        // Decided from an unlocked peek; the transaction below re-checks.
        let peek_old = match self.meta("schema_version")?.as_deref() {
            Some("1") => true,
            None => has_files(&self.conn)?,
            _ => false,
        };
        if peek_old {
            self.backup_v1()?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match version.as_deref() {
            Some("2") => return Ok(()), // another process finished first
            Some("1") => migrate_v1(&tx)?,
            None if has_files(&tx)? => migrate_v1(&tx)?,
            None => {
                tx.execute_batch(SCHEMA)?;
                tx.execute_batch(SCHEMA_ALBUMS)?;
            }
            Some(v) => bail!(
                "library database has schema version {v}; this build supports {SCHEMA_VERSION}"
            ),
        }
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Copies the database file to `library.db.v1.bak` next to it before an in-place upgrade.
    /// Does nothing for in-memory databases or when that file already exists.
    fn backup_v1(&self) -> Result<()> {
        let Some(path) = self
            .conn
            .path()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
        else {
            return Ok(());
        };
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".v1.bak");
        let bak = path.with_file_name(name);
        if bak.exists() {
            return Ok(());
        }
        // Fold the write-ahead log into the main file so the copy is complete.
        let _: rusqlite::Result<()> =
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        let tmp = bak.with_extension("bak.part");
        std::fs::copy(&path, &tmp)
            .and_then(|_| std::fs::rename(&tmp, &bak))
            .with_context(|| format!("backing up the library database to {}", bak.display()))?;
        Ok(())
    }
}
