//! Which dialogs and sheets are open, and the window frame and confirm dialog they share.

use super::style::{self};
use super::{App, albums, folders, help, settings};

/// The dialogs and sheets that can be open on top of the main window.
#[derive(Default)]
pub(super) struct Dialogs {
    /// A name being typed for a new album, a rename and a pending deletion.
    pub(super) new_album: Option<albums::NewAlbum>,
    pub(super) rename_album: Option<albums::RenameAlbum>,
    pub(super) delete_album: Option<albums::DeleteAlbum>,
    pub(super) add_folder_dialog: Option<folders::AddFolder>,
    pub(super) stop_folder: Option<folders::StopFolder>,
    pub(super) help_open: bool,
    pub(super) help_topic: help::HelpTopic,
    pub(super) settings_open: bool,
    pub(super) settings_section: settings::Section,
    /// A smart-search choice waiting for the user to confirm the switch.
    pub(super) model_confirm: Option<&'static str>,
}

impl Dialogs {
    /// Whether a question or the help sheet is open (the Settings sheet is not counted: it
    /// closes after them).
    fn any_open(&self) -> bool {
        self.new_album.is_some()
            || self.rename_album.is_some()
            || self.delete_album.is_some()
            || self.add_folder_dialog.is_some()
            || self.stop_folder.is_some()
            || self.help_open
            || self.model_confirm.is_some()
    }

    /// Esc: closes the questions and the help sheet (a deletion already sent to the library
    /// stays up until it is answered).
    pub(super) fn cancel(&mut self) {
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
}

impl App {
    pub(super) fn dialog_open(&self) -> bool {
        self.dialogs.any_open()
    }

    /// True when keys must not reach the views: a text field has focus, Settings is open, an
    /// album dialog is, or a menu is open (keys then belong to the menu).
    pub(super) fn keys_blocked(&self, ctx: &egui::Context) -> bool {
        self.dialogs.settings_open
            || self.dialog_open()
            || ctx.text_edit_focused()
            || self.search_help.inside
            || egui::Popup::is_any_open(ctx)
    }
}

/// What was chosen in a [`confirm_dialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Confirmation {
    Confirmed,
    Cancelled,
}

/// A [`modal_window`] with a body and two buttons, `confirm` then `cancel`, which are disabled
/// unless `enabled`. Returns what was clicked, if anything.
pub(super) fn confirm_dialog(
    ctx: &egui::Context,
    title: &str,
    confirm: &str,
    cancel: &str,
    enabled: bool,
    add_body: impl FnOnce(&mut egui::Ui),
) -> Option<Confirmation> {
    let (mut confirmed, mut cancelled) = (false, false);
    modal_window(ctx, title, |ui| {
        add_body(ui);
        ui.horizontal(|ui| {
            confirmed = ui
                .add_enabled(enabled, egui::Button::new(confirm))
                .clicked();
            cancelled = ui.add_enabled(enabled, egui::Button::new(cancel)).clicked();
        });
    });
    if confirmed {
        Some(Confirmation::Confirmed)
    } else if cancelled {
        Some(Confirmation::Cancelled)
    } else {
        None
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
