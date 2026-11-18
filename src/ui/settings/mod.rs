//! Settings: its own window (or, where windows can't be opened, a centered sheet over the main
//! one) with a list of sections on the left and the chosen section on the right. Changes apply
//! as soon as they are made; only changing the search style asks first, because it makes
//! Recollect look at every photo again.
//!
//! Each section has its own file; `rows` and `widgets` hold the controls they share.

mod about;
mod appearance;
mod features;
mod library;
mod rows;
mod smart_search;
mod storage;
mod widgets;

pub(super) use smart_search::{quality_blurb, quality_name};
pub(super) use widgets::choice_card;

use super::App;
use super::sidebar::plain_row;
use super::style::{self, Palette, icons};
use crate::config::Settings;
use egui::{Color32, CornerRadius, Margin, Stroke, Vec2};
use egui_material_icons::MaterialIcon;

/// The sections of Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Section {
    #[default]
    Appearance,
    Features,
    SmartSearch,
    Library,
    Storage,
    About,
}

/// Width of the section list.
const NAV_WIDTH: f32 = 200.0;
/// Corner radius of the sheet.
const SHEET_RADIUS: u8 = 12;
/// The window's size when it opens, and the smallest it can be made.
const WINDOW_SIZE: [f32; 2] = [820.0, 600.0];
const WINDOW_MIN_SIZE: [f32; 2] = [640.0, 480.0];

impl Section {
    const ALL: [Section; 6] = [
        Section::Appearance,
        Section::Features,
        Section::SmartSearch,
        Section::Library,
        Section::Storage,
        Section::About,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Features => "Features",
            Section::SmartSearch => "Smart search",
            Section::Library => "Library",
            Section::Storage => "Storage",
            Section::About => "About",
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            Section::Appearance => icons::ICON_PALETTE,
            Section::Features => icons::ICON_TUNE,
            Section::SmartSearch => icons::ICON_SEARCH,
            Section::Library => icons::ICON_FOLDER,
            Section::Storage => icons::ICON_STORAGE,
            Section::About => icons::ICON_INFO,
        }
    }
}

/// The fill of the Settings content: a slightly raised grey in the dark theme.
pub(super) fn surface(pal: &Palette) -> Color32 {
    if pal.dark {
        style::SETTINGS_WINDOW_DARK
    } else {
        pal.bg
    }
}

/// A line of quiet small text under a control.
fn hint(ui: &mut egui::Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.label(egui::RichText::new(text).size(12.0).color(pal.muted));
}

fn settings_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("recollect-settings")
}

/// The section list on the left. `radius` rounds its outer corners (the sheet's).
fn settings_nav(ui: &mut egui::Ui, pal: &Palette, current: &mut Section, h: f32, radius: u8) {
    egui::Frame::new()
        .fill(pal.side)
        .corner_radius(CornerRadius {
            nw: radius,
            sw: radius,
            ne: 0,
            se: 0,
        })
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(NAV_WIDTH - 24.0);
                ui.set_height(h - 24.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(4.0);
                ui.label(style::semibold("Settings").size(15.0));
                ui.add_space(10.0);
                for s in Section::ALL {
                    let (_, r) = plain_row(
                        ui,
                        s.icon(),
                        s.name(),
                        &format!("{} settings", s.name()),
                        None,
                        *current == s,
                    );
                    if r.clicked() {
                        *current = s;
                    }
                }
            });
        });
}

impl App {
    /// Opens Settings on the section the user saw last, or brings the open window forward.
    pub(super) fn open_settings(&mut self) {
        self.dialogs.settings_focus = self.dialogs.settings_open;
        self.dialogs.settings_open = true;
    }

    /// Opens Settings on `section`.
    pub(super) fn open_settings_at(&mut self, section: Section) {
        self.dialogs.settings_section = section;
        self.open_settings();
    }

    /// Changes the settings `f` touches (and only those) and saves them at once.
    pub(super) fn change_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        match self.lib.update_settings(f) {
            Ok(()) => self.settings = self.lib.settings(),
            Err(e) => self.notify_problem(
                "Couldn't save your settings. Check that your disk isn't full or write-protected, \
                 then try again.",
                format!("{e:#}"),
                None,
            ),
        }
    }

    /// Remembers that the user did (or no longer does) not want the smart-search invitation
    /// above the photos.
    pub(super) fn set_search_prompt_dismissed(&mut self, dismissed: bool) {
        self.change_settings(|s| s.search_prompt_dismissed = dismissed);
    }

    /// The title, (in the sheet) a close button, and the settings of the chosen section; true
    /// when the button was clicked.
    fn settings_content(&mut self, ui: &mut egui::Ui, size: Vec2, with_close: bool) -> bool {
        let mut close = false;
        let section = self.dialogs.settings_section;
        egui::Frame::new()
            .inner_margin(Margin::symmetric(28, 20))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(size.x - 56.0);
                    ui.set_height(size.y - 40.0);
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                    ui.horizontal(|ui| {
                        ui.label(style::semibold(section.name()).size(18.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if with_close
                                && style::icon_button_framed(
                                    ui,
                                    icons::ICON_CLOSE,
                                    "Close settings",
                                )
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    // A scrollbar that takes its own room: the content never runs under it.
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::vertical()
                        .id_salt(("settings-scroll", section as u8))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width() - 8.0);
                            self.settings_section(ui, section);
                            ui.add_space(24.0);
                        });
                });
            });
        close
    }

    fn settings_section(&mut self, ui: &mut egui::Ui, section: Section) {
        match section {
            Section::Appearance => self.settings_appearance(ui),
            Section::Features => self.settings_features(ui),
            Section::SmartSearch => self.settings_smart_search(ui),
            Section::Library => self.settings_library(ui),
            Section::Storage => self.settings_storage(ui),
            Section::About => Self::settings_about(ui),
        }
    }

    /// The section list and the chosen section side by side, filling `ui`.
    fn settings_body(&mut self, ui: &mut egui::Ui, sheet: bool) -> bool {
        let pal = Palette::of(ui.ctx());
        let size = ui.available_size();
        let mut close = false;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.horizontal_top(|ui| {
            let radius = if sheet { SHEET_RADIUS } else { 0 };
            settings_nav(ui, &pal, &mut self.dialogs.settings_section, size.y, radius);
            close = self.settings_content(ui, Vec2::new(size.x - NAV_WIDTH, size.y), sheet);
        });
        close
    }

    /// Settings: its own window where the system allows it, otherwise a sheet over the main
    /// window with the same content.
    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if ctx.embed_viewports() {
            self.settings_sheet(ctx);
            return;
        }
        let id = settings_viewport_id();
        if std::mem::take(&mut self.dialogs.settings_focus) {
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
        }
        let builder = egui::ViewportBuilder::default()
            .with_title("Settings")
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(WINDOW_MIN_SIZE)
            .with_resizable(true);
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            close = self.settings_viewport_ui(ui);
        });
        if close {
            self.dialogs.settings_open = false;
        }
    }

    /// The content of the Settings window; true when it asks to be closed (its close button
    /// or Esc).
    fn settings_viewport_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let ctx = ui.ctx().clone();
        let pal = Palette::of(&ctx);
        let mut close = ctx.input(|i| i.viewport().close_requested());
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            // A pending question is answered "no" first.
            if self.dialogs.model_confirm.take().is_none() {
                close = true;
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(surface(&pal)))
            .show(ui, |ui| {
                self.settings_body(ui, false);
            });
        close
    }

    /// The sheet: Settings drawn over the main window. Closed by its × button, by Esc and by a
    /// click outside it.
    fn settings_sheet(&mut self, ctx: &egui::Context) {
        let pal = Palette::of(ctx);
        let screen = ctx.content_rect().size();
        let w = (screen.x - 48.0).clamp(380.0, WINDOW_SIZE[0]);
        let h = (screen.y - 48.0).clamp(320.0, WINDOW_SIZE[1]);
        let frame = egui::Frame::new()
            .fill(surface(&pal))
            .stroke(Stroke::new(1.0, pal.line))
            .corner_radius(CornerRadius::same(SHEET_RADIUS))
            .shadow(ctx.global_style().visuals.popup_shadow)
            .inner_margin(Margin::ZERO);
        let backdrop = if pal.dark {
            style::SETTINGS_BACKDROP_DARK.gamma_multiply(0.8)
        } else {
            Color32::from_black_alpha(90)
        };
        let mut close = false;
        let out = egui::Modal::new(egui::Id::NULL.with("settings-sheet"))
            .frame(frame)
            .backdrop_color(backdrop)
            .show(ctx, |ui| {
                ui.set_width(w);
                ui.set_height(h);
                close = self.settings_body(ui, true);
            });
        let nothing_on_top = !self.dialog_open();
        if close || (out.backdrop_response.clicked() && nothing_on_top) {
            self.dialogs.settings_open = false;
        }
    }
}

#[cfg(test)]
mod tests;
