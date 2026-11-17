//! The details panel beside the picture: when, camera, size, where the file is, which albums
//! it is in, and the ways to find more like it.

use super::entry::ViewerEntry;
use super::{Action, Viewer};
use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::store::db::{AlbumInfo, FileDetails};
use crate::ui::fmt_time;
use crate::ui::preview::Previews;
use crate::ui::style::{self, Palette};
use crate::ui::timeline::{DATE_FROM_FILE, fmt_date};
use egui::RichText;

/// Size of a section's label and of its value.
const LABEL_SIZE: f32 = 12.0;
const VALUE_SIZE: f32 = 13.0;

impl Viewer {
    /// The panel's contents.
    pub(super) fn info_panel(
        &mut self,
        ui: &mut egui::Ui,
        previews: &mut Previews,
        albums: &[AlbumInfo],
        can_find_similar: bool,
        actions: &mut Vec<Action>,
    ) {
        let e = self.current.clone();
        let details: Option<FileDetails> = previews.details(e.file_id).cloned();
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(style::semibold("Details").size(15.0));
        ui.add_space(8.0);
        when_section(ui, &e, details.as_ref());
        let camera = details.as_ref().and_then(|d| d.camera.clone());
        if let Some(c) = camera.or_else(|| e.camera.clone()) {
            section(ui, "Camera", |ui| value(ui, &c));
        }
        section(ui, "Size", |ui| value(ui, &size_text(&e, details.as_ref())));
        section(ui, "File", |ui| value(ui, &e.name()));
        folder_section(ui, &e, actions);
        let ids = details.as_ref().map_or(&[][..], |d| &d.album_ids[..]);
        section(ui, "Albums", |ui| albums_value(ui, albums, ids));
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
            ui.add_space(8.0);
            self.moment_chips(ui, &e);
        }
    }

    /// "Matching moments": one chip per moment of a video; the chosen one is highlighted.
    fn moment_chips(&mut self, ui: &mut egui::Ui, e: &ViewerEntry) {
        ui.label(label_text(ui, "Matching moments"));
        ui.horizontal_wrapped(|ui| {
            for m in &e.moments {
                let label = format!("{} – {}", fmt_time(m.t_start), fmt_time(m.t_end));
                if ui.selectable_label(e.t == Some(m.t_start), label).clicked() {
                    self.pick_moment(m.t_start);
                }
            }
        });
    }
}

/// A muted section label over what `add_value` draws.
fn section(ui: &mut egui::Ui, label: &str, add_value: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.label(label_text(ui, label));
    add_value(ui);
}

fn label_text(ui: &egui::Ui, text: &str) -> RichText {
    RichText::new(text)
        .size(LABEL_SIZE)
        .color(Palette::of(ui.ctx()).muted)
}

fn value(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(VALUE_SIZE));
}

/// The date and time, and a note when the date is only the file's own.
fn when_section(ui: &mut egui::Ui, e: &ViewerEntry, details: Option<&FileDetails>) {
    let taken = details.map_or(e.taken_at, |d| d.taken_at);
    let source = details.map(|d| d.date_source).or(e.date_source);
    ui.label(label_text(ui, "When"));
    value(ui, &fmt_date(taken));
    if source == Some(DateSource::File) {
        ui.label(label_text(ui, DATE_FROM_FILE));
    }
}

/// "4000×3000 · 3.2 MB", with the length of a video.
fn size_text(e: &ViewerEntry, details: Option<&FileDetails>) -> String {
    let mut parts = Vec::new();
    if let Some((w, h)) = e.dims() {
        parts.push(format!("{w}×{h}"));
    }
    if let Some(s) = details.and_then(|d| d.size) {
        parts.push(fmt_size(s));
    }
    if e.kind == MediaKind::Video
        && let Some(d) = e.duration
    {
        parts.push(format!("length {}", fmt_time(d)));
    }
    if parts.is_empty() {
        "Unknown".to_string()
    } else {
        parts.join(" · ")
    }
}

/// The folder's name (the whole path on hover) and a button that shows it.
fn folder_section(ui: &mut egui::Ui, e: &ViewerEntry, actions: &mut Vec<Action>) {
    section(ui, "Folder", |ui| {
        if let Some(dir) = e.path.parent() {
            let name = dir.file_name().map_or_else(
                || dir.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            ui.label(RichText::new(name).size(VALUE_SIZE))
                .on_hover_text(dir.display().to_string());
        }
        if ui.button("Show in folder").clicked() {
            actions.push(Action::Reveal(e.path.clone()));
        }
    });
}

/// The names of the albums with these ids, or a note that there are none.
fn albums_value(ui: &mut egui::Ui, albums: &[AlbumInfo], ids: &[i64]) {
    let names: Vec<&str> = albums
        .iter()
        .filter(|a| ids.contains(&a.id))
        .map(|a| a.name.as_str())
        .collect();
    if names.is_empty() {
        value(ui, "Not in any album");
    }
    for n in names {
        ui.add(egui::Label::new(RichText::new(n).size(VALUE_SIZE)).truncate());
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
