//! Settings › Library: the folders, what has been looked at so far, and the advanced knobs.

use super::rows::{Rows, footnote, group};
use super::widgets::committed_slider;
use crate::store::db::Stats;
use crate::ui::style::{self, icons};
use crate::ui::timeline::thousands;
use crate::ui::{App, folder_labels};
use egui::{Margin, RichText};

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

const BUTTON_WIDTH: f32 = 150.0;

impl App {
    pub(super) fn settings_library(&mut self, ui: &mut egui::Ui) {
        self.settings_folders(ui);
        ui.add_space(16.0);
        self.settings_library_summary(ui);
        ui.add_space(14.0);
        self.settings_advanced(ui);
    }

    fn settings_folders(&mut self, ui: &mut egui::Ui) {
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
        let mut stop = None;
        if !rows.is_empty() {
            group(ui, |ui, group_rows| {
                for (id, name, path) in &rows {
                    group_rows.row(ui, name, Some(path), BUTTON_WIDTH, |ui| {
                        let label = format!(
                            "Stop showing “{}”…",
                            crate::ui::albums::short_name(name, 24)
                        );
                        let r = ui.button("Stop showing");
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, &label)
                        });
                        if r.clicked() {
                            stop = Some((*id, name.clone()));
                        }
                    });
                }
            });
        }
        if let Some((id, name)) = stop {
            self.dialogs.stop_folder = Some(crate::ui::folders::StopFolder { id, name });
        }
        footnote(
            ui,
            "Stopping a folder only hides it here. Your files aren’t touched.",
        );
        ui.add_space(4.0);
        if style::icon_label_button(ui, icons::ICON_CREATE_NEW_FOLDER, "Add a folder…").clicked()
        {
            self.open_add_folder();
        }
    }

    fn settings_library_summary(&mut self, ui: &mut egui::Ui) {
        let busy = self.lib.is_busy();
        let stats = self.snapshot.stats.clone();
        let text = counts_text(&stats, self.snapshot.model_installed);
        let text = if text.is_empty() {
            "No photos or videos yet.".to_string()
        } else {
            text
        };
        let can_check = !busy && !self.snapshot.folders.is_empty();
        let mut check = false;
        let mut toggle_failed = false;
        group(ui, |ui, rows| {
            rows.row(ui, "Your library", Some(&text), 0.0, |_| {});
            if stats.errors > 0 {
                self.failed_files_row(ui, rows, &mut toggle_failed);
            }
            rows.row(
                ui,
                "Check for new photos",
                Some("Recollect also checks when it starts. Use this after adding photos."),
                BUTTON_WIDTH + 40.0,
                |ui| {
                    ui.add_enabled_ui(can_check, |ui| {
                        check = ui.button("Check for new photos").clicked();
                    });
                },
            );
        });
        if toggle_failed {
            self.failed_open = !self.failed_open;
        }
        if check {
            self.start_indexing();
        }
    }

    /// The row about files that couldn't be opened, with the list under it when shown.
    fn failed_files_row(&mut self, ui: &mut egui::Ui, rows: &mut Rows, toggle: &mut bool) {
        let open = self.failed_open;
        rows.row(
            ui,
            "Files that couldn’t be opened",
            Some("They may be damaged or unsupported."),
            80.0,
            |ui| *toggle = ui.link(if open { "Hide" } else { "Show" }).clicked(),
        );
        if open {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.failed_files_list(ui, "settings-failed", 140.0);
                });
        }
    }

    fn settings_advanced(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced")
            .id_salt("settings-advanced")
            .show(ui, |ui| {
                let s = self.settings.clone();
                let max = crate::config::available_threads().unwrap_or(4) as u64;
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
