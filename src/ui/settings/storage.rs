//! Settings › Storage: space used for previews and where Recollect keeps its data.

use super::widgets::{committed_slider, fmt_space};
use super::{heading, hint};
use crate::ui::style::{self, Palette};
use crate::ui::{App, platform, prune_thumbnails};
use egui::{CornerRadius, Margin, RichText};
use std::path::Path;

impl App {
    pub(super) fn settings_storage(&mut self, ui: &mut egui::Ui) {
        self.preview_space_setting(ui);
        ui.add_space(12.0);
        heading(ui, "Where Recollect keeps its data");
        hint(
            ui,
            "Recollect never writes anything into your photo folders.",
        );
        ui.add_space(4.0);
        let paths = self.lib.paths().clone();
        for (what, desc, dir) in [
            (
                "library",
                "Your library and smart search",
                paths.data.clone(),
            ),
            ("previews", "Previews", paths.cache.clone()),
            ("settings", "Settings", paths.config),
        ] {
            if data_folder_row(ui, what, desc, &dir) {
                self.open_data_folder(&dir);
            }
            ui.add_space(2.0);
        }
    }

    /// The heading, explanation and slider for how much room previews may take.
    fn preview_space_setting(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Space for previews");
        ui.label(
            "Recollect keeps small copies of your photos so scrolling stays fast. They are \
             made again whenever they are needed, so you can give them as much or as little \
             room as you like.",
        );
        ui.add_space(6.0);
        let current = self.settings.thumb_cache_mb;
        if let Some(mb) = committed_slider(
            ui,
            "Space for previews",
            current,
            64..=8192,
            64.0,
            fmt_space,
        ) {
            self.change_settings(|s| s.thumb_cache_mb = mb);
            prune_thumbnails(&self.lib);
        }
    }

    /// Opens `dir` in the file manager (creating it first if it does not exist yet).
    fn open_data_folder(&mut self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        if let Err(e) = platform::open(dir) {
            self.notify_problem(
                "Couldn't open that folder. Open it from your file manager instead.",
                format!("{e:#}"),
                None,
            );
        }
    }
}

/// A card naming one of the folders Recollect keeps data in, with its path and an "Open
/// folder" button (named after `what`). Returns true when the button was clicked.
fn data_folder_row(ui: &mut egui::Ui, what: &str, desc: &str, dir: &Path) -> bool {
    let pal = Palette::of(ui.ctx());
    let mut open = false;
    egui::Frame::new()
        .fill(pal.field)
        .corner_radius(CornerRadius::same(style::CARD_RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let text_w = (ui.available_width() - 140.0).max(80.0);
                ui.vertical(|ui| {
                    ui.set_max_width(text_w);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(style::semibold(desc));
                    ui.add(
                        egui::Label::new(
                            RichText::new(dir.display().to_string())
                                .small()
                                .color(pal.text2),
                        )
                        .truncate(),
                    )
                    .on_hover_text(dir.display().to_string());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.button("Open folder");
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            format!("Open folder for {what}"),
                        )
                    });
                    open = r.clicked();
                });
            });
        });
    open
}
