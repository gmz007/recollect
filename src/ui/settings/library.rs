//! Settings › Library: the folders, what has been looked at so far, and the advanced knobs.

use super::widgets::committed_slider;
use super::{heading, hint};
use crate::store::db::Stats;
use crate::ui::style::{self, Palette, icons};
use crate::ui::timeline::thousands;
use crate::ui::{App, folder_labels};
use egui::{CornerRadius, Margin, RichText};

/// "1,204 photos · 37 videos · 1,241 ready for search · 3 couldn't be opened".
pub(in crate::ui) fn counts_text(stats: &Stats, installed: bool) -> String {
    let mut parts = Vec::new();
    let noun = |n: usize, one: &str, many: &str| {
        format!("{} {}", thousands(n), if n == 1 { one } else { many })
    };
    if stats.images > 0 {
        parts.push(noun(stats.images, "photo", "photos"));
    }
    if stats.videos > 0 {
        parts.push(noun(stats.videos, "video", "videos"));
    }
    if installed && stats.indexed > 0 {
        parts.push(format!("{} ready for search", thousands(stats.indexed)));
    }
    if stats.errors > 0 {
        parts.push(format!("{} couldn't be opened", thousands(stats.errors)));
    }
    parts.join(" · ")
}

impl App {
    pub(super) fn settings_library(&mut self, ui: &mut egui::Ui) {
        self.settings_folders(ui);
        self.settings_library_summary(ui);
        ui.add_space(14.0);
        self.settings_advanced(ui);
    }

    pub(super) fn settings_folders(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        heading(ui, "Folders");
        if self.snapshot.folders.is_empty() {
            ui.label("No folders yet. Add the folders where your photos and videos are.");
        }
        let labels = folder_labels(self.snapshot.folders.iter().map(|f| f.path.as_path()));
        let rows: Vec<(i64, String, String)> = self
            .snapshot
            .folders
            .iter()
            .zip(labels)
            .map(|(f, l)| (f.id, l, f.path.display().to_string()))
            .collect();
        for (id, name, path) in rows {
            egui::Frame::new()
                .fill(pal.raised)
                .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            style::icon_text(icons::ICON_FOLDER, 22.0).color(pal.text_secondary),
                        );
                        let stop_label = format!(
                            "Stop showing “{}”…",
                            crate::ui::albums::short_name(&name, 24)
                        );
                        let button_w = ui
                            .painter()
                            .layout_no_wrap(
                                stop_label.clone(),
                                egui::TextStyle::Button.resolve(ui.style()),
                                pal.text,
                            )
                            .size()
                            .x
                            + 40.0;
                        let text_w = (ui.available_width() - button_w - 16.0).max(80.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(text_w);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(style::semibold(&name)).truncate());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&path).small().color(pal.text_secondary),
                                )
                                .truncate(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(&stop_label).clicked() {
                                self.dialogs.stop_folder = Some(crate::ui::sidebar::StopFolder {
                                    id,
                                    name: name.clone(),
                                });
                            }
                        });
                    });
                });
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        if style::icon_label_button(ui, icons::ICON_CREATE_NEW_FOLDER, "Add a folder…").clicked()
        {
            self.open_add_folder();
        }
    }

    pub(super) fn settings_library_summary(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let busy = self.lib.is_busy();
        heading(ui, "Your library");
        let stats = self.snapshot.stats.clone();
        let text = counts_text(&stats, self.snapshot.model_installed);
        if text.is_empty() {
            ui.label("No photos or videos yet.");
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label(text);
                if stats.errors > 0
                    && ui
                        .link(if self.failed_open { "Hide" } else { "Show" })
                        .clicked()
                {
                    self.failed_open = !self.failed_open;
                }
            });
        }
        if stats.errors > 0 && self.failed_open {
            hint(
                ui,
                "These files couldn't be opened. They may be damaged or unsupported.",
            );
            egui::Frame::new()
                .fill(pal.raised)
                .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.failed_files_list(ui, "settings-failed", 140.0);
                });
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!busy && !self.snapshot.folders.is_empty(), |ui| {
            if style::icon_label_button(ui, icons::ICON_REFRESH, "Check for new photos").clicked() {
                self.start_indexing();
            }
        });
        hint(
            ui,
            "Recollect also checks when it starts. Use this after adding photos to a folder.",
        );
    }

    pub(super) fn settings_advanced(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced")
            .id_salt("settings-advanced")
            .show(ui, |ui| {
                let s = self.settings.clone();
                let max =
                    std::thread::available_parallelism().map_or(4, std::num::NonZero::get) as u64;
                ui.label("Indexing threads (decoding workers while photos are indexed)");
                if let Some(n) = committed_slider(
                    ui,
                    "Indexing threads",
                    s.decode_threads() as u64,
                    1..=max,
                    1.0,
                    |n| format!("{}", n as u64),
                ) {
                    self.change_settings(|s| {
                        s.index_threads = Some(usize::try_from(n).unwrap_or(usize::MAX));
                    });
                }
                ui.add_space(6.0);
                ui.label(format!(
                    "Embedding model: {} ({})",
                    self.lib.model().name,
                    self.lib.model().id
                ));
                ui.label(
                    RichText::new(format!(
                        "Model files: {}",
                        self.lib.paths().models_dir().display()
                    ))
                    .small(),
                );
            });
    }
}
