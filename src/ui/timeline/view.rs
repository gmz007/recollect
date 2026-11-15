//! Drawing the grid of the current view and handling clicks and keys in it.

use super::dates::{MonthEntry, date_of, fmt_taken, header_text, thousands, until_next_midnight};
use super::layout::{HEADER_H, Layout, Row, reveal};
use super::{TimelineState, View, file_name, thumb_key};
use crate::media::MediaKind;
use crate::store::db::AlbumInfo;
use crate::ui::grid::grid_scroll_area;
use crate::ui::selection::{self, ClickMods};
use crate::ui::style::{self, GAP};
use crate::ui::tiles::{self, Tile, TileEvent, TileMark};
use crate::ui::workers::ThumbCache;
use crate::ui::{App, ClickOutcome, FavMark, GridEvents, albums, fmt_time, marked};
use chrono::Datelike;
use egui::{RichText, Sense, Vec2};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// What the visible rows of the timeline need to be drawn.
pub(super) struct RowsView<'a> {
    pub(super) tl: &'a TimelineState,
    pub(super) layout: &'a Layout,
    pub(super) marks: &'a HashMap<i64, FavMark>,
    pub(super) albums: &'a [AlbumInfo],
    pub(super) width: f32,
}

impl RowsView<'_> {
    /// Draws the rows that intersect `viewport` and reports what the user did to them.
    pub(super) fn show(
        &self,
        ui: &mut egui::Ui,
        thumbs: &mut ThumbCache,
        viewport: egui::Rect,
    ) -> GridEvents {
        let (tl, layout) = (self.tl, self.layout);
        let mut events = GridEvents::default();
        let (origin, _) =
            ui.allocate_exact_size(Vec2::new(self.width, layout.total), Sense::hover());
        let range = layout.visible(viewport.min.y, viewport.max.y);
        if let Some(&row) = layout.rows.get(range.start) {
            events.top_item = Some(row.first_item());
        }
        for r in range {
            let top = origin.min.y + layout.tops[r];
            match layout.rows[r] {
                Row::Header { group, .. } => {
                    let g = tl.groups[group];
                    let text = header_text(tl.group, g.key, g.end - g.start);
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.min.x, top),
                        Vec2::new(self.width, HEADER_H),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new(text).text_style(style::section()));
                    });
                }
                Row::Tiles { start, end } => {
                    for i in start..end {
                        let x = origin.min.x + (i - start) as f32 * (layout.tile + GAP);
                        let rect =
                            egui::Rect::from_min_size(egui::pos2(x, top), Vec2::splat(layout.tile));
                        self.tile(ui, thumbs, i, rect, &mut events);
                    }
                }
            }
        }
        events
    }

    pub(super) fn tile(
        &self,
        ui: &mut egui::Ui,
        thumbs: &mut ThumbCache,
        i: usize,
        rect: egui::Rect,
        events: &mut GridEvents,
    ) {
        let tl = self.tl;
        let picking = tl.grid.picking;
        let item = &tl.items[i];
        let key = thumb_key(item);
        let name = || file_name(item);
        let tip = || format!("{}\n{}", file_name(item), fmt_taken(item));
        let mark = TileMark::of(
            tl.grid.sel.contains(item.file_id),
            tl.grid.selected == Some(i),
            picking || tl.grid.sel.is_multi(),
        );
        let tile = Tile::new(&key, "Item", &name)
            .tip(&tip)
            .badge((item.kind == MediaKind::Video).then(|| fmt_time(item.duration.unwrap_or(0.0))))
            .favorite(marked(self.marks, item.file_id, item.favorite))
            .selectable(mark, picking);
        let id = ui.make_persistent_id(("tile", i));
        let out = tiles::show(ui, thumbs, id, rect, &tile);
        let in_album = matches!(tl.view, View::Album(_));
        events.record(i, &out, |ui| albums::tile_menu(ui, self.albums, in_album));
    }
}

/// What was clicked in the "Jump to date" list.
#[derive(Default)]
struct DateClicks {
    /// The first item to scroll to.
    jump: Option<usize>,
    /// The year whose months are shown or hidden.
    toggle: Option<i32>,
}

/// One indented month of the "Jump to date" list; true when it was clicked.
fn month_row(ui: &mut egui::Ui, m: &MonthEntry) -> bool {
    let name = u8::try_from(m.month)
        .ok()
        .and_then(|n| chrono::Month::try_from(n).ok())
        .map_or("", |m| m.name());
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.selectable_label(false, format!("{name} ({})", thousands(m.count)))
            .clicked()
    })
    .inner
}

impl App {
    pub(in crate::ui) fn set_view(&mut self, view: View) {
        if self.tl.view != view {
            self.tl.view = view;
            self.tl.grid.selected = None;
            self.tl.grid.scroll_to = Some(0.0);
            self.tl.pending = None;
        }
        if self.searching() {
            self.clear_search();
        }
    }

    /// Sidebar: years and months of the current view; clicking scrolls the grid there.
    pub(in crate::ui) fn date_navigator(&mut self, ui: &mut egui::Ui) {
        if self.tl.view == View::RecentlyAdded || self.tl.dates.is_empty() {
            return;
        }
        let mut clicks = DateClicks::default();
        egui::CollapsingHeader::new("Jump to date")
            .id_salt("jump-to-date")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("dates")
                    .max_height(260.0)
                    .show(ui, |ui| self.date_rows(ui, &mut clicks));
            });
        if let Some(y) = clicks.toggle
            && !self.tl.expanded.remove(&y)
        {
            self.tl.expanded.insert(y);
        }
        if let Some(i) = clicks.jump {
            self.tl.pending = Some((i, true));
        }
    }

    /// One row per year, followed by its months while the year is expanded.
    fn date_rows(&self, ui: &mut egui::Ui, clicks: &mut DateClicks) {
        for y in &self.tl.dates {
            let label = format!("{} ({})", y.year, thousands(y.count));
            if ui
                .selectable_label(self.tl.top_year == Some(y.year), label)
                .clicked()
            {
                clicks.jump = Some(y.first);
                clicks.toggle = Some(y.year);
            }
            if self.tl.expanded.contains(&y.year) {
                for m in &y.months {
                    if month_row(ui, m) {
                        clicks.jump = Some(m.first);
                    }
                }
            }
        }
    }

    /// The album grid for the current view.
    pub(in crate::ui) fn timeline_view(&mut self, ui: &mut egui::Ui) {
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
        self.timeline_header(ui);
        if self.tl.items.is_empty() {
            if self.tl.awaiting.is_none() {
                self.timeline_empty_state(ui);
            }
            return;
        }
        self.timeline_rows(ui);
    }

    /// The selection bar or view header, the tip and the "On this day" strip.
    fn timeline_header(&mut self, ui: &mut egui::Ui) {
        if !self.selection_bar_ui(ui) {
            self.view_header(ui);
        }
        self.tip_card(ui);
        if self.tl.view == View::Timeline {
            // "Today" changes at midnight, whether or not anything else happens by then.
            let now = chrono::Local::now().naive_local();
            ui.ctx()
                .request_repaint_after(until_next_midnight(now) + Duration::from_secs(1));
            self.on_this_day_strip(ui);
        }
    }

    /// The scrolling grid of items and what the user did in it.
    fn timeline_rows(&mut self, ui: &mut egui::Ui) {
        self.tl.grid.shown = true;
        let width = ui.available_width() - style::SCROLLBAR_ALLOWANCE;
        self.tl.ensure_layout(width);
        self.tl.resolve_pending();
        let area = grid_scroll_area("timeline", &mut self.tl.grid.scroll_to);
        let Some(layout) = self.tl.layout.as_ref() else {
            return;
        };
        let rows = RowsView {
            tl: &self.tl,
            layout,
            marks: &self.fav_marks,
            albums: &self.albums,
            width,
        };
        let mut events = GridEvents::default();
        let out = area.show_viewport(ui, |ui, viewport| {
            events = rows.show(ui, &mut self.thumbs, viewport);
        });
        self.tl.top_item = events.top_item;
        self.tl.top_year = events
            .top_item
            .and_then(|i| self.tl.items.get(i))
            .map(|it| date_of(it.taken_at).year());
        self.follow_keyboard_selection(ui, &out);
        if self.tl.pending.is_some() {
            ui.ctx().request_repaint();
        }
        if let Some((i, ev, mods)) = events.clicked {
            self.click_timeline_item(i, ev, mods);
        }
        self.apply_menu_events(&mut events);
        // Applying a scroll offset needs one more frame to take effect.
        if self.tl.grid.scroll_to.is_some() {
            ui.ctx().request_repaint();
        }
    }

    /// Scrolls so that an item the keyboard moved to is in view.
    pub(super) fn follow_keyboard_selection<R>(
        &mut self,
        ui: &egui::Ui,
        out: &egui::scroll_area::ScrollAreaOutput<R>,
    ) {
        let Some(i) = self
            .tl
            .grid
            .selected
            .filter(|_| self.tl.grid.keyboard_moved)
        else {
            return;
        };
        self.tl.grid.keyboard_moved = false;
        if let Some(layout) = &self.tl.layout {
            let (top, h) = layout.reveal_span(i);
            self.tl.grid.scroll_to = reveal(
                top,
                h,
                out.inner_rect.height(),
                out.state.clamped_offset().y,
            );
            if self.tl.grid.scroll_to.is_some() {
                ui.ctx().request_repaint();
            }
        }
    }

    /// What a click on tile `i` does: favorite it, change the selection or open it.
    pub(super) fn click_timeline_item(&mut self, i: usize, ev: TileEvent, mods: ClickMods) {
        let items = Arc::clone(&self.tl.items);
        match self
            .tl
            .grid
            .apply_click(i, ev, mods, items.len(), &|k| items[k].file_id)
        {
            ClickOutcome::Favorite => {
                let it = &items[i];
                self.toggle_favorite(it.file_id, it.favorite);
            }
            ClickOutcome::Selected => {}
            ClickOutcome::Open => self.open_timeline_item(i),
        }
    }

    pub(super) fn open_timeline_item(&mut self, i: usize) {
        if i < self.tl.items.len() {
            self.open_viewer_timeline(i);
        }
    }

    /// Keys for the grid (same rules as for search results): arrows, Shift+arrows,
    /// Ctrl/Cmd+A, Enter and F.
    pub(in crate::ui) fn handle_timeline_keys(
        &mut self,
        ctx: &egui::Context,
        grid_was_shown: bool,
    ) {
        if !grid_was_shown || self.tl.items.is_empty() || self.keys_blocked(ctx) {
            return;
        }
        let multi = self.tl.grid.sel.is_multi();
        let primary = self.tl.grid.selected.and_then(|i| self.tl.items.get(i));
        let primary = primary.map(|it| (it.file_id, it.favorite));
        self.favorite_key(ctx, multi, primary);
        let input = selection::read_nav_keys(ctx);
        let items = Arc::clone(&self.tl.items);
        let len = items.len();
        let layout = self.tl.layout.as_ref();
        let before = self.tl.grid.selected;
        let next = selection::apply_keys(
            &input,
            &mut self.tl.grid.sel,
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
            self.tl.grid.selected = next;
            self.tl.grid.keyboard_moved = true;
        }
        if input.enter
            && let Some(i) = self.tl.grid.selected
        {
            self.open_timeline_item(i);
        }
    }
}
