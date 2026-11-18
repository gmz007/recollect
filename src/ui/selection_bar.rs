//! The bar shown above a grid while items are selected, and what its buttons do.

use super::App;
use super::albums::{AlbumChoice, album_menu};
use super::favorites::marked;
use super::style::{self, icons};
use super::timeline::{View, thousands};
use super::toast;
use crate::library::AlbumOp;
use crate::store::db::AlbumInfo;
use std::collections::HashSet;

/// What the selection bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BulkAction {
    AddToAlbum(AlbumChoice),
    Favorite(bool),
    RemoveFromAlbum,
    Clear,
}

/// What the selection bar shows.
pub(super) struct BarState<'a> {
    pub count: usize,
    pub albums: &'a [AlbumInfo],
    pub in_album: bool,
    /// Every selected item is a favorite: the heart then takes the mark away.
    pub all_favorite: bool,
}

/// The content of the floating selection pill: how many are selected and what can be done.
pub(super) fn selection_bar(ui: &mut egui::Ui, state: &BarState<'_>) -> Option<BulkAction> {
    let mut action = None;
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.label(style::semibold(format!(
        "{} selected",
        thousands(state.count)
    )));
    ui.add_space(6.0);
    if let Some(c) = album_menu(ui, state.albums) {
        action = Some(BulkAction::AddToAlbum(c));
    }
    if state.in_album {
        let r = style::icon_label_button(ui, icons::ICON_PLAYLIST_REMOVE, "Remove from album")
            .on_hover_text("Takes them out of this album. The photos stay in your library.");
        if r.clicked() {
            action = Some(BulkAction::RemoveFromAlbum);
        }
    }
    let (icon, label) = if state.all_favorite {
        (icons::ICON_FAVORITE, "Unfavorite")
    } else {
        (icons::ICON_FAVORITE.outlined(), "Favorite")
    };
    if style::icon_button_framed(ui, icon, label).clicked() {
        action = Some(BulkAction::Favorite(!state.all_favorite));
    }
    if style::icon_button_framed(ui, icons::ICON_CLOSE, "Cancel").clicked() {
        action = Some(BulkAction::Clear);
    }
    action
}

impl App {
    /// File ids of the selected items in list order, each file once (a search can list a file
    /// more than once).
    pub(super) fn selected_file_ids(&self) -> Vec<i64> {
        let mut seen = HashSet::new();
        let sel = &self.grid().sel;
        self.listed()
            .map(|(id, _)| id)
            .filter(|id| sel.contains(*id) && seen.insert(*id))
            .collect()
    }

    /// Notes where the pill floats and shows the selection bar over the grid, unless a
    /// confirmation (which takes its place) is showing. Nothing reflows.
    pub(super) fn selection_bar_ui(&mut self, ui: &mut egui::Ui) {
        // Noted on every frame the grid shows, selection or not, so a toast keeps its place
        // when the selection ends instead of jumping to the window's centre.
        self.pill_anchor = Some(toast::pill_anchor_of(ui));
        if self.adding_to().is_some() {
            self.adding_bar(ui);
            return;
        }
        let grid = self.grid();
        let (count, picking) = (grid.sel.len(), grid.picking);
        if count == 0 || !picking || self.confirmation_showing() {
            return;
        }
        let in_album = !self.searching() && matches!(self.tl.view, View::Album(_));
        let all_favorite = self.selection_all_favorite();
        let state = BarState {
            count,
            albums: &self.albums,
            in_album,
            all_favorite,
        };
        self.bar_anchor = self.pill_anchor;
        let ctx = ui.ctx().clone();
        let id = egui::Id::unique("selection-bar");
        if let Some(a) =
            toast::pill_area(&ctx, id, self.pill_anchor, |ui| selection_bar(ui, &state)).inner
        {
            self.bulk_action(a);
        }
    }

    /// While photos are being chosen for an album: a slim bar above the grid with what to do,
    /// how many are chosen, "Cancel" and the button that adds them.
    fn adding_bar(&mut self, ui: &mut egui::Ui) {
        let Some(album) = self.adding_to() else {
            return;
        };
        let pal = style::Palette::of(ui.ctx());
        let name = super::albums::short_name(self.album_name(album).unwrap_or("the album"), 40);
        let count = self.grid().sel.len();
        let (mut add, mut cancel) = (false, false);
        egui::Frame::new()
            .fill(pal.tint)
            .corner_radius(egui::CornerRadius::same(style::CARD_RADIUS))
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(style::semibold(format!("Choose photos to add to {name}")));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_enabled_ui(count > 0, |ui| {
                            add = style::primary_button(ui, &format!("Add to {name}")).clicked();
                        });
                        cancel = super::dialogs::secondary_button(ui, "Cancel").clicked();
                        if count > 0 {
                            ui.label(
                                egui::RichText::new(format!("{} selected", thousands(count)))
                                    .color(pal.text2),
                            );
                        }
                    });
                });
            });
        ui.add_space(8.0);
        if add {
            self.finish_adding();
        } else if cancel {
            self.stop_adding();
        }
    }

    /// Whether every selected item is a favorite (as shown, with pending marks).
    pub(super) fn selection_all_favorite(&self) -> bool {
        let sel = &self.grid().sel;
        self.listed()
            .filter(|&(id, _)| sel.contains(id))
            .all(|(id, fav)| marked(&self.fav_marks, id, fav))
    }

    pub(super) fn bulk_action(&mut self, action: BulkAction) {
        let files = self.selected_file_ids();
        match action {
            BulkAction::AddToAlbum(c) => self.add_to_album_choice(files, c),
            BulkAction::Favorite(fav) => self.favorite_selection(&files, fav),
            BulkAction::RemoveFromAlbum => {
                // The grid drops them once the library confirms (see `report_album_write`).
                if let View::Album(album) = self.tl.view {
                    self.lib.queue_album(AlbumOp::Remove { album, files });
                }
            }
            BulkAction::Clear => {}
        }
        // The action is done (Undo still brings it back): the selection has served its purpose.
        // A new album still to be named keeps the selection until it is created.
        if !matches!(action, BulkAction::AddToAlbum(AlbumChoice::New)) {
            self.clear_selection();
        }
    }
}
