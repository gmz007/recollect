//! Browsing: the timeline, recently added, favorites, per-folder and per-album views.
//!
//! Files are loaded in the background, grouped by capture day, month or year in memory and laid
//! out as rows (section headers and rows of tiles) with known vertical offsets. Only the rows
//! inside the scroll viewport are drawn, found by binary search, so very large libraries stay
//! smooth.

use super::selection::{self, ClickMods, Selection};
use super::workers::ThumbCache;
use super::{App, Nav, Query, albums, fmt_time, marked, platform, take_plain_key};
use crate::Library;
use crate::library::{Activity, AlbumOp};
use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::store::db::{TimelineFilter, TimelineItem, TimelineOrder};
use crate::thumbs::ThumbKey;
use chrono::{Datelike, NaiveDate};
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Color32, RichText, Sense, Vec2};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

const GAP: f32 = 8.0;
const HEADER_H: f32 = 34.0;
const SECTION_GAP: f32 = 10.0;
/// Reload interval while a scan or indexing run keeps adding files.
const LIVE_RELOAD: Duration = Duration::from_secs(10);
/// Wait before loading the same view again after a failed load.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// What the central area shows when no search is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Timeline,
    RecentlyAdded,
    Favorites,
    Folder(i64),
    Album(i64),
}

/// How the timeline is split into sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Day,
    Month,
    Year,
}

// ---------------------------------------------------------------------------------------------
// Grouping and layout (pure functions, unit tested)
// ---------------------------------------------------------------------------------------------

fn date_of(taken_at: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

/// Section key of a capture time (local wall-clock seconds stored as if UTC). Keys of
/// neighbouring sections are not necessarily adjacent integers; they only identify a section.
pub fn group_key(taken_at: i64, group: Group) -> i64 {
    let d = date_of(taken_at);
    match group {
        Group::Day => d.num_days_from_ce() as i64,
        Group::Month => d.year() as i64 * 12 + d.month0() as i64,
        Group::Year => d.year() as i64,
    }
}

/// `1204` becomes `1,204`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Section title such as "Saturday 4 July 2019 · 12", "July 2019 · 132" or "2019 · 1,204".
pub fn header_text(group: Group, key: i64, count: usize) -> String {
    let name = match group {
        Group::Day => i32::try_from(key)
            .ok()
            .and_then(NaiveDate::from_num_days_from_ce_opt)
            .map(|d| d.format("%A %-d %B %Y").to_string()),
        Group::Month => {
            NaiveDate::from_ymd_opt(key.div_euclid(12) as i32, key.rem_euclid(12) as u32 + 1, 1)
                .map(|d| d.format("%B %Y").to_string())
        }
        Group::Year => Some(key.to_string()),
    }
    .unwrap_or_default();
    format!("{name} · {}", thousands(count))
}

/// A run of consecutive items that share a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSpan {
    pub key: i64,
    pub start: usize,
    pub end: usize,
}

/// Splits `items` (already in display order) into sections. Without headers everything is one
/// section.
pub fn compute_groups(items: &[TimelineItem], group: Group, headers: bool) -> Vec<GroupSpan> {
    let mut out: Vec<GroupSpan> = Vec::new();
    if items.is_empty() {
        return out;
    }
    if !headers {
        out.push(GroupSpan {
            key: 0,
            start: 0,
            end: items.len(),
        });
        return out;
    }
    for (i, it) in items.iter().enumerate() {
        let key = group_key(it.taken_at, group);
        match out.last_mut() {
            Some(g) if g.key == key => g.end = i + 1,
            _ => out.push(GroupSpan {
                key,
                start: i,
                end: i + 1,
            }),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header { group: usize, first: usize },
    Tiles { start: usize, end: usize },
}

impl Row {
    fn first_item(self) -> usize {
        match self {
            Row::Header { first, .. } => first,
            Row::Tiles { start, .. } => start,
        }
    }
}

/// Rows with vertical offsets for one width / tile size / grouping.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub rows: Vec<Row>,
    /// Top of each row, ascending.
    pub tops: Vec<f32>,
    pub total: f32,
    pub cols: usize,
    /// Edge length of a tile after fitting whole columns into the width.
    pub tile: f32,
    /// Index of the first row of each group (its header, when there are headers).
    pub group_row: Vec<usize>,
}

impl Layout {
    pub fn build(groups: &[GroupSpan], width: f32, tile_size: f32, headers: bool) -> Self {
        let cols = (((width + GAP) / (tile_size + GAP)).floor() as usize).max(1);
        let tile = ((width - GAP * (cols - 1) as f32) / cols as f32).max(1.0);
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        let mut group_row = Vec::with_capacity(groups.len());
        let mut y = 0.0;
        for (gi, g) in groups.iter().enumerate() {
            group_row.push(rows.len());
            if headers {
                tops.push(y);
                rows.push(Row::Header {
                    group: gi,
                    first: g.start,
                });
                y += HEADER_H;
            }
            let mut s = g.start;
            while s < g.end {
                let e = (s + cols).min(g.end);
                tops.push(y);
                rows.push(Row::Tiles { start: s, end: e });
                y += tile + GAP;
                s = e;
            }
            if headers {
                y += SECTION_GAP;
            }
        }
        Self {
            rows,
            tops,
            total: y,
            cols,
            tile,
            group_row,
        }
    }

    /// Rows that intersect the vertical span `y0..y1`.
    pub fn visible(&self, y0: f32, y1: f32) -> Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }
        let start = self.tops.partition_point(|&t| t <= y0).saturating_sub(1);
        let end = self.tops.partition_point(|&t| t < y1);
        start..end.max(start)
    }

    /// The row that draws `item`.
    pub fn row_of_item(&self, item: usize) -> usize {
        self.rows
            .partition_point(|r| r.first_item() <= item)
            .saturating_sub(1)
    }

    /// Vertical span `(top, height)` to reveal for `item`; includes the section header when the
    /// item is in the first tile row of its section.
    pub fn reveal_span(&self, item: usize) -> (f32, f32) {
        let r = self.row_of_item(item);
        let mut top = self.tops[r];
        if r > 0 && matches!(self.rows[r - 1], Row::Header { .. }) {
            top = self.tops[r - 1];
        }
        (top, self.tops[r] + self.tile - top)
    }
}

impl Layout {
    /// Scroll offset for jumping to `item`: the section header when the item starts a section,
    /// else the row that contains it (e.g. a month inside a year section).
    pub fn jump_top(&self, groups: &[GroupSpan], item: usize) -> f32 {
        let g = groups.partition_point(|g| g.end <= item);
        if groups.get(g).is_some_and(|g| g.start == item)
            && let Some(&top) = self.group_row.get(g).and_then(|&r| self.tops.get(r))
        {
            return top;
        }
        self.tops
            .get(self.row_of_item(item))
            .copied()
            .unwrap_or(0.0)
    }

    /// Item reached by an arrow key from `cur` (`cur < len`). Up and down go to the same
    /// column of the previous / next row of tiles in the layout, across section boundaries,
    /// clamped to the last tile of a shorter row; left and right go to the previous / next item.
    pub fn step(&self, cur: usize, len: usize, key: Nav) -> usize {
        match key {
            Nav::Left => cur.saturating_sub(1),
            Nav::Right => (cur + 1).min(len.saturating_sub(1)),
            Nav::Up | Nav::Down => {
                let r = self.row_of_item(cur);
                let Some(Row::Tiles { start, .. }) = self.rows.get(r).copied() else {
                    return cur;
                };
                let col = cur - start;
                let target = if key == Nav::Up {
                    self.rows[..r].iter().rev().find_map(tiles_of)
                } else {
                    self.rows[r + 1..].iter().find_map(tiles_of)
                };
                match target {
                    Some((s, e)) => (s + col).min(e - 1),
                    None => cur,
                }
            }
        }
    }
}

fn tiles_of(r: &Row) -> Option<(usize, usize)> {
    match *r {
        Row::Tiles { start, end } => Some((start, end)),
        Row::Header { .. } => None,
    }
}

/// Scroll offset that makes `top..top + height` visible, or `None` if it already is.
fn reveal(top: f32, height: f32, view: f32, offset: f32) -> Option<f32> {
    if top < offset {
        Some(top)
    } else if top + height > offset + view {
        Some((top + height - view).max(0.0))
    } else {
        None
    }
}

/// Years (newest first) with their months, for the date navigator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearEntry {
    pub year: i32,
    pub count: usize,
    /// Index of the year's first item in display order.
    pub first: usize,
    pub months: Vec<MonthEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthEntry {
    pub month: u32,
    pub count: usize,
    pub first: usize,
}

pub fn date_index(items: &[TimelineItem]) -> Vec<YearEntry> {
    let mut by_month: BTreeMap<(i32, u32), (usize, usize)> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        let d = date_of(it.taken_at);
        let e = by_month.entry((d.year(), d.month())).or_insert((0, i));
        e.0 += 1;
        e.1 = e.1.min(i);
    }
    let mut years: Vec<YearEntry> = Vec::new();
    for (&(year, month), &(count, first)) in by_month.iter().rev() {
        let m = MonthEntry {
            month,
            count,
            first,
        };
        match years.last_mut() {
            Some(y) if y.year == year => {
                y.count += count;
                y.first = y.first.min(first);
                y.months.push(m);
            }
            _ => years.push(YearEntry {
                year,
                count,
                first,
                months: vec![m],
            }),
        }
    }
    years
}

fn fmt_taken(item: &TimelineItem) -> String {
    fmt_taken_at(item.taken_at, Some(item.date_source))
}

/// "Saturday 4 July 2019, 12:30", with "(file date)" appended when the time is only the file's.
pub fn fmt_taken_at(taken_at: i64, source: Option<DateSource>) -> String {
    let t = chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.naive_utc().format("%A %-d %B %Y, %H:%M").to_string())
        .unwrap_or_default();
    if source == Some(DateSource::File) {
        format!("{t} (file date)")
    } else {
        t
    }
}

/// One "On this day" entry: an item of the album and how many years ago it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnThisDay {
    pub index: usize,
    pub years_ago: i32,
}

/// Most entries the "On this day" strip shows.
const ON_THIS_DAY_MAX: usize = 30;

/// Items taken on the same month and day as `today` in earlier years, newest first (at most
/// [`ON_THIS_DAY_MAX`]). `index` points into `items`, whatever order they are in. Items whose
/// date is only the file's own time are left out: that is when the file was saved or copied,
/// not when it was taken. In years without a 29 February, its photos show up on 28 February.
pub fn on_this_day(items: &[TimelineItem], today: NaiveDate) -> Vec<OnThisDay> {
    let leap_day_too = today.month() == 2
        && today.day() == 28
        && NaiveDate::from_ymd_opt(today.year(), 2, 29).is_none();
    let mut hits: Vec<(i64, usize, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            if it.date_source == DateSource::File {
                return None;
            }
            let d = date_of(it.taken_at);
            let same_day = d.month() == today.month() && d.day() == today.day()
                || leap_day_too && d.month() == 2 && d.day() == 29;
            (same_day && d.year() < today.year()).then_some((
                it.taken_at,
                i,
                today.year() - d.year(),
            ))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(items[a.1].path.cmp(&items[b.1].path)));
    hits.truncate(ON_THIS_DAY_MAX);
    hits.into_iter()
        .map(|(_, index, years_ago)| OnThisDay { index, years_ago })
        .collect()
}

/// Time until the next local midnight after `now` (when "today" changes).
pub fn until_next_midnight(now: chrono::NaiveDateTime) -> Duration {
    now.date()
        .succ_opt()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|m| (m - now).to_std().ok())
        .unwrap_or(Duration::from_secs(3600))
}

fn years_ago_text(n: i32) -> String {
    if n == 1 {
        "1 year ago".to_string()
    } else {
        format!("{n} years ago")
    }
}

pub fn thumb_key(item: &TimelineItem) -> ThumbKey {
    ThumbKey {
        path: item.path.clone(),
        mtime: item.mtime,
        t_ms: (item.kind == MediaKind::Video)
            .then(|| ((item.duration.unwrap_or(0.0) * 0.1).clamp(0.0, 5.0) * 1000.0) as i64),
    }
}

// ---------------------------------------------------------------------------------------------
// Background loading
// ---------------------------------------------------------------------------------------------

pub(super) struct Loaded {
    pub(super) id: u64,
    pub(super) filter: TimelineFilter,
    pub(super) items: Result<Vec<TimelineItem>, String>,
}

/// Loads timelines on a dedicated thread; only the newest pending request is executed.
pub(super) struct TimelineWorker {
    tx: Sender<(u64, TimelineFilter)>,
    rx: Receiver<Loaded>,
    next_id: u64,
    /// Lets tests deliver a load result of their own making.
    #[cfg(test)]
    inject: Sender<Loaded>,
}

impl TimelineWorker {
    fn new(lib: Arc<Library>, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, req_rx) = unbounded::<(u64, TimelineFilter)>();
        let (resp_tx, rx) = unbounded();
        #[cfg(test)]
        let inject = resp_tx.clone();
        std::thread::Builder::new()
            .name("recollect-timeline".into())
            .spawn(move || {
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    let (id, filter) = req;
                    let run = std::panic::AssertUnwindSafe(|| lib.timeline(&filter));
                    let items = match std::panic::catch_unwind(run) {
                        Ok(r) => r.map_err(|e| format!("{e:#}")),
                        Err(_) => Err("loading the library failed unexpectedly".to_string()),
                    };
                    if resp_tx.send(Loaded { id, filter, items }).is_err() {
                        break;
                    }
                    repaint();
                }
            })
            .expect("spawning timeline thread");
        Self {
            tx,
            rx,
            next_id: 0,
            #[cfg(test)]
            inject,
        }
    }

    #[cfg(test)]
    pub(super) fn inject(&self, loaded: Loaded) {
        let _ = self.inject.send(loaded);
    }

    fn submit(&mut self, filter: TimelineFilter) -> u64 {
        self.next_id += 1;
        let _ = self.tx.send((self.next_id, filter));
        self.next_id
    }

    fn poll(&self) -> Option<Loaded> {
        let mut last = None;
        while let Ok(r) = self.rx.try_recv() {
            last = Some(r);
        }
        last
    }
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

pub(super) struct TimelineState {
    pub(super) worker: TimelineWorker,
    pub(super) view: View,
    pub(super) group: Group,
    pub(super) newest_first: bool,
    pub(super) tile_size: f32,
    /// Shared with the viewer, which keeps its own snapshot of the list while it is open.
    pub(super) items: Arc<Vec<TimelineItem>>,
    pub(super) generation: u64,
    groups: Vec<GroupSpan>,
    groups_key: Option<(u64, Group, bool)>,
    pub(super) dates: Vec<YearEntry>,
    layout: Option<Layout>,
    /// "On this day" entries for the loaded items and the date they were computed for.
    otd: Option<(u64, NaiveDate, Vec<OnThisDay>)>,
    layout_key: Option<(u64, Group, bool, i32, u32)>,
    pub(super) loaded: bool,
    pub(super) awaiting: Option<(u64, TimelineFilter)>,
    loaded_filter: Option<TimelineFilter>,
    pub(super) requested: Option<(TimelineFilter, u64)>,
    requested_at: Instant,
    /// The filter whose last load failed, and when; it is not retried right away.
    failed: Option<(TimelineFilter, Instant)>,
    error: Option<String>,
    /// The primary selected item (last clicked); see [`Selection`].
    pub(super) selected: Option<usize>,
    pub(super) sel: Selection,
    expanded: HashSet<i32>,
    top_year: Option<i32>,
    top_item: Option<usize>,
    scroll_to: Option<f32>,
    /// Item to scroll to once the layout is known; `true` aligns to its section header.
    pending: Option<(usize, bool)>,
    pub(super) keyboard_moved: bool,
    /// Set when the grid was drawn; read (and cleared) at the start of the next frame.
    pub(super) shown: bool,
}

impl TimelineState {
    pub(super) fn new(lib: Arc<Library>, ctx: &egui::Context) -> Self {
        let c = ctx.clone();
        Self {
            worker: TimelineWorker::new(lib, move || c.request_repaint()),
            view: View::Timeline,
            group: Group::Month,
            newest_first: true,
            tile_size: 180.0,
            items: Arc::new(Vec::new()),
            generation: 0,
            groups: Vec::new(),
            groups_key: None,
            dates: Vec::new(),
            layout: None,
            otd: None,
            layout_key: None,
            loaded: false,
            awaiting: None,
            loaded_filter: None,
            requested: None,
            requested_at: Instant::now(),
            failed: None,
            error: None,
            selected: None,
            sel: Selection::default(),
            expanded: HashSet::new(),
            top_year: None,
            top_item: None,
            scroll_to: None,
            pending: None,
            keyboard_moved: false,
            shown: false,
        }
    }

    fn headers(&self) -> bool {
        self.view != View::RecentlyAdded
    }

    /// Entries of the "On this day" strip, recomputed only when the items or the date change.
    fn on_this_day(&mut self, today: NaiveDate) -> &[OnThisDay] {
        if self
            .otd
            .as_ref()
            .is_none_or(|(g, d, _)| *g != self.generation || *d != today)
        {
            self.otd = Some((self.generation, today, on_this_day(&self.items, today)));
        }
        self.otd.as_ref().map_or(&[], |o| o.2.as_slice())
    }

    pub(super) fn filter(&self, kind: Option<MediaKind>) -> TimelineFilter {
        TimelineFilter {
            folder: match self.view {
                View::Folder(id) => Some(id),
                _ => None,
            },
            kind,
            favorites_only: self.view == View::Favorites,
            album: match self.view {
                View::Album(id) => Some(id),
                _ => None,
            },
            order: match (self.view, self.newest_first) {
                (View::RecentlyAdded, _) => TimelineOrder::AddedDesc,
                (_, true) => TimelineOrder::TakenDesc,
                (_, false) => TimelineOrder::TakenAsc,
            },
        }
    }

    /// Replaces the items; keeps the selection on the same file.
    pub(super) fn set_items(&mut self, items: Vec<TimelineItem>) {
        if self.loaded && items == *self.items {
            return; // a reload that found nothing new: keep layout and scroll position
        }
        let prev = self
            .selected
            .and_then(|i| self.items.get(i))
            .map(|it| it.file_id);
        self.items = Arc::new(items);
        self.generation += 1;
        self.dates = date_index(&self.items);
        self.selected = prev.and_then(|f| self.items.iter().position(|it| it.file_id == f));
        // Several selected items stay selected by file id; those that are gone are dropped.
        self.selected = self
            .sel
            .rebase(self.items.iter().map(|it| it.file_id), self.selected);
        self.top_item = None;
        self.loaded = true;
    }

    /// Takes a file out of the list (see [`Self::remove_files`]).
    #[cfg(test)]
    pub(super) fn remove_file(&mut self, file_id: i64) {
        self.remove_files(&HashSet::from([file_id]));
    }

    /// Takes files out of the list (they no longer belong to the view, e.g. they were
    /// unfavorited in Favorites or taken out of the album): one pass over the list, one rebuild
    /// of the date index and one fix-up of the selection and scroll position however many go.
    /// The selection stays on the same file, or moves to the one after a removed primary; the
    /// scroll position stays on the photo at the top. The list is copied only if the viewer
    /// holds a snapshot of it.
    pub(super) fn remove_files(&mut self, ids: &HashSet<i64>) {
        if ids.is_empty() || !self.items.iter().any(|it| ids.contains(&it.file_id)) {
            return;
        }
        // How many removed items precede index `at` (where an index lands after the removal).
        let before = |items: &[TimelineItem], at: usize| {
            items[..at.min(items.len())]
                .iter()
                .filter(|it| ids.contains(&it.file_id))
                .count()
        };
        let primary = self.selected.filter(|&s| s < self.items.len());
        let primary_removed = primary.is_some_and(|s| ids.contains(&self.items[s].file_id));
        let sel_shift = primary.map(|s| before(&self.items, s));
        let top = self.top_item.filter(|&t| t > 0);
        let top_shift = top.map(|t| before(&self.items, t));
        Arc::make_mut(&mut self.items).retain(|it| !ids.contains(&it.file_id));
        self.sel.remove_all(ids);
        self.generation += 1;
        self.dates = date_index(&self.items);
        let len = self.items.len();
        self.selected = match (primary, sel_shift) {
            (Some(s), Some(shift)) => {
                let moved = s - shift;
                if !primary_removed {
                    Some(moved)
                } else if self.sel.len() > 0 {
                    // Others stay selected: the first of them takes over.
                    self.items
                        .iter()
                        .position(|it| self.sel.contains(it.file_id))
                } else {
                    (len > 0).then(|| moved.min(len - 1))
                }
            }
            _ => None,
        };
        if self.selected.is_some() {
            self.keyboard_moved = true;
        }
        if let Some((t, shift)) = top.zip(top_shift)
            && len > 0
        {
            self.pending = Some(((t - shift).min(len - 1), false));
        }
        self.top_item = None;
    }

    /// Before `new` replaces the items: remembers which photo is at the top of the viewport so
    /// the scroll position follows it (or the nearest position if it is gone). Nothing is done
    /// at the very top, where new items should simply show up.
    fn anchor_scroll(&mut self, new: &[TimelineItem]) {
        let Some(top) = self.top_item.filter(|&t| t > 0) else {
            return;
        };
        let Some(old) = self.items.get(top) else {
            return;
        };
        if new.is_empty() || new == self.items.as_slice() {
            return;
        }
        let at = new
            .iter()
            .position(|it| it.file_id == old.file_id)
            .unwrap_or_else(|| top.min(new.len() - 1));
        self.pending = Some((at, false));
    }

    /// True when the loaded items belong to the view that is selected now.
    fn matches(&self, filter: &TimelineFilter) -> bool {
        self.loaded_filter.as_ref() == Some(filter)
    }

    fn ensure_layout(&mut self, width: f32) {
        let headers = self.headers();
        let w = width.floor().max(1.0) as i32;
        let key = (
            self.generation,
            self.group,
            headers,
            w,
            self.tile_size.to_bits(),
        );
        if self.layout_key == Some(key) {
            return;
        }
        let gkey = (self.generation, self.group, headers);
        if self.groups_key != Some(gkey) {
            self.groups = compute_groups(&self.items, self.group, headers);
            self.groups_key = Some(gkey);
        }
        // Resizing or regrouping keeps what is at the top of the view in place.
        let same_items = self.layout_key.is_some_and(|k| k.0 == self.generation);
        if same_items
            && self.pending.is_none()
            && let Some(a) = self.top_item
        {
            self.pending = Some((a, false));
        }
        self.layout = Some(Layout::build(
            &self.groups,
            w as f32,
            self.tile_size,
            headers,
        ));
        self.layout_key = Some(key);
    }

    /// Scroll offset for a requested jump, once a layout exists.
    fn resolve_pending(&mut self) {
        let (Some((item, to_header)), Some(layout)) = (self.pending, &self.layout) else {
            return;
        };
        self.pending = None;
        if self.items.is_empty() {
            return;
        }
        let item = item.min(self.items.len() - 1);
        let top = if to_header {
            layout.jump_top(&self.groups, item)
        } else {
            layout.reveal_span(item).0
        };
        self.scroll_to = Some(top);
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing helpers shared with the search results grid
// ---------------------------------------------------------------------------------------------

/// Play triangle and label in the bottom left corner of a tile.
pub(super) fn paint_play_badge(painter: &egui::Painter, rect: egui::Rect, label: String) {
    let galley = painter.layout_no_wrap(label, egui::FontId::proportional(13.0), Color32::WHITE);
    let h = galley.size().y;
    let size = Vec2::new(galley.size().x + h + 4.0, h);
    let min = rect.left_bottom() + Vec2::new(6.0, -6.0 - h);
    let bg = egui::Rect::from_min_size(min, size).expand(3.0);
    painter.rect_filled(bg, 4.0, Color32::from_black_alpha(170));
    let tri = [
        min + Vec2::new(1.0, 2.0),
        min + Vec2::new(1.0, h - 2.0),
        min + Vec2::new(h - 3.0, h / 2.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        tri.to_vec(),
        Color32::WHITE,
        egui::Stroke::NONE,
    ));
    painter.galley(min + Vec2::new(h + 2.0, 0.0), galley, Color32::WHITE);
}

/// A five-pointed star centred on `c`, gold when `filled`, otherwise just an outline. (egui
/// fills convex shapes only, so the star is drawn as a fan of triangles.)
pub(super) fn paint_star(painter: &egui::Painter, c: egui::Pos2, radius: f32, filled: bool) {
    let gold = Color32::from_rgb(255, 200, 40);
    let pts: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { radius } else { radius * 0.42 };
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    if filled {
        for i in 0..10 {
            painter.add(egui::Shape::convex_polygon(
                vec![c, pts[i], pts[(i + 1) % 10]],
                gold,
                egui::Stroke::NONE,
            ));
        }
    }
    let mut outline = pts.clone();
    outline.push(pts[0]);
    painter.add(egui::Shape::line(
        outline,
        egui::Stroke::new(
            1.0,
            if filled {
                Color32::from_rgb(160, 110, 0)
            } else {
                gold
            },
        ),
    ));
}

/// What is drawn on a tile besides its thumbnail.
#[derive(Default)]
pub(super) struct TileMarks {
    /// Video badge text (a time).
    pub badge: Option<String>,
    pub favorite: bool,
    pub selected: bool,
    pub hovered: bool,
}

/// Thumbnail, optional video badge, favorite star and selection / hover outline.
pub(super) fn paint_tile(
    ui: &egui::Ui,
    thumbs: &mut ThumbCache,
    key: &ThumbKey,
    rect: egui::Rect,
    marks: TileMarks,
) {
    let TileMarks {
        badge,
        favorite,
        selected,
        hovered,
    } = marks;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    let failed = thumbs.failed(key);
    if let Some(tex) = thumbs.get(key) {
        let size = tex.size_vec2();
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let r = egui::Rect::from_center_size(rect.center(), size * scale);
        painter.image(
            tex.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else if rect.width() > 110.0 {
        let text = if failed { "preview unavailable" } else { "…" };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(13.0),
            ui.visuals().weak_text_color(),
        );
    }
    if let Some(label) = badge {
        paint_play_badge(&painter, rect, label);
    }
    if favorite && rect.width() > 60.0 {
        let c = rect.right_top() + Vec2::new(-15.0, 15.0);
        painter.circle_filled(c, 12.0, Color32::from_black_alpha(150));
        paint_star(&painter, c, 8.0, true);
    }
    if selected {
        painter.rect_stroke(
            rect.shrink(1.0),
            6.0,
            egui::Stroke::new(3.0, ui.visuals().selection.bg_fill),
            egui::StrokeKind::Inside,
        );
    } else if hovered {
        painter.rect_stroke(
            rect.shrink(1.0),
            6.0,
            egui::Stroke::new(1.5, ui.visuals().widgets.hovered.bg_stroke.color),
            egui::StrokeKind::Inside,
        );
    }
}

fn file_name(item: &TimelineItem) -> String {
    item.path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// App wiring
// ---------------------------------------------------------------------------------------------

impl App {
    /// True while the central area shows search results instead of the current view.
    pub(super) fn searching(&self) -> bool {
        self.current.is_some() || !self.query.trim().is_empty()
    }

    /// Leaves search results (back to the current view).
    pub(super) fn clear_search(&mut self) {
        self.query.clear();
        self.query_changed_at = None;
        self.current = None;
        self.awaiting = None;
        self.results = Arc::default();
        self.result_label.clear();
        self.result_error = None;
        self.selected = None;
        self.selected_t = None;
    }

    pub(super) fn set_view(&mut self, view: View) {
        if self.tl.view != view {
            self.tl.view = view;
            self.tl.selected = None;
            self.tl.scroll_to = Some(0.0);
            self.tl.pending = None;
        }
        if self.searching() {
            self.clear_search();
        }
    }

    /// Per-frame: starts reloads when what is shown or the library changed, applies results.
    pub(super) fn update_timeline(&mut self) {
        if let Some(r) = self.tl.worker.poll()
            && self.tl.awaiting.as_ref().map(|a| a.0) == Some(r.id)
        {
            self.tl.awaiting = None;
            self.tl.requested_at = Instant::now();
            let same_view = self.tl.loaded_filter.as_ref() == Some(&r.filter);
            match r.items {
                Ok(mut items) => {
                    if r.filter.favorites_only {
                        // A list read before an unfavorite was saved still has the item.
                        items.retain(|it| self.fav_marks.get(&it.file_id).is_none_or(|m| m.value));
                    }
                    if same_view {
                        // A reload of what is shown: stay on the same photo.
                        self.tl.anchor_scroll(&items);
                    } else {
                        self.tl.scroll_to = Some(0.0);
                        self.tl.selected = None;
                        self.tl.pending = None;
                    }
                    self.tl.loaded_filter = Some(r.filter);
                    self.tl.error = None;
                    self.tl.failed = None;
                    self.tl.set_items(items);
                }
                Err(e) => {
                    if !same_view {
                        // Never keep showing another view's items.
                        self.tl.selected = None;
                        self.tl.pending = None;
                        self.tl.scroll_to = Some(0.0);
                        self.tl.loaded_filter = Some(r.filter.clone());
                        self.tl.set_items(Vec::new());
                    }
                    self.tl.error = Some(e);
                    self.tl.loaded = true;
                    self.tl.failed = Some((r.filter, Instant::now()));
                    self.tl.requested = None; // allow a retry
                }
            }
        }
        if let View::Folder(id) = self.tl.view
            && !self.snapshot.folders.is_empty()
            && !self.snapshot.folders.iter().any(|f| f.id == id)
        {
            self.set_view(View::Timeline);
        }
        let filter = self.tl.filter(self.kind);
        let jobs = self.lib.jobs_finished();
        // Files are being added: reload now and then. Embedding adds no files, so it does not
        // trigger reloads; the reload when the job finishes picks up its results.
        let growing = matches!(
            self.status.activity,
            Activity::Scanning { .. } | Activity::ReadingDates { .. }
        );
        let backing_off = self
            .tl
            .failed
            .as_ref()
            .is_some_and(|(f, at)| *f == filter && at.elapsed() < RETRY_AFTER);
        let stale = self.tl.requested.as_ref() != Some(&(filter.clone(), jobs)) && !backing_off;
        let live = growing && self.tl.requested_at.elapsed() > LIVE_RELOAD;
        if self.tl.awaiting.is_none() {
            if stale || live {
                let id = self.tl.worker.submit(filter.clone());
                self.tl.awaiting = Some((id, filter.clone()));
                self.tl.requested = Some((filter, jobs));
                self.tl.requested_at = Instant::now();
            }
        } else if stale {
            // The view or library changed while a load is in flight: load again once it is
            // done. (A live tick during a load does nothing.)
            self.tl.requested = None;
        }
        if backing_off {
            self.ctx.request_repaint_after(RETRY_AFTER);
        }
        if growing {
            self.ctx.request_repaint_after(LIVE_RELOAD);
        }
    }

    #[cfg(test)]
    pub(super) fn inject_timeline(&mut self, items: Vec<TimelineItem>) {
        let filter = self.tl.filter(self.kind);
        self.tl.requested = Some((filter.clone(), self.lib.jobs_finished()));
        self.tl.loaded_filter = Some(filter);
        self.tl.set_items(items);
    }

    /// Sidebar: the library views.
    pub(super) fn view_selector(&mut self, ui: &mut egui::Ui) {
        let mut pick = None;
        if ui
            .selectable_label(self.tl.view == View::Timeline, "Timeline")
            .clicked()
        {
            pick = Some(View::Timeline);
        }
        if ui
            .selectable_label(self.tl.view == View::RecentlyAdded, "Recently added")
            .clicked()
        {
            pick = Some(View::RecentlyAdded);
        }
        if ui
            .selectable_label(self.tl.view == View::Favorites, "Favorites")
            .clicked()
        {
            pick = Some(View::Favorites);
        }
        if let Some(v) = pick {
            self.set_view(v);
        }
    }

    /// Sidebar: years and months of the current view; clicking scrolls the grid there.
    pub(super) fn date_navigator(&mut self, ui: &mut egui::Ui) {
        if self.tl.view == View::RecentlyAdded || self.tl.dates.is_empty() {
            return;
        }
        ui.separator();
        ui.label(RichText::new("Dates").strong());
        let mut jump = None;
        let mut toggle = None;
        egui::ScrollArea::vertical()
            .id_salt("dates")
            .max_height((ui.available_height() - 44.0).max(80.0))
            .show(ui, |ui| {
                for y in &self.tl.dates {
                    let label = format!("{} ({})", y.year, thousands(y.count));
                    if ui
                        .selectable_label(self.tl.top_year == Some(y.year), label)
                        .clicked()
                    {
                        jump = Some(y.first);
                        toggle = Some(y.year);
                    }
                    if self.tl.expanded.contains(&y.year) {
                        for m in &y.months {
                            let name = chrono::Month::try_from(m.month as u8)
                                .map(|m| m.name())
                                .unwrap_or("");
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                if ui
                                    .selectable_label(
                                        false,
                                        format!("{name} ({})", thousands(m.count)),
                                    )
                                    .clicked()
                                {
                                    jump = Some(m.first);
                                }
                            });
                        }
                    }
                }
            });
        if let Some(y) = toggle
            && !self.tl.expanded.remove(&y)
        {
            self.tl.expanded.insert(y);
        }
        if let Some(i) = jump {
            self.tl.pending = Some((i, true));
        }
    }

    fn timeline_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if self.tl.awaiting.is_some() {
                ui.spinner();
            }
            if self.tl.headers() {
                ui.label("Group by");
                for (g, name) in [
                    (Group::Day, "Day"),
                    (Group::Month, "Month"),
                    (Group::Year, "Year"),
                ] {
                    ui.selectable_value(&mut self.tl.group, g, name);
                }
                ui.separator();
                ui.label("Sort");
                ui.selectable_value(&mut self.tl.newest_first, true, "Newest first");
                ui.selectable_value(&mut self.tl.newest_first, false, "Oldest first");
                ui.separator();
            }
            ui.add(
                egui::Slider::new(&mut self.tl.tile_size, 96.0..=320.0)
                    .show_value(false)
                    .text("Tile size"),
            );
            ui.label(RichText::new(format!("{} items", thousands(self.tl.items.len()))).weak());
            if let Some(e) = &self.tl.error {
                ui.colored_label(Color32::from_rgb(230, 120, 90), e);
            }
        });
        ui.add_space(4.0);
    }

    /// The album grid for the current view.
    pub(super) fn timeline_view(&mut self, ui: &mut egui::Ui) {
        // Until items for the selected view arrive, show a spinner rather than the previous
        // view's items laid out under the new view's rules.
        let wanted = self.tl.filter(self.kind);
        if !self.tl.loaded || !self.tl.matches(&wanted) {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.spinner();
            });
            return;
        }
        if self.tl.view == View::Timeline {
            // "Today" changes at midnight, whether or not anything else happens by then.
            let now = chrono::Local::now().naive_local();
            ui.ctx()
                .request_repaint_after(until_next_midnight(now) + Duration::from_secs(1));
            self.on_this_day_strip(ui);
        }
        if let View::Album(id) = self.tl.view {
            self.album_header(ui, id);
        }
        self.selection_bar_ui(ui);
        self.timeline_toolbar(ui);
        if self.tl.items.is_empty() {
            if self.tl.awaiting.is_none() {
                if matches!(self.tl.view, View::Album(_)) {
                    self.empty_state(
                        ui,
                        "This album is empty",
                        "Select photos or videos and use “Add to album…”.",
                    );
                } else if self.tl.view == View::Favorites {
                    self.empty_state(
                        ui,
                        "No favorites yet",
                        "Select a photo or video and press F to add it here.",
                    );
                } else {
                    self.empty_state(
                        ui,
                        "Nothing to show yet",
                        "Photos and videos appear here as soon as a folder has been scanned.",
                    );
                }
            }
            return;
        }
        self.tl.shown = true;
        let width = ui.available_width() - 12.0; // leave room for the scroll bar
        self.tl.ensure_layout(width);
        self.tl.resolve_pending();
        let mut area = egui::ScrollArea::vertical()
            .id_salt("timeline")
            .auto_shrink([false, false]);
        if let Some(y) = self.tl.scroll_to.take() {
            area = area.vertical_scroll_offset(y);
        }
        let tl = &self.tl;
        let marks = &self.fav_marks;
        let thumbs = &mut self.thumbs;
        let layout = tl.layout.as_ref().expect("layout was just built");
        let albums = &self.albums;
        let in_album = matches!(tl.view, View::Album(_));
        let mut clicked: Option<(usize, bool, ClickMods)> = None;
        let mut right_clicked = None;
        let mut menu = None;
        let mut top_item = None;
        let out = area.show_viewport(ui, |ui, viewport| {
            let (origin, _) =
                ui.allocate_exact_size(Vec2::new(width, layout.total), Sense::hover());
            let range = layout.visible(viewport.min.y, viewport.max.y);
            if let Some(&row) = layout.rows.get(range.start) {
                top_item = Some(row.first_item());
            }
            for r in range {
                let top = origin.min.y + layout.tops[r];
                match layout.rows[r] {
                    Row::Header { group, .. } => {
                        let g = tl.groups[group];
                        let text = header_text(tl.group, g.key, g.end - g.start);
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(origin.min.x, top),
                            Vec2::new(width, HEADER_H),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(text).strong().size(16.0));
                        });
                    }
                    Row::Tiles { start, end } => {
                        for i in start..end {
                            let x = origin.min.x + (i - start) as f32 * (layout.tile + GAP);
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(x, top),
                                Vec2::splat(layout.tile),
                            );
                            let item = &tl.items[i];
                            let is_selected =
                                tl.selected == Some(i) || tl.sel.contains(item.file_id);
                            let resp = ui.interact(
                                rect,
                                ui.make_persistent_id(("tile", i)),
                                Sense::click(),
                            );
                            if ui.is_rect_visible(rect) {
                                let badge = (item.kind == MediaKind::Video)
                                    .then(|| fmt_time(item.duration.unwrap_or(0.0)));
                                paint_tile(
                                    ui,
                                    thumbs,
                                    &thumb_key(item),
                                    rect,
                                    TileMarks {
                                        badge,
                                        favorite: marked(marks, item.file_id, item.favorite),
                                        selected: is_selected,
                                        hovered: resp.hovered(),
                                    },
                                );
                            }
                            let name = file_name(item);
                            let label = format!("Item {name}");
                            resp.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::Role::Button,
                                    true,
                                    is_selected,
                                    &label,
                                )
                            });
                            if resp.double_clicked() || resp.clicked() {
                                let mods = ClickMods::of_click(ui.ctx());
                                clicked = Some((i, resp.double_clicked() && mods.is_plain(), mods));
                            }
                            if resp.secondary_clicked() {
                                right_clicked = Some(i);
                            }
                            resp.context_menu(|ui| {
                                if let Some(m) = albums::tile_menu(ui, albums, in_album) {
                                    menu = Some((i, m));
                                }
                            });
                            resp.on_hover_text(format!("{name}\n{}", fmt_taken(item)));
                        }
                    }
                }
            }
        });
        self.tl.top_item = top_item;
        self.tl.top_year = top_item
            .and_then(|i| self.tl.items.get(i))
            .map(|it| date_of(it.taken_at).year());
        if let Some(i) = self.tl.selected.filter(|_| self.tl.keyboard_moved) {
            self.tl.keyboard_moved = false;
            if let Some(layout) = &self.tl.layout {
                let (top, h) = layout.reveal_span(i);
                self.tl.scroll_to = reveal(
                    top,
                    h,
                    out.inner_rect.height(),
                    out.state.clamped_offset().y,
                );
                if self.tl.scroll_to.is_some() {
                    ui.ctx().request_repaint();
                }
            }
        }
        if self.tl.pending.is_some() {
            ui.ctx().request_repaint();
        }
        if let Some((i, open, mods)) = clicked {
            let items = Arc::clone(&self.tl.items);
            self.tl.selected = self.tl.sel.click(
                self.tl.selected,
                items.len(),
                &|k| items[k].file_id,
                i,
                mods,
            );
            if open {
                self.open_timeline_item(i);
            }
        }
        if let Some(i) = right_clicked {
            self.select_for_menu(i);
        }
        if let Some((i, m)) = menu {
            self.apply_tile_menu(i, m);
        }
        // Applying a scroll offset needs one more frame to take effect.
        if self.tl.scroll_to.is_some() {
            ui.ctx().request_repaint();
        }
    }

    /// Today's date; tests can pin it.
    pub(super) fn today(&self) -> NaiveDate {
        #[cfg(test)]
        if let Some(d) = self.today_override {
            return d;
        }
        chrono::Local::now().date_naive()
    }

    /// A strip of photos taken on today's date in earlier years, above the timeline.
    fn on_this_day_strip(&mut self, ui: &mut egui::Ui) {
        let today = self.today();
        let entries: Vec<OnThisDay> = self.tl.on_this_day(today).to_vec();
        if entries.is_empty() {
            return;
        }
        ui.label(RichText::new("On this day").strong().size(16.0));
        const SIZE: f32 = 88.0;
        let mut open = None;
        egui::ScrollArea::horizontal()
            .id_salt("on-this-day")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (pos, e) in entries.iter().enumerate() {
                        let item = &self.tl.items[e.index];
                        ui.vertical(|ui| {
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::splat(SIZE), Sense::click());
                            if ui.is_rect_visible(rect) {
                                let badge = (item.kind == MediaKind::Video)
                                    .then(|| fmt_time(item.duration.unwrap_or(0.0)));
                                paint_tile(
                                    ui,
                                    &mut self.thumbs,
                                    &thumb_key(item),
                                    rect,
                                    TileMarks {
                                        badge,
                                        favorite: marked(
                                            &self.fav_marks,
                                            item.file_id,
                                            item.favorite,
                                        ),
                                        hovered: resp.hovered(),
                                        ..Default::default()
                                    },
                                );
                            }
                            let name = file_name(item);
                            let label = format!("On this day {name}");
                            resp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Button, true, &label)
                            });
                            if resp.clicked() {
                                open = Some(pos);
                            }
                            resp.on_hover_text(format!("{name}\n{}", fmt_taken(item)));
                            ui.label(RichText::new(years_ago_text(e.years_ago)).small());
                        });
                    }
                });
            });
        ui.add_space(6.0);
        if let Some(pos) = open {
            let idx: Vec<usize> = entries.iter().map(|e| e.index).collect();
            self.open_viewer_subset(&idx, pos);
        }
    }

    fn open_timeline_item(&mut self, i: usize) {
        if i < self.tl.items.len() {
            self.open_viewer_timeline(i);
        }
    }

    /// Keys for the grid (same rules as for search results): arrows, Shift+arrows,
    /// Ctrl/Cmd+A, Enter and F.
    pub(super) fn handle_timeline_keys(&mut self, ctx: &egui::Context, grid_was_shown: bool) {
        if !grid_was_shown || self.tl.items.is_empty() || self.keys_blocked(ctx) {
            return;
        }
        if take_plain_key(ctx, egui::Key::F) {
            if self.tl.sel.is_multi() {
                self.toggle_favorite_of_selection();
            } else if let Some(item) = self.tl.selected.and_then(|i| self.tl.items.get(i)) {
                let id = item.file_id;
                let fav = !marked(&self.fav_marks, id, item.favorite);
                self.set_favorite(id, fav);
            }
        }
        let input = selection::read_nav_keys(ctx);
        let items = Arc::clone(&self.tl.items);
        let len = items.len();
        let layout = self.tl.layout.as_ref();
        let before = self.tl.selected;
        let next = selection::apply_keys(
            &input,
            &mut self.tl.sel,
            before,
            len,
            &|i| items[i].file_id,
            |sel, n| {
                // Nothing (valid) selected: any arrow picks the first item.
                let Some(cur) = sel.filter(|&i| i < len) else {
                    return Some(0);
                };
                Some(layout.map_or(cur, |l| l.step(cur, len, n)))
            },
        );
        if next != before {
            self.tl.selected = next;
            self.tl.keyboard_moved = true;
        }
        if input.enter
            && let Some(i) = self.tl.selected
        {
            self.open_timeline_item(i);
        }
    }

    pub(super) fn timeline_detail(&mut self, ui: &mut egui::Ui, i: usize) {
        let item = self.tl.items[i].clone();
        ui.add_space(6.0);
        let key = thumb_key(&item);
        let w = ui.available_width();
        if let Some(tex) = self.thumbs.get(&key) {
            let size = tex.size_vec2();
            let scale = (w / size.x).min(320.0 / size.y);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        } else {
            ui.allocate_exact_size(Vec2::new(w, 200.0), Sense::hover());
        }
        ui.add_space(6.0);
        let name = file_name(&item);
        ui.label(RichText::new(&name).strong());
        ui.label(
            RichText::new(
                item.path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
            .small()
            .weak(),
        );
        ui.add_space(4.0);
        ui.label(fmt_taken(&item));
        if let Some(c) = &item.camera {
            ui.label(RichText::new(c).small());
        }
        let mut facts = Vec::new();
        if let (Some(w), Some(h)) = (item.width, item.height) {
            facts.push(format!("{w}×{h}"));
        }
        if let Some(d) = item.duration {
            facts.push(format!("length {}", fmt_time(d)));
        }
        if !facts.is_empty() {
            ui.label(RichText::new(facts.join(" · ")).small());
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open").clicked()
                && let Err(e) = platform::open(&item.path)
            {
                self.notify_error(format!("{e:#}"));
            }
            if ui.button("Show in folder").clicked()
                && let Err(e) = platform::reveal(&item.path)
            {
                self.notify_error(format!("{e:#}"));
            }
            if let Some(item_id) = item.item_id.filter(|_| self.snapshot.model_installed)
                && ui.button("Find similar").clicked()
            {
                self.tl.selected = None;
                self.query.clear();
                self.submit(Query::Similar {
                    item_id,
                    label: name.clone(),
                });
            }
            let fav = marked(&self.fav_marks, item.file_id, item.favorite);
            let label = if fav { "Unfavorite" } else { "Favorite" };
            if ui.button(label).clicked() {
                self.set_favorite(item.file_id, !fav);
            }
            if let Some(c) = albums::album_menu(ui, &self.albums) {
                self.add_to_album_choice(vec![item.file_id], c);
            }
            if let View::Album(album) = self.tl.view
                && ui.button("Remove from album").clicked()
            {
                // The grid drops it once the library confirms.
                self.lib.queue_album(AlbumOp::Remove {
                    album,
                    files: vec![item.file_id],
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
            .and_utc()
            .timestamp()
    }

    fn item(taken_at: i64) -> TimelineItem {
        TimelineItem {
            file_id: taken_at,
            item_id: None,
            path: format!("/p/{taken_at}.jpg").into(),
            kind: MediaKind::Image,
            mtime: 0,
            taken_at,
            date_source: DateSource::Exif,
            duration: None,
            width: None,
            height: None,
            favorite: false,
            camera: None,
        }
    }

    #[test]
    fn keys_split_at_midnight_and_year_end() {
        let last = ts(2019, 12, 31, 23, 59, 59);
        let first = ts(2020, 1, 1, 0, 0, 0);
        for g in [Group::Day, Group::Month, Group::Year] {
            assert_ne!(group_key(last, g), group_key(first, g), "{g:?}");
        }
        assert_eq!(
            group_key(ts(2019, 7, 4, 0, 0, 0), Group::Day),
            group_key(ts(2019, 7, 4, 23, 59, 59), Group::Day)
        );
        assert_eq!(
            group_key(ts(2019, 7, 1, 0, 0, 0), Group::Month),
            group_key(ts(2019, 7, 31, 23, 59, 59), Group::Month)
        );
        assert_ne!(
            group_key(ts(2019, 7, 31, 23, 59, 59), Group::Month),
            group_key(ts(2019, 8, 1, 0, 0, 0), Group::Month)
        );
        // Leap day is its own day and belongs to February.
        let leap = ts(2020, 2, 29, 12, 0, 0);
        assert_ne!(
            group_key(leap, Group::Day),
            group_key(ts(2020, 3, 1, 0, 0, 0), Group::Day)
        );
        assert_eq!(
            group_key(leap, Group::Month),
            group_key(ts(2020, 2, 1, 0, 0, 0), Group::Month)
        );
        assert_eq!(
            group_key(ts(2020, 3, 1, 0, 0, 0), Group::Day) - group_key(leap, Group::Day),
            1
        );
    }

    #[test]
    fn headers_read_naturally() {
        let t = ts(2019, 7, 6, 10, 0, 0);
        assert_eq!(
            header_text(Group::Day, group_key(t, Group::Day), 12),
            "Saturday 6 July 2019 · 12"
        );
        assert_eq!(
            header_text(Group::Month, group_key(t, Group::Month), 132),
            "July 2019 · 132"
        );
        assert_eq!(
            header_text(Group::Year, group_key(t, Group::Year), 1204),
            "2019 · 1,204"
        );
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn groups_follow_display_order() {
        let items = vec![
            item(ts(2020, 1, 1, 0, 0, 0)),
            item(ts(2019, 12, 31, 23, 59, 59)),
            item(ts(2019, 12, 1, 8, 0, 0)),
            item(ts(2019, 11, 30, 8, 0, 0)),
        ];
        let g = compute_groups(&items, Group::Month, true);
        let spans: Vec<_> = g.iter().map(|g| (g.start, g.end)).collect();
        assert_eq!(spans, [(0, 1), (1, 3), (3, 4)]);
        assert_eq!(compute_groups(&items, Group::Year, true).len(), 2);
        let flat = compute_groups(&items, Group::Day, false);
        assert_eq!(flat.len(), 1);
        assert_eq!((flat[0].start, flat[0].end), (0, 4));
        assert!(compute_groups(&[], Group::Day, true).is_empty());
    }

    #[test]
    fn layout_offsets_and_visibility() {
        // Two sections of 5 and 2 items; width fits 3 tiles of 100 (gap 8): 3*100 + 2*8 = 316.
        let groups = [
            GroupSpan {
                key: 1,
                start: 0,
                end: 5,
            },
            GroupSpan {
                key: 2,
                start: 5,
                end: 7,
            },
        ];
        let l = Layout::build(&groups, 316.0, 100.0, true);
        assert_eq!(l.cols, 3);
        assert!((l.tile - 100.0).abs() < 1e-3);
        assert_eq!(l.rows.len(), 5);
        assert_eq!(l.group_row, [0, 3]);
        let row = 100.0 + GAP;
        let expect = [
            0.0,
            HEADER_H,
            HEADER_H + row,
            HEADER_H + 2.0 * row + SECTION_GAP,
            2.0 * HEADER_H + 2.0 * row + SECTION_GAP,
        ];
        for (a, b) in l.tops.iter().zip(expect) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        assert!((l.total - (expect[4] + row + SECTION_GAP)).abs() < 1e-3);
        // Visible rows: any row intersecting the span.
        assert_eq!(l.visible(0.0, 10.0), 0..1);
        assert_eq!(l.visible(0.0, HEADER_H + 1.0), 0..2);
        assert_eq!(l.visible(HEADER_H + 5.0, HEADER_H + 6.0), 1..2);
        assert_eq!(l.visible(expect[2] + 1.0, expect[4] - 1.0), 2..4);
        assert_eq!(l.visible(1000.0, 1100.0).start, 4);
        // Item to row.
        assert_eq!(l.row_of_item(0), 1);
        assert_eq!(l.row_of_item(2), 1);
        assert_eq!(l.row_of_item(3), 2);
        assert_eq!(l.row_of_item(5), 4);
        // Revealing the first tile row of a section includes its header.
        assert_eq!(l.reveal_span(5).0, expect[3]);
        assert_eq!(l.reveal_span(3).0, expect[2]);
        // No headers: tiles only.
        let flat = Layout::build(
            &[GroupSpan {
                key: 0,
                start: 0,
                end: 7,
            }],
            316.0,
            100.0,
            false,
        );
        assert_eq!(flat.rows.len(), 3);
        assert_eq!(flat.tops[0], 0.0);
        assert!(
            Layout::build(&[], 300.0, 100.0, true)
                .visible(0.0, 50.0)
                .is_empty()
        );
    }

    #[test]
    fn visible_rows_stay_cheap_for_huge_libraries() {
        let n = 200_000;
        let items: Vec<_> = (0..n)
            .map(|i| item(1_500_000_000 - i as i64 * 3000))
            .collect();
        let groups = compute_groups(&items, Group::Month, true);
        let l = Layout::build(&groups, 1000.0, 180.0, true);
        let y = l.total * 0.5;
        let r = l.visible(y, y + 800.0);
        assert!(!r.is_empty() && r.len() < 20, "{r:?}");
        let first = l.rows[r.start].first_item();
        assert!(first > 0 && first < n);
    }

    /// Sections of 5 and 2 items with 3 columns: rows are
    /// header, [0 1 2], [3 4], header, [5 6].
    fn two_sections() -> (Vec<GroupSpan>, Layout) {
        let groups = vec![
            GroupSpan {
                key: 1,
                start: 0,
                end: 5,
            },
            GroupSpan {
                key: 2,
                start: 5,
                end: 7,
            },
        ];
        let l = Layout::build(&groups, 316.0, 100.0, true);
        (groups, l)
    }

    #[test]
    fn arrows_follow_the_tile_rows_across_sections() {
        let (_, l) = two_sections();
        let n = 7;
        // Down across the boundary keeps the column.
        assert_eq!(l.step(3, n, Nav::Down), 5);
        assert_eq!(l.step(4, n, Nav::Down), 6);
        // Down into a shorter row clamps to its last tile (within a section and across).
        assert_eq!(l.step(2, n, Nav::Down), 4);
        assert_eq!(l.step(4, n, Nav::Down), 6);
        // Up across the boundary, from the short last row into the full row above... and from
        // a column that the short row above does not have.
        assert_eq!(l.step(5, n, Nav::Up), 3);
        assert_eq!(l.step(6, n, Nav::Up), 4);
        assert_eq!(l.step(3, n, Nav::Up), 0);
        // Nothing above the first row / below the last one.
        assert_eq!(l.step(1, n, Nav::Up), 1);
        assert_eq!(l.step(6, n, Nav::Down), 6);
        // Left and right walk the items in order, also across sections.
        assert_eq!(l.step(5, n, Nav::Left), 4);
        assert_eq!(l.step(4, n, Nav::Right), 5);
        assert_eq!(l.step(6, n, Nav::Right), 6);
        assert_eq!(l.step(0, n, Nav::Left), 0);
    }

    #[test]
    fn arrows_clamp_to_a_short_row_in_the_next_section() {
        // Sections of 4 and 1 item with 3 columns: [0 1 2], [3], header, [4].
        let groups = [
            GroupSpan {
                key: 1,
                start: 0,
                end: 4,
            },
            GroupSpan {
                key: 2,
                start: 4,
                end: 5,
            },
        ];
        let l = Layout::build(&groups, 316.0, 100.0, true);
        assert_eq!(l.step(2, 5, Nav::Down), 3);
        assert_eq!(l.step(3, 5, Nav::Down), 4);
        assert_eq!(l.step(4, 5, Nav::Up), 3);
    }

    #[test]
    fn jumps_go_to_the_header_only_for_section_starts() {
        let (groups, l) = two_sections();
        // Item 5 starts the second section: its header.
        assert_eq!(l.jump_top(&groups, 5), l.tops[3]);
        assert_eq!(l.jump_top(&groups, 0), l.tops[0]);
        // Item 4 is in the middle of a section (a month inside a year): its own row.
        assert_eq!(l.jump_top(&groups, 4), l.tops[2]);
        assert_eq!(l.jump_top(&groups, 1), l.tops[1]);
    }

    #[test]
    fn reload_keeps_the_photo_at_the_top_in_view() {
        let d = tempfile::tempdir().unwrap();
        let lib = crate::Library::open(
            crate::config::Paths::under(d.path()),
            Some(crate::model::registry::CLIP_B32.id),
        )
        .unwrap();
        let mut st = TimelineState::new(lib, &egui::Context::default());
        let items: Vec<_> = (0..20).map(|i| item(1_600_000_000 - i * 86_400)).collect();
        st.set_items(items.clone());
        let gen0 = st.generation;
        st.set_items(items.clone());
        assert_eq!(st.generation, gen0, "identical items keep the layout");
        // Scrolled to item 10; three newer photos arrive at the front.
        st.top_item = Some(10);
        let mut newer: Vec<_> = (1..=3).map(|i| item(1_600_000_000 + i * 86_400)).collect();
        newer.extend(items.clone());
        st.anchor_scroll(&newer);
        assert_eq!(st.pending, Some((13, false)));
        // The photo disappeared: the nearest position (clamped to the new end).
        st.pending = None;
        st.top_item = Some(19);
        st.anchor_scroll(&items[..5]);
        assert_eq!(st.pending, Some((4, false)));
        // At the very top nothing is anchored.
        st.pending = None;
        st.top_item = Some(0);
        st.anchor_scroll(&newer);
        assert_eq!(st.pending, None);
    }

    #[test]
    fn date_index_lists_newest_year_first() {
        let items = vec![
            item(ts(2021, 3, 1, 0, 0, 0)),
            item(ts(2020, 12, 31, 0, 0, 0)),
            item(ts(2020, 1, 5, 0, 0, 0)),
            item(ts(2020, 1, 4, 0, 0, 0)),
        ];
        let d = date_index(&items);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].year, d[0].count, d[0].first), (2021, 1, 0));
        assert_eq!((d[1].year, d[1].count, d[1].first), (2020, 3, 1));
        let months: Vec<_> = d[1].months.iter().map(|m| (m.month, m.count)).collect();
        assert_eq!(months, [(12, 1), (1, 2)]);
    }

    #[test]
    fn reveal_scrolls_minimally() {
        assert_eq!(reveal(100.0, 50.0, 400.0, 0.0), None);
        assert_eq!(reveal(500.0, 50.0, 400.0, 0.0), Some(150.0));
        assert_eq!(reveal(100.0, 50.0, 400.0, 300.0), Some(100.0));
    }

    #[test]
    fn on_this_day_lists_earlier_years_newest_first() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
        let items = vec![
            item(ts(2026, 7, 4, 9, 0, 0)),    // today, this year: not "earlier"
            item(ts(2019, 7, 4, 8, 0, 0)),    // 7 years ago
            item(ts(2019, 7, 5, 8, 0, 0)),    // next day
            item(ts(2020, 6, 4, 8, 0, 0)),    // other month
            item(ts(2024, 7, 4, 23, 59, 59)), // 2 years ago, late in the day
            item(ts(2025, 7, 4, 0, 0, 0)),    // 1 year ago, first second of the day
            item(ts(2019, 7, 4, 20, 0, 0)),   // same day as another 2019 photo
        ];
        let got = on_this_day(&items, today);
        assert_eq!(
            got,
            vec![
                OnThisDay {
                    index: 5,
                    years_ago: 1
                },
                OnThisDay {
                    index: 4,
                    years_ago: 2
                },
                OnThisDay {
                    index: 6,
                    years_ago: 7
                },
                OnThisDay {
                    index: 1,
                    years_ago: 7
                },
            ]
        );
        assert_eq!(years_ago_text(1), "1 year ago");
        assert_eq!(years_ago_text(7), "7 years ago");
        // A different day finds different photos; nothing matching gives an empty strip.
        let other = NaiveDate::from_ymd_opt(2026, 7, 5).unwrap();
        assert_eq!(on_this_day(&items, other).len(), 1);
        assert!(on_this_day(&items, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).is_empty());
        assert!(on_this_day(&[], today).is_empty());
    }

    #[test]
    fn on_this_day_is_capped_and_ignores_the_list_order() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
        // 40 earlier years on the same day, oldest first.
        let items: Vec<_> = (1980..2020).map(|y| item(ts(y, 7, 4, 12, 0, 0))).collect();
        let got = on_this_day(&items, today);
        assert_eq!(got.len(), ON_THIS_DAY_MAX);
        assert_eq!(items[got[0].index].taken_at, ts(2019, 7, 4, 12, 0, 0));
        assert_eq!(got[0].years_ago, 7);
        assert_eq!(got.last().unwrap().years_ago, 7 + 29);
    }

    #[test]
    fn leap_day_photos_show_on_28_february_in_years_without_one() {
        let leap = vec![
            item(ts(2020, 2, 29, 12, 0, 0)),
            item(ts(2021, 2, 28, 9, 0, 0)),
        ];
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        // 2026 has no 29 February: both photos belong to 28 February.
        let got = on_this_day(&leap, day(2026, 2, 28));
        assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [1, 0]);
        // 2028 does: 28 February is its own day, and 29 February matches only 29 February.
        let got = on_this_day(&leap, day(2028, 2, 28));
        assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [1]);
        let got = on_this_day(&leap, day(2028, 2, 29));
        assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [0]);
        assert!(on_this_day(&leap, day(2026, 3, 1)).is_empty());
    }

    #[test]
    fn dates_taken_from_the_file_time_are_not_memories() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
        let mut copied = item(ts(2019, 7, 4, 12, 0, 0));
        copied.date_source = DateSource::File;
        let mut video = item(ts(2018, 7, 4, 12, 0, 0));
        video.date_source = DateSource::Video;
        let items = vec![copied, video, item(ts(2017, 7, 4, 12, 0, 0))];
        let got = on_this_day(&items, today);
        assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn the_day_rolls_over_at_local_midnight() {
        let at = |h, m, s| {
            NaiveDate::from_ymd_opt(2026, 12, 31)
                .unwrap()
                .and_hms_opt(h, m, s)
                .unwrap()
        };
        assert_eq!(until_next_midnight(at(23, 59, 0)), Duration::from_secs(60));
        assert_eq!(
            until_next_midnight(at(0, 0, 0)),
            Duration::from_secs(86_400)
        );
    }

    #[test]
    fn removing_a_file_keeps_the_selection_sane() {
        let mut tl_items: Vec<TimelineItem> = (0..5).map(|i| item(1_000_000 + i)).collect();
        let mut st = state_for_test();
        st.set_items(std::mem::take(&mut tl_items));
        st.selected = Some(3);
        st.remove_file(1_000_001);
        assert_eq!(st.items.len(), 4);
        assert_eq!(st.selected, Some(2), "still the same file");
        assert_eq!(st.items[2].file_id, 1_000_003);
        // Removing the selected one selects the next; at the end, the new last one.
        st.remove_file(1_000_003);
        assert_eq!(st.selected, Some(2));
        assert_eq!(st.items[2].file_id, 1_000_004);
        st.remove_file(1_000_004);
        assert_eq!(st.selected, Some(1));
        st.remove_file(1_000_000);
        st.remove_file(1_000_002);
        assert_eq!(st.selected, None);
        assert!(st.items.is_empty());
        st.remove_file(42); // unknown: nothing happens
    }

    #[test]
    fn removing_many_files_fixes_selection_and_scroll_once() {
        let mut st = state_for_test();
        let items: Vec<_> = (0..50_000)
            .map(|i| {
                let mut it = item(1_000_000 + i);
                it.file_id = i;
                it
            })
            .collect();
        st.set_items(items);
        // Every even file goes; the selection is file 11.
        let gone: HashSet<i64> = (0..50_000).filter(|i| i % 2 == 0).collect();
        st.selected = Some(11);
        st.sel.only(11);
        st.top_item = Some(1_001);
        let generation = st.generation;
        st.remove_files(&gone);
        assert_eq!(st.items.len(), 25_000);
        assert_eq!(
            st.generation,
            generation + 1,
            "one rebuild, not one per file"
        );
        assert!(st.items.iter().all(|it| it.file_id % 2 == 1));
        // File 11 is now at index 5; the file at the top (1001, odd) is at 500.
        assert_eq!(st.selected, Some(5));
        assert!(st.sel.contains(11));
        assert_eq!(st.pending, Some((500, false)));
        // Removing the primary with others selected hands it to the first remaining one.
        let mut st = state_for_test();
        st.set_items(
            (0..10)
                .map(|i| {
                    let mut it = item(1_000_000 + i);
                    it.file_id = i;
                    it
                })
                .collect(),
        );
        st.selected = Some(2);
        st.sel.only(2);
        let ids = |v: &[i64]| v.iter().copied().collect::<HashSet<i64>>();
        st.sel.click(
            Some(2),
            10,
            &|k| k as i64,
            6,
            selection::ClickMods {
                toggle: true,
                range: false,
            },
        );
        st.sel.click(
            Some(6),
            10,
            &|k| k as i64,
            8,
            selection::ClickMods {
                toggle: true,
                range: false,
            },
        );
        st.selected = Some(8);
        st.remove_files(&ids(&[8, 2, 0]));
        // Items 1,3,4,5,6,7,9 remain; 6 is the first selected one left, at index 4.
        assert_eq!(st.selected, Some(4));
        assert_eq!(st.items[4].file_id, 6);
        assert_eq!(st.sel.len(), 1);
    }

    /// A timeline state without a real library behind it.
    fn state_for_test() -> TimelineState {
        crate::gpu::force_cpu(true);
        let d = tempfile::tempdir().unwrap();
        let lib = Library::open(
            crate::config::Paths::under(d.path()),
            Some(crate::model::registry::CLIP_B32.id),
        )
        .unwrap();
        TimelineState::new(lib, &egui::Context::default())
    }
}
