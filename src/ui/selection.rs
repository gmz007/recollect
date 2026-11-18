//! Multi-selection for the grids (timeline views, album views and search results).
//!
//! A [`Selection`] holds the selected files by id, so it survives reloads and re-sorting. The
//! grid additionally keeps a *primary* item (by list index): the one last clicked or reached
//! with the keyboard, which drives the viewer. The primary is always part of
//! a non-empty selection; [`Selection::sync`] restores that after the grid changed the primary
//! by itself.

use super::favorites::FavoriteKey;
use super::grid::take_plain_key;
use super::{GridState, Nav};
use std::collections::HashSet;

/// Which modifier keys a click carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ClickMods {
    /// Ctrl (Cmd on macOS): toggle one item.
    pub toggle: bool,
    /// Shift: select the range from the anchor.
    pub range: bool,
}

impl From<egui::Modifiers> for ClickMods {
    fn from(m: egui::Modifiers) -> Self {
        Self {
            toggle: m.command,
            range: m.shift && !m.command,
        }
    }
}

impl ClickMods {
    /// The modifiers of this frame's click: those the release event carried, else the keys held
    /// right now.
    pub fn of_click(ctx: &egui::Context) -> Self {
        ctx.input(|i| {
            let on_release = i.events.iter().rev().find_map(|e| match e {
                egui::Event::PointerButton {
                    pressed: false,
                    modifiers,
                    ..
                } => Some(*modifiers),
                _ => None,
            });
            Self::from(on_release.unwrap_or(i.modifiers))
        })
    }

    /// True when the click only selects (a double click with modifiers is not an "open").
    pub fn is_plain(self) -> bool {
        !self.toggle && !self.range
    }
}

#[derive(Debug, Default)]
pub(super) struct Selection {
    ids: HashSet<i64>,
    /// Where a Shift range starts: the last item selected without Shift.
    anchor: Option<i64>,
}

impl Selection {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: i64) -> bool {
        self.ids.contains(&id)
    }

    /// True while two or more items are selected.
    pub fn is_multi(&self) -> bool {
        self.ids.len() >= 2
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }

    /// Deselects every file of `ids` in one pass.
    pub fn remove_all(&mut self, ids: &HashSet<i64>) {
        self.ids.retain(|id| !ids.contains(id));
        if self.anchor.is_some_and(|a| ids.contains(&a)) {
            self.anchor = None;
        }
    }

    /// Selects all of `ids`, or deselects them all if every one is selected already. Returns
    /// true when they are selected afterwards.
    pub fn toggle_all(&mut self, ids: &[i64]) -> bool {
        if ids.iter().all(|id| self.ids.contains(id)) {
            for id in ids {
                self.ids.remove(id);
            }
            if self.anchor.is_some_and(|a| !self.ids.contains(&a)) {
                self.anchor = None;
            }
            false
        } else {
            self.ids.extend(ids.iter().copied());
            self.anchor = ids.first().copied();
            true
        }
    }

    /// Selects just `id`.
    pub fn only(&mut self, id: i64) {
        self.ids.clear();
        self.ids.insert(id);
        self.anchor = Some(id);
    }

    /// Makes the selection consistent with the primary item's file id: nothing without a
    /// primary, and the primary is always selected.
    pub fn sync(&mut self, primary: Option<i64>) {
        match primary {
            None => self.clear(),
            Some(id) if !self.ids.contains(&id) => self.only(id),
            Some(_) => {}
        }
    }

    /// After the list was replaced: drops ids that are gone and, if the primary item vanished
    /// while others remain, moves the primary to the first remaining one. `new_ids` are the file
    /// ids of the new list in order; `primary` is the index already found for the old primary.
    pub fn rebase(
        &mut self,
        new_ids: impl Iterator<Item = i64>,
        primary: Option<usize>,
    ) -> Option<usize> {
        if self.ids.is_empty() {
            return primary;
        }
        let mut kept = HashSet::new();
        let mut first = None;
        for (pos, id) in new_ids.enumerate() {
            if self.ids.contains(&id) {
                kept.insert(id);
                first.get_or_insert(pos);
            }
        }
        self.ids = kept;
        if self.anchor.is_some_and(|a| !self.ids.contains(&a)) {
            self.anchor = None;
        }
        primary.or(first)
    }

    /// Applies a click on item `i` of a list of `len` items whose file ids are `id_at`;
    /// returns the new primary index.
    pub fn click(
        &mut self,
        primary: Option<usize>,
        len: usize,
        id_at: &dyn Fn(usize) -> i64,
        i: usize,
        mods: ClickMods,
    ) -> Option<usize> {
        let id = id_at(i);
        if mods.toggle {
            if self.ids.remove(&id) {
                // The primary was deselected (the same file, which a list may show at several
                // positions): the lowest remaining item takes over.
                if primary.is_some_and(|p| p < len && id_at(p) == id) {
                    self.anchor = None;
                    return (0..len).find(|&k| self.ids.contains(&id_at(k)));
                }
                return primary;
            }
            self.ids.insert(id);
            self.anchor = Some(id);
            Some(i)
        } else if mods.range {
            self.extend_to(primary, len, id_at, i);
            Some(i)
        } else {
            self.only(id);
            Some(i)
        }
    }

    /// Selects the range between the anchor (else the primary, else `to`) and `to`.
    pub fn extend_to(
        &mut self,
        primary: Option<usize>,
        len: usize,
        id_at: &dyn Fn(usize) -> i64,
        to: usize,
    ) {
        if len == 0 {
            return;
        }
        let to = to.min(len - 1);
        // The anchor is a file: when it is listed twice, the primary's position wins.
        let from = self
            .anchor
            .and_then(|a| {
                primary
                    .filter(|&p| p < len && id_at(p) == a)
                    .or_else(|| (0..len).find(|&k| id_at(k) == a))
            })
            .or(primary.filter(|&p| p < len))
            .unwrap_or(to);
        if self.anchor.is_none() {
            self.anchor = Some(id_at(from));
        }
        let (lo, hi) = (from.min(to), from.max(to));
        self.ids = (lo..=hi).map(id_at).collect();
    }

    /// Selects every item; returns the primary (kept, or the first item).
    pub fn select_all(
        &mut self,
        primary: Option<usize>,
        len: usize,
        id_at: &dyn Fn(usize) -> i64,
    ) -> Option<usize> {
        if len == 0 {
            return None;
        }
        self.ids = (0..len).map(id_at).collect();
        let p = primary.filter(|&p| p < len).unwrap_or(0);
        self.anchor = Some(id_at(p));
        Some(p)
    }
}

/// The selection-related keys pressed this frame.
#[derive(Debug, Default, PartialEq, Eq)]
struct NavInput {
    /// Arrow presses without modifiers, in order.
    pub plain: Vec<Nav>,
    /// Arrow presses with Shift only.
    pub shift: Vec<Nav>,
    pub enter: bool,
    pub select_all: bool,
}

/// Reads (and consumes) this frame's arrows, Shift+arrows, Enter (unless a widget has focus)
/// and Ctrl/Cmd+A. A focused
/// button would otherwise move with the arrows, so focus is dropped when one was used.
fn read_nav_keys(ctx: &egui::Context) -> NavInput {
    use egui::Key;
    let mut out = NavInput::default();
    // Enter belongs to whatever widget has keyboard focus (a button, a filter chip, a tile);
    // it opens the selected item only when nothing is focused.
    let nothing_focused = ctx.memory(egui::Memory::focused).is_none();
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = e
            else {
                return true;
            };
            let nav = match key {
                Key::ArrowLeft => Some(Nav::Left),
                Key::ArrowRight => Some(Nav::Right),
                Key::ArrowUp => Some(Nav::Up),
                Key::ArrowDown => Some(Nav::Down),
                _ => None,
            };
            if let Some(n) = nav {
                if !modifiers.any() {
                    out.plain.push(n);
                    return false;
                }
                if modifiers.shift && !modifiers.command && !modifiers.alt {
                    out.shift.push(n);
                    return false;
                }
            } else if *key == Key::Enter && !modifiers.any() && nothing_focused {
                out.enter = true;
                return false;
            } else if *key == Key::A && modifiers.command && !modifiers.shift && !modifiers.alt {
                out.select_all = true;
                return false;
            }
            true
        });
    });
    if (!out.plain.is_empty() || !out.shift.is_empty())
        && let Some(id) = ctx.memory(egui::Memory::focused)
    {
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
    out
}

/// The list a grid shows, as far as the keys need to know it.
pub(super) struct GridList<'a> {
    pub(super) len: usize,
    pub(super) id_at: &'a dyn Fn(usize) -> i64,
    /// The stored favorite flag of the item at an index.
    pub(super) favorite_at: &'a dyn Fn(usize) -> bool,
}

/// What this frame's grid keys asked for beyond moving the selection.
pub(super) struct GridKeyResult {
    /// The primary selection moved (to a new item or off the list).
    pub(super) moved: bool,
    /// F was pressed: what to favorite, taken from the selection after the arrows were applied.
    pub(super) favorite: Option<FavoriteKey>,
    /// Enter was pressed on a selected item.
    pub(super) open: Option<usize>,
}

/// The flow shared by the results grid and the timeline: read the keys, move the selection
/// with `step` (the grid's own idea of up, down, left and right), then look at F and Enter
/// against the item the arrows moved to.
pub(super) fn apply_grid_keys(
    ctx: &egui::Context,
    grid: &mut GridState,
    list: &GridList<'_>,
    step: impl Fn(Option<usize>, Nav) -> Option<usize>,
) -> GridKeyResult {
    let input = read_nav_keys(ctx);
    let before = grid.selected;
    let next = apply_keys(&input, &mut grid.sel, before, list.len, list.id_at, step);
    let moved = next != before;
    if moved {
        grid.selected = next;
        grid.keyboard_moved = true;
    }
    let selected = grid.selected.filter(|&i| i < list.len);
    let favorite = take_plain_key(ctx, egui::Key::F)
        .then(|| {
            if grid.sel.is_multi() {
                Some(FavoriteKey::Selection)
            } else {
                selected.map(|i| FavoriteKey::Item((list.id_at)(i), (list.favorite_at)(i)))
            }
        })
        .flatten();
    let open = selected.filter(|_| input.enter);
    GridKeyResult {
        moved,
        favorite,
        open,
    }
}

/// Applies this frame's keys to the selection of a list and returns the new primary index.
/// Plain arrows move a single selection (a multi-selection collapses to the new primary, even
/// at the edge of the list); Shift+arrows extend from the anchor; Ctrl/Cmd+A selects all.
fn apply_keys(
    input: &NavInput,
    sel: &mut Selection,
    primary: Option<usize>,
    len: usize,
    id_at: &dyn Fn(usize) -> i64,
    step: impl Fn(Option<usize>, Nav) -> Option<usize>,
) -> Option<usize> {
    let mut cur = primary;
    if input.select_all {
        cur = sel.select_all(cur, len, id_at);
    }
    if !input.plain.is_empty() {
        for &n in &input.plain {
            cur = step(cur, n);
        }
        if let Some(c) = cur.filter(|&c| c < len) {
            sel.only(id_at(c));
        }
    }
    if !input.shift.is_empty() {
        let start = cur;
        let mut moved = cur;
        for &n in &input.shift {
            moved = step(moved, n);
        }
        if let Some(m) = moved.filter(|&m| m < len) {
            if start.is_none() {
                sel.only(id_at(m));
            } else {
                sel.extend_to(start, len, id_at, m);
            }
            cur = Some(m);
        }
    }
    cur
}

#[cfg(test)]
mod tests {
    #![expect(clippy::cast_possible_wrap, reason = "small test fixture indices")]
    use super::*;

    const ID: &dyn Fn(usize) -> i64 = &|i| 100 + i as i64;

    fn ids(s: &Selection) -> Vec<i64> {
        let mut v: Vec<i64> = s.ids.iter().copied().collect();
        v.sort_unstable();
        v
    }

    const TOGGLE: ClickMods = ClickMods {
        toggle: true,
        range: false,
    };
    const RANGE: ClickMods = ClickMods {
        toggle: false,
        range: true,
    };

    #[test]
    fn plain_toggle_and_range_clicks() {
        let mut s = Selection::default();
        let mut p = s.click(None, 10, ID, 2, ClickMods::default());
        assert_eq!((p, ids(&s)), (Some(2), vec![102]));
        p = s.click(p, 10, ID, 5, TOGGLE);
        assert_eq!((p, ids(&s)), (Some(5), vec![102, 105]));
        // Shift ranges from the anchor (the last non-Shift click, 5), replacing the selection.
        p = s.click(p, 10, ID, 8, RANGE);
        assert_eq!((p, ids(&s)), (Some(8), vec![105, 106, 107, 108]));
        p = s.click(p, 10, ID, 3, RANGE);
        assert_eq!((p, ids(&s)), (Some(3), vec![103, 104, 105]));
        // Toggling the primary off hands it to the lowest remaining item.
        p = s.click(p, 10, ID, 3, TOGGLE);
        assert_eq!((p, ids(&s)), (Some(4), vec![104, 105]));
        // A plain click collapses.
        p = s.click(p, 10, ID, 9, ClickMods::default());
        assert_eq!((p, ids(&s)), (Some(9), vec![109]));
        // Toggling the last item off leaves nothing.
        p = s.click(p, 10, ID, 9, TOGGLE);
        assert_eq!((p, s.len()), (None, 0));
    }

    #[test]
    fn shift_without_an_anchor_starts_at_the_primary() {
        let mut s = Selection::default();
        let p = s.click(Some(1), 10, ID, 3, RANGE);
        assert_eq!((p, ids(&s)), (Some(3), vec![101, 102, 103]));
    }

    #[test]
    fn keys_collapse_extend_and_select_all() {
        let step = |sel: Option<usize>, n: Nav| match (sel, n) {
            (None, _) => Some(0),
            (Some(i), Nav::Right) => Some((i + 1).min(5)),
            (Some(i), Nav::Left) => Some(i.saturating_sub(1)),
            (s, _) => s,
        };
        let mut s = Selection::default();
        let all = NavInput {
            select_all: true,
            ..Default::default()
        };
        let mut p = apply_keys(&all, &mut s, Some(2), 6, ID, step);
        assert_eq!((p, s.len()), (Some(2), 6));
        // A plain arrow collapses to one, moved.
        let right = NavInput {
            plain: vec![Nav::Right],
            ..Default::default()
        };
        p = apply_keys(&right, &mut s, p, 6, ID, step);
        assert_eq!((p, ids(&s)), (Some(3), vec![103]));
        // Shift+arrow extends from the anchor and back.
        let shift_right = NavInput {
            shift: vec![Nav::Right, Nav::Right],
            ..Default::default()
        };
        p = apply_keys(&shift_right, &mut s, p, 6, ID, step);
        assert_eq!((p, ids(&s)), (Some(5), vec![103, 104, 105]));
        let shift_left = NavInput {
            shift: vec![Nav::Left],
            ..Default::default()
        };
        p = apply_keys(&shift_left, &mut s, p, 6, ID, step);
        assert_eq!((p, ids(&s)), (Some(4), vec![103, 104]));
        // At the edge a plain arrow still collapses.
        let mut s2 = Selection::default();
        s2.select_all(Some(5), 6, ID);
        let p = apply_keys(&right, &mut s2, Some(5), 6, ID, step);
        assert_eq!((p, ids(&s2)), (Some(5), vec![105]));
    }

    #[test]
    fn rebase_drops_vanished_ids_and_keeps_a_primary() {
        let mut s = Selection::default();
        s.select_all(Some(0), 5, ID);
        // 100 and 103 are gone; the old primary (100) is not found any more.
        let new: Vec<i64> = vec![101, 102, 104, 110];
        let p = s.rebase(new.iter().copied(), None);
        assert_eq!((p, ids(&s)), (Some(0), vec![101, 102, 104]));
        // Nothing left: no primary.
        let mut s = Selection::default();
        s.only(100);
        assert_eq!(s.rebase([1, 2].into_iter(), None), None);
        assert_eq!(s.len(), 0);
    }

    /// A search can list one file twice: ids at indices 0 and 3 are the same file.
    const DUP: &dyn Fn(usize) -> i64 = &|i| [7, 8, 9, 7, 10][i];

    #[test]
    fn a_file_listed_twice_is_selected_and_toggled_as_one() {
        let mut s = Selection::default();
        let mut p = s.click(None, 5, DUP, 0, ClickMods::default());
        p = s.click(p, 5, DUP, 1, TOGGLE);
        assert_eq!((p, s.len()), (Some(1), 2));
        // Ctrl+click on the other hit of selected file 7 deselects that file only.
        p = s.click(p, 5, DUP, 3, TOGGLE);
        assert_eq!((p, s.len()), (Some(1), 1));
        assert!(!s.contains(7) && s.contains(8));
        // Toggling the primary's file through its twin hands the primary over.
        p = s.click(p, 5, DUP, 4, TOGGLE);
        p = s.click(p, 5, DUP, 1, TOGGLE);
        assert_eq!(p, Some(4));
        // Select all counts files, not hits.
        let all = s.select_all(None, 5, DUP);
        assert_eq!((all, s.len()), (Some(0), 4));
        // A range runs over the visible order and also counts files.
        s.only(9);
        let p = s.click(Some(2), 5, DUP, 4, RANGE);
        assert_eq!((p, ids(&s)), (Some(4), vec![7, 9, 10]));
    }

    #[test]
    fn removing_many_ids_clears_a_stale_anchor() {
        let mut s = Selection::default();
        s.select_all(Some(1), 4, ID);
        s.remove_all(&[100, 101].into_iter().collect());
        assert_eq!(ids(&s), vec![102, 103]);
        assert_eq!(s.anchor, None);
    }

    #[test]
    fn extending_with_a_stale_primary_stays_inside_the_list() {
        let mut s = Selection::default();
        // The primary points past the end (the list shrank): the range starts at `to`.
        s.extend_to(Some(10), 3, ID, 1);
        assert_eq!(ids(&s), vec![101]);
        // A target past the end is clamped, an empty list selects nothing.
        let mut s = Selection::default();
        s.extend_to(Some(0), 3, ID, 50);
        assert_eq!(ids(&s), vec![100, 101, 102]);
        let mut s = Selection::default();
        s.extend_to(None, 0, ID, 0);
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn sync_keeps_the_primary_selected() {
        let mut s = Selection::default();
        s.only(1);
        s.sync(Some(2));
        assert_eq!(ids(&s), vec![2]);
        s.sync(None);
        assert_eq!(s.len(), 0);
    }
}
