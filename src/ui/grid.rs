//! What the timeline grid and the results grid share: selection state, what a click
//! does, and moving through tiles with the keyboard.

use super::selection::{ClickMods, Selection};
use super::{App, albums, tiles};

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
    /// Whether the keyboard moved the selection since the last frame.
    pub(super) keyboard_moved: bool,
    /// Scroll offset to apply on the next frame so the keyboard selection stays visible.
    pub(super) scroll_to: Option<f32>,
    /// Set when the grid was drawn; read (and cleared) at the start of the next frame.
    pub(super) shown: bool,
}

impl GridState {
    /// Keeps the selection consistent with its primary item (nothing is selected without a
    /// primary; the primary is always selected) and follows it into or out of selection mode.
    pub(super) fn sync(&mut self, primary: Option<i64>) {
        self.sel.sync(primary);
        if self.sel.is_multi() {
            self.picking = true;
        } else if self.sel.is_empty() {
            self.picking = false;
        }
    }

    /// Drops the selection and leaves selection mode.
    pub(super) fn clear(&mut self) {
        self.picking = false;
        self.selected = None;
        self.sel.clear();
    }
}

/// What the user did to the tiles of a grid during one frame.
#[derive(Default)]
pub(super) struct GridEvents {
    pub(super) clicked: Option<(usize, tiles::TileEvent, ClickMods)>,
    pub(super) right_clicked: Option<usize>,
    pub(super) menu: Option<(usize, albums::TileMenu)>,
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
        menu: impl FnOnce(&mut egui::Ui) -> Option<albums::TileMenu>,
    ) {
        if let Some((ev, mods)) = out.event {
            self.clicked = Some((i, ev, mods));
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

/// A vertical scroll area for a grid that fills its panel; it starts at `scroll_to` if one
/// is waiting (which is used up).
pub(super) fn grid_scroll_area(
    id_salt: &'static str,
    scroll_to: &mut Option<f32>,
) -> egui::ScrollArea {
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id_salt)
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
                self.picking = !self.sel.is_empty();
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

#[cfg(test)]
mod tests {
    use super::{Nav, scroll_offset_for_row, step};

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
