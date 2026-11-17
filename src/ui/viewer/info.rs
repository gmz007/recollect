//! The details panel beside the picture: date, camera, size, place on disk.

use super::entry::ViewerEntry;
use super::{Action, Viewer};
use crate::media::metadata::DateSource;
use crate::store::db::FileDetails;
use crate::ui::fmt_time;
use crate::ui::preview::Previews;
use crate::ui::style::{self, icons};
use crate::ui::timeline::{DATE_FROM_FILE, fmt_date};
use egui::RichText;

impl Viewer {
    pub(super) fn info_panel(
        &mut self,
        ui: &mut egui::Ui,
        previews: &mut Previews,
        can_find_similar: bool,
        actions: &mut Vec<Action>,
    ) {
        let e = self.current.clone();
        let details: Option<FileDetails> = previews.details(e.file_id).cloned();
        ui.add_space(6.0);
        info_heading(ui, &e);
        ui.add_space(6.0);
        info_facts(ui, &e, details.as_ref());
        if can_find_similar && let Some(item_id) = e.similar_item() {
            ui.add_space(8.0);
            if ui.button("Find similar photos").clicked() {
                actions.push(Action::FindSimilar {
                    item_id,
                    label: e.name(),
                });
            }
        }
        if e.moments.len() > 1 || (e.moments.len() == 1 && e.t.is_some()) {
            ui.add_space(10.0);
            self.moment_chips(ui, &e);
        }
    }

    /// "Matching moments": one chip per moment of a video; the chosen one is highlighted.
    fn moment_chips(&mut self, ui: &mut egui::Ui, e: &ViewerEntry) {
        ui.label(RichText::new("Matching moments").strong());
        ui.horizontal_wrapped(|ui| {
            for m in &e.moments {
                let label = format!("{} – {}", fmt_time(m.t_start), fmt_time(m.t_end));
                if ui.selectable_label(e.t == Some(m.t_start), label).clicked() {
                    self.moments.insert(self.index, m.t_start);
                    self.current.t = Some(m.t_start);
                }
            }
        });
    }
}

/// The file's name (with a star when it is a favorite) and, below it, its folder's name; the
/// whole path shows on hover.
fn info_heading(ui: &mut egui::Ui, e: &ViewerEntry) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(e.name()).strong());
        if e.favorite {
            ui.label(
                style::icon_text(icons::ICON_STAR, 16.0).color(style::Palette::of(ui.ctx()).text),
            );
        }
    });
    if let Some(dir) = e.path.parent() {
        let name = dir.file_name().map_or_else(
            || dir.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        ui.label(RichText::new(name).small().weak())
            .on_hover_text(dir.display().to_string());
    }
}

/// The facts about the file: date, camera, size, length.
fn info_facts(ui: &mut egui::Ui, e: &ViewerEntry, details: Option<&FileDetails>) {
    let taken = details.map_or(e.taken_at, |d| d.taken_at);
    let source = details.map(|d| d.date_source).or(e.date_source);
    ui.label(fmt_date(taken));
    if source == Some(DateSource::File) {
        ui.label(RichText::new(DATE_FROM_FILE).small().weak());
    }
    let camera = details.and_then(|d| d.camera.clone()).or(e.camera.clone());
    if let Some(c) = camera {
        ui.label(RichText::new(c).small());
    }
    if let Some((w, h)) = e.dims() {
        ui.label(format!("{w}×{h}"));
    }
    if let Some(s) = details.and_then(|d| d.size) {
        ui.label(fmt_size(s));
    }
    if let Some(d) = e.duration {
        ui.label(format!("length {}", fmt_time(d)));
    }
}

/// `3.2 MB`, `812 KB`, `2.0 GB`.
pub(super) fn fmt_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}
