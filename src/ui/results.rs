//! Search: starting one when typing pauses, taking in the answer, and the results grid.

use super::ruler::{Mapping, Month, RulerState};
use super::selection::ClickMods;
use super::style::GAP;
use super::tiles::{Tile, TileMark};
use super::workers::Query;
use super::{
    App, ClickOutcome, GridEvents, GridState, fmt_time, grid::grid_scroll_area, guide, marked,
    scroll_offset_for_row, selection, step, tiles, to_millis,
};
use crate::media::MediaKind;
use crate::search::{SearchHit, SearchOptions};
use crate::thumbs::ThumbKey;
use egui::Vec2;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long typing must pause before a search starts.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(250);

/// Files asked for by one search.
const SEARCH_LIMIT: usize = 300;

/// Where the tiles of the results grid go at the current width.
struct ResultsMetrics {
    /// Width of the tile area.
    avail: f32,
    cols: usize,
    rows: usize,
    tile: f32,
    /// Distance between row tops (and between tile left edges).
    stride: f32,
}

impl ResultsMetrics {
    /// For `width` points of space, `len` results and tiles of about `tile_size` points.
    fn new(width: f32, len: usize, tile_size: f32) -> Self {
        let avail = width;
        let cols = (((avail + GAP) / (tile_size + GAP)).floor() as usize).max(1);
        let tile = ((avail - GAP * (cols - 1) as f32) / cols as f32).max(tile_size * 0.6);
        Self {
            avail,
            cols,
            rows: len.div_ceil(cols),
            tile,
            stride: tile + GAP,
        }
    }
}

/// The current search and what it found.
pub(super) struct ResultsState {
    pub(super) grid: GridState,
    pub(super) current: Option<Query>,
    pub(super) awaiting: Option<u64>,
    /// Shared with the viewer while it is open.
    pub(super) hits: Arc<Vec<SearchHit>>,
    pub(super) label: String,
    pub(super) error: Option<guide::SearchProblem>,
    /// When the last awaited answer was applied (the timed refresh counts from here, so a slow
    /// search never runs back to back with the next one).
    pub(super) last_answer_at: Option<Instant>,
    /// Selected moment (seconds) for the selected video.
    pub(super) selected_t: Option<f64>,
    /// Columns in the results grid as of the last layout (for up/down navigation).
    pub(super) cols: usize,
}

impl Default for ResultsState {
    fn default() -> Self {
        Self {
            grid: GridState::default(),
            current: None,
            awaiting: None,
            hits: Arc::default(),
            label: String::new(),
            error: None,
            last_answer_at: None,
            selected_t: None,
            cols: 1,
        }
    }
}

impl ResultsState {
    /// Runs `f` with the ruler's state and a way to find the date of the result at a point of
    /// the ruler (the first result at the top, the last at the bottom).
    pub(super) fn with_ruler<R>(
        &mut self,
        f: impl FnOnce(&mut RulerState, &dyn Fn(f32) -> Option<Month>) -> R,
    ) -> R {
        let hits = &self.hits;
        let date_at = |frac: f32| {
            let last = hits.len().checked_sub(1)?;
            let at = (frac.clamp(0.0, 1.0) * last as f32).round() as usize;
            Month::of_timestamp(hits.get(at)?.taken_at)
        };
        f(&mut self.grid.ruler, &date_at)
    }
}

impl App {
    /// True while the central area shows search results instead of the current view.
    pub(super) fn searching(&self) -> bool {
        self.res.current.is_some() || !self.query.trim().is_empty()
    }

    /// Leaves search results (back to the current view).
    pub(super) fn clear_search(&mut self) {
        self.query.clear();
        self.query_changed_at = None;
        self.res.current = None;
        self.res.awaiting = None;
        self.res.hits = Arc::default();
        self.res.label.clear();
        self.res.error = None;
        self.res.grid.selected = None;
        self.res.selected_t = None;
        self.res.grid.sel.clear();
        self.res.grid.picking = false;
        self.res.grid.scroll_to = None;
        self.res.grid.keyboard_moved = false;
    }

    pub(super) fn submit(&mut self, q: Query) {
        let opts = SearchOptions {
            limit: SEARCH_LIMIT,
            kind: self.kind,
            taken_range: self.date.range(chrono::Datelike::year(&self.today())),
            sort: self.sort,
        };
        if self.res.current.is_none() {
            // Starting a search leaves the timeline's selection behind.
            self.tl.grid.sel.clear();
            self.tl.grid.selected = None;
            self.tl.grid.picking = false;
        }
        self.res.awaiting = Some(self.search.submit(q.clone(), opts));
        self.res.current = Some(q);
    }

    pub(super) fn rerun(&mut self) {
        if let Some(q) = self.res.current.clone() {
            self.submit(q);
        }
    }

    /// Takes in the answer of the search worker, if it is the one waited for.
    pub(super) fn poll_search(&mut self) {
        let Some(r) = self.search.poll() else {
            return;
        };
        if Some(r.id) != self.res.awaiting {
            return;
        }
        self.res.awaiting = None;
        self.res.last_answer_at = Some(Instant::now());
        self.res.label = r.query.label();
        let prev = self
            .res
            .grid
            .selected
            .and_then(|i| self.res.hits.get(i))
            .map(|h| h.path.clone());
        match r.hits {
            Ok(h) => {
                self.res.hits = Arc::new(h);
                self.res.error = None;
            }
            Err(e) => {
                tracing::warn!("search failed: {}", e.detail);
                self.res.error = Some(guide::search_problem(e.kind));
            }
        }
        // Keep the selection on the same file across refreshes.
        self.res.grid.selected = prev.and_then(|p| self.res.hits.iter().position(|h| h.path == p));
        self.res.grid.selected = self.res.grid.sel.rebase(
            self.res.hits.iter().map(|h| h.file_id),
            self.res.grid.selected,
        );
        if self.res.grid.selected.is_none() {
            self.res.selected_t = None;
        }
    }

    /// Starts the search once typing has paused.
    pub(super) fn poll_debounce(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.query_changed_at
            && t.elapsed() >= DEBOUNCE
        {
            self.query_changed_at = None;
            let q = self.query.trim().to_string();
            if q.is_empty() {
                self.clear_search();
            } else {
                // A search the person asked for (Enter, or a pause in typing): worth
                // remembering. Leaving the field or pressing Escape never gets here.
                self.remember_search(&q);
                self.submit(Query::Text(q));
            }
        }
        if let Some(t) = self.query_changed_at {
            ctx.request_repaint_after(DEBOUNCE.saturating_sub(t.elapsed()));
        }
    }

    pub(super) fn results_grid(&mut self, ui: &mut egui::Ui) {
        self.selection_bar_ui(ui);
        self.results_header(ui);
        if self.res.hits.is_empty() {
            self.results_empty_state(ui);
            return;
        }
        let m = ResultsMetrics::new(ui.available_width(), self.res.hits.len(), self.tl.tile_size);
        self.res.cols = m.cols;
        if self.res.grid.ruler.needs_mapping(0) {
            self.res.grid.ruler.set_mapping(0, Mapping::Flat);
        }
        self.res.grid.shown = true;
        let area = grid_scroll_area("results", &mut self.res.grid.scroll_to);
        let results = Arc::clone(&self.res.hits);
        let mut events = GridEvents::default();
        let out = area.show_viewport(ui, |ui, viewport| {
            self.result_rows(ui, viewport, &m, &results, &mut events);
        });
        self.res.grid.ruler.observe(
            out.state.clamped_offset().y,
            out.content_size.y - out.inner_rect.height(),
        );
        self.follow_result_selection(ui, &out, &m);
        self.apply_menu_events(&mut events);
        if let Some((i, ev, mods)) = events.clicked {
            self.click_result(&results, i, ev, mods);
        }
    }

    /// Draws the rows of results that intersect `viewport`.
    fn result_rows(
        &mut self,
        ui: &mut egui::Ui,
        viewport: egui::Rect,
        m: &ResultsMetrics,
        results: &[SearchHit],
        events: &mut GridEvents,
    ) {
        let (origin, _) = ui.allocate_exact_size(
            Vec2::new(m.avail, m.rows as f32 * m.stride),
            egui::Sense::hover(),
        );
        let first = (viewport.min.y / m.stride).floor().max(0.0) as usize;
        let last = ((viewport.max.y / m.stride).ceil() as usize + 1).min(m.rows);
        for row in first..last {
            let (start, end) = (row * m.cols, ((row + 1) * m.cols).min(results.len()));
            for (i, hit) in results.iter().enumerate().take(end).skip(start) {
                let x = origin.min.x + (i - start) as f32 * (m.tile + GAP);
                let rect = egui::Rect::from_min_size(
                    egui::pos2(x, origin.min.y + row as f32 * m.stride),
                    Vec2::splat(m.tile),
                );
                self.result_tile(ui, hit, i, rect, events);
            }
        }
    }

    /// Scrolls so that a result the keyboard moved to is in view.
    fn follow_result_selection<R>(
        &mut self,
        ui: &egui::Ui,
        out: &egui::scroll_area::ScrollAreaOutput<R>,
        m: &ResultsMetrics,
    ) {
        let Some(i) = self
            .res
            .grid
            .selected
            .filter(|_| self.res.grid.keyboard_moved)
        else {
            return;
        };
        // Rows are virtualized, so the selected tile may not exist on screen yet:
        // scroll by row arithmetic instead of asking the tile to scroll itself.
        self.res.grid.keyboard_moved = false;
        let view = out.inner_rect.height();
        let offset = out.state.clamped_offset().y;
        self.res.grid.scroll_to = scroll_offset_for_row(i / m.cols, m.stride, m.tile, view, offset);
        if self.res.grid.scroll_to.is_some() {
            ui.ctx().request_repaint();
        }
    }

    /// Draws one search result and records what the user did to it.
    fn result_tile(
        &mut self,
        ui: &mut egui::Ui,
        hit: &SearchHit,
        i: usize,
        rect: egui::Rect,
        events: &mut GridEvents,
    ) {
        let key = Self::thumb_key(hit, None);
        let name = || {
            hit.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let grid = &self.res.grid;
        let mark = TileMark::of(
            grid.sel.contains(hit.file_id),
            grid.selected == Some(i),
            grid.picking || grid.sel.is_multi(),
        );
        let tile = Tile::new(&key, "Result", &name)
            .badge(
                (hit.kind == MediaKind::Video)
                    .then(|| format!("at {}", fmt_time(hit.t_start.unwrap_or(0.0)))),
            )
            .favorite(marked(&self.fav_marks, hit.file_id, hit.favorite))
            .selectable(mark);
        let id = ui.make_persistent_id(("result", i));
        let out = tiles::show(ui, &mut self.thumbs, id, rect, &tile);
        events.record(i, &out, |ui| {
            super::grid::tile_menu(ui, &self.albums, false)
        });
    }

    /// What a click on result `i` does: favorite it, change the selection or open it.
    fn click_result(
        &mut self,
        results: &[SearchHit],
        i: usize,
        ev: tiles::TileEvent,
        mods: ClickMods,
    ) {
        let outcome = self
            .res
            .grid
            .apply_click(i, ev, mods, results.len(), &|k| results[k].file_id);
        match outcome {
            ClickOutcome::Favorite => {
                let h = &results[i];
                self.toggle_favorite(h.file_id, h.favorite);
            }
            ClickOutcome::Selected => {
                self.res.selected_t = self.res.grid.selected.and_then(|p| results[p].t_start);
            }
            ClickOutcome::Open => {
                self.res.selected_t = results[i].t_start;
                self.open_viewer_results(i);
            }
        }
    }

    pub(super) fn thumb_key(hit: &SearchHit, t: Option<f64>) -> ThumbKey {
        ThumbKey {
            path: hit.path.clone(),
            mtime: hit.mtime,
            t_ms: if hit.kind == MediaKind::Video {
                Some(to_millis(t.or(hit.t_start).unwrap_or(0.0)))
            } else {
                None
            },
        }
    }

    /// Arrow keys move the selection, Shift+arrows extend it, Ctrl/Cmd+A selects everything and
    /// Enter opens the primary item. Only active when the results grid was drawn last frame, the
    /// Settings window and the album dialogs are closed and no text field has keyboard focus, so typing in the search
    /// box is never hijacked. Keys with other modifiers are ignored.
    pub(super) fn handle_navigation_keys(&mut self, ctx: &egui::Context, grid_was_shown: bool) {
        if !grid_was_shown || self.res.hits.is_empty() || self.keys_blocked(ctx) {
            return;
        }
        let results = Arc::clone(&self.res.hits);
        let (len, cols) = (results.len(), self.res.cols);
        let keys = selection::apply_grid_keys(
            ctx,
            &mut self.res.grid,
            &selection::GridList {
                len,
                id_at: &|i| results[i].file_id,
                favorite_at: &|i| results[i].favorite,
            },
            |sel, n| step(sel, len, cols, n),
        );
        if keys.moved {
            self.res.selected_t = self
                .res
                .grid
                .selected
                .and_then(|i| results.get(i))
                .and_then(|h| h.t_start);
        }
        self.favorite_key(keys.favorite);
        if let Some(i) = keys.open {
            self.open_viewer_results(i);
        }
    }
}
