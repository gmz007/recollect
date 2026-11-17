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
            || super::ruler::stop_focused(ctx)
    }
}

/// What was chosen in a [`confirm_dialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Confirmation {
    Confirmed,
    Cancelled,
}

/// How the confirming button of a [`confirm_dialog`] looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Normal,
    /// The action cannot be undone: the button is in the warning colour.
    Danger,
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
    confirm_dialog_tone(
        ctx,
        title,
        (confirm, Tone::Normal),
        cancel,
        enabled,
        add_body,
    )
}

/// Like [`confirm_dialog`], with the look of the confirming button chosen.
pub(super) fn confirm_dialog_tone(
    ctx: &egui::Context,
    title: &str,
    (confirm, tone): (&str, Tone),
    cancel: &str,
    enabled: bool,
    add_body: impl FnOnce(&mut egui::Ui),
) -> Option<Confirmation> {
    let (mut confirmed, mut cancelled) = (false, false);
    modal_window(ctx, title, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        add_body(ui);
        dialog_buttons(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                confirmed = match tone {
                    Tone::Normal => style::primary_button(ui, confirm),
                    Tone::Danger => style::danger_button(ui, confirm),
                }
                .clicked();
                cancelled = secondary_button(ui, cancel).clicked();
            });
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

/// A button on the field colour, 32 points high.
pub(super) fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(style::medium(label)).min_size(egui::Vec2::new(0.0, 32.0)));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// The row of buttons at the bottom of a dialog, at the right. Add the main button first, then
/// the one that backs out (the row runs right to left).
pub(super) fn dialog_buttons(ui: &mut egui::Ui, add_buttons: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add_buttons(ui);
    });
}

/// Corner radius of dialogs.
const DIALOG_RADIUS: u8 = 12;
/// Width of the text of a dialog.
const DIALOG_WIDTH: f32 = 340.0;

/// A centred dialog over a dimmed backdrop, used by the album and folder dialogs.
pub(super) fn modal_window(
    ctx: &egui::Context,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let pal = style::Palette::of(ctx);
    let frame = egui::Frame::new()
        .fill(pal.pop)
        .stroke(egui::Stroke::new(1.0, pal.line))
        .corner_radius(egui::CornerRadius::same(DIALOG_RADIUS))
        .inner_margin(egui::Margin::same(20))
        .shadow(ctx.global_style().visuals.popup_shadow);
    egui::Modal::new(egui::Id::unique(("dialog", title)))
        .frame(frame)
        .show(ctx, |ui| {
            ui.label(style::semibold(title).size(17.0));
            ui.add_space(8.0);
            add_contents(ui);
        });
}
