//! Drawing the grid of the current view and handling clicks and keys in it.

use super::banner::{self, BANNER_GAP, BANNER_HEIGHT};
use super::dates::{day_header_text, fmt_taken, until_next_midnight};
use super::header::{self, DaySelection};
use super::layout::{HEADER_H, Layout, Row, reveal};
use super::{TimelineState, View, file_name, thumb_key};
use crate::media::MediaKind;
use crate::store::db::AlbumInfo;
use crate::ui::grid::grid_scroll_area;
use crate::ui::selection::{self, ClickMods};
use crate::ui::style::{self, GAP};
use crate::ui::tiles::{self, Tile, TileEvent, TileMark};
use crate::ui::workers::ThumbCache;
use crate::ui::{App, ClickOutcome, FavMark, GridEvents, fmt_time, marked};
use chrono::NaiveDate;
use egui::{Sense, Vec2};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// What the visible rows of the timeline need to be drawn.
struct RowsView<'a> {
    pub(super) tl: &'a TimelineState,
    pub(super) layout: &'a Layout,
    pub(super) marks: &'a HashMap<i64, FavMark>,
    pub(super) albums: &'a [AlbumInfo],
    pub(super) width: f32,
    pub(super) today: NaiveDate,
}

impl RowsView<'_> {
    /// Draws the rows that intersect `viewport` (in the rows' own coordinates, which start at
    /// the first row) and reports what the user did to them. The heading of the section at the
    /// top of the view stays pinned there.
    pub(super) fn show(
        &self,
        ui: &mut egui::Ui,
        thumbs: &mut ThumbCache,
        viewport: egui::Rect,
    ) -> GridEvents {
        let layout = self.layout;
        let mut events = GridEvents::default();
        let (origin, _) =
            ui.allocate_exact_size(Vec2::new(self.width, layout.total), Sense::hover());
        let range = layout.visible(viewport.min.y, viewport.max.y);
        // Above the first row (the banner is in view) there is no photo to keep in place.
        if viewport.min.y > 0.0
            && let Some(&row) = layout.rows.get(range.start)
        {
            events.top_item = Some(row.first_item());
        }
        let pin = layout.pinned_header(viewport.min.y);
        let full_clip = ui.clip_rect();
        // Photos under the pinned heading take neither hover nor clicks.
        let mut tile_clip = full_clip;
        if let Some((_, y)) = pin {
            tile_clip.min.y = tile_clip.min.y.max(origin.min.y + y + HEADER_H);
        }
        for r in range {
            let top = origin.min.y + layout.tops[r];
            match layout.rows[r] {
                Row::Header { group, .. } if pin.map(|p| p.0) != Some(group) => {
                    self.header(ui, origin, group, top, false, &mut events);
                }
                Row::Header { .. } => {}
                Row::Tiles { start, end } => {
                    ui.set_clip_rect(tile_clip);
                    for i in start..end {
                        let x = origin.min.x + (i - start) as f32 * (layout.tile + GAP);
                        let rect =
                            egui::Rect::from_min_size(egui::pos2(x, top), Vec2::splat(layout.tile));
                        self.tile(ui, thumbs, i, rect, &mut events);
                    }
                    ui.set_clip_rect(full_clip);
                }
            }
        }
        if let Some((group, y)) = pin {
            self.header(ui, origin, group, origin.min.y + y, true, &mut events);
        }
        events
    }

    /// Draws the heading of section `group` with its top at `top`.
    fn header(
        &self,
        ui: &mut egui::Ui,
        origin: egui::Rect,
        group: usize,
        top: f32,
        pinned: bool,
        events: &mut GridEvents,
    ) {
        let tl = self.tl;
        let g = tl.groups[group];
        let text = day_header_text(g.key, self.today);
        let rect = egui::Rect::from_min_size(
            egui::pos2(origin.min.x, top),
            Vec2::new(self.width, HEADER_H),
        );
        let selected = &tl.grid.sel;
        let all = !selected.is_empty()
            && selected.len() >= g.end - g.start
            && (g.start..g.end).all(|i| selected.contains(tl.items[i].file_id));
        let state = if all {
            DaySelection::All
        } else {
            DaySelection::NotAll
        };
        let id = ui.make_persistent_id(("day", group));
        if header::show(ui, id, rect, &text, state, pinned) {
            events.day_toggled = Some(group);
        }
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
            .selectable(mark);
        let id = ui.make_persistent_id(("tile", i));
        let out = tiles::show(ui, thumbs, id, rect, &tile);
        let in_album = matches!(tl.view, View::Album(_));
        events.record(i, &out, |ui| {
            crate::ui::grid::tile_menu(ui, self.albums, in_album)
        });
    }
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

    /// The album grid for the current view.
    pub(in crate::ui) fn timeline_view(&mut self, ui: &mut egui::Ui) {
        // Until items for the selected view arrive, show a spinner rather than the previous
        // view's items laid out under the new view's rules.
        let wanted = self.tl.filter();
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

    /// The selection bar, the title of an album or folder, a load problem and the tip.
    fn timeline_header(&mut self, ui: &mut egui::Ui) {
        self.selection_bar_ui(ui);
        self.view_header(ui);
        self.tip_card(ui);
        if self.tl.view == View::Timeline {
            // "Today" changes at midnight, whether or not anything else happens by then.
            let now = chrono::Local::now().naive_local();
            ui.ctx()
                .request_repaint_after(until_next_midnight(now) + Duration::from_secs(1));
        }
    }

    /// The scrolling grid of items and what the user did in it.
    fn timeline_rows(&mut self, ui: &mut egui::Ui) {
        self.tl.grid.shown = true;
        let width = ui.available_width() - style::SCROLLBAR_ALLOWANCE;
        let banner = (self.tl.view == View::Timeline)
            .then(|| self.on_this_day_banner())
            .flatten();
        self.tl.lead = banner.as_ref().map_or(0.0, |_| BANNER_HEIGHT + BANNER_GAP);
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
            today: self.today(),
        };
        let lead = self.tl.lead;
        let items = Arc::clone(&self.tl.items);
        let mut events = GridEvents::default();
        let mut banner_event = None;
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if let Some(b) = &banner {
                banner_event = banner::show(ui, &mut self.thumbs, &items, b, width);
                ui.add_space(BANNER_GAP);
            }
            events = rows.show(
                ui,
                &mut self.thumbs,
                viewport.translate(egui::vec2(0.0, -lead)),
            );
        });
        self.tl.top_item = events.top_item;
        self.follow_keyboard_selection(ui, &out);
        if self.tl.pending.is_some() {
            ui.ctx().request_repaint();
        }
        if let Some((b, e)) = banner.as_ref().zip(banner_event) {
            self.banner_event(b, e);
        }
        self.apply_timeline_events(events);
        // Applying a scroll offset needs one more frame to take effect.
        if self.tl.grid.scroll_to.is_some() {
            ui.ctx().request_repaint();
        }
    }

    /// Acts on what the user did to the heading circles and the tiles.
    fn apply_timeline_events(&mut self, mut events: GridEvents) {
        if let Some(group) = events.day_toggled.take() {
            self.toggle_day(group);
        }
        if let Some((i, ev, mods)) = events.clicked {
            self.click_timeline_item(i, ev, mods);
        }
        self.apply_menu_events(&mut events);
    }

    /// Selects every photo of day `group`, or deselects them if all are selected.
    fn toggle_day(&mut self, group: usize) {
        let Some(g) = self.tl.groups.get(group).copied() else {
            return;
        };
        let items = Arc::clone(&self.tl.items);
        self.tl
            .grid
            .toggle_range(g.start..g.end, items.len(), &|k| items[k].file_id);
    }

    /// Scrolls so that an item the keyboard moved to is in view.
    fn follow_keyboard_selection<R>(
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
            let (mut top, mut h) = layout.reveal_span(i);
            if self.tl.headers() && top >= layout.tops[layout.row_of_item(i)] {
                // The pinned heading covers the top of the view: keep the row below it.
                let pad = HEADER_H.min(top);
                top -= pad;
                h += pad;
            }
            let offset = out.state.clamped_offset().y - self.tl.lead;
            self.tl.grid.scroll_to =
                reveal(top, h, out.inner_rect.height(), offset).map(|y| y + self.tl.lead);
            if self.tl.grid.scroll_to.is_some() {
                ui.ctx().request_repaint();
            }
        }
    }

    /// What a click on tile `i` does: favorite it, change the selection or open it.
    fn click_timeline_item(&mut self, i: usize, ev: TileEvent, mods: ClickMods) {
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

    fn open_timeline_item(&mut self, i: usize) {
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
        let items = Arc::clone(&self.tl.items);
        let len = items.len();
        let layout = self.tl.layout.as_ref();
        let keys = selection::apply_grid_keys(
            ctx,
            &mut self.tl.grid,
            &selection::GridList {
                len,
                id_at: &|i| items[i].file_id,
                favorite_at: &|i| items[i].favorite,
            },
            |sel, n| {
                // Nothing (valid) selected: any arrow picks the first item.
                let Some(cur) = sel.filter(|&i| i < len) else {
                    return Some(0);
                };
                Some(layout.map_or(cur, |l| l.step(cur, len, n)))
            },
        );
        self.favorite_key(keys.favorite);
        if let Some(i) = keys.open {
            self.open_timeline_item(i);
        }
    }
}
