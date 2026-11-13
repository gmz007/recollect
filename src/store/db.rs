//! SQLite persistence: folders, files, embedded items. The database is the source of truth;
//! the in-memory vector index is rebuilt from it at startup (see `docs/design.md`).

use crate::media::metadata::{Capture, DateSource, local_wall_clock};
use crate::media::{MediaKind, ScannedFile};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = r#"
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
"#;

/// Tables and indexes added after the first schema version; also part of a fresh database.
const SCHEMA_ALBUMS: &str = r#"
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
"#;

/// Upgrades a version 1 database in place (the caller runs this inside a transaction).
const MIGRATE_V1_TO_V2: &str = r#"
ALTER TABLE files ADD COLUMN taken_at INTEGER;
ALTER TABLE files ADD COLUMN date_source INTEGER;
ALTER TABLE files ADD COLUMN camera TEXT;
ALTER TABLE files ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE files ADD COLUMN added_at INTEGER;
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Pending,
    Indexed,
    Error,
}

impl FileStatus {
    fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Indexed,
            2 => Self::Error,
            _ => Self::Pending,
        }
    }
}

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineOrder {
    /// Newest capture first.
    #[default]
    TakenDesc,
    TakenAsc,
    /// Most recently added to the library first.
    AddedDesc,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimelineFilter {
    pub folder: Option<i64>,
    pub kind: Option<MediaKind>,
    pub favorites_only: bool,
    pub album: Option<i64>,
    pub order: TimelineOrder,
}

/// One file on the timeline, indexed or not.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineItem {
    pub file_id: i64,
    /// One searchable item of the file (its first), if it has been indexed.
    pub item_id: Option<i64>,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub mtime: i64,
    /// Local wall-clock capture time (see `media::metadata`); the file time if unknown.
    pub taken_at: i64,
    pub date_source: DateSource,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub favorite: bool,
    pub camera: Option<String>,
}

/// How many files have a capture time from each source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateCounts {
    pub exif: usize,
    pub video: usize,
    pub file: usize,
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

pub struct Db {
    conn: Connection,
}

fn bump_epoch(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('items_epoch', '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [],
    )?;
    Ok(())
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

/// Random id tying an index cache to this database (a copied or recreated library must not
/// accept another library's cache even if the epoch happens to match).
fn new_library_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

fn path_str(p: &Path) -> Result<&str> {
    p.to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", p.display()))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
        match db.meta("schema_version")?.map(|v| v.parse::<i64>()) {
            None => {
                let tx = db.conn.transaction()?;
                tx.execute_batch(SCHEMA)?;
                tx.execute_batch(SCHEMA_ALBUMS)?;
                tx.commit()?;
                db.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
            }
            Some(Ok(SCHEMA_VERSION)) => {}
            Some(Ok(1)) => db.migrate_v1()?,
            Some(other) => bail!(
                "library database has schema version {}; this build supports {SCHEMA_VERSION}",
                match other {
                    Ok(v) => v.to_string(),
                    Err(_) => "(unreadable)".into(),
                }
            ),
        }
        if db.meta("library_id")?.is_none() {
            db.set_meta("library_id", &new_library_id())?;
        }
        Ok(db)
    }

    fn migrate_v1(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(MIGRATE_V1_TO_V2)?;
        // Existing files count as added when they were indexed (best guess), else now.
        tx.execute(
            "UPDATE files SET added_at = COALESCE(indexed_at, ?1)",
            [now()],
        )?;
        tx.execute_batch(SCHEMA_ALBUMS)?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('schema_version', '2')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        tx.commit()?;
        Ok(())
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

    /// Adds a library folder. `path` should be absolute and canonical. A folder inside an
    /// existing one is rejected; adding a parent of existing folders absorbs them.
    pub fn add_folder(&mut self, path: &Path) -> Result<i64> {
        let existing = self.folders()?;
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
        let tx = self.conn.transaction()?;
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
        let mut st = self
            .conn
            .prepare("SELECT id, path FROM folders ORDER BY path")?;
        let rows = st.query_map([], |r| {
            Ok(Folder {
                id: r.get(0)?,
                path: PathBuf::from(r.get::<_, String>(1)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Removes a folder and everything indexed under it; returns the removed file ids.
    pub fn remove_folder(&mut self, folder_id: i64) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
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
        let tx = self.conn.transaction()?;
        // The folder may have been removed (or absorbed by a parent) since the scan started.
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?1)",
            [folder_id],
            |r| r.get(0),
        )?;
        if !present {
            return Ok(SyncStats::default());
        }
        let mut existing: HashMap<String, (i64, u64, i64)> = HashMap::new();
        {
            let mut st =
                tx.prepare("SELECT path, id, size, mtime FROM files WHERE folder_id = ?1")?;
            let rows = st.query_map([folder_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get(1)?, r.get::<_, i64>(2)? as u64, r.get(3)?),
                ))
            })?;
            for row in rows {
                let (p, v) = row?;
                existing.insert(p, v);
            }
        }
        let mut stats = SyncStats::default();
        let mut seen = HashSet::new();
        {
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
                            f.kind.as_i64(),
                            f.size as i64,
                            f.mtime,
                            added_at
                        ])?;
                    }
                    Some(&(id, size, mtime)) if size != f.size || mtime != f.mtime => {
                        upd.execute(params![id, f.size as i64, f.mtime])?;
                        del_items.execute([id])?;
                        stats.changed += 1;
                        stats.changed_ids.push(id);
                    }
                    Some(_) => stats.unchanged += 1,
                }
            }
            let mut del = tx.prepare("DELETE FROM files WHERE id = ?1")?;
            for (p, &(id, _, _)) in &existing {
                if !seen.contains(p) && !protected.iter().any(|d| Path::new(p).starts_with(d)) {
                    del.execute([id])?;
                    stats.removed.push(id);
                }
            }
        }
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
        let rows = st.query_map([], |r| {
            Ok(FileRow {
                id: r.get(0)?,
                path: PathBuf::from(r.get::<_, String>(1)?),
                kind: MediaKind::from_i64(r.get(2)?),
                size: r.get::<_, i64>(3)? as u64,
                mtime: r.get(4)?,
                status: FileStatus::from_i64(r.get(5)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Stores capture times in one transaction. Files that vanished meanwhile are skipped.
    pub fn set_captures(&mut self, batch: &[(i64, Capture)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upd = tx.prepare(
                "UPDATE files SET taken_at = ?2, date_source = ?3, camera = ?4 WHERE id = ?1",
            )?;
            for (id, c) in batch {
                upd.execute(params![id, c.taken_at, c.source.as_i64(), c.camera])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Number of files per capture-time source (files without a date yet are not counted).
    pub fn date_counts(&self) -> Result<DateCounts> {
        let (exif, video, file): (i64, i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(date_source = 1), 0), COALESCE(SUM(date_source = 2), 0),
                    COALESCE(SUM(date_source = 0), 0) FROM files WHERE taken_at IS NOT NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(DateCounts {
            exif: exif as usize,
            video: video as usize,
            file: file as usize,
        })
    }

    /// All files matching `filter`, indexed or not, in display order. Files whose capture
    /// time is not known yet are placed by their modification time. One query and a sort in
    /// memory: the sort key for such files depends on the machine's time zone.
    pub fn timeline(&self, filter: &TimelineFilter) -> Result<Vec<TimelineItem>> {
        let mut sql = String::from(
            "SELECT id, path, kind, mtime, taken_at, date_source, duration, width, height,
                    favorite, camera, COALESCE(added_at, 0),
                    (SELECT MIN(id) FROM items WHERE items.file_id = files.id)
             FROM files WHERE 1 = 1",
        );
        let mut args: Vec<i64> = Vec::new();
        if let Some(f) = filter.folder {
            sql.push_str(" AND folder_id = ?");
            args.push(f);
        }
        if let Some(k) = filter.kind {
            sql.push_str(" AND kind = ?");
            args.push(k.as_i64());
        }
        if filter.favorites_only {
            sql.push_str(" AND favorite = 1");
        }
        if let Some(a) = filter.album {
            sql.push_str(" AND id IN (SELECT file_id FROM album_items WHERE album_id = ?)");
            args.push(a);
        }
        let mut st = self.conn.prepare(&sql)?;
        let mut rows = st.query(rusqlite::params_from_iter(args))?;
        let mut out: Vec<(i64, TimelineItem)> = Vec::new();
        while let Some(r) = rows.next()? {
            let mtime: i64 = r.get(3)?;
            let taken: Option<i64> = r.get(4)?;
            out.push((
                r.get(11)?,
                TimelineItem {
                    file_id: r.get(0)?,
                    item_id: r.get(12)?,
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    kind: MediaKind::from_i64(r.get(2)?),
                    mtime,
                    taken_at: taken.unwrap_or_else(|| local_wall_clock(mtime)),
                    date_source: DateSource::from_i64(r.get::<_, Option<i64>>(5)?.unwrap_or(0)),
                    duration: r.get(6)?,
                    width: r.get(7)?,
                    height: r.get(8)?,
                    favorite: r.get::<_, i64>(9)? != 0,
                    camera: r.get(10)?,
                },
            ));
        }
        match filter.order {
            TimelineOrder::TakenDesc => out.sort_by(|a, b| {
                b.1.taken_at
                    .cmp(&a.1.taken_at)
                    .then(a.1.path.cmp(&b.1.path))
            }),
            TimelineOrder::TakenAsc => out.sort_by(|a, b| {
                a.1.taken_at
                    .cmp(&b.1.taken_at)
                    .then(a.1.path.cmp(&b.1.path))
            }),
            TimelineOrder::AddedDesc => {
                out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)))
            }
        }
        Ok(out.into_iter().map(|(_, i)| i).collect())
    }

    /// Files that need (re-)embedding with `model`: never indexed, changed, or indexed with
    /// another model. Images first, then videos (cheap wins first).
    pub fn pending(&self, model: &str) -> Result<Vec<FileRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, path, kind, size, mtime, status FROM files
             WHERE status = 0 OR (status = 1 AND (model IS NULL OR model != ?1))
             ORDER BY kind, path",
        )?;
        let rows = st.query_map([model], |r| {
            Ok(FileRow {
                id: r.get(0)?,
                path: PathBuf::from(r.get::<_, String>(1)?),
                kind: MediaKind::from_i64(r.get(2)?),
                size: r.get::<_, i64>(3)? as u64,
                mtime: r.get(4)?,
                status: FileStatus::from_i64(r.get(5)?),
            })
        })?;
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
        Ok(n as usize)
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
            "SELECT items.id, items.file_id, files.kind, items.embedding FROM items
             JOIN files ON files.id = items.file_id
             WHERE files.status = 1 AND files.model = ?1 ORDER BY items.id",
        )?;
        let mut rows = st.query([model])?;
        while let Some(r) = rows.next()? {
            f(StoredItem {
                id: r.get(0)?,
                file_id: r.get(1)?,
                kind: MediaKind::from_i64(r.get(2)?),
                embedding: blob_to_emb(r.get_ref(3)?.as_blob()?),
            });
        }
        Ok(())
    }

    /// Runs `sql` (which must contain `IN (#)`) for chunks of ids and collects rows.
    fn by_ids<T>(
        &self,
        sql: &str,
        ids: &[i64],
        mut row: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>,
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

    pub fn embeddings(&self, item_ids: &[i64]) -> Result<HashMap<i64, Vec<f32>>> {
        let rows = self.by_ids(
            "SELECT id, embedding FROM items WHERE id IN (#)",
            item_ids,
            |r| Ok((r.get::<_, i64>(0)?, blob_to_emb(r.get_ref(1)?.as_blob()?))),
        )?;
        Ok(rows.into_iter().collect())
    }

    /// Display information for items, in the order of `item_ids` (missing items are skipped).
    pub fn item_info(&self, item_ids: &[i64]) -> Result<Vec<ItemInfo>> {
        let rows = self.by_ids(
            "SELECT items.id, items.file_id, files.path, files.kind, files.mtime, items.t_start, items.t_end,
                    files.width, files.height, files.duration
             FROM items JOIN files ON files.id = items.file_id WHERE items.id IN (#)",
            item_ids,
            |r| {
                Ok(ItemInfo {
                    item_id: r.get(0)?,
                    file_id: r.get(1)?,
                    path: PathBuf::from(r.get::<_, String>(2)?),
                    kind: MediaKind::from_i64(r.get(3)?),
                    mtime: r.get(4)?,
                    t_start: r.get(5)?,
                    t_end: r.get(6)?,
                    width: r.get(7)?,
                    height: r.get(8)?,
                    duration: r.get(9)?,
                })
            },
        )?;
        let mut by_id: HashMap<i64, ItemInfo> = rows.into_iter().map(|i| (i.item_id, i)).collect();
        Ok(item_ids.iter().filter_map(|id| by_id.remove(id)).collect())
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
            folders: folders as usize,
            images: images as usize,
            videos: videos as usize,
            indexed: indexed as usize,
            pending: pending as usize,
            errors: errors as usize,
            items: items as usize,
        })
    }

    pub fn folder_file_count(&self, folder_id: i64) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Makes every failed file pending again (e.g. after fixing a drive or updating Recollect).
    pub fn retry_errors(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE files SET status = 0, error = NULL WHERE status = 2",
            [],
        )?)
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

    /// Most recent indexing errors: (path, message).
    pub fn errors(&self, limit: usize) -> Result<Vec<(PathBuf, String)>> {
        let mut st = self.conn.prepare(
            "SELECT path, COALESCE(error, '') FROM files WHERE status = 2 ORDER BY indexed_at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| {
            Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf(p: &str, size: u64, mtime: i64) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from(p),
            kind: crate::media::scan::kind_of(Path::new(p)).unwrap(),
            size,
            mtime,
        }
    }

    fn item(v: f32) -> NewItem {
        NewItem {
            t_start: None,
            t_end: None,
            embedding: vec![v, 1.0 - v],
        }
    }

    #[test]
    fn folder_nesting_rules() {
        let mut db = Db::open_in_memory().unwrap();
        let a = db.add_folder(Path::new("/m/photos/2020")).unwrap();
        assert_eq!(db.add_folder(Path::new("/m/photos/2020")).unwrap(), a);
        assert!(db.add_folder(Path::new("/m/photos/2020/x")).is_err());
        db.sync_folder(a, &[sf("/m/photos/2020/a.jpg", 1, 1)], &[])
            .unwrap();
        // Adding a parent absorbs the child folder and its files.
        let p = db.add_folder(Path::new("/m/photos")).unwrap();
        let f = db.folders().unwrap();
        assert_eq!(
            f,
            vec![Folder {
                id: p,
                path: PathBuf::from("/m/photos")
            }]
        );
        assert_eq!(db.stats("m").unwrap().images, 1);
        // Sibling with common string prefix is not "inside".
        assert!(db.add_folder(Path::new("/m/photos2")).is_ok());
    }

    #[test]
    fn epoch_only_moves_on_real_changes() {
        let mut db = Db::open_in_memory().unwrap();
        let e0 = db.items_epoch().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        assert_eq!(db.items_epoch().unwrap(), e0, "add_folder");
        let files = [sf("/lib/a.jpg", 1, 1)];
        db.sync_folder(f, &files, &[]).unwrap();
        let e1 = db.items_epoch().unwrap();
        assert!(e1 > e0, "new file");
        db.sync_folder(f, &files, &[]).unwrap();
        assert_eq!(db.items_epoch().unwrap(), e1, "no-op sync");
        db.sync_folder(f, &[sf("/lib/a.jpg", 2, 1)], &[]).unwrap();
        let e2 = db.items_epoch().unwrap();
        assert!(e2 > e1, "changed file");
        db.sync_folder(f, &[], &[]).unwrap();
        assert!(db.items_epoch().unwrap() > e2, "removed file");
    }

    #[test]
    fn library_id_is_created_once() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db.sqlite");
        let id = Db::open(&p).unwrap().library_id().unwrap();
        assert_eq!(id.len(), 16);
        assert_eq!(Db::open(&p).unwrap().library_id().unwrap(), id);
        assert_ne!(Db::open_in_memory().unwrap().library_id().unwrap(), id);
    }

    #[test]
    fn sync_detects_add_change_remove() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        let s = db
            .sync_folder(f, &[sf("/lib/a.jpg", 1, 1), sf("/lib/b.mp4", 2, 2)], &[])
            .unwrap();
        assert_eq!(
            (s.added, s.changed, s.unchanged, s.removed.len()),
            (2, 0, 0, 0)
        );
        let pend = db.pending("m").unwrap();
        assert_eq!(pend.len(), 2);
        assert_eq!(pend[0].kind, MediaKind::Image);
        let ids = db
            .put_items("m", &[(pend[0].id, FileMeta::default(), vec![item(0.5)])])
            .unwrap();
        assert_eq!(ids[0].len(), 1);
        assert_eq!(db.pending("m").unwrap().len(), 1);
        // Another model needs everything again.
        assert_eq!(db.pending("other").unwrap().len(), 2);

        let s = db.sync_folder(f, &[sf("/lib/a.jpg", 1, 9)], &[]).unwrap();
        assert_eq!((s.added, s.changed, s.unchanged), (0, 1, 0));
        assert_eq!(s.removed, vec![pend[1].id]);
        assert_eq!(db.pending("m").unwrap().len(), 1);
        assert_eq!(
            db.stats("m").unwrap().items,
            0,
            "stale items of changed files are dropped"
        );
    }

    #[test]
    fn unreadable_subfolders_are_not_emptied() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        db.sync_folder(
            f,
            &[sf("/lib/a.jpg", 1, 1), sf("/lib/sub/b.jpg", 1, 1)],
            &[],
        )
        .unwrap();
        let s = db
            .sync_folder(f, &[sf("/lib/a.jpg", 1, 1)], &[PathBuf::from("/lib/sub")])
            .unwrap();
        assert!(s.removed.is_empty());
        let s = db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1)], &[]).unwrap();
        assert_eq!(s.removed.len(), 1);
    }

    #[test]
    fn items_round_trip_and_errors() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1), sf("/lib/v.mp4", 1, 1)], &[])
            .unwrap();
        let p = db.pending("m").unwrap();
        let v = NewItem {
            t_start: Some(1.0),
            t_end: Some(3.0),
            embedding: vec![0.25, -0.5],
        };
        let meta = FileMeta {
            width: Some(640),
            height: Some(480),
            duration: Some(10.0),
        };
        let ids = db
            .put_items(
                "m",
                &[
                    (p[0].id, FileMeta::default(), vec![item(0.1)]),
                    (p[1].id, meta, vec![v.clone()]),
                ],
            )
            .unwrap();
        let mut loaded = Vec::new();
        db.for_each_item("m", |it| loaded.push(it)).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].embedding, v.embedding);
        assert_eq!(loaded[1].kind, MediaKind::Video);
        let info = db.item_info(&[ids[1][0]]).unwrap();
        assert_eq!(info[0].t_start, Some(1.0));
        assert_eq!(info[0].width, Some(640));
        assert_eq!(db.embeddings(&ids[1]).unwrap()[&ids[1][0]], v.embedding);
        db.set_error(p[0].id, "boom").unwrap();
        assert_eq!(
            db.errors(10).unwrap(),
            vec![(PathBuf::from("/lib/a.jpg"), "boom".to_string())]
        );
        let st = db.stats("m").unwrap();
        assert_eq!((st.errors, st.indexed, st.items), (1, 1, 1));
        let removed = db.remove_folder(f).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(db.stats("m").unwrap(), Stats::default());
    }

    #[test]
    fn put_items_for_deleted_file_is_dropped() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        let g = db.add_folder(Path::new("/other")).unwrap();
        db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1)], &[]).unwrap();
        db.sync_folder(g, &[sf("/other/b.jpg", 1, 1)], &[]).unwrap();
        let p = db.pending("m").unwrap();
        db.remove_folder(f).unwrap();
        // A batch mixing a removed file with a live one: the live one is stored, the other skipped.
        let ids = db
            .put_items(
                "m",
                &[
                    (p[0].id, FileMeta::default(), vec![item(0.1)]),
                    (p[1].id, FileMeta::default(), vec![item(0.2)]),
                ],
            )
            .unwrap();
        assert!(ids[0].is_empty());
        assert_eq!(ids[1].len(), 1);
        assert_eq!(db.stats("m").unwrap().items, 1);
    }

    #[test]
    fn reopen_keeps_data() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x/lib.db");
        {
            let mut db = Db::open(&p).unwrap();
            db.add_folder(Path::new("/lib")).unwrap();
            db.set_meta("model", "m").unwrap();
        }
        let db = Db::open(&p).unwrap();
        assert_eq!(db.folders().unwrap().len(), 1);
        assert_eq!(db.meta("model").unwrap().as_deref(), Some("m"));
    }

    /// The schema as shipped in version 1, to prove old libraries keep working.
    const V1_SCHEMA: &str = r#"
        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE folders (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, added_at INTEGER NOT NULL);
        CREATE TABLE files (
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
            indexed_at INTEGER
        );
        CREATE INDEX files_folder ON files(folder_id);
        CREATE INDEX files_status ON files(status);
        CREATE TABLE items (
            id INTEGER PRIMARY KEY,
            file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            t_start REAL,
            t_end REAL,
            embedding BLOB NOT NULL
        );
        CREATE INDEX items_file ON items(file_id);
        INSERT INTO meta VALUES ('schema_version', '1'), ('library_id', 'abc'), ('items_epoch', '4');
        INSERT INTO folders VALUES (1, '/lib', 10);
        INSERT INTO files VALUES (1, 1, '/lib/a.jpg', 0, 5, 1000, 1, 'm', NULL, 640, 480, NULL, 555);
        INSERT INTO files VALUES (2, 1, '/lib/b.mp4', 1, 6, 2000, 0, NULL, NULL, NULL, NULL, NULL, NULL);
        INSERT INTO items VALUES (1, 1, NULL, NULL, x'0000803f');
    "#;

    #[test]
    fn v1_database_is_migrated() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("lib.db");
        Connection::open(&p)
            .unwrap()
            .execute_batch(V1_SCHEMA)
            .unwrap();
        let db = Db::open(&p).unwrap();
        assert_eq!(db.meta("schema_version").unwrap().as_deref(), Some("2"));
        assert_eq!(db.library_id().unwrap(), "abc");
        assert_eq!(db.items_epoch().unwrap(), 4);
        assert_eq!(db.folders().unwrap().len(), 1);
        let st = db.stats("m").unwrap();
        assert_eq!((st.images, st.videos, st.indexed, st.items), (1, 1, 1, 1));
        let tl = db.timeline(&TimelineFilter::default()).unwrap();
        assert_eq!(tl.len(), 2);
        assert!(
            tl.iter()
                .all(|i| i.date_source == DateSource::File && !i.favorite)
        );
        let added: Vec<(i64, i64)> = db
            .conn
            .prepare("SELECT id, added_at FROM files ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(added[0], (1, 555), "uses indexed_at");
        assert!(added[1].1 >= now() - 60, "falls back to now");
        assert_eq!(db.pending_dates().unwrap().len(), 2);
        db.conn
            .execute("INSERT INTO albums(name, created_at) VALUES ('x', 1)", [])
            .unwrap();
        drop(db);
        // Reopening a migrated database is a no-op.
        assert_eq!(Db::open(&p).unwrap().folders().unwrap().len(), 1);
    }

    #[test]
    fn newer_schema_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("lib.db");
        drop(Db::open(&p).unwrap());
        Connection::open(&p)
            .unwrap()
            .execute(
                "UPDATE meta SET value = '3' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        let e = Db::open(&p).err().expect("must refuse").to_string();
        assert!(e.contains("schema version 3"), "{e}");
    }

    fn capture(t: i64, source: DateSource) -> Capture {
        Capture {
            taken_at: t,
            source,
            camera: Some("cam".into()),
        }
    }

    #[test]
    fn captures_are_stored_and_reset_when_a_file_changes() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1), sf("/lib/b.jpg", 1, 1)], &[])
            .unwrap();
        let pend = db.pending_dates().unwrap();
        assert_eq!(pend.len(), 2);
        db.set_captures(&[
            (pend[0].id, capture(100_000_000, DateSource::Exif)),
            (pend[1].id, capture(200_000_000, DateSource::File)),
        ])
        .unwrap();
        assert!(db.pending_dates().unwrap().is_empty());
        assert_eq!(
            db.date_counts().unwrap(),
            DateCounts {
                exif: 1,
                video: 0,
                file: 1
            }
        );
        db.sync_folder(f, &[sf("/lib/a.jpg", 2, 1), sf("/lib/b.jpg", 1, 1)], &[])
            .unwrap();
        let pend = db.pending_dates().unwrap();
        assert_eq!(pend.len(), 1);
        assert_eq!(pend[0].path, PathBuf::from("/lib/a.jpg"));
    }

    fn timeline_db() -> (Db, i64, i64) {
        let mut db = Db::open_in_memory().unwrap();
        let f1 = db.add_folder(Path::new("/one")).unwrap();
        let f2 = db.add_folder(Path::new("/two")).unwrap();
        db.sync_folder(f1, &[sf("/one/a.jpg", 1, 10), sf("/one/b.mp4", 1, 10)], &[])
            .unwrap();
        db.sync_folder(f2, &[sf("/two/c.jpg", 1, 10), sf("/two/d.jpg", 1, 10)], &[])
            .unwrap();
        (db, f1, f2)
    }

    fn id_of(db: &Db, path: &str) -> i64 {
        db.conn
            .query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0))
            .unwrap()
    }

    fn names(items: &[TimelineItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn timeline_order_and_ties() {
        let (mut db, _, _) = timeline_db();
        let ids: Vec<i64> = ["/one/a.jpg", "/one/b.mp4", "/two/c.jpg", "/two/d.jpg"]
            .iter()
            .map(|p| id_of(&db, p))
            .collect();
        db.set_captures(&[
            (ids[0], capture(300_000_000, DateSource::Exif)),
            (ids[1], capture(100_000_000, DateSource::Video)),
            (ids[2], capture(300_000_000, DateSource::Exif)), // ties with a.jpg
            (ids[3], capture(200_000_000, DateSource::File)),
        ])
        .unwrap();
        let desc = db.timeline(&TimelineFilter::default()).unwrap();
        assert_eq!(names(&desc), ["a.jpg", "c.jpg", "d.jpg", "b.mp4"]);
        let asc = db
            .timeline(&TimelineFilter {
                order: TimelineOrder::TakenAsc,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(names(&asc), ["b.mp4", "d.jpg", "a.jpg", "c.jpg"]);
        assert_eq!(asc[0].camera.as_deref(), Some("cam"));
        assert_eq!(asc[0].date_source, DateSource::Video);
        // Added order: everything was added in the same second, so path order decides; make
        // one file "newer" explicitly.
        db.conn
            .execute(
                "UPDATE files SET added_at = added_at + 100 WHERE id = ?1",
                [ids[3]],
            )
            .unwrap();
        let added = db
            .timeline(&TimelineFilter {
                order: TimelineOrder::AddedDesc,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(names(&added), ["d.jpg", "a.jpg", "b.mp4", "c.jpg"]);
    }

    #[test]
    fn timeline_filters() {
        let (db, f1, _) = timeline_db();
        let a = id_of(&db, "/one/a.jpg");
        let c = id_of(&db, "/two/c.jpg");
        db.conn
            .execute("UPDATE files SET favorite = 1 WHERE id IN (?1, ?2)", [a, c])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO albums(id, name, created_at) VALUES (7, 'trip', 1)",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO album_items(album_id, file_id, added_at) VALUES (7, ?1, 1), (7, ?2, 1)",
                [c, id_of(&db, "/two/d.jpg")],
            )
            .unwrap();
        let q = |f: TimelineFilter| names(&db.timeline(&f).unwrap());
        let all = TimelineFilter {
            order: TimelineOrder::TakenAsc,
            ..Default::default()
        };
        assert_eq!(q(all.clone()).len(), 4);
        assert_eq!(
            q(TimelineFilter {
                folder: Some(f1),
                ..all.clone()
            }),
            ["a.jpg", "b.mp4"]
        );
        assert_eq!(
            q(TimelineFilter {
                kind: Some(MediaKind::Video),
                ..all.clone()
            }),
            ["b.mp4"]
        );
        assert_eq!(
            q(TimelineFilter {
                favorites_only: true,
                ..all.clone()
            }),
            ["a.jpg", "c.jpg"]
        );
        assert_eq!(
            q(TimelineFilter {
                album: Some(7),
                ..all.clone()
            }),
            ["c.jpg", "d.jpg"]
        );
        assert_eq!(
            q(TimelineFilter {
                album: Some(7),
                favorites_only: true,
                folder: Some(f1),
                ..all.clone()
            }),
            Vec::<String>::new()
        );
        // Deleting a file or an album cleans up memberships.
        db.conn
            .execute("DELETE FROM files WHERE id = ?1", [c])
            .unwrap();
        assert_eq!(
            q(TimelineFilter {
                album: Some(7),
                ..all.clone()
            }),
            ["d.jpg"]
        );
        db.conn
            .execute("DELETE FROM albums WHERE id = 7", [])
            .unwrap();
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM album_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn timeline_uses_file_time_when_the_date_is_unknown() {
        let mut db = Db::open_in_memory().unwrap();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        db.sync_folder(
            f,
            &[sf("/lib/a.jpg", 1, 1_500_000_000), sf("/lib/b.jpg", 1, 1)],
            &[],
        )
        .unwrap();
        let b = id_of(&db, "/lib/b.jpg");
        db.set_captures(&[(b, capture(1_600_000_000, DateSource::Exif))])
            .unwrap();
        let tl = db.timeline(&TimelineFilter::default()).unwrap();
        assert_eq!(names(&tl), ["b.jpg", "a.jpg"]);
        assert_eq!(tl[1].taken_at, local_wall_clock(1_500_000_000));
        assert_eq!(tl[1].date_source, DateSource::File);
    }
}
