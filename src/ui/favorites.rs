//! Favorite marks: shown at once, saved in the background, and taken back if saving fails.

use super::App;
use super::timeline::View;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A favorite mark the user set that the library may not have stored yet. While one exists it
/// wins over what a list loaded from the library says (a reload that started before the write
/// finished still carries the old value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FavMark {
    pub(super) value: bool,
    /// The write was committed (or given up on); the mark is dropped once no list that was
    /// loaded earlier can still arrive.
    pub(super) confirmed: bool,
}

/// The favorite flag to show for a file: the pending mark if there is one, else `listed`.
pub(super) fn marked(marks: &HashMap<i64, FavMark>, file_id: i64, listed: bool) -> bool {
    marks.get(&file_id).map_or(listed, |m| m.value)
}

impl App {
    /// F with several items selected: favorite them all, or unfavorite them all if they
    /// already are.
    pub(super) fn toggle_favorite_of_selection(&mut self) {
        let ids = self.selected_file_ids();
        let all_fav = self
            .listed()
            .filter(|&(id, _)| self.grid().sel.contains(id))
            .all(|(id, fav)| marked(&self.fav_marks, id, fav));
        self.set_favorites(&ids, !all_fav);
    }

    /// Marks (or unmarks) a favorite everywhere it is shown right away, and has it stored in
    /// the background. The mark wins over lists loaded from the library until the write is
    /// confirmed; if the write fails, the mark is taken back (see [`Self::poll_favorites`]).
    pub(super) fn set_favorite(&mut self, file_id: i64, fav: bool) {
        self.set_favorites(&[file_id], fav);
    }

    /// [`Self::set_favorite`] for several files, patching the lists once.
    pub(super) fn set_favorites(&mut self, file_ids: &[i64], fav: bool) {
        for &file_id in file_ids {
            self.fav_marks.insert(
                file_id,
                FavMark {
                    value: fav,
                    confirmed: false,
                },
            );
        }
        self.patch_lists();
        for &file_id in file_ids {
            if let Some(v) = &mut self.viewer {
                v.set_favorite(file_id, fav);
            }
            self.lib.queue_favorite(file_id, fav);
        }
        if self.tl.view == View::Favorites && !fav {
            self.tl
                .remove_files(&file_ids.iter().copied().collect::<HashSet<i64>>());
        }
    }

    /// Writes the marks into the lists. A list the viewer holds a snapshot of is left alone
    /// (copying it just for a flag could mean hundreds of thousands of items); the grids draw
    /// marks over their lists anyway, and the list is patched once the viewer is gone.
    pub(super) fn patch_lists(&mut self) {
        let marks = &self.fav_marks;
        if marks.is_empty() {
            return;
        }
        if let Some(items) = Arc::get_mut(&mut self.tl.items) {
            for it in items.iter_mut() {
                if let Some(m) = marks.get(&it.file_id) {
                    it.favorite = m.value;
                }
            }
        }
        if let Some(hits) = Arc::get_mut(&mut self.res.hits) {
            for h in hits.iter_mut() {
                if let Some(m) = marks.get(&h.file_id) {
                    h.favorite = m.value;
                }
            }
        }
    }

    /// Per frame: confirmations and failures of favorite writes, and what the writes changed.
    /// Failures are handled as one batch: one notice and one pass over the lists.
    pub(super) fn poll_favorites(&mut self) {
        let mut failed = 0;
        let mut first_error = String::new();
        let mut reverted = false;
        for w in self.lib.take_favorite_results() {
            let pending = self
                .fav_marks
                .get(&w.file_id)
                .is_some_and(|m| m.value == w.fav);
            match w.error {
                None => {
                    if pending && let Some(m) = self.fav_marks.get_mut(&w.file_id) {
                        m.confirmed = true;
                    }
                }
                Some(e) => {
                    failed += 1;
                    if first_error.is_empty() {
                        first_error = e;
                    }
                    // A newer mark for the same file is still on its way: it decides.
                    if pending {
                        // The library still has the earlier state.
                        let value = !w.fav;
                        self.fav_marks.insert(
                            w.file_id,
                            FavMark {
                                value,
                                confirmed: true,
                            },
                        );
                        if let Some(v) = &mut self.viewer {
                            v.set_favorite(w.file_id, value);
                        }
                        reverted = true;
                    }
                }
            }
        }
        if failed == 1 {
            tracing::warn!("could not save a favorite mark: {first_error}");
            self.notify_error("Couldn't save your favorite. Try again in a moment.");
        } else if failed > 1 {
            self.notify_error(format!(
                "Couldn't save favorites for {failed} items. Try again in a moment."
            ));
        }
        if reverted {
            // An item taken out of the Favorites list has to come back.
            if self.tl.view == View::Favorites {
                self.tl.requested = None;
            }
            self.patch_lists();
        }
        let version = self.lib.favorites_version();
        if version != self.fav_version_seen {
            self.fav_version_seen = version;
            if self.tl.view == View::Favorites {
                // What the view lists has changed. (Search results have no favorites filter or
                // order: patching their flags is all they need.)
                self.tl.requested = None;
            }
        }
    }

    /// Drops confirmed marks once nothing loaded earlier can arrive any more and the lists
    /// carry the values themselves.
    pub(super) fn settle_favorites(&mut self) {
        if !self.fav_marks.values().any(|m| m.confirmed)
            || self.tl.awaiting.is_some()
            || self.res.awaiting.is_some()
        {
            return;
        }
        self.patch_lists();
        if Arc::strong_count(&self.tl.items) == 1 && Arc::strong_count(&self.res.hits) == 1 {
            self.fav_marks.retain(|_, m| !m.confirmed);
        }
    }
}
