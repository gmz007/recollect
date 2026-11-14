//! Date-ordered lists of files for browsing, filtered by folder, kind, favorites or album.

use super::{Db, count_of, typed, typed_or_zero};
use crate::media::MediaKind;
use crate::media::metadata::{DateSource, effective_taken};
use anyhow::Result;
use std::path::PathBuf;

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

/// One `files` row as read for the timeline, before conversion.
struct RawTimelineRow {
    file_id: i64,
    path: String,
    kind: MediaKind,
    mtime: i64,
    taken_at: Option<i64>,
    date_source: DateSource,
    duration: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    favorite: i64,
    camera: Option<String>,
    added_at: i64,
    item_id: Option<i64>,
}

/// Unconverted timeline rows; turning them into sorted [`TimelineItem`]s is CPU work that
/// needs no database access.
pub struct TimelineRows(Vec<RawTimelineRow>);

impl std::fmt::Debug for TimelineRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimelineRows")
            .field("rows", &self.0.len())
            .finish()
    }
}

impl TimelineRows {
    /// Converts and sorts into display order. Files whose capture time is not known yet are
    /// placed by their modification time (the sort key then depends on the machine's time zone).
    pub fn into_sorted(self, order: TimelineOrder) -> Vec<TimelineItem> {
        let mut out: Vec<(i64, TimelineItem)> = self
            .0
            .into_iter()
            .map(|r| {
                (
                    r.added_at,
                    TimelineItem {
                        file_id: r.file_id,
                        item_id: r.item_id,
                        path: PathBuf::from(r.path),
                        kind: r.kind,
                        mtime: r.mtime,
                        taken_at: effective_taken(r.taken_at, r.mtime),
                        date_source: r.date_source,
                        duration: r.duration,
                        width: r.width,
                        height: r.height,
                        favorite: r.favorite != 0,
                        camera: r.camera,
                    },
                )
            })
            .collect();
        match order {
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
                out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
            }
        }
        out.into_iter().map(|(_, i)| i).collect()
    }
}

impl Db {
    /// Number of files per capture-time source (files without a date yet are not counted).
    pub fn date_counts(&self) -> Result<DateCounts> {
        let (exif, video, file): (i64, i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(date_source = 1), 0), COALESCE(SUM(date_source = 2), 0),
                    COALESCE(SUM(date_source = 0), 0) FROM files WHERE taken_at IS NOT NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(DateCounts {
            exif: count_of(exif),
            video: count_of(video),
            file: count_of(file),
        })
    }

    /// Years (newest first) in which any file was taken, over the whole library: no view or
    /// type filter applies. Undated files count by their file time, like everywhere else.
    pub fn years(&self) -> Result<Vec<i32>> {
        use chrono::Datelike;
        let year = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.year());
        let mut years = std::collections::BTreeSet::new();
        let mut st = self.conn.prepare(
            "SELECT DISTINCT CAST(strftime('%Y', taken_at, 'unixepoch') AS INTEGER)
             FROM files WHERE taken_at IS NOT NULL",
        )?;
        for y in st.query_map([], |r| r.get::<_, Option<i32>>(0))? {
            years.extend(y?);
        }
        let mut st = self
            .conn
            .prepare("SELECT DISTINCT mtime FROM files WHERE taken_at IS NULL")?;
        for m in st.query_map([], |r| r.get::<_, i64>(0))? {
            years.extend(year(effective_taken(None, m?)));
        }
        Ok(years.into_iter().rev().collect())
    }

    /// All files matching `filter`, indexed or not, in display order. See
    /// [`Self::timeline_rows`] and [`TimelineRows::into_sorted`]; callers holding a lock on the
    /// database should split the two so the sort happens after releasing it.
    pub fn timeline(&self, filter: &TimelineFilter) -> Result<Vec<TimelineItem>> {
        Ok(self.timeline_rows(filter)?.into_sorted(filter.order))
    }

    /// Raw timeline rows straight from SQLite (no conversion, no sorting).
    pub fn timeline_rows(&self, filter: &TimelineFilter) -> Result<TimelineRows> {
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
            args.push(i64::from(k));
        }
        if filter.favorites_only {
            sql.push_str(" AND favorite = 1");
        }
        if let Some(a) = filter.album {
            sql.push_str(" AND id IN (SELECT file_id FROM album_items WHERE album_id = ?)");
            args.push(a);
        }
        let mut st = self.conn.prepare(&sql)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(args), |r| {
                Ok(RawTimelineRow {
                    file_id: r.get(0)?,
                    path: r.get(1)?,
                    kind: typed(r, 2)?,
                    mtime: r.get(3)?,
                    taken_at: r.get(4)?,
                    date_source: typed_or_zero(r, 5)?,
                    duration: r.get(6)?,
                    width: r.get(7)?,
                    height: r.get(8)?,
                    favorite: r.get(9)?,
                    camera: r.get(10)?,
                    added_at: r.get(11)?,
                    item_id: r.get(12)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(TimelineRows(rows))
    }
}
