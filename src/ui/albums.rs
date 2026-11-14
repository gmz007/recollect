//! Albums in the UI: the sidebar section, the name and confirmation dialogs, the
//! "Add to album…" menu and the bar shown above a grid while items are selected.
//!
//! Album changes go to the library's write queue and come back as [`AlbumWrite`]s. The UI
//! changes only once a write is confirmed (nothing is shown optimistically that could need
//! taking back). The sidebar's album list is fetched in the background whenever the library's
//! album version moves or a job finishes.

use super::App;
use super::style::{self, Palette, icons};
use super::timeline::{View, thousands};
use crate::library::{AlbumOp, AlbumWrite};
use crate::store::db::{self, AlbumInfo};
use egui::RichText;
use std::collections::HashSet;
use std::sync::Arc;

/// Longest album name shown whole in the sidebar and menus; longer ones end in an ellipsis
/// (the full name is in the hover text).
const SHORT_NAME_CHARS: usize = 28;

/// `name` cut to at most `max` characters, ending in an ellipsis when it was longer.
pub(super) fn short_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let mut out: String = name.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

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

    fn with_name_and_items(name: String, add: Vec<i64>) -> Self {
        Self {
            name,
            add,
            ..Default::default()
        }
    }
}

/// A deletion waiting for the user's confirmation, then for the writer's.
#[derive(Debug)]
pub(super) struct DeleteAlbum {
    pub id: i64,
    /// Queued: the buttons are disabled until the outcome arrives.
    pending: bool,
}

impl DeleteAlbum {
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn new(id: i64) -> Self {
        Self { id, pending: false }
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

/// The entries of the "Add to album…" menus: the albums, then "New album…".
fn album_menu_items(ui: &mut egui::Ui, albums: &[AlbumInfo], choice: &mut Option<AlbumChoice>) {
    egui::ScrollArea::vertical()
        .max_height(280.0)
        .show(ui, |ui| {
            for a in albums {
                let b = ui.button(short_name(&a.name, SHORT_NAME_CHARS));
                if b.on_hover_text(&a.name).clicked() {
                    *choice = Some(AlbumChoice::Existing(a.id));
                    ui.close();
                }
            }
        });
    if !albums.is_empty() {
        ui.separator();
    }
    if ui.button("New album…").clicked() {
        *choice = Some(AlbumChoice::New);
        ui.close();
    }
}

/// The "Add to album…" menu button: the albums, then "New album…".
pub(super) fn album_menu(ui: &mut egui::Ui, albums: &[AlbumInfo]) -> Option<AlbumChoice> {
    let mut choice = None;
    style::icon_menu_button(ui, icons::ICON_PLAYLIST_ADD, "Add to album…", |ui| {
        album_menu_items(ui, albums, &mut choice);
    });
    choice
}

/// The same menu behind a toolbar button: an icon, with `text` next to it when there is room.
pub(super) fn album_menu_icon(
    ui: &mut egui::Ui,
    albums: &[AlbumInfo],
    text: Option<&str>,
) -> Option<AlbumChoice> {
    let mut choice = None;
    let r = ui.scope(|ui| {
        // Like the other toolbar icons: no box until hovered.
        let w = &mut ui.visuals_mut().widgets.inactive;
        w.weak_bg_fill = egui::Color32::TRANSPARENT;
        w.bg_stroke = egui::Stroke::NONE;
        let mut job = egui::text::LayoutJob::default();
        let color = ui.visuals().widgets.inactive.fg_stroke.color;
        job.append(
            icons::ICON_PLAYLIST_ADD.codepoint,
            0.0,
            egui::TextFormat {
                font_id: style::icon_font_id(icons::ICON_PLAYLIST_ADD, 22.0),
                color,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
        if let Some(text) = text {
            job.append(
                text,
                6.0,
                egui::TextFormat {
                    font_id: egui::TextStyle::Button.resolve(ui.style()),
                    color,
                    valign: egui::Align::Center,
                    ..Default::default()
                },
            );
        }
        ui.menu_button(job, |ui| {
            album_menu_items(ui, albums, &mut choice);
        })
    });
    let r = r.inner.response;
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add to album…"));
    r.on_hover_text("Add to album…");
    choice
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
            // Buttons on the accent colour: light text on a faint veil.
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
        self.albums
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.name.as_str())
    }

    /// Sidebar: the albums, and the "+" that makes a new one (an inline name field while one is
    /// being made).
    pub(super) fn albums_section(&mut self, ui: &mut egui::Ui) {
        // While a window with items waiting for a name is open, a second one would drop them.
        let free = self.new_album.is_none();
        let r = super::sidebar::section_header(
            ui,
            "Albums",
            &mut [(icons::ICON_ADD, "New album…", free)],
        );
        if r[0].clicked() {
            self.new_album = Some(NewAlbum::default());
        }
        let mut open = None;
        if self.albums.is_empty() {
            ui.label(RichText::new("No albums yet.").weak());
        }
        egui::ScrollArea::vertical()
            .id_salt("albums")
            .max_height(180.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for a in &self.albums {
                    let short = short_name(&a.name, SHORT_NAME_CHARS);
                    let count = thousands(a.count);
                    let (_, r) = super::sidebar::nav_row(
                        ui,
                        icons::ICON_PHOTO_ALBUM,
                        &short,
                        &format!("{short} ({count})"),
                        Some(&count),
                        self.tl.view == View::Album(a.id),
                        0.0,
                    );
                    if r.on_hover_text(&a.name).clicked() {
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
            let enter = name_field(ui, &mut na.name, &mut na.focused, ui.available_width());
            let (mut create, mut cancel) = (enter, false);
            ui.horizontal(|ui| {
                create |= ui.button("Create").clicked();
                cancel |= ui.button("Cancel").clicked();
            });
            self.new_album = (!cancel).then_some(na);
            if create {
                self.submit_new_album();
            }
        }
    }

    /// Creates the album being named (keeping the dialog open if the name is no good). The
    /// dialog reopens if the library refuses the name or fails to write.
    fn submit_new_album(&mut self) {
        let Some(na) = self.new_album.take() else {
            return;
        };
        let name = na.name.trim().to_string();
        if let Err(e) = self.check_album_name(&name, None) {
            self.notify_error(e);
            self.new_album = Some(na);
            return;
        }
        self.lib.queue_album(AlbumOp::Create { name, add: na.add });
    }

    /// Quick check against the albums we know of with the library's own rules (it has the
    /// final say).
    fn check_album_name(&self, name: &str, except: Option<i64>) -> Result<(), String> {
        db::check_album_name(
            name,
            self.albums.iter().map(|a| (a.id, a.name.as_str())),
            except,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
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
        self.new_album.is_some()
            || self.rename_album.is_some()
            || self.delete_album.is_some()
            || self.add_folder_dialog.is_some()
            || self.stop_folder.is_some()
            || self.help_open
            || self.model_confirm.is_some()
    }

    /// True when keys must not reach the views: a text field has focus, Settings is open or an
    /// album dialog is.
    pub(super) fn keys_blocked(&self, ctx: &egui::Context) -> bool {
        self.settings_open || self.dialog_open() || ctx.text_edit_focused()
    }

    /// Esc: closes the album dialogs (a deletion already sent to the library stays up until
    /// it is answered).
    pub(super) fn cancel_album_dialogs(&mut self) {
        self.new_album = None;
        self.rename_album = None;
        self.add_folder_dialog = None;
        self.stop_folder = None;
        self.help_open = false;
        self.model_confirm = None;
        if self.delete_album.as_ref().is_some_and(|d| !d.pending) {
            self.delete_album = None;
        }
    }

    /// The windows for naming a new album (when it starts with items), renaming and confirming
    /// a deletion. The inline field for an empty new album lives in the sidebar.
    pub(super) fn album_dialogs(&mut self, ctx: &egui::Context) {
        if self.new_album.as_ref().is_some_and(|n| !n.add.is_empty()) {
            let mut na = self.new_album.take().unwrap_or_default();
            let (mut create, mut cancel) = (false, false);
            modal_window(ctx, "New album", |ui| {
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
            modal_window(ctx, "Rename album", |ui| {
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
                        self.notify_error(e);
                        self.rename_album = Some(r);
                    }
                }
            } else if !cancel {
                self.rename_album = Some(r);
            }
        }
        if let Some(d) = &self.delete_album {
            let (id, pending) = (d.id, d.pending);
            let name = short_name(self.album_name(id).unwrap_or("this album"), 60);
            let (mut delete, mut cancel) = (false, false);
            modal_window(ctx, "Delete album", |ui| {
                ui.label(format!("Delete the album “{name}”?"));
                ui.label(RichText::new("The photos and videos in it are not deleted.").weak());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    delete = ui
                        .add_enabled(!pending, egui::Button::new("Delete"))
                        .clicked();
                    cancel = ui
                        .add_enabled(!pending, egui::Button::new("Cancel"))
                        .clicked();
                });
            });
            if delete {
                // The view changes once the library has deleted it.
                if let Some(d) = &mut self.delete_album {
                    d.pending = true;
                }
                self.lib.queue_album(AlbumOp::Delete { id });
            } else if cancel {
                self.delete_album = None;
            }
        }
    }

    /// Next to an album's title: renaming and deleting it.
    pub(super) fn album_actions(&mut self, ui: &mut egui::Ui, id: i64) {
        let name = self.album_name(id).unwrap_or("Album").to_string();
        if style::icon_label_button(ui, icons::ICON_EDIT, "Rename…").clicked() {
            self.rename_album = Some(RenameAlbum {
                id,
                name,
                focused: false,
            });
        }
        if style::icon_label_button(ui, icons::ICON_DELETE, "Delete album…").clicked() {
            self.delete_album = Some(DeleteAlbum::new(id));
        }
    }

    /// Per frame: outcomes of album writes, and what they changed.
    pub(super) fn poll_albums(&mut self) {
        for w in self.lib.take_album_results() {
            self.report_album_write(&w);
        }
        let version = self.lib.albums_version();
        if version != self.albums_version_seen {
            self.albums_version_seen = version;
            self.refresh_albums();
            if matches!(self.tl.view, View::Album(_)) {
                // What the view lists has changed.
                self.tl.requested = None;
            }
        }
        if let Some(rx) = &self.albums_rx {
            match rx.try_recv() {
                Ok(list) => {
                    self.albums_rx = None;
                    match list {
                        Ok(list) => self.albums = list,
                        Err(e) => tracing::warn!("could not read the albums: {e:#}"),
                    }
                    if std::mem::take(&mut self.albums_again) {
                        self.refresh_albums();
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.albums_rx = None,
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
    }

    /// Reads the albums (with counts and covers) on a background thread; asking while a read
    /// is under way schedules one more afterwards.
    pub(super) fn refresh_albums(&mut self) {
        if self.albums_rx.is_some() {
            self.albums_again = true;
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let lib = Arc::clone(&self.lib);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(lib.albums());
            ctx.request_repaint();
        });
        self.albums_rx = Some(rx);
    }

    fn report_album_write(&mut self, w: &AlbumWrite) {
        if let Some(e) = &w.error {
            self.notify_error(e.clone());
            // Nothing was changed: put back what the user was working on.
            match &w.op {
                AlbumOp::Create { name, add } if self.new_album.is_none() => {
                    self.new_album = Some(NewAlbum::with_name_and_items(name.clone(), add.clone()));
                }
                AlbumOp::Rename { id, name } if self.rename_album.is_none() => {
                    self.rename_album = Some(RenameAlbum {
                        id: *id,
                        name: name.clone(),
                        focused: false,
                    });
                }
                AlbumOp::Delete { id } => {
                    if let Some(d) = self.delete_album.as_mut().filter(|d| d.id == *id) {
                        d.pending = false;
                    }
                }
                _ => {}
            }
            return;
        }
        let album = w
            .album
            .and_then(|id| self.album_name(id))
            .map(|n| short_name(n, 40));
        match &w.op {
            AlbumOp::Create { name, add } if !add.is_empty() => {
                self.notify(format!(
                    "Created “{}” with {}",
                    short_name(name.trim(), 40),
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
            AlbumOp::Remove { album: id, files } => {
                let from = album.unwrap_or_else(|| "the album".into());
                self.notify(format!("Removed {} from “{from}”", items_text(w.changed)));
                if self.tl.view == View::Album(*id) {
                    let gone: HashSet<i64> = files.iter().copied().collect();
                    let multi = gone.len() > 1;
                    self.tl.remove_files(&gone);
                    if multi {
                        self.tl.selected = None;
                        self.tl.sel.clear();
                    }
                }
            }
            AlbumOp::Delete { id } => {
                if self.delete_album.as_ref().is_some_and(|d| d.id == *id) {
                    self.delete_album = None;
                }
                self.refresh_albums();
                if self.tl.view == View::Album(*id) {
                    self.set_view(View::Timeline);
                }
            }
            _ => {}
        }
    }

    /// File ids of the selected items in list order, each file once (a search can list a file
    /// more than once).
    pub(super) fn selected_file_ids(&self) -> Vec<i64> {
        let mut seen = HashSet::new();
        if self.searching() {
            self.results
                .iter()
                .map(|h| h.file_id)
                .filter(|id| self.result_sel.contains(*id) && seen.insert(*id))
                .collect()
        } else {
            self.tl
                .items
                .iter()
                .map(|it| it.file_id)
                .filter(|id| self.tl.sel.contains(*id) && seen.insert(*id))
                .collect()
        }
    }

    /// The selection bar, in place of the view header while items are selected. Returns whether
    /// it was shown.
    pub(super) fn selection_bar_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let searching = self.searching();
        let count = if searching {
            self.result_sel.len()
        } else {
            self.tl.sel.len()
        };
        let picking = if searching {
            self.result_picking
        } else {
            self.picking
        };
        if count == 0 || !picking {
            return false;
        }
        let in_album = !searching && matches!(self.tl.view, View::Album(_));
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

    /// A right-click on tile `i` of the shown grid: a tile outside the selection becomes the
    /// selection; one inside it leaves the selection alone (the menu then acts on all of it).
    pub(super) fn select_for_menu(&mut self, i: usize) {
        if self.searching() {
            let Some(id) = self.results.get(i).map(|h| h.file_id) else {
                return;
            };
            if !self.result_sel.contains(id) {
                self.result_sel.only(id);
                self.selected = Some(i);
            }
        } else {
            let Some(id) = self.tl.items.get(i).map(|it| it.file_id) else {
                return;
            };
            if !self.tl.sel.contains(id) {
                self.tl.sel.only(id);
                self.tl.selected = Some(i);
            }
        }
    }

    /// Applies a choice from a tile's right-click menu to the selection it was opened on.
    pub(super) fn apply_tile_menu(&mut self, i: usize, menu: TileMenu) {
        self.select_for_menu(i);
        self.bulk_action(match menu {
            TileMenu::Add(c) => BulkAction::AddToAlbum(c),
            TileMenu::Remove => BulkAction::RemoveFromAlbum,
        });
    }

    /// Drops the selection of whatever grid is shown.
    pub(super) fn clear_selection(&mut self) {
        self.picking = false;
        self.result_picking = false;
        self.selected = None;
        self.result_sel.clear();
        self.tl.selected = None;
        self.tl.sel.clear();
    }
}

/// A centred dialog over a dimmed backdrop, used by the album and folder dialogs.
pub(super) fn modal_window(
    ctx: &egui::Context,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Modal::new(egui::Id::unique(("dialog", title))).show(ctx, |ui| {
        ui.label(style::semibold(title).size(17.0));
        ui.add_space(8.0);
        add_contents(ui);
    });
}
