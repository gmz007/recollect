//! Settings › Storage: the room photo previews take and where Recollect keeps its data.

use super::rows::{footnote, group, group_title};
use super::widgets::{committed_slider, fmt_space};
use crate::ui::{App, platform, prune_thumbnails};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BUTTON_WIDTH: f32 = 130.0;
const SLIDER_WIDTH: f32 = 340.0;
/// How often the space the previews take is measured while this section is shown.
const MEASURE_EVERY: Duration = Duration::from_secs(5);

/// The measured size of the previews, refreshed in the background.
fn space_id() -> egui::Id {
    egui::Id::NULL.with("preview-space")
}

#[derive(Clone)]
struct PreviewSpace {
    bytes: Arc<AtomicU64>,
    measured: Instant,
}

impl App {
    pub(super) fn settings_storage(&mut self, ui: &mut egui::Ui) {
        let used = self.preview_space_used(ui);
        let mut clear = false;
        let mut new_limit = None;
        let limit = self.settings.thumb_cache_mb;
        group(ui, |ui, rows| {
            let desc = format!(
                "{} used. Safe to clear: they’re made again when needed.",
                fmt_space(used as f64 / f64::from(1 << 20))
            );
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
        self.data_folders(ui);
    }

    /// The bytes previews take; measured in the background and refreshed now and then.
    fn preview_space_used(&self, ui: &egui::Ui) -> u64 {
        let id = space_id();
        let stale = |p: &PreviewSpace| p.measured.elapsed() > MEASURE_EVERY;
        let known = ui.data(|d| d.get_temp::<PreviewSpace>(id));
        if let Some(p) = known.as_ref().filter(|p| !stale(p)) {
            return p.bytes.load(Ordering::Relaxed);
        }
        let bytes = known.map_or_else(Arc::default, |p| p.bytes);
        let (dir, out) = (self.lib.paths().thumbs_dir(), Arc::clone(&bytes));
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            out.store(crate::thumbs::used_bytes(&dir), Ordering::Relaxed);
            ctx.request_repaint();
        });
        let current = bytes.load(Ordering::Relaxed);
        let space = PreviewSpace {
            bytes,
            measured: Instant::now(),
        };
        ui.data_mut(|d| d.insert_temp(id, space));
        current
    }

    fn data_folders(&mut self, ui: &mut egui::Ui) {
        group_title(ui, "Where Recollect keeps its data");
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
        footnote(
            ui,
            "Recollect never writes anything into your photo folders.",
        );
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
