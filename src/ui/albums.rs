//! Albums in the UI: the sidebar section, the name and delete dialogs and the
//! "Add to album…" menu.
//!
//! Album changes go to the library's write queue and come back as [`AlbumWrite`]s. The UI
//! changes only once a write is confirmed (nothing is shown optimistically that could need
//! taking back). The sidebar's album list is fetched in the background whenever the library's
//! album version moves or a job finishes.

use super::App;
use super::dialogs::{Confirmation, confirm_dialog, modal_window};
use super::notices::Undo;
use super::sidebar::{Leading, Row};
use super::style::{self, icons};
use super::timeline::{View, thousands};
use super::workers::ThumbCache;
use crate::library::{AlbumOp, AlbumWrite};
use crate::media::MediaKind;
use crate::store::db::{self, AlbumInfo};
use crate::thumbs::ThumbKey;
use egui::RichText;
use std::collections::HashSet;

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
    pub(super) pending: bool,
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

/// "1 item" / "3 items".
fn items_text(n: usize) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{} items", thousands(n))
    }
}

/// The entries of the "Add to album…" menus: the albums, then "New album…".
fn album_menu_items(ui: &mut egui::Ui, albums: &[AlbumInfo], choice: &mut Option<AlbumChoice>) {
    style::menu_body(ui);
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
        let color = ui.visuals().widgets.inactive.fg_stroke.color;
        let job = style::icon_label_job(
            ui,
            icons::ICON_PLAYLIST_ADD,
            22.0,
            color,
            text.map(|t| (t, color)),
            6.0,
        );
        ui.menu_button(job, |ui| {
            album_menu_items(ui, albums, &mut choice);
        })
    });
    let r = r.inner.response;
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add to album…"));
    r.on_hover_text("Add to album…");
    choice
}

/// The key of the thumbnail that stands for an album: its cover when that is a photo.
fn cover_key(album: &AlbumInfo) -> Option<ThumbKey> {
    let c = album.cover.as_ref()?;
    (c.kind == MediaKind::Image).then(|| ThumbKey {
        path: c.path.clone(),
        mtime: c.mtime,
        t_ms: None,
    })
}

/// The sidebar's album rows in a scroll area. Returns the album that was clicked.
fn album_rows(
    ui: &mut egui::Ui,
    thumbs: &mut ThumbCache,
    albums: &[AlbumInfo],
    current: View,
) -> Option<i64> {
    let mut open = None;
    egui::ScrollArea::vertical()
        .id_salt("albums")
        .max_height(180.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for a in albums {
                let short = short_name(&a.name, SHORT_NAME_CHARS);
                let accessible = format!("{short} ({})", thousands(a.count));
                let key = cover_key(a);
                let mut row = Row::new(
                    Leading::Cover(key.as_ref()),
                    &short,
                    current == View::Album(a.id),
                );
                row.accessible = &accessible;
                let (_, r) = super::sidebar::nav_row(ui, Some(thumbs), &row);
                if r.on_hover_text(&a.name).clicked() {
                    open = Some(a.id);
                }
            }
        });
    open
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
        let free = self.dialogs.new_album.is_none();
        let r = super::sidebar::section_header(
            ui,
            "Albums",
            &mut [(icons::ICON_ADD, "New album…", free)],
        );
        if r[0].clicked() {
            self.dialogs.new_album = Some(NewAlbum::default());
        }
        if self.albums.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("No albums yet.")
                        .color(super::style::Palette::of(ui.ctx()).muted),
                );
            });
        }
        if let Some(id) = album_rows(ui, &mut self.thumbs, &self.albums, self.tl.view) {
            self.set_view(View::Album(id));
        }
        self.inline_new_album(ui);
    }

    /// The name field under the album list while an empty new album is being made.
    fn inline_new_album(&mut self, ui: &mut egui::Ui) {
        let inline = self
            .dialogs
            .new_album
            .as_ref()
            .is_some_and(|n| n.add.is_empty());
        if inline {
            let mut na = self.dialogs.new_album.take().unwrap_or_default();
            let enter = name_field(ui, &mut na.name, &mut na.focused, ui.available_width());
            let (mut create, mut cancel) = (enter, false);
            ui.horizontal(|ui| {
                create |= ui.button("Create").clicked();
                cancel |= ui.button("Cancel").clicked();
            });
            self.dialogs.new_album = (!cancel).then_some(na);
            if create {
                self.submit_new_album();
            }
        }
    }

    /// Creates the album being named (keeping the dialog open if the name is no good). The
    /// dialog reopens if the library refuses the name or fails to write.
    fn submit_new_album(&mut self) {
        let Some(mut na) = self.dialogs.new_album.take() else {
            return;
        };
        let name = na.name.trim().to_string();
        if let Err(e) = self.check_album_name(&name, None) {
            let (text, details) = super::guide::album_problem(Some(&e), &e.to_string());
            self.notify_problem(text, details.unwrap_or_default(), None);
            // Enter took focus out of the field: ask for it again so typing carries on.
            na.focused = false;
            self.dialogs.new_album = Some(na);
            return;
        }
        if !na.add.is_empty() {
            self.clear_selection();
        }
        self.lib.queue_album(AlbumOp::Create { name, add: na.add });
    }

    /// Quick check against the albums we know of with the library's own rules (it has the
    /// final say).
    fn check_album_name(&self, name: &str, except: Option<i64>) -> Result<(), db::AlbumError> {
        db::check_album_name(
            name,
            self.albums.iter().map(|a| (a.id, a.name.as_str())),
            except,
        )
        .map(|_| ())
    }

    /// Items go to an existing album, or to a new one once it has a name.
    pub(super) fn add_to_album_choice(&mut self, files: Vec<i64>, choice: AlbumChoice) {
        match choice {
            AlbumChoice::Existing(album) => {
                self.lib.queue_album(AlbumOp::Add { album, files });
            }
            AlbumChoice::New => self.dialogs.new_album = Some(NewAlbum::with_items(files)),
        }
    }

    /// The windows for naming a new album (when it starts with items), renaming and confirming
    /// a deletion. The inline field for an empty new album lives in the sidebar.
    pub(super) fn album_dialogs(&mut self, ctx: &egui::Context) {
        self.new_album_dialog(ctx);
        self.rename_album_dialog(ctx);
        self.delete_album_dialog(ctx);
    }

    fn new_album_dialog(&mut self, ctx: &egui::Context) {
        if (self.dialogs.new_album.as_ref()).is_none_or(|n| n.add.is_empty()) {
            return;
        }
        let mut na = self.dialogs.new_album.take().unwrap_or_default();
        let (mut create, mut cancel) = (false, false);
        modal_window(ctx, "New album", |ui| {
            ui.label(format!("{} will be added to it.", items_text(na.add.len())));
            create |= name_field(ui, &mut na.name, &mut na.focused, 240.0);
            ui.horizontal(|ui| {
                create |= ui.button("Create").clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        self.dialogs.new_album = (!cancel).then_some(na);
        if create {
            self.submit_new_album();
        }
    }

    fn rename_album_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut r) = self.dialogs.rename_album.take() else {
            return;
        };
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
                    let (text, details) = super::guide::album_problem(Some(&e), &e.to_string());
                    self.notify_problem(text, details.unwrap_or_default(), None);
                    r.focused = false;
                    self.dialogs.rename_album = Some(r);
                }
            }
        } else if !cancel {
            self.dialogs.rename_album = Some(r);
        }
    }

    fn delete_album_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = &self.dialogs.delete_album else {
            return;
        };
        let (id, pending) = (d.id, d.pending);
        let name = short_name(self.album_name(id).unwrap_or("this album"), 60);
        let choice = confirm_dialog(ctx, "Delete album", "Delete", "Cancel", !pending, |ui| {
            ui.label(format!("Delete the album “{name}”?"));
            ui.label(RichText::new("The photos and videos in it are not deleted.").weak());
            ui.add_space(6.0);
        });
        match choice {
            Some(Confirmation::Confirmed) => {
                // The view changes once the library has deleted it.
                if let Some(d) = &mut self.dialogs.delete_album {
                    d.pending = true;
                }
                self.lib.queue_album(AlbumOp::Delete { id });
            }
            Some(Confirmation::Cancelled) => self.dialogs.delete_album = None,
            None => {}
        }
    }

    /// Next to an album's title: renaming and deleting it.
    pub(super) fn album_actions(&mut self, ui: &mut egui::Ui, id: i64) {
        let name = self.album_name(id).unwrap_or("Album").to_string();
        if style::icon_label_button(ui, icons::ICON_EDIT, "Rename…").clicked() {
            self.dialogs.rename_album = Some(RenameAlbum {
                id,
                name,
                focused: false,
            });
        }
        if style::icon_label_button(ui, icons::ICON_DELETE, "Delete album…").clicked() {
            self.dialogs.delete_album = Some(DeleteAlbum::new(id));
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
        if let Some(list) = self.albums_read.poll() {
            match list {
                Ok(list) => self.albums = list,
                Err(e) => tracing::warn!("could not read the albums: {e:#}"),
            }
        }
    }

    /// Reads the albums (with counts and covers) on a background thread; asking while a read
    /// is under way schedules one more afterwards.
    pub(super) fn refresh_albums(&mut self) {
        self.albums_read.request();
    }

    fn report_album_write(&mut self, w: &AlbumWrite) {
        let silent = self.silent_album_writes.remove(&w.ticket);
        if let Some(e) = &w.error {
            let (text, details) = super::guide::album_problem(w.error_kind.as_ref(), e);
            self.notify_problem(text, details.unwrap_or_default(), None);
            self.restore_after_failed_write(&w.op);
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
            AlbumOp::Add { album: id, files } if !silent => {
                let to = album.unwrap_or_else(|| "the album".into());
                let text = added_message(w.changed, files.len(), &to);
                if w.changed_files.is_empty() {
                    self.notify(text);
                } else {
                    let undo = Undo::TakeOutOfAlbum {
                        album: *id,
                        files: w.changed_files.clone(),
                    };
                    self.notify_undoable(text, undo);
                }
            }
            AlbumOp::Remove { album: id, files } => {
                if !silent {
                    self.report_removal(*id, w, album);
                }
                self.drop_removed_from_view(*id, files);
            }
            AlbumOp::Delete { id } => self.album_deleted(*id),
            _ => {}
        }
    }

    /// "Removed 3 from X" with Undo, or a plain note when nothing was in the album.
    fn report_removal(&mut self, id: i64, w: &AlbumWrite, album: Option<String>) {
        let from = album.unwrap_or_else(|| "the album".into());
        if w.changed_files.is_empty() {
            self.notify(format!("Nothing to remove from {from}"));
            return;
        }
        let text = format!("Removed {} from {from}", thousands(w.changed));
        let undo = Undo::PutBackInAlbum {
            album: id,
            files: w.changed_files.clone(),
        };
        self.notify_undoable(text, undo);
    }

    /// Nothing was changed: puts back what the user was working on.
    fn restore_after_failed_write(&mut self, op: &AlbumOp) {
        match op {
            AlbumOp::Create { name, add } if self.dialogs.new_album.is_none() => {
                self.dialogs.new_album =
                    Some(NewAlbum::with_name_and_items(name.clone(), add.clone()));
            }
            AlbumOp::Rename { id, name } if self.dialogs.rename_album.is_none() => {
                self.dialogs.rename_album = Some(RenameAlbum {
                    id: *id,
                    name: name.clone(),
                    focused: false,
                });
            }
            AlbumOp::Delete { id } => {
                if let Some(d) = self.dialogs.delete_album.as_mut().filter(|d| d.id == *id) {
                    d.pending = false;
                }
            }
            _ => {}
        }
    }

    /// The grid of album `id` loses `files` once they are out of the album; a multi-item
    /// removal also clears the selection.
    fn drop_removed_from_view(&mut self, id: i64, files: &[i64]) {
        if self.tl.view != View::Album(id) {
            return;
        }
        let gone: HashSet<i64> = files.iter().copied().collect();
        let multi = gone.len() > 1;
        self.tl.remove_files(&gone);
        if multi {
            self.tl.grid.selected = None;
            self.tl.grid.sel.clear();
        }
    }

    /// Album `id` has been deleted: its dialog closes and a view of it gives way to the timeline.
    fn album_deleted(&mut self, id: i64) {
        if self
            .dialogs
            .delete_album
            .as_ref()
            .is_some_and(|d| d.id == id)
        {
            self.dialogs.delete_album = None;
        }
        self.refresh_albums();
        if self.tl.view == View::Album(id) {
            self.set_view(View::Timeline);
        }
    }
}

/// What was said about adding `requested` items to album `to` when `changed` were new to it.
fn added_message(changed: usize, requested: usize, to: &str) -> String {
    if changed == requested {
        format!("Added {} to {to}", thousands(changed))
    } else if changed == 0 {
        format!("Already in {to}")
    } else {
        format!(
            "Added {} of {} to {to} (the rest were already in it)",
            thousands(changed),
            thousands(requested)
        )
    }
}
