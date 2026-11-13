//! The egui desktop app. All work happens on background threads; this module only renders
//! state and sends commands to the [`Library`].

mod platform;
mod workers;

use crate::Library;
use crate::config::Settings;
use crate::library::{Activity, Status};
use crate::media::MediaKind;
use crate::model::{download, registry};
use crate::search::{SearchHit, SearchOptions};
use crate::store::db::{Folder, Stats};
use crate::thumbs::ThumbKey;
use egui::{Align, Color32, Layout, RichText, Sense, Vec2};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
pub use workers::Query;
use workers::{SearchWorker, ThumbCache};

const TILE: f32 = 180.0;
const GAP: f32 = 8.0;
const DEBOUNCE: Duration = Duration::from_millis(250);

pub fn run(lib: Arc<Library>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Recollect")
            .with_app_id("recollect")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Recollect",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, lib)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}

/// Cached library facts, refreshed a few times per second rather than every frame.
#[derive(Default)]
struct Snapshot {
    stats: Stats,
    folders: Vec<Folder>,
    errors: Vec<(PathBuf, String)>,
    model_installed: bool,
    taken: Option<Instant>,
}

impl Snapshot {
    /// Database queries: run off the UI thread (the database may be busy for a while).
    fn fetch(lib: &Library) -> Self {
        Self {
            stats: lib.stats().unwrap_or_default(),
            folders: lib.folders().unwrap_or_default(),
            errors: lib.errors(50).unwrap_or_default(),
            model_installed: lib.model_installed(),
            taken: Some(Instant::now()),
        }
    }
}

pub struct App {
    lib: Arc<Library>,
    search: SearchWorker,
    thumbs: ThumbCache,
    query: String,
    query_changed_at: Option<Instant>,
    kind: Option<MediaKind>,
    current: Option<Query>,
    awaiting: Option<u64>,
    results: Vec<SearchHit>,
    result_label: String,
    result_ms: f32,
    result_error: Option<String>,
    last_search_at: Option<Instant>,
    selected: Option<usize>,
    /// Selected moment (seconds) for the selected video.
    selected_t: Option<f64>,
    folder_input: String,
    notice: Option<(String, Instant)>,
    snapshot: Snapshot,
    snapshot_rx: Option<crossbeam_channel::Receiver<Snapshot>>,
    snapshot_again: bool,
    ctx: egui::Context,
    status: Status,
    was_busy: bool,
    jobs_seen: u64,
    last_prune: Instant,
    consent_dismissed: bool,
    settings_open: bool,
    settings_draft: Settings,
}

impl App {
    pub fn new(ctx: &egui::Context, lib: Arc<Library>) -> Self {
        let c = ctx.clone();
        lib.set_notifier(move || c.request_repaint());
        let c = ctx.clone();
        let search = SearchWorker::new(Arc::clone(&lib), move || c.request_repaint());
        let thumbs = ThumbCache::new(ctx, lib.paths().thumbs_dir(), 3);
        let settings = lib.settings();
        prune_thumbnails(&lib);
        let mut app = Self {
            lib,
            search,
            thumbs,
            query: String::new(),
            query_changed_at: None,
            kind: None,
            current: None,
            awaiting: None,
            results: Vec::new(),
            result_label: String::new(),
            result_ms: 0.0,
            result_error: None,
            last_search_at: None,
            selected: None,
            selected_t: None,
            folder_input: String::new(),
            notice: None,
            snapshot: Snapshot::default(),
            snapshot_rx: None,
            snapshot_again: false,
            ctx: ctx.clone(),
            status: Status::default(),
            was_busy: false,
            jobs_seen: 0,
            last_prune: Instant::now(),
            consent_dismissed: false,
            settings_open: false,
            settings_draft: settings,
        };
        app.snapshot = Snapshot::fetch(&app.lib);
        if app.snapshot.model_installed {
            // Load the text model in the background so the first search is fast.
            let lib = Arc::clone(&app.lib);
            std::thread::spawn(move || lib.embed_text("warm up").ok());
        }
        // Pick up changes made while the app was closed.
        if app.snapshot.model_installed && !app.snapshot.folders.is_empty() {
            app.start_indexing();
        }
        app
    }

    fn notify(&mut self, msg: impl Into<String>) {
        self.notice = Some((msg.into(), Instant::now()));
    }

    /// Requests fresh library facts from a background thread (never blocks the UI).
    fn refresh_snapshot(&mut self, force: bool) {
        if self.snapshot_rx.is_some() {
            self.snapshot_again |= force;
            return;
        }
        if !force
            && self
                .snapshot
                .taken
                .is_some_and(|t| t.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let lib = Arc::clone(&self.lib);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Snapshot::fetch(&lib));
            ctx.request_repaint();
        });
        self.snapshot_rx = Some(rx);
    }

    fn poll_snapshot(&mut self) {
        if let Some(rx) = &self.snapshot_rx
            && let Ok(s) = rx.try_recv()
        {
            self.snapshot = s;
            self.snapshot_rx = None;
            if std::mem::take(&mut self.snapshot_again) {
                self.refresh_snapshot(true);
            }
        }
    }

    fn start_indexing(&mut self) {
        if let Err(e) = self.lib.start_indexing() {
            self.notify(format!("{e:#}"));
        }
    }

    fn submit(&mut self, q: Query) {
        let opts = SearchOptions {
            limit: 300,
            kind: self.kind,
        };
        self.awaiting = Some(self.search.submit(q.clone(), opts));
        self.current = Some(q);
        self.last_search_at = Some(Instant::now());
    }

    fn rerun(&mut self) {
        if let Some(q) = self.current.clone() {
            self.submit(q);
        }
    }

    /// Per-frame bookkeeping: results, status transitions, debounce, live refresh.
    fn update_state(&mut self, ctx: &egui::Context) {
        self.thumbs.begin_frame(ctx);
        self.poll_snapshot();
        self.status = self.lib.status();
        if let Some(r) = self.search.poll()
            && Some(r.id) == self.awaiting
        {
            self.awaiting = None;
            self.result_ms = r.elapsed.as_secs_f32() * 1000.0;
            self.result_label = r.query.label();
            let prev = self
                .selected
                .and_then(|i| self.results.get(i))
                .map(|h| h.path.clone());
            match r.hits {
                Ok(h) => {
                    self.results = h;
                    self.result_error = None;
                }
                Err(e) => self.result_error = Some(e),
            }
            // Keep the selection on the same file across refreshes.
            self.selected = prev.and_then(|p| self.results.iter().position(|h| h.path == p));
            if self.selected.is_none() {
                self.selected_t = None;
            }
        }
        if let Some(t) = self.query_changed_at
            && t.elapsed() >= DEBOUNCE
        {
            self.query_changed_at = None;
            let q = self.query.trim().to_string();
            if q.is_empty() {
                self.current = None;
                self.results.clear();
                self.selected = None;
            } else {
                self.submit(Query::Text(q));
            }
        }
        if let Some(t) = self.query_changed_at {
            ctx.request_repaint_after(DEBOUNCE.saturating_sub(t.elapsed()));
        }
        let busy = self.lib.is_busy();
        if busy {
            self.refresh_snapshot(false);
            ctx.request_repaint_after(Duration::from_millis(500));
            // Results grow while indexing: refresh them every few seconds.
            if matches!(self.status.activity, Activity::Indexing(_))
                && self
                    .last_search_at
                    .is_some_and(|t| t.elapsed() > Duration::from_secs(3))
            {
                self.rerun();
            }
        }
        let jobs = self.lib.jobs_finished();
        if (self.was_busy && !busy) || jobs != self.jobs_seen {
            self.jobs_seen = jobs;
            self.refresh_snapshot(true);
            if !busy
                && self.lib.model_installed()
                && !self.snapshot.folders.is_empty()
                && self.status.last_run.is_none()
                && self.status.last_error.is_none()
            {
                // A model download or model switch just finished: index right away.
                self.start_indexing();
            }
            self.rerun();
        }
        self.was_busy = busy;
        if self.last_prune.elapsed() > Duration::from_secs(600) {
            self.last_prune = Instant::now();
            prune_thumbnails(&self.lib);
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|n| n.1.elapsed() > Duration::from_secs(6))
        {
            self.notice = None;
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Recollect").strong());
            ui.add_space(12.0);
            let width = (ui.available_width() - 300.0).max(200.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(
                        "Describe what you're looking for, e.g. \"kids on the beach at sunset\"",
                    )
                    .desired_width(width)
                    .id_salt("search"),
            );
            if resp.changed() {
                self.query_changed_at = Some(Instant::now());
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.query_changed_at = Some(Instant::now() - DEBOUNCE);
            }
            if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
                resp.request_focus();
            }
            let before = self.kind;
            ui.selectable_value(&mut self.kind, None, "All");
            ui.selectable_value(&mut self.kind, Some(MediaKind::Image), "Images");
            ui.selectable_value(&mut self.kind, Some(MediaKind::Video), "Videos");
            if before != self.kind {
                self.rerun();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.settings_draft = self.lib.settings();
                    if self.settings_draft.model.is_none() {
                        self.settings_draft.model = Some(self.lib.model().id.to_string());
                    }
                    self.settings_open = true;
                }
            });
        });
    }

    fn library_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Library");
        ui.add_space(4.0);
        let mut remove = None;
        if self.snapshot.folders.is_empty() {
            ui.label(RichText::new("No folders yet.").weak());
        }
        for f in &self.snapshot.folders {
            ui.horizontal(|ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Remove from library (files are not touched)")
                    .clicked()
                {
                    remove = Some(f.id);
                }
                let name = f
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| f.path.display().to_string());
                ui.label(name).on_hover_text(f.path.display().to_string());
            });
        }
        if let Some(id) = remove {
            // Cancels any running job first; runs in the background.
            self.lib.start_remove_folder(id);
            self.snapshot.folders.retain(|f| f.id != id);
            // A fetch already in flight may predate the removal: fetch again after it.
            self.snapshot_again = true;
        }
        ui.add_space(8.0);
        if ui.button("Add folder…").clicked()
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Add a folder to Recollect")
                .pick_folder()
        {
            self.add_folder(dir);
        }
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.folder_input)
                    .hint_text("or paste a path")
                    .desired_width(150.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || enter) && !self.folder_input.trim().is_empty() {
                let p = PathBuf::from(self.folder_input.trim());
                self.folder_input.clear();
                self.add_folder(p);
            }
        });
        ui.add_space(8.0);
        let busy = self.lib.is_busy();
        if ui
            .add_enabled(
                !busy && self.snapshot.model_installed && !self.snapshot.folders.is_empty(),
                egui::Button::new("Rescan"),
            )
            .on_hover_text("Look for new, changed and deleted files")
            .clicked()
        {
            self.start_indexing();
        }
        ui.separator();
        let s = &self.snapshot.stats;
        ui.label(format!("{} images · {} videos", s.images, s.videos));
        ui.label(format!("{} indexed · {} waiting", s.indexed, s.pending));
        ui.label(RichText::new(format!("{} searchable items", self.lib.indexed_items())).weak());
        if s.errors > 0 {
            egui::CollapsingHeader::new(format!("{} files could not be read", s.errors))
                .id_salt("errors")
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for (p, e) in &self.snapshot.errors {
                                let name = p
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                ui.label(RichText::new(name).small())
                                    .on_hover_text(format!("{}\n{e}", p.display()));
                            }
                        });
                    if ui
                        .add_enabled(!self.lib.is_busy(), egui::Button::new("Retry failed files"))
                        .clicked()
                    {
                        match self.lib.retry_errors() {
                            Ok(_) => self.start_indexing(),
                            Err(e) => self.notify(format!("{e:#}")),
                        }
                    }
                });
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Model: {}", self.lib.model().name))
                    .small()
                    .weak(),
            );
            ui.label(
                RichText::new("Everything stays on this computer")
                    .small()
                    .weak(),
            );
        });
    }

    fn add_folder(&mut self, p: PathBuf) {
        // Runs in the background; a running indexing job picks the new folder up.
        self.lib.start_add_folder(p);
        self.snapshot_again = true;
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match &self.status.activity {
                Activity::Idle => {
                    let waiting = self.snapshot.stats.pending;
                    if let Some(r) = &self.status.last_run {
                        ui.label(run_summary_text(r, waiting));
                    } else if self.status.last_error.is_some() {
                        ui.label(format!("Stopped. {waiting} files waiting."));
                    } else {
                        ui.label("Ready.");
                    }
                }
                Activity::Downloading { model, done, total } => {
                    ui.label(format!("Downloading model {model}…"));
                    ui.add(
                        egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                            .desired_width(260.0)
                            .text(format!("{} / {} MB", done / 1_000_000, total / 1_000_000)),
                    );
                }
                Activity::Scanning { folder } => {
                    ui.spinner();
                    ui.label(format!("Scanning {}…", folder.display()));
                }
                Activity::LoadingModel => {
                    ui.spinner();
                    ui.label("Loading model…");
                }
                Activity::SwitchingModel => {
                    ui.spinner();
                    ui.label("Switching model…");
                }
                Activity::Indexing(p) => {
                    let dev = p.device.map(|d| d.to_string()).unwrap_or_default();
                    ui.add(
                        egui::ProgressBar::new(p.done as f32 / p.total.max(1) as f32)
                            .desired_width(260.0)
                            .text(format!("{} / {} files", p.done, p.total)),
                    );
                    let name = p
                        .current
                        .as_ref()
                        .and_then(|c| c.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    ui.label(format!("{:.0} items/s on {dev} · {name}", p.rate));
                }
            }
            if matches!(self.status.activity, Activity::Downloading { .. })
                && ui.button("Cancel download").clicked()
            {
                self.lib.cancel();
                self.consent_dismissed = true;
            }
            let pausable = matches!(
                self.status.activity,
                Activity::Scanning { .. } | Activity::LoadingModel | Activity::Indexing(_)
            );
            if pausable && self.lib.is_busy() {
                let paused = self.status.paused;
                if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
                    self.lib.pause(!paused);
                }
                if ui.button("Cancel").clicked() {
                    self.lib.cancel();
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(e) = &self.status.last_error {
                    ui.colored_label(Color32::from_rgb(230, 120, 90), e);
                } else if let Some((n, _)) = &self.notice {
                    ui.label(n.clone());
                }
            });
        });
    }

    fn consent_card(&mut self, ui: &mut egui::Ui) {
        let spec = self.lib.model();
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.18);
            ui.heading("One-time setup: download the search model");
            ui.add_space(10.0);
            ui.label(format!(
                "Recollect needs an AI model that understands both pictures and text.\n\
                 {} — {} MB, license {}, from huggingface.co/{}.",
                spec.name,
                spec.total_size() / 1_000_000,
                spec.license,
                spec.repo
            ));
            ui.add_space(6.0);
            ui.label(RichText::new(
                "This is the only time Recollect uses the internet. Your photos and videos never leave this computer.",
            ).weak());
            ui.add_space(16.0);
            if downloading {
                ui.spinner();
            } else {
                ui.horizontal(|ui| {
                    let w = 380.0;
                    ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
                    if ui.add(egui::Button::new(RichText::new("Download model").strong())).clicked()
                        && let Err(e) = self.lib.start_download()
                    {
                        self.notify(format!("{e:#}"));
                    }
                    if ui.button("Choose another model…").clicked() {
                        self.settings_draft = self.lib.settings();
                        self.settings_open = true;
                    }
                    if ui.button("Not now").clicked() {
                        self.consent_dismissed = true;
                    }
                });
            }
        });
    }

    fn empty_state(&self, ui: &mut egui::Ui, title: &str, body: &str) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading(title);
            ui.add_space(8.0);
            ui.label(RichText::new(body).weak());
        });
    }

    fn results_grid(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.awaiting.is_some() {
                ui.spinner();
            }
            if let Some(e) = &self.result_error {
                ui.colored_label(Color32::from_rgb(230, 120, 90), e);
            } else if !self.result_label.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Best {} matches for “{}” · {:.0} ms",
                        self.results.len(),
                        self.result_label,
                        self.result_ms
                    ))
                    .weak(),
                );
            }
        });
        ui.add_space(4.0);
        if self.results.is_empty() {
            if self.awaiting.is_none()
                && self.result_error.is_none()
                && !self.result_label.is_empty()
            {
                self.empty_state(ui, "No matches", "Nothing indexed yet matches this search.");
            }
            return;
        }
        let avail = ui.available_width() - 12.0; // leave room for the scroll bar
        let cols = (((avail + GAP) / (TILE + GAP)).floor() as usize).max(1);
        let tile = ((avail - GAP * (cols - 1) as f32) / cols as f32).max(TILE * 0.6);
        let rows = self.results.len().div_ceil(cols);
        let mut clicked: Option<(usize, bool)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, tile + GAP, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        for i in row * cols..((row + 1) * cols).min(self.results.len()) {
                            let resp = self.tile(ui, i, tile);
                            if resp.double_clicked() {
                                clicked = Some((i, true));
                            } else if resp.clicked() {
                                clicked = Some((i, false));
                            }
                        }
                    });
                    ui.add_space(GAP - ui.spacing().item_spacing.y);
                }
            });
        if let Some((i, open)) = clicked {
            self.selected = Some(i);
            self.selected_t = self.results[i].t_start;
            if open {
                self.open_hit(i);
            }
        }
    }

    fn thumb_key(hit: &SearchHit, t: Option<f64>) -> ThumbKey {
        ThumbKey {
            path: hit.path.clone(),
            mtime: hit.mtime,
            t_ms: if hit.kind == MediaKind::Video {
                Some((t.or(hit.t_start).unwrap_or(0.0) * 1000.0) as i64)
            } else {
                None
            },
        }
    }

    fn tile(&mut self, ui: &mut egui::Ui, i: usize, size: f32) -> egui::Response {
        let hit = &self.results[i];
        let key = Self::thumb_key(hit, None);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
            let failed = self.thumbs.failed(&key);
            if let Some(tex) = self.thumbs.get(&key) {
                let size = tex.size_vec2();
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                let r = egui::Rect::from_center_size(rect.center(), size * scale);
                painter.image(
                    tex.id(),
                    r,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                let text = if failed { "preview unavailable" } else { "…" };
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(13.0),
                    ui.visuals().weak_text_color(),
                );
            }
            if hit.kind == MediaKind::Video {
                // Badge: play triangle + timestamp of the matching moment.
                let label = fmt_time(hit.t_start.unwrap_or(0.0));
                let galley =
                    painter.layout_no_wrap(label, egui::FontId::proportional(13.0), Color32::WHITE);
                let h = galley.size().y;
                let size = Vec2::new(galley.size().x + h + 4.0, h);
                let min = rect.left_bottom() + Vec2::new(6.0, -6.0 - h);
                let bg = egui::Rect::from_min_size(min, size).expand(3.0);
                painter.rect_filled(bg, 4.0, Color32::from_black_alpha(170));
                let tri = [
                    min + Vec2::new(1.0, 2.0),
                    min + Vec2::new(1.0, h - 2.0),
                    min + Vec2::new(h - 3.0, h / 2.0),
                ];
                painter.add(egui::Shape::convex_polygon(
                    tri.to_vec(),
                    Color32::WHITE,
                    egui::Stroke::NONE,
                ));
                painter.galley(min + Vec2::new(h + 2.0, 0.0), galley, Color32::WHITE);
            }
            if self.selected == Some(i) {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    6.0,
                    egui::Stroke::new(3.0, ui.visuals().selection.bg_fill),
                    egui::StrokeKind::Inside,
                );
            } else if resp.hovered() {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    6.0,
                    egui::Stroke::new(1.5, ui.visuals().widgets.hovered.bg_stroke.color),
                    egui::StrokeKind::Inside,
                );
            }
        }
        let name = hit
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        resp.on_hover_text(format!("{name}\nmatch {:.3}", hit.score))
    }

    fn open_hit(&mut self, i: usize) {
        let hit = &self.results[i];
        let res = match (hit.kind, self.selected_t.or(hit.t_start)) {
            (MediaKind::Video, Some(t)) => platform::open_video_at(&hit.path, t).map(|_| ()),
            _ => platform::open(&hit.path),
        };
        if let Err(e) = res {
            self.notify(format!("{e:#}"));
        }
    }

    fn detail_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        let hit = self.results[i].clone();
        let t = self.selected_t.or(hit.t_start);
        ui.add_space(6.0);
        let key = Self::thumb_key(&hit, t);
        let w = ui.available_width();
        if let Some(tex) = self.thumbs.get(&key) {
            let size = tex.size_vec2();
            let scale = (w / size.x).min(320.0 / size.y);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        } else {
            ui.allocate_exact_size(Vec2::new(w, 200.0), Sense::hover());
        }
        ui.add_space(6.0);
        let name = hit
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        ui.label(RichText::new(&name).strong());
        ui.label(
            RichText::new(
                hit.path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
            .small()
            .weak(),
        );
        let mut facts = Vec::new();
        if let (Some(w), Some(h)) = (hit.width, hit.height) {
            facts.push(format!("{w}×{h}"));
        }
        if let Some(d) = hit.duration {
            facts.push(format!("length {}", fmt_time(d)));
        }
        facts.push(format!("match {:.3}", hit.score));
        ui.label(RichText::new(facts.join(" · ")).small());
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let open_label = if hit.kind == MediaKind::Video {
                format!("Play at {}", fmt_time(t.unwrap_or(0.0)))
            } else {
                "Open".to_string()
            };
            if ui.button(open_label).clicked() {
                self.open_hit(i);
            }
            if ui.button("Show in folder").clicked()
                && let Err(e) = platform::reveal(&hit.path)
            {
                self.notify(format!("{e:#}"));
            }
            if ui.button("Find similar").clicked() {
                let item_id = if hit.kind == MediaKind::Video {
                    hit.moments
                        .iter()
                        .find(|m| Some(m.t_start) == t)
                        .map(|m| m.item_id)
                        .unwrap_or(hit.item_id)
                } else {
                    hit.item_id
                };
                self.query.clear();
                self.submit(Query::Similar {
                    item_id,
                    label: name.clone(),
                });
            }
        });
        if hit.kind == MediaKind::Video {
            ui.add_space(10.0);
            ui.label(RichText::new("Matching moments").strong());
            let mut moments = vec![(
                hit.t_start.unwrap_or(0.0),
                hit.t_end.unwrap_or(0.0),
                hit.score,
            )];
            moments.extend(hit.moments.iter().map(|m| (m.t_start, m.t_end, m.score)));
            for (s, e, score) in moments {
                let label = format!("{} – {}   ({score:.3})", fmt_time(s), fmt_time(e));
                if ui.selectable_label(t == Some(s), label).clicked() {
                    self.selected_t = Some(s);
                }
            }
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut apply = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.label(RichText::new("Search model").strong());
                let current = self
                    .settings_draft
                    .model
                    .clone()
                    .unwrap_or_else(|| self.lib.model().id.to_string());
                for m in registry::MODELS {
                    let installed = download::is_installed(&self.lib.paths().models_dir(), m);
                    let text = format!(
                        "{}  ({} MB{})",
                        m.name,
                        m.total_size() / 1_000_000,
                        if installed { ", downloaded" } else { "" }
                    );
                    if ui
                        .radio(current == m.id, text)
                        .on_hover_text(m.summary)
                        .clicked()
                    {
                        self.settings_draft.model = Some(m.id.to_string());
                    }
                }
                ui.label(
                    RichText::new(
                        "Switching models (also back to an earlier one) re-indexes your library.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.settings_draft.use_gpu,
                    "Use the GPU when available",
                );
                let mut threads = self.settings_draft.decode_threads();
                let max = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4);
                if ui
                    .add(egui::Slider::new(&mut threads, 1..=max).text("indexing threads"))
                    .changed()
                {
                    self.settings_draft.index_threads = Some(threads);
                }
                ui.add(
                    egui::Slider::new(&mut self.settings_draft.thumb_cache_mb, 64..=8192)
                        .text("MB thumbnail cache"),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                });
            });
        if apply {
            let switching = self
                .settings_draft
                .model
                .as_deref()
                .is_some_and(|m| m != self.lib.model().id);
            if switching && self.lib.is_busy() {
                self.notify(
                    "Wait for the current job to finish (or cancel it) before switching models.",
                );
            } else if let Err(e) = self.lib.apply_settings(self.settings_draft.clone()) {
                self.notify(format!("{e:#}"));
            } else {
                open = false;
                if switching {
                    // Old results and queries belong to the previous model.
                    self.consent_dismissed = false;
                    self.results.clear();
                    self.current = None;
                    self.query.clear();
                    self.result_label.clear();
                    self.selected = None;
                }
                self.refresh_snapshot(true);
                prune_thumbnails(&self.lib);
            }
        }
        self.settings_open = open;
    }
}

/// Keeps the thumbnail cache within the configured size (background thread).
fn prune_thumbnails(lib: &Library) {
    let dir = lib.paths().thumbs_dir();
    let max = lib.settings().thumb_cache_mb.saturating_mul(1 << 20);
    std::thread::spawn(move || crate::thumbs::prune(&dir, max));
}

/// Shown instead of the app when the library can't be opened (no console on desktop launches).
pub fn run_error(message: String) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Recollect")
            .with_app_id("recollect")
            .with_inner_size([640.0, 280.0]),
        ..Default::default()
    };
    eframe::run_ui_native("Recollect", options, move |ui, _frame| {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Recollect could not start");
            ui.add_space(8.0);
            ui.label(&message);
            ui.add_space(8.0);
            ui.label(RichText::new("Your photos and videos were not touched.").weak());
            ui.add_space(12.0);
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    })
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}

fn run_summary_text(r: &crate::library::RunSummary, waiting: usize) -> String {
    let mut parts = Vec::new();
    if r.cancelled {
        parts.push(format!("Stopped. {waiting} files waiting"));
    } else if waiting > 0 {
        parts.push(format!("Done, {waiting} files still waiting"));
    } else {
        parts.push("Up to date".to_string());
    }
    parts.push(format!(
        "last scan: {} new, {} changed, {} removed",
        r.added, r.changed, r.removed
    ));
    if r.files > 0 || r.errors > 0 {
        let dev = r.device.map(|d| format!(" on {d}")).unwrap_or_default();
        parts.push(format!(
            "{} indexed, {} failed in {:.0}s{dev}",
            r.files, r.errors, r.seconds
        ));
    }
    if !r.skipped_folders.is_empty() {
        parts.push(format!(
            "{} folder(s) unavailable, kept as they were",
            r.skipped_folders.len()
        ));
    }
    parts.join(" · ")
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl App {
    /// Draws the whole app into `ui` (separate from `eframe::App` for headless UI tests).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.update_state(&ctx);
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(3.0);
            self.status_bar(ui);
            ui.add_space(3.0);
        });
        egui::Panel::left("library")
            .resizable(true)
            .default_size(230.0)
            .min_size(180.0)
            .show(ui, |ui| {
                self.library_panel(ui);
            });
        if let Some(i) = self.selected.filter(|&i| i < self.results.len()) {
            egui::Panel::right("detail")
                .resizable(true)
                .default_size(340.0)
                .min_size(260.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.detail_panel(ui, i));
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let needs_model = !self.snapshot.model_installed;
            if needs_model && (!self.consent_dismissed || matches!(self.status.activity, Activity::Downloading { .. })) {
                self.consent_card(ui);
            } else if needs_model {
                self.empty_state(ui, "No model yet", "Recollect needs to download a model once before it can search.");
                ui.vertical_centered(|ui| {
                    if ui.button("Download model…").clicked() {
                        self.consent_dismissed = false;
                    }
                });
            } else if self.snapshot.folders.is_empty() {
                self.empty_state(ui, "Add a folder to get started", "Use “Add folder…” on the left. Recollect only reads your files; it never changes them.");
            } else if self.current.is_none() && self.query.trim().is_empty() {
                self.empty_state(ui, "What are you looking for?", "Describe a photo or a moment in a video: “a dog playing in the snow”, “birthday cake with candles”.");
            } else {
                self.results_grid(ui);
            }
        });
        if self.settings_open {
            self.settings_window(&ctx);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.selected = None;
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Closing the window stops background work cleanly (the database stays consistent
        // either way, but this avoids killing a write mid-transaction).
        self.lib.cancel();
        // A download can be stuck in a network read; don't keep the process alive for it.
        self.lib.wait_for(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests;
