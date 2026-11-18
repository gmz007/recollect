//! Settings › Storage: the room photo previews take and where Recollect keeps its data.

use super::rows::{footnote, group};
use super::widgets::{committed_slider, fmt_space};
use crate::ui::{App, platform, prune_thumbnails};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BUTTON_WIDTH: f32 = 130.0;
const SLIDER_WIDTH: f32 = 340.0;
/// How often the space the previews take is measured while this section is shown.
const MEASURE_EVERY: Duration = Duration::from_secs(5);

/// The measured size of the previews, refreshed in the background.
fn space_id() -> egui::Id {
    egui::Id::NULL.with("preview-space")
}

/// Stands for "not measured yet" in [`PreviewSpace::bytes`].
const UNMEASURED: u64 = u64::MAX;

#[derive(Clone)]
struct PreviewSpace {
    bytes: Arc<AtomicU64>,
    /// A walk over the previews is under way; only one runs at a time.
    walking: Arc<AtomicBool>,
    /// When the last walk was started.
    measured: Option<Instant>,
}

impl Default for PreviewSpace {
    fn default() -> Self {
        Self {
            bytes: Arc::new(AtomicU64::new(UNMEASURED)),
            walking: Arc::default(),
            measured: None,
        }
    }
}

/// The description of the previews row: the space used, or that it is being measured.
fn preview_desc(used: Option<u64>) -> String {
    let rest = "Safe to clear: they’re made again when needed.";
    match used {
        Some(bytes) => format!(
            "{} used. {rest}",
            fmt_space(bytes as f64 / f64::from(1 << 20))
        ),
        None => format!("Measuring… {rest}"),
    }
}

impl App {
    pub(super) fn settings_storage(&mut self, ui: &mut egui::Ui) {
        let used = self.preview_space_used(ui);
        let mut clear = false;
        let mut new_limit = None;
        let limit = self.settings.thumb_cache_mb;
        group(ui, |ui, rows| {
            let desc = preview_desc(used);
            rows.row(ui, "Photo previews", Some(&desc), BUTTON_WIDTH, |ui| {
                clear = ui.button("Clear previews").clicked();
            });
            rows.row(
                ui,
                "Space for previews",
                Some("The most room previews may take."),
                SLIDER_WIDTH,
                |ui| {
                    new_limit = committed_slider(
                        ui,
                        "Space for previews",
                        limit,
                        64..=8192,
                        64.0,
                        fmt_space,
                    );
                },
            );
        });
        if let Some(mb) = new_limit {
            self.change_settings(|s| s.thumb_cache_mb = mb);
            prune_thumbnails(&self.lib);
        }
        if clear {
            let dir = self.lib.paths().thumbs_dir();
            std::thread::spawn(move || crate::thumbs::prune(&dir, 0));
            // Shown as empty at once; measured again in a few seconds.
            if let Some(p) = ui.data(|d| d.get_temp::<PreviewSpace>(space_id())) {
                p.bytes.store(0, Ordering::Relaxed);
            }
        }
        // The folder paths are technical (and name system folders like "Caches"): collapsed.
        egui::CollapsingHeader::new("Where Recollect keeps its data")
            .id_salt("settings-data-folders")
            .show(ui, |ui| self.data_folders(ui));
        footnote(
            ui,
            "Recollect never writes anything into your photo folders.",
        );
    }

    /// The bytes previews take, or `None` until the first measurement is in. Measured in the
    /// background (one walk at a time) and refreshed now and then.
    fn preview_space_used(&self, ui: &egui::Ui) -> Option<u64> {
        let id = space_id();
        let mut space = ui
            .data(|d| d.get_temp::<PreviewSpace>(id))
            .unwrap_or_default();
        let stale = space.measured.is_none_or(|t| t.elapsed() > MEASURE_EVERY);
        if stale && !space.walking.swap(true, Ordering::AcqRel) {
            let dir = self.lib.paths().thumbs_dir();
            let (out, walking) = (Arc::clone(&space.bytes), Arc::clone(&space.walking));
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                out.store(crate::thumbs::used_bytes(&dir), Ordering::Relaxed);
                walking.store(false, Ordering::Release);
                ctx.request_repaint();
            });
            space.measured = Some(Instant::now());
        }
        let bytes = space.bytes.load(Ordering::Relaxed);
        ui.data_mut(|d| d.insert_temp(id, space));
        (bytes != UNMEASURED).then_some(bytes)
    }

    fn data_folders(&mut self, ui: &mut egui::Ui) {
        let paths = self.lib.paths().clone();
        let places = [
            (
                "library",
                "Your library and smart search",
                paths.data.clone(),
            ),
            ("previews", "Photo previews", paths.cache.clone()),
            ("settings", "Settings", paths.config),
        ];
        let mut open = None;
        group(ui, |ui, rows| {
            for (what, desc, dir) in &places {
                let path = dir.display().to_string();
                rows.row(ui, desc, Some(&path), BUTTON_WIDTH, |ui| {
                    let r = ui.button("Open folder");
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            format!("Open folder for {what}"),
                        )
                    });
                    if r.clicked() {
                        open = Some(dir.clone());
                    }
                });
            }
        });
        if let Some(dir) = open {
            self.open_data_folder(&dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_previews_row_says_measuring_until_a_size_is_known() {
        assert!(preview_desc(None).starts_with("Measuring…"));
        assert!(preview_desc(Some(3 << 20)).contains("used."));
    }
}
