//! What the timeline grid and the results grid share: selection state, what a click
//! does, and moving through tiles with the keyboard.

use super::albums::{AlbumChoice, album_menu};
use super::selection::{ClickMods, Selection};
use super::selection_bar::BulkAction;
use super::{App, tiles};
use crate::store::db::AlbumInfo;

/// What a grid of files (the timeline or the search results) remembers about the user's
/// selection and scrolling.
#[derive(Default)]
pub(super) struct GridState {
    /// The primary selected item (last clicked); see [`Selection`].
    pub(super) selected: Option<usize>,
    /// All selected items (by file id); `selected` is the primary one.
    pub(super) sel: Selection,
    /// Selection mode: the user picked items with a circle or a modifier key, so plain clicks
    /// toggle items instead of opening them and the selection bar is shown.
    pub(super) picking: bool,
    /// The album the user is choosing photos for ("Add photos"): selection mode stays on, even
    /// with nothing selected yet.
    pub(super) choosing_for: Option<i64>,
    /// Whether the keyboard moved the selection since the last frame.
    pub(super) keyboard_moved: bool,
    /// Scroll offset to apply on the next frame so the keyboard selection stays visible.
    pub(super) scroll_to: Option<f32>,
    /// Set when the grid was drawn; read (and cleared) at the start of the next frame.
    pub(super) shown: bool,
    /// The timeline ruler beside the grid, which is its scroll bar.
    pub(super) ruler: super::ruler::RulerState,
}

impl GridState {
    /// Keeps the selection consistent with its primary item (nothing is selected without a
    /// primary; the primary is always selected) and follows it into or out of selection mode.
    pub(super) fn sync(&mut self, primary: Option<i64>) {
        self.sel.sync(primary);
        if self.sel.is_multi() {
            self.picking = true;
        } else if self.sel.is_empty() {
            self.picking = self.choosing_for.is_some();
        }
    }

    /// Whether selection mode applies with the current selection: something is selected, or
    /// photos are being chosen for an album.
    fn picking_now(&self) -> bool {
        !self.sel.is_empty() || self.choosing_for.is_some()
    }

    /// Drops the selection and leaves selection mode (unless photos are being chosen for an
    /// album, which stays in selection mode).
    pub(super) fn clear(&mut self) {
        self.picking = self.choosing_for.is_some();
        self.selected = None;
        self.sel.clear();
    }
}

/// What the user did to the tiles of a grid during one frame.
#[derive(Default)]
pub(super) struct GridEvents {
    pub(super) clicked: Option<(usize, tiles::TileEvent, ClickMods)>,
    /// A click on an extra button of tile `i` ("Find similar", the playable pill).
    pub(super) action: Option<(usize, tiles::TileAction)>,
    pub(super) right_clicked: Option<usize>,
    pub(super) menu: Option<(usize, TileMenu)>,
    /// The circle of a day heading was clicked: the index of the day.
    pub(super) day_toggled: Option<usize>,
    /// The first item of the first visible row.
    pub(super) top_item: Option<usize>,
}

impl GridEvents {
    /// Notes what the user did to tile `i` (drawn as `out`); `menu` draws the tile's
    /// context menu and says which entry was chosen.
    pub(super) fn record(
        &mut self,
        i: usize,
        out: &tiles::TileOut,
        menu: impl FnOnce(&mut egui::Ui) -> Option<TileMenu>,
    ) {
        if let Some((ev, mods)) = out.event {
            self.clicked = Some((i, ev, mods));
        }
        if let Some(a) = out.action {
            self.action = Some((i, a));
        }
        if out.resp.secondary_clicked() {
            self.right_clicked = Some(i);
        }
        out.resp.context_menu(|ui| {
            if let Some(m) = menu(ui) {
                self.menu = Some((i, m));
            }
        });
    }
}

impl App {
    /// Acts on a right click and on a choice from a tile's context menu.
    pub(super) fn apply_menu_events(&mut self, events: &mut GridEvents) {
        if let Some(i) = events.right_clicked.take() {
            self.select_for_menu(i);
        }
        if let Some((i, m)) = events.menu.take() {
            self.apply_tile_menu(i, m);
        }
    }
}

impl GridState {
    /// Selects the items `range` of a list of `len` items (a day), or deselects them if they
    /// are all selected already.
    pub(super) fn toggle_range(
        &mut self,
        range: std::ops::Range<usize>,
        len: usize,
        id_at: &dyn Fn(usize) -> i64,
    ) {
        let ids: Vec<i64> = range.clone().filter(|&k| k < len).map(id_at).collect();
        if self.sel.toggle_all(&ids) {
            self.selected = Some(range.start);
        } else if !self
            .selected
            .is_some_and(|p| p < len && self.sel.contains(id_at(p)))
        {
            // The primary item was in the day: another selected item takes over.
            self.selected = (0..len).find(|&k| self.sel.contains(id_at(k)));
        }
        self.picking = self.picking_now();
    }
}

/// A vertical scroll area for a grid that fills its panel; it starts at `scroll_to` if one
/// is waiting (which is used up).
pub(super) fn grid_scroll_area(
    id_salt: &'static str,
    scroll_to: &mut Option<f32>,
) -> egui::ScrollArea {
    // The ruler beside the grid is its scroll bar.
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, false]);
    if let Some(y) = scroll_to.take() {
        area = area.vertical_scroll_offset(y);
    }
    area
}

/// What a click on a tile left to do after the selection was updated.
pub(super) enum ClickOutcome {
    Favorite,
    Selected,
    Open,
}

impl GridState {
    /// Applies a click on item `i` of a list of `len` items (`id_at` gives their file ids) to
    /// the selection.
    pub(super) fn apply_click(
        &mut self,
        i: usize,
        ev: tiles::TileEvent,
        mods: ClickMods,
        len: usize,
        id_at: &dyn Fn(usize) -> i64,
    ) -> ClickOutcome {
        match tiles::click_plan(ev, mods, self.picking) {
            tiles::ClickPlan::Favorite => ClickOutcome::Favorite,
            tiles::ClickPlan::Select { mods, fresh } => {
                if fresh {
                    self.sel.clear();
                    self.selected = None;
                }
                self.selected = self.sel.click(self.selected, len, id_at, i, mods);
                self.picking = self.picking_now();
                ClickOutcome::Selected
            }
            tiles::ClickPlan::Open => {
                self.selected = self
                    .sel
                    .click(self.selected, len, id_at, i, ClickMods::default());
                ClickOutcome::Open
            }
        }
    }
}

/// Direction of a keyboard move through the results grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nav {
    Left,
    Right,
    Up,
    Down,
}

/// Where the selection goes after an arrow key. Moves clamp at the ends of the list;
/// up/down stay put when there is no row above/below. With nothing selected, any arrow
/// picks the first result.
pub(super) fn step(selected: Option<usize>, len: usize, cols: usize, key: Nav) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(cur) = selected.filter(|&i| i < len) else {
        return Some(0);
    };
    let cols = cols.max(1);
    Some(match key {
        Nav::Left => cur.saturating_sub(1),
        Nav::Right => (cur + 1).min(len - 1),
        Nav::Up if cur >= cols => cur - cols,
        Nav::Down if cur / cols < (len - 1) / cols => (cur + cols).min(len - 1),
        Nav::Up | Nav::Down => cur,
    })
}

/// True once if `key` was pressed without modifiers this frame (the press is consumed).
pub(super) fn take_plain_key(ctx: &egui::Context, key: egui::Key) -> bool {
    ctx.input_mut(|i| consume_plain_key(i, key)) > 0
}

/// Removes and counts presses of `key` that carry no modifiers at all.
pub(super) fn consume_plain_key(i: &mut egui::InputState, key: egui::Key) -> usize {
    let mut count = 0;
    i.events.retain(|e| {
        let hit = matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. }
            if *k == key && !modifiers.any());
        count += usize::from(hit);
        !hit
    });
    count
}

/// Where the scroll offset should go so that `row` is fully visible, or `None` when it
/// already is. `stride` is the distance between row tops, `tile` the row's height.
pub(super) fn scroll_offset_for_row(
    row: usize,
    stride: f32,
    tile: f32,
    view: f32,
    offset: f32,
) -> Option<f32> {
    let top = row as f32 * stride;
    if top < offset {
        Some(top)
    } else if top + tile > offset + view {
        Some(top + tile - view)
    } else {
        None
    }
}

/// What the right-click menu of a tile asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TileMenu {
    Add(AlbumChoice),
    Remove,
}

/// The right-click menu of a tile: "Add to album…" and, inside an album, "Remove from album".
pub(super) fn tile_menu(
    ui: &mut egui::Ui,
    albums: &[AlbumInfo],
    in_album: bool,
) -> Option<TileMenu> {
    let mut out = album_menu(ui, albums).map(TileMenu::Add);
    if in_album && ui.button("Remove from album").clicked() {
        out = Some(TileMenu::Remove);
        ui.close();
    }
    out
}

impl App {
    /// A right-click on tile `i` of the shown grid: a tile outside the selection becomes the
    /// selection; one inside it leaves the selection alone (the menu then acts on all of it).
    fn select_for_menu(&mut self, i: usize) {
        let Some(id) = self.file_id_at(i) else {
            return;
        };
        let grid = self.active_grid();
        if !grid.sel.contains(id) {
            grid.sel.only(id);
            grid.selected = Some(i);
        }
    }

    /// Applies a choice from a tile's right-click menu to the selection it was opened on.
    fn apply_tile_menu(&mut self, i: usize, menu: TileMenu) {
        self.select_for_menu(i);
        self.bulk_action(match menu {
            TileMenu::Add(c) => BulkAction::AddToAlbum(c),
            TileMenu::Remove => BulkAction::RemoveFromAlbum,
        });
    }

    /// Drops the selection of whatever grid is shown.
    pub(super) fn clear_selection(&mut self) {
        self.tl.grid.clear();
        self.res.grid.clear();
    }

    /// The grid that is shown: the search results while a search is active, else the timeline.
    pub(super) fn active_grid(&mut self) -> &mut GridState {
        if self.searching() {
            &mut self.res.grid
        } else {
            &mut self.tl.grid
        }
    }

    /// [`Self::active_grid`] for reading.
    pub(super) fn grid(&self) -> &GridState {
        if self.searching() {
            &self.res.grid
        } else {
            &self.tl.grid
        }
    }

    /// Every file of the shown list, in order, with its favorite flag as loaded (a search can
    /// list a file more than once).
    pub(super) fn listed(&self) -> Box<dyn Iterator<Item = (i64, bool)> + '_> {
        if self.searching() {
            Box::new(self.res.hits.iter().map(|h| (h.file_id, h.favorite)))
        } else {
            Box::new(self.tl.items.iter().map(|it| (it.file_id, it.favorite)))
        }
    }

    /// The file id of item `i` of the shown list.
    fn file_id_at(&self, i: usize) -> Option<i64> {
        if self.searching() {
            self.res.hits.get(i).map(|h| h.file_id)
        } else {
            self.tl.items.get(i).map(|it| it.file_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Nav, grid_scroll_area, scroll_offset_for_row, step};

    #[test]
    fn the_grid_scroll_area_hides_its_scroll_bar() {
        // With a solid (always visible) bar style, a visible bar would take width from the
        // content; the ruler is the scroll bar, so the content gets all of it.
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.spacing.scroll = egui::style::ScrollStyle::solid());
        let mut inner_width = 0.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut full = ctx.run_ui(input, |ui| {
            let out = grid_scroll_area("t", &mut None)
                .show(ui, |ui| ui.allocate_space(egui::vec2(10.0, 5000.0)));
            inner_width = out.inner_rect.width();
            assert!(out.content_size.y > out.inner_rect.height());
            assert!((inner_width - ui.max_rect().width()).abs() < 0.5);
        });
        full.textures_delta.clear();
        assert!(inner_width > 0.0);
    }

    #[test]
    fn scroll_offset_keeps_rows_visible() {
        // 100 rows, 200 px tiles, 4 px item spacing, 500 px viewport.
        let (stride, tile, view) = (212.0, 200.0, 500.0);
        assert_eq!(scroll_offset_for_row(0, stride, tile, view, 0.0), None);
        assert_eq!(scroll_offset_for_row(1, stride, tile, view, 0.0), None);
        // Row 2 ends at 624 > 500: scroll so its bottom edge meets the viewport bottom.
        assert_eq!(
            scroll_offset_for_row(2, stride, tile, view, 0.0),
            Some(124.0)
        );
        // Far down the grid the offset uses the real stride.
        let o = scroll_offset_for_row(99, stride, tile, view, 0.0).unwrap();
        assert_eq!(o, 99.0 * stride + tile - view);
        assert_eq!(scroll_offset_for_row(99, stride, tile, view, o), None);
        // Scrolling back up aligns the row's top with the viewport top.
        assert_eq!(
            scroll_offset_for_row(50, stride, tile, view, o),
            Some(50.0 * stride)
        );
        // Already fully visible in the middle of the list.
        assert_eq!(
            scroll_offset_for_row(50, stride, tile, view, 50.0 * stride - 100.0),
            None
        );
    }

    #[test]
    fn steps_through_the_grid() {
        assert_eq!(step(None, 0, 4, Nav::Right), None);
        assert_eq!(step(None, 5, 4, Nav::Down), Some(0));
        assert_eq!(step(Some(0), 5, 4, Nav::Left), Some(0));
        assert_eq!(step(Some(4), 5, 4, Nav::Right), Some(4));
        assert_eq!(step(Some(1), 5, 4, Nav::Right), Some(2));
        assert_eq!(step(Some(2), 5, 4, Nav::Up), Some(2));
        assert_eq!(step(Some(5), 9, 4, Nav::Up), Some(1));
        // Down from a full row into a short last row clamps to its final tile.
        assert_eq!(step(Some(3), 6, 4, Nav::Down), Some(5));
        // No row below: stay.
        assert_eq!(step(Some(5), 6, 4, Nav::Down), Some(5));
        assert_eq!(step(Some(0), 1, 4, Nav::Down), Some(0));
        assert_eq!(step(Some(9), 3, 0, Nav::Down), Some(0));
    }
}
