//! The Settings sheet: a centered panel with a list of sections on the left and the chosen
//! section on the right. Changes apply as soon as they are made; only switching the
//! smart-search choice asks first, because it makes Recollect look at every photo again.
//!
//! Each section has its own file (`smart_search`, `library`, `storage`); `widgets` holds the
//! controls they share.

mod library;
mod smart_search;
mod storage;
mod widgets;

pub(super) use smart_search::{quality_blurb, quality_name};
pub(super) use widgets::choice_card;

use super::App;
use super::photo_size::PhotoSizeLevel;
use super::sidebar::plain_row;
use super::style::{self, Palette, icons};
use crate::config::{Settings, TextSize, Theme};
use egui::{CornerRadius, Margin, RichText, Stroke, Vec2};
use egui_material_icons::MaterialIcon;
use widgets::segmented;

/// The sections of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Section {
    #[default]
    Appearance,
    SmartSearch,
    Library,
    Storage,
    About,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::Appearance,
        Section::SmartSearch,
        Section::Library,
        Section::Storage,
        Section::About,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::SmartSearch => "Smart search",
            Section::Library => "Library",
            Section::Storage => "Storage",
            Section::About => "About",
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            Section::Appearance => icons::ICON_PALETTE,
            Section::SmartSearch => icons::ICON_AUTO_AWESOME,
            Section::Library => icons::ICON_PHOTO_LIBRARY,
            Section::Storage => icons::ICON_STORAGE,
            Section::About => icons::ICON_INFO,
        }
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(style::semibold(text).size(15.0));
    ui.add_space(2.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.label(RichText::new(text).small().color(pal.text2));
}

/// The section list on the left of the sheet.
fn settings_nav(ui: &mut egui::Ui, pal: &Palette, current: &mut Section, nav_w: f32, h: f32) {
    egui::Frame::new()
        .fill(pal.field)
        .corner_radius(CornerRadius {
            nw: 16,
            sw: 16,
            ne: 0,
            se: 0,
        })
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(nav_w - 24.0);
                ui.set_height(h - 24.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(4.0);
                ui.label(style::semibold("Settings").size(18.0));
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
    /// Opens Settings on the section the user saw last.
    pub(super) fn open_settings(&mut self) {
        self.dialogs.settings_open = true;
    }

    /// Opens Settings on `section`.
    pub(super) fn open_settings_at(&mut self, section: Section) {
        self.dialogs.settings_section = section;
        self.dialogs.settings_open = true;
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

    /// The title, close button and settings of the chosen section; true when the button was
    /// clicked.
    fn settings_content(&mut self, ui: &mut egui::Ui, width: f32, height: f32) -> bool {
        let mut close = false;
        let section = self.dialogs.settings_section;
        egui::Frame::new()
            .inner_margin(Margin::symmetric(20, 14))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.set_height(height);
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(section.name()).text_style(egui::TextStyle::Heading),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if style::icon_button_framed(ui, icons::ICON_CLOSE, "Close settings")
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    // A scrollbar that takes its own room: the content never runs under it.
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::vertical()
                        .id_salt(("settings-scroll", section as u8))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(2.0);
                            // Breathing room between the content and the bar.
                            ui.set_max_width(ui.available_width() - 8.0);
                            match section {
                                Section::Appearance => self.settings_appearance(ui),
                                Section::SmartSearch => self.settings_smart_search(ui),
                                Section::Library => self.settings_library(ui),
                                Section::Storage => self.settings_storage(ui),
                                Section::About => Self::settings_about(ui),
                            }
                            ui.add_space(24.0);
                        });
                });
            });
        close
    }

    /// The sheet itself. Closed by its × button, by Esc and by a click outside it.
    pub(super) fn settings_sheet(&mut self, ctx: &egui::Context) {
        let pal = Palette::of(ctx);
        let screen = ctx.content_rect().size();
        let w = (screen.x - 48.0).clamp(380.0, 640.0);
        let h = (screen.y - 48.0).clamp(320.0, 560.0);
        let nav_w = if w < 520.0 { 150.0 } else { 188.0 };
        let frame = egui::Frame::new()
            .fill(pal.side)
            .stroke(Stroke::new(1.0, pal.line))
            .corner_radius(CornerRadius::same(16))
            .shadow(ctx.global_style().visuals.popup_shadow)
            .inner_margin(Margin::ZERO);
        let mut close = false;
        let out = egui::Modal::new(egui::Id::unique("settings-sheet"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(w);
                ui.set_height(h);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.horizontal_top(|ui| {
                    settings_nav(ui, &pal, &mut self.dialogs.settings_section, nav_w, h);
                    close = self.settings_content(ui, w - nav_w - 40.0, h - 28.0);
                });
            });
        let nothing_on_top = !self.dialog_open();
        if close || (out.backdrop_response.clicked() && nothing_on_top) {
            self.dialogs.settings_open = false;
        }
    }

    fn settings_appearance(&mut self, ui: &mut egui::Ui) {
        let s = self.settings.clone();
        heading(ui, "Colors");
        if let Some(t) = segmented(
            ui,
            &[
                (Theme::System, "Match my computer"),
                (Theme::Light, "Light"),
                (Theme::Dark, "Dark"),
            ],
            s.theme,
        ) {
            self.change_settings(|s| s.theme = t);
        }
        hint(
            ui,
            "“Match my computer” follows your system’s light or dark setting.",
        );
        ui.add_space(14.0);
        heading(ui, "Text size");
        if let Some(t) = segmented(
            ui,
            &[
                (TextSize::Small, "Small"),
                (TextSize::Medium, "Medium"),
                (TextSize::Large, "Large"),
            ],
            s.text_size,
        ) {
            self.change_settings(|s| s.text_size = t);
        }
        hint(ui, "Everything scales with the text size.");
        ui.add_space(14.0);
        self.settings_photo_size(ui);
        ui.add_space(14.0);
        heading(ui, "Tips");
        ui.label("Short hints appear now and then to show you what Recollect can do.");
        ui.add_space(4.0);
        if ui.button("Show tips again").clicked() {
            self.change_settings(|s| s.tips_dismissed.clear());
        }
    }

    /// The size of the photos in the grids: Small, Medium or Large (none is marked while the
    /// size was set to something in between by pinching or scrolling).
    fn settings_photo_size(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Photo size");
        let items = PhotoSizeLevel::ALL.map(|l| (Some(l), l.label()));
        if let Some(Some(level)) = segmented(ui, &items, PhotoSizeLevel::of(self.tl.tile_size)) {
            self.choose_photo_size(level);
        }
        hint(
            ui,
            "You can also pinch, or hold Ctrl (Cmd on a Mac) and scroll, to make photos bigger \
             or smaller.",
        );
    }

    fn settings_about(ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(style::icon_text(icons::ICON_PHOTO_LIBRARY, 34.0).color(pal.accent));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(style::semibold("Recollect").size(18.0));
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(pal.text2),
                );
            });
        });
        heading(ui, "Your photos stay private");
        ui.label(
            "Everything Recollect does happens on this computer. Your photos, videos and \
             searches never leave it, and nothing is sent to anyone. The only time Recollect \
             uses the internet is the one-time download for smart search.",
        );
        heading(ui, "Free and open source");
        ui.label(
            "Recollect is free software. You can read how it works, and how to build it, in \
             the project's documentation.",
        );
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Open-source licenses")
            .id_salt("settings-licenses")
            .show(ui, |ui| {
                for line in [
                    "Recollect: MIT or Apache License 2.0",
                    "Inter typeface: SIL Open Font License 1.1",
                    "Material Symbols icons: Apache License 2.0",
                    "ONNX Runtime: MIT License",
                    "FFmpeg: GNU LGPL",
                ] {
                    ui.label(line);
                }
            });
    }
}

#[cfg(test)]
mod tests;
