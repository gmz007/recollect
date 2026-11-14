//! Which dialogs and sheets are open.

use super::{albums, help, settings, sidebar};

/// The dialogs and sheets that can be open on top of the main window.
#[derive(Default)]
pub(super) struct Dialogs {
    /// A name being typed for a new album, a rename and a pending deletion.
    pub(super) new_album: Option<albums::NewAlbum>,
    pub(super) rename_album: Option<albums::RenameAlbum>,
    pub(super) delete_album: Option<albums::DeleteAlbum>,
    pub(super) add_folder_dialog: Option<sidebar::AddFolder>,
    pub(super) stop_folder: Option<sidebar::StopFolder>,
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
    pub(super) fn any_open(&self) -> bool {
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
