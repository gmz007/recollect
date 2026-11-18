//! Settings › Appearance: color, text size and photo size.

use super::rows::{Rows, footnote, group};
use super::widgets::segmented;
use crate::config::{TextSize, Theme};
use crate::ui::App;
use crate::ui::photo_size::PhotoSizeLevel;
use crate::ui::style;

const CONTROL_WIDTH: f32 = 250.0;

/// The changes picked in this frame.
#[derive(Default)]
struct Picks {
    theme: Option<Theme>,
    text_size: Option<TextSize>,
    photo_size: Option<PhotoSizeLevel>,
}

impl App {
    pub(super) fn settings_appearance(&mut self, ui: &mut egui::Ui) {
        let mut picks = Picks::default();
        group(ui, |ui, rows| {
            self.color_rows(ui, rows, &mut picks);
            self.photo_size_row(ui, rows, &mut picks);
        });
        footnote(ui, "Changes apply straight away — try them here.");
        if let Some(t) = picks.theme {
            self.change_settings(|s| s.theme = t);
        }
        if let Some(t) = picks.text_size {
            self.change_settings(|s| s.text_size = t);
        }
        if let Some(l) = picks.photo_size {
            self.choose_photo_size(l);
        }
    }

    /// The Color and Text size rows.
    fn color_rows(&self, ui: &mut egui::Ui, rows: &mut Rows, picks: &mut Picks) {
        let s = &self.settings;
        let desc = "System follows your computer’s light or dark setting.";
        rows.row(ui, "Color", Some(desc), CONTROL_WIDTH, |ui| {
            let items = [
                (Theme::System, "System"),
                (Theme::Light, "Light"),
                (Theme::Dark, "Dark"),
            ];
            picks.theme = segmented(ui, &items, s.theme);
        });
        rows.row(ui, "Text size", None, CONTROL_WIDTH, |ui| {
            let items = [
                (TextSize::Small, "Small"),
                (TextSize::Medium, "Medium"),
                (TextSize::Large, "Large"),
            ];
            picks.text_size = segmented(ui, &items, s.text_size);
        });
    }

    fn photo_size_row(&self, ui: &mut egui::Ui, rows: &mut Rows, picks: &mut Picks) {
        let level = PhotoSizeLevel::of(self.tl.tile_size);
        let desc = format!(
            "While browsing you can also pinch, or press {}+ = and −.",
            style::command_key()
        );
        rows.row(ui, "Photo size", Some(&desc), CONTROL_WIDTH, |ui| {
            let items = PhotoSizeLevel::ALL.map(|l| (Some(l), l.label()));
            picks.photo_size = segmented(ui, &items, level).flatten();
        });
    }
}
