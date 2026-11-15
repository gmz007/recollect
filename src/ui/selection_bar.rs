//! The bar shown above a grid while items are selected, and what its buttons do.

use super::App;
use super::albums::{AlbumChoice, album_menu};
use super::style::{self, Palette, icons};
use super::timeline::{View, thousands};
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

/// Buttons on the accent colour: light text on a faint veil.
pub(super) fn veil_buttons(ui: &mut egui::Ui, pal: Palette) {
    let v = ui.visuals_mut();
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.fg_stroke.color = pal.on_accent;
        w.weak_bg_fill = pal.on_accent.gamma_multiply(0.16);
        w.bg_fill = w.weak_bg_fill;
    }
    v.widgets.hovered.weak_bg_fill = pal.on_accent.gamma_multiply(0.28);
    v.widgets.open.fg_stroke.color = pal.on_accent;
    v.widgets.open.weak_bg_fill = pal.on_accent.gamma_multiply(0.28);
}

/// The accent bar that replaces the view header while items are selected.
pub(super) fn selection_bar(
    ui: &mut egui::Ui,
    count: usize,
    albums: &[AlbumInfo],
    in_album: bool,
) -> Option<BulkAction> {
    let pal = Palette::of(ui.ctx());
    let mut action = None;
    egui::Frame::new()
        .fill(pal.accent)
        .corner_radius(egui::CornerRadius::same(style::CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            veil_buttons(ui, pal);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    style::semibold(format!("{} selected", thousands(count)))
                        .size(16.0)
                        .color(pal.on_accent),
                );
                ui.add_space(8.0);
                if let Some(c) = album_menu(ui, albums) {
                    action = Some(BulkAction::AddToAlbum(c));
                }
                if style::icon_label_button(ui, icons::ICON_STAR, "Favorite").clicked() {
                    action = Some(BulkAction::Favorite(true));
                }
                if style::icon_label_button(ui, icons::ICON_STAR.outlined(), "Unfavorite").clicked()
                {
                    action = Some(BulkAction::Favorite(false));
                }
                if in_album
                    && style::icon_label_button(
                        ui,
                        icons::ICON_PLAYLIST_REMOVE,
                        "Remove from album",
                    )
                    .clicked()
                {
                    action = Some(BulkAction::RemoveFromAlbum);
                }
                if style::icon_label_button(ui, icons::ICON_CLOSE, "Cancel").clicked() {
                    action = Some(BulkAction::Clear);
                }
            });
        });
    ui.add_space(8.0);
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

    /// The selection bar, in place of the view header while items are selected. Returns whether
    /// it was shown.
    pub(super) fn selection_bar_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let grid = self.grid();
        let (count, picking) = (grid.sel.len(), grid.picking);
        if count == 0 || !picking {
            return false;
        }
        let in_album = !self.searching() && matches!(self.tl.view, View::Album(_));
        if let Some(a) = selection_bar(ui, count, &self.albums, in_album) {
            self.bulk_action(a);
        }
        true
    }

    pub(super) fn bulk_action(&mut self, action: BulkAction) {
        let files = self.selected_file_ids();
        match action {
            BulkAction::AddToAlbum(c) => self.add_to_album_choice(files, c),
            BulkAction::Favorite(fav) => self.set_favorites(&files, fav),
            BulkAction::RemoveFromAlbum => {
                // The grid drops them once the library confirms (see `report_album_write`).
                if let View::Album(album) = self.tl.view {
                    self.lib.queue_album(AlbumOp::Remove { album, files });
                }
            }
            BulkAction::Clear => self.clear_selection(),
        }
    }
}
