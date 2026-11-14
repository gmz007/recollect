//! Albums in the UI: the sidebar section, the name and confirmation dialogs, the
//! "Add to album…" menu and the bar shown above a grid while several items are selected.
//!
//! Album changes go to the library's write queue and come back as [`AlbumWrite`]s; the
//! sidebar's album list is part of the periodic library snapshot and is fetched again whenever
//! the library's album version moves.

use super::App;
use super::timeline::{View, thousands};
use crate::library::{AlbumOp, AlbumWrite};
use crate::store::db::AlbumInfo;
use egui::RichText;

/// Where "Add to album…" should put the items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AlbumChoice {
    Existing(i64),
    /// Ask for a name, create the album and add the items to it.
    New,
}

/// A name being typed for a new album, with the items it starts with.
#[derive(Debug, Default)]
pub(super) struct NewAlbum {
    pub name: String,
    pub add: Vec<i64>,
    focused: bool,
}

impl NewAlbum {
    pub fn with_items(add: Vec<i64>) -> Self {
        Self {
            add,
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub(super) struct RenameAlbum {
    pub id: i64,
    pub name: String,
    focused: bool,
}

/// What the selection bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BulkAction {
    AddToAlbum(AlbumChoice),
    Favorite(bool),
    RemoveFromAlbum,
    Clear,
}

/// "1 item" / "3 items".
pub(super) fn items_text(n: usize) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{} items", thousands(n))
    }
}

/// The "Add to album…" menu button: the albums, then "New album…".
pub(super) fn album_menu(ui: &mut egui::Ui, albums: &[AlbumInfo]) -> Option<AlbumChoice> {
    let mut choice = None;
    ui.menu_button("Add to album…", |ui| {
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                for a in albums {
                    if ui.button(&a.name).clicked() {
                        choice = Some(AlbumChoice::Existing(a.id));
                        ui.close();
                    }
                }
            });
        if !albums.is_empty() {
            ui.separator();
        }
        if ui.button("New album…").clicked() {
            choice = Some(AlbumChoice::New);
            ui.close();
        }
    });
    choice
}

/// The bar above a grid while two or more items are selected.
pub(super) fn selection_bar(
    ui: &mut egui::Ui,
    count: usize,
    albums: &[AlbumInfo],
    in_album: bool,
) -> Option<BulkAction> {
    let mut action = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} selected", thousands(count))).strong());
            if let Some(c) = album_menu(ui, albums) {
                action = Some(BulkAction::AddToAlbum(c));
            }
            if ui.button("Favorite").clicked() {
                action = Some(BulkAction::Favorite(true));
            }
            if ui.button("Unfavorite").clicked() {
                action = Some(BulkAction::Favorite(false));
            }
            if in_album && ui.button("Remove from album").clicked() {
                action = Some(BulkAction::RemoveFromAlbum);
            }
            if ui.button("Clear selection").clicked() {
                action = Some(BulkAction::Clear);
            }
        });
    });
    ui.add_space(4.0);
    action
}

/// A single-line name field. Returns true when Enter was pressed in it.
fn name_field(ui: &mut egui::Ui, text: &mut String, focused: &mut bool, width: f32) -> bool {
    let r = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text("Album name")
            .desired_width(width),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Album name"));
    if !*focused {
        *focused = true;
        r.request_focus();
    }
    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

impl App {
    pub(super) fn album_name(&self, id: i64) -> Option<&str> {
        self.snapshot
            .albums
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.name.as_str())
    }

    /// Sidebar: the albums, and "New album…" (an inline name field while one is being made).
    pub(super) fn albums_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Albums").strong());
        let mut open = None;
        if self.snapshot.albums.is_empty() {
            ui.label(RichText::new("No albums yet.").weak());
        }
        egui::ScrollArea::vertical()
            .id_salt("albums")
            .max_height(120.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for a in &self.snapshot.albums {
                    let label = format!("{} ({})", a.name, thousands(a.count));
                    if ui
                        .selectable_label(self.tl.view == View::Album(a.id), label)
                        .clicked()
                    {
                        open = Some(a.id);
                    }
                }
            });
        if let Some(id) = open {
            self.set_view(View::Album(id));
        }
        let inline = self.new_album.as_ref().is_some_and(|n| n.add.is_empty());
        if inline {
            let mut na = self.new_album.take().unwrap_or_default();
            let enter = name_field(ui, &mut na.name, &mut na.focused, 150.0);
            let (mut create, mut cancel) = (enter, false);
            ui.horizontal(|ui| {
                create |= ui.button("Create").clicked();
                cancel |= ui.button("Cancel").clicked();
            });
            self.new_album = (!cancel).then_some(na);
            if create {
                self.submit_new_album();
            }
        } else if ui.button("New album…").clicked() {
            self.new_album = Some(NewAlbum::default());
        }
    }

    /// Creates the album being named (keeping the dialog open if the name is no good).
    fn submit_new_album(&mut self) {
        let Some(na) = self.new_album.take() else {
            return;
        };
        let name = na.name.trim().to_string();
        if let Err(e) = self.check_album_name(&name, None) {
            self.notify(e);
            self.new_album = Some(na);
            return;
        }
        self.lib.queue_album(AlbumOp::Create { name, add: na.add });
    }

    /// Quick check against the albums we know of; the library has the final say.
    fn check_album_name(&self, name: &str, except: Option<i64>) -> Result<(), String> {
        if name.is_empty() {
            return Err("An album needs a name".into());
        }
        let wanted = name.to_lowercase();
        match self
            .snapshot
            .albums
            .iter()
            .find(|a| Some(a.id) != except && a.name.to_lowercase() == wanted)
        {
            Some(a) => Err(format!("An album named “{}” already exists", a.name)),
            None => Ok(()),
        }
    }

    /// Items go to an existing album, or to a new one once it has a name.
    pub(super) fn add_to_album_choice(&mut self, files: Vec<i64>, choice: AlbumChoice) {
        match choice {
            AlbumChoice::Existing(album) => {
                self.lib.queue_album(AlbumOp::Add { album, files });
            }
            AlbumChoice::New => self.new_album = Some(NewAlbum::with_items(files)),
        }
    }

    pub(super) fn dialog_open(&self) -> bool {
        self.new_album.is_some() || self.rename_album.is_some() || self.delete_album.is_some()
    }

    pub(super) fn cancel_album_dialogs(&mut self) {
        self.new_album = None;
        self.rename_album = None;
        self.delete_album = None;
    }

    /// The windows for naming a new album (when it starts with items), renaming and confirming
    /// a deletion. The inline field for an empty new album lives in the sidebar.
    pub(super) fn album_dialogs(&mut self, ctx: &egui::Context) {
        if self.new_album.as_ref().is_some_and(|n| !n.add.is_empty()) {
            let mut na = self.new_album.take().unwrap_or_default();
            let (mut create, mut cancel) = (false, false);
            egui::Window::new("New album")
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    ui.label(format!("{} will be added to it.", items_text(na.add.len())));
                    create |= name_field(ui, &mut na.name, &mut na.focused, 240.0);
                    ui.horizontal(|ui| {
                        create |= ui.button("Create").clicked();
                        cancel |= ui.button("Cancel").clicked();
                    });
                });
            self.new_album = (!cancel).then_some(na);
            if create {
                self.submit_new_album();
            }
        }
        if let Some(mut r) = self.rename_album.take() {
            let (mut apply, mut cancel) = (false, false);
            egui::Window::new("Rename album")
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    apply |= name_field(ui, &mut r.name, &mut r.focused, 240.0);
                    ui.horizontal(|ui| {
                        apply |= ui.button("Rename").clicked();
                        cancel |= ui.button("Cancel").clicked();
                    });
                });
            if apply {
                let name = r.name.trim().to_string();
                match self.check_album_name(&name, Some(r.id)) {
                    Ok(()) => {
                        self.lib.queue_album(AlbumOp::Rename { id: r.id, name });
                    }
                    Err(e) => {
                        self.notify(e);
                        self.rename_album = Some(r);
                    }
                }
            } else if !cancel {
                self.rename_album = Some(r);
            }
        }
        if let Some(id) = self.delete_album {
            let name = self.album_name(id).unwrap_or("this album").to_string();
            let (mut delete, mut cancel) = (false, false);
            egui::Window::new("Delete album")
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    ui.label(format!("Delete the album “{name}”?"));
                    ui.label(RichText::new("The photos and videos in it are not deleted.").weak());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        delete = ui.button("Delete").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if delete {
                self.delete_album = None;
                self.lib.queue_album(AlbumOp::Delete { id });
                self.snapshot.albums.retain(|a| a.id != id);
                self.snapshot_again = true;
                if self.tl.view == View::Album(id) {
                    self.set_view(View::Timeline);
                }
            } else if cancel {
                self.delete_album = None;
            }
        }
    }

    /// Above an album's grid: its name, and renaming and deleting it.
    pub(super) fn album_header(&mut self, ui: &mut egui::Ui, id: i64) {
        let name = self.album_name(id).unwrap_or("Album").to_string();
        ui.horizontal(|ui| {
            ui.heading(&name);
            if ui.button("Rename…").clicked() {
                self.rename_album = Some(RenameAlbum {
                    id,
                    name: name.clone(),
                    focused: false,
                });
            }
            if ui.button("Delete album…").clicked() {
                self.delete_album = Some(id);
            }
        });
        ui.add_space(4.0);
    }

    /// Per frame: outcomes of album writes, and what they changed.
    pub(super) fn poll_albums(&mut self) {
        for w in self.lib.take_album_results() {
            self.report_album_write(&w);
        }
        let version = self.lib.albums_version();
        if version != self.albums_version_seen {
            self.albums_version_seen = version;
            self.refresh_snapshot(true);
            if matches!(self.tl.view, View::Album(_)) {
                // What the view lists has changed.
                self.tl.requested = None;
            }
        }
    }

    fn report_album_write(&mut self, w: &AlbumWrite) {
        if let Some(e) = &w.error {
            self.notify(e.clone());
            return;
        }
        let album = w
            .album
            .and_then(|id| self.album_name(id))
            .map(str::to_string);
        match &w.op {
            AlbumOp::Create { name, add } if !add.is_empty() => {
                self.notify(format!(
                    "Created “{}” with {}",
                    name.trim(),
                    items_text(w.changed)
                ));
            }
            AlbumOp::Add { files, .. } => {
                let to = album.unwrap_or_else(|| "the album".into());
                let msg = if w.changed == files.len() {
                    format!("Added {} to “{to}”", items_text(w.changed))
                } else if w.changed == 0 {
                    format!("Already in “{to}”")
                } else {
                    format!(
                        "Added {} of {} to “{to}” (the rest were already in it)",
                        thousands(w.changed),
                        items_text(files.len())
                    )
                };
                self.notify(msg);
            }
            AlbumOp::Remove { .. } => {
                let from = album.unwrap_or_else(|| "the album".into());
                self.notify(format!("Removed {} from “{from}”", items_text(w.changed)));
            }
            _ => {}
        }
    }

    /// File ids of the selected items in list order.
    pub(super) fn selected_file_ids(&self) -> Vec<i64> {
        if self.searching() {
            self.results
                .iter()
                .filter(|h| self.result_sel.contains(h.file_id))
                .map(|h| h.file_id)
                .collect()
        } else {
            self.tl
                .items
                .iter()
                .filter(|it| self.tl.sel.contains(it.file_id))
                .map(|it| it.file_id)
                .collect()
        }
    }

    /// The selection bar (and its twin for search results): shown while 2+ items are selected.
    pub(super) fn selection_bar_ui(&mut self, ui: &mut egui::Ui) {
        let searching = self.searching();
        let count = if searching {
            self.result_sel.len()
        } else {
            self.tl.sel.len()
        };
        if count < 2 {
            return;
        }
        let in_album = !searching && matches!(self.tl.view, View::Album(_));
        if let Some(a) = selection_bar(ui, count, &self.snapshot.albums, in_album) {
            self.bulk_action(a);
        }
    }

    fn bulk_action(&mut self, action: BulkAction) {
        let files = self.selected_file_ids();
        match action {
            BulkAction::AddToAlbum(c) => self.add_to_album_choice(files, c),
            BulkAction::Favorite(fav) => self.set_favorites(&files, fav),
            BulkAction::RemoveFromAlbum => {
                if let View::Album(album) = self.tl.view {
                    self.lib.queue_album(AlbumOp::Remove {
                        album,
                        files: files.clone(),
                    });
                    for f in files {
                        self.tl.remove_file(f);
                    }
                    self.tl.selected = None;
                    self.tl.sel.clear();
                }
            }
            BulkAction::Clear => self.clear_selection(),
        }
    }

    /// Drops the selection of whatever grid is shown.
    pub(super) fn clear_selection(&mut self) {
        self.selected = None;
        self.result_sel.clear();
        self.tl.selected = None;
        self.tl.sel.clear();
    }

    /// What the detail panel shows while several items are selected.
    pub(super) fn selection_summary(&mut self, ui: &mut egui::Ui, count: usize) {
        ui.add_space(6.0);
        ui.heading(format!("{} selected", items_text(count)));
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Use the buttons above the grid to add them to an album or mark them as \
                 favorites. Esc clears the selection.",
            )
            .weak(),
        );
    }
}
