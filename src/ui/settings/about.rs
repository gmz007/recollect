//! Settings › About: the app, the privacy promise and the licenses.

use super::rows::group;
use crate::ui::App;
use crate::ui::style::{self, Palette};
use egui::RichText;

/// Side of the app mark, in points.
const MARK_SIZE: f32 = 44.0;

const LICENSES: [&str; 5] = [
    "Recollect: MIT or Apache License 2.0",
    "Inter typeface: SIL Open Font License 1.1",
    "Material Symbols icons: Apache License 2.0",
    "ONNX Runtime: MIT License",
    "FFmpeg: GNU LGPL",
];

impl App {
    pub(super) fn settings_about(ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            style::app_mark(ui, MARK_SIZE, super::surface(&pal));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(style::semibold("Recollect").size(15.0));
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(pal.muted),
                );
            });
        });
        ui.add_space(16.0);
        group(ui, |ui, rows| {
            rows.row(
                ui,
                "Your photos stay private",
                Some(
                    "Your photos never leave this computer. Recollect never changes or \
                     deletes your files.",
                ),
                0.0,
                |_| {},
            );
            rows.row(
                ui,
                "Free and open source",
                Some(
                    "You can read how Recollect works, and how to build it, in the \
                     project's documentation. The only time it uses the internet is the \
                     one-time download for smart search.",
                ),
                0.0,
                |_| {},
            );
        });
        ui.add_space(12.0);
        egui::CollapsingHeader::new("Open-source licenses")
            .id_salt("settings-licenses")
            .show(ui, |ui| {
                for line in LICENSES {
                    ui.label(line);
                }
            });
    }
}
