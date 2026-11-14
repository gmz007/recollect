//! Browsing: the timeline, recently added, favorites, per-folder and per-album views.
//!
//! Files are loaded in the background (`load`), grouped by capture day, month or year in
//! memory (`dates`) and laid out as rows (section headers and rows of tiles) with known
//! vertical offsets (`layout`). Only the rows inside the scroll viewport are drawn (`view`),
//! found by binary search, so very large libraries stay smooth. `memories` is the "On this
//! day" strip.

mod dates;
mod layout;
mod load;
mod memories;
mod view;

pub use dates::{DATE_FROM_FILE, YearEntry, date_index, fmt_date, thousands};
pub use layout::{GroupSpan, Layout, compute_groups};
pub use memories::{OnThisDay, on_this_day};

use super::GridState;
use super::bg::LatestWorker;
use super::style::TILE;
use crate::Library;
use crate::media::MediaKind;
use crate::store::db::{TimelineFilter, TimelineItem, TimelineOrder};
use crate::thumbs::ThumbKey;
use chrono::NaiveDate;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

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

pub fn thumb_key(item: &TimelineItem) -> ThumbKey {
    ThumbKey {
        path: item.path.clone(),
        mtime: item.mtime,
        t_ms: (item.kind == MediaKind::Video)
            .then(|| super::to_millis((item.duration.unwrap_or(0.0) * 0.1).clamp(0.0, 5.0))),
    }
}

pub(super) struct Loaded {
    pub(super) id: u64,
    pub(super) filter: TimelineFilter,
    pub(super) items: Result<Vec<TimelineItem>, String>,
}

/// Loads timelines on a dedicated thread; only the newest pending request is executed.
pub(super) struct TimelineWorker(LatestWorker<TimelineFilter, Loaded>);

impl TimelineWorker {
    fn new(lib: Arc<Library>, repaint: impl Fn() + Send + 'static) -> Self {
        Self(LatestWorker::spawn(
            "timeline",
            repaint,
            move |filter| lib.timeline(filter),
            |id, filter, outcome| Loaded {
                id,
                filter,
                items: match outcome {
                    Some(r) => r.map_err(|e| format!("{e:#}")),
                    None => Err("loading the library failed unexpectedly".to_string()),
                },
            },
        ))
    }

    /// Lets tests deliver a load result of their own making.
    #[cfg(test)]
    pub(super) fn inject(&self, loaded: Loaded) {
        self.0.inject(loaded);
    }

    fn submit(&mut self, filter: TimelineFilter) -> u64 {
        self.0.submit(filter)
    }

    fn poll(&self) -> Option<Loaded> {
        self.0.poll()
    }
}

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
    /// How many of the loaded items are photos and how many are videos.
    pub(super) n_photos: usize,
    pub(super) n_videos: usize,
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
    pub(super) error: Option<String>,
    pub(super) grid: GridState,
    expanded: HashSet<i32>,
    top_year: Option<i32>,
    top_item: Option<usize>,
    /// Item to scroll to once the layout is known; `true` aligns to its section header.
    pending: Option<(usize, bool)>,
}

impl TimelineState {
    pub(super) fn new(lib: Arc<Library>, ctx: &egui::Context) -> Self {
        let c = ctx.clone();
        Self {
            worker: TimelineWorker::new(lib, move || c.request_repaint()),
            view: View::Timeline,
            group: Group::Month,
            newest_first: true,
            tile_size: TILE,
            items: Arc::new(Vec::new()),
            generation: 0,
            groups: Vec::new(),
            groups_key: None,
            dates: Vec::new(),
            n_photos: 0,
            n_videos: 0,
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
            grid: GridState::default(),
            expanded: HashSet::new(),
            top_year: None,
            top_item: None,
            pending: None,
        }
    }

    /// Recounts photos and videos after the items changed.
    fn recount(&mut self) {
        self.n_videos = self
            .items
            .iter()
            .filter(|it| it.kind == MediaKind::Video)
            .count();
        self.n_photos = self.items.len() - self.n_videos;
    }

    pub(super) fn headers(&self) -> bool {
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
            .grid
            .selected
            .and_then(|i| self.items.get(i))
            .map(|it| it.file_id);
        self.items = Arc::new(items);
        self.generation += 1;
        self.dates = date_index(&self.items);
        self.recount();
        self.grid.selected = prev.and_then(|f| self.items.iter().position(|it| it.file_id == f));
        // Several selected items stay selected by file id; those that are gone are dropped.
        self.grid.selected = self
            .grid
            .sel
            .rebase(self.items.iter().map(|it| it.file_id), self.grid.selected);
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
        let primary = self.grid.selected.filter(|&s| s < self.items.len());
        let primary_removed = primary.is_some_and(|s| ids.contains(&self.items[s].file_id));
        let sel_shift = primary.map(|s| before(&self.items, s));
        let top = self.top_item.filter(|&t| t > 0);
        let top_shift = top.map(|t| before(&self.items, t));
        Arc::make_mut(&mut self.items).retain(|it| !ids.contains(&it.file_id));
        self.grid.sel.remove_all(ids);
        self.generation += 1;
        self.dates = date_index(&self.items);
        self.recount();
        let len = self.items.len();
        self.grid.selected = match (primary, sel_shift) {
            (Some(s), Some(shift)) => {
                let moved = s - shift;
                if !primary_removed {
                    Some(moved)
                } else if !self.grid.sel.is_empty() {
                    // Others stay selected: the first of them takes over.
                    self.items
                        .iter()
                        .position(|it| self.grid.sel.contains(it.file_id))
                } else {
                    (len > 0).then(|| moved.min(len - 1))
                }
            }
            _ => None,
        };
        if self.grid.selected.is_some() {
            self.grid.keyboard_moved = true;
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
        self.grid.scroll_to = Some(top);
    }
}

fn file_name(item: &TimelineItem) -> String {
    item.path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
