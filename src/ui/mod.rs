//! The egui desktop app. All work happens on background threads; this module only renders
//! state and sends commands to the [`Library`].

mod albums;
mod frame_stats;
mod platform;
mod preview;
mod selection;
mod timeline;
mod viewer;
mod when;
mod workers;

use crate::Library;
use crate::config::Settings;
use crate::library::{Activity, Status};
use crate::media::MediaKind;
use crate::model::{download, registry};
use crate::search::{SearchHit, SearchOptions, SearchSort};
use crate::store::db::{AlbumInfo, Folder, Stats};
use crate::thumbs::ThumbKey;
use egui::{Align, Color32, Layout, RichText, Sense, Vec2};
use frame_stats::FrameStats;
use preview::Previews;
use selection::{ClickMods, Selection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use timeline::{TimelineState, View};
use viewer::Viewer;
use when::DateFilter;
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

/// A favorite mark the user set that the library may not have stored yet. While one exists it
/// wins over what a list loaded from the library says (a reload that started before the write
/// finished still carries the old value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FavMark {
    value: bool,
    /// The write was committed (or given up on); the mark is dropped once no list that was
    /// loaded earlier can still arrive.
    confirmed: bool,
}

/// The favorite flag to show for a file: the pending mark if there is one, else `listed`.
fn marked(marks: &HashMap<i64, FavMark>, file_id: i64, listed: bool) -> bool {
    marks.get(&file_id).map_or(listed, |m| m.value)
}

/// Cached library facts, refreshed a few times per second rather than every frame.
#[derive(Default)]
struct Snapshot {
    stats: Stats,
    folders: Vec<Folder>,
    albums: Vec<AlbumInfo>,
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
            albums: lib.albums().unwrap_or_default(),
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
    /// Album views (timeline, recently added, folder) shown while no search is active.
    tl: TimelineState,
    /// The open viewer; while set it replaces the whole window.
    viewer: Option<Viewer>,
    /// Whether the viewer's info panel is open (kept across items).
    viewer_info: bool,
    previews: Previews,
    query: String,
    query_changed_at: Option<Instant>,
    kind: Option<MediaKind>,
    /// Search narrowing and ordering (the "When" and "Sort" selectors).
    date: DateFilter,
    sort: SearchSort,
    /// Years that have files in the library, newest first (feeds the "When" selector); read in
    /// the background and refreshed when a job finishes.
    known_years: Vec<i32>,
    years_rx: Option<crossbeam_channel::Receiver<Vec<i32>>>,
    /// `jobs_finished` as of the last years request.
    years_seen: Option<u64>,
    /// Favorite marks waiting for the writer or for lists to catch up.
    fav_marks: HashMap<i64, FavMark>,
    fav_version_seen: u64,
    albums_version_seen: u64,
    /// A name being typed for a new album, a rename and a pending deletion.
    new_album: Option<albums::NewAlbum>,
    rename_album: Option<albums::RenameAlbum>,
    delete_album: Option<i64>,
    current: Option<Query>,
    awaiting: Option<u64>,
    /// Shared with the viewer while it is open.
    results: Arc<Vec<SearchHit>>,
    result_label: String,
    result_ms: f32,
    result_error: Option<String>,
    last_search_at: Option<Instant>,
    selected: Option<usize>,
    /// All selected results (by file id); `selected` is the primary one.
    result_sel: Selection,
    /// Selected moment (seconds) for the selected video.
    selected_t: Option<f64>,
    /// Columns in the results grid as of the last layout (for up/down navigation).
    grid_cols: usize,
    /// Scroll offset to apply on the next frame so the keyboard selection stays visible.
    scroll_to: Option<f32>,
    keyboard_moved: bool,
    /// Set by `results_grid`; read (and cleared) at the start of the next frame.
    grid_shown: bool,
    frame_stats: Option<FrameStats>,
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
    #[cfg(test)]
    today_override: Option<chrono::NaiveDate>,
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
        let tl = TimelineState::new(Arc::clone(&lib), ctx);
        let previews = Previews::new(ctx, Arc::clone(&lib), 2);
        let mut app = Self {
            lib,
            search,
            thumbs,
            tl,
            viewer: None,
            viewer_info: true,
            previews,
            query: String::new(),
            query_changed_at: None,
            kind: None,
            date: DateFilter::default(),
            sort: SearchSort::Relevance,
            known_years: Vec::new(),
            years_rx: None,
            years_seen: None,
            fav_marks: HashMap::new(),
            fav_version_seen: 0,
            albums_version_seen: 0,
            new_album: None,
            rename_album: None,
            delete_album: None,
            current: None,
            awaiting: None,
            results: Arc::default(),
            result_label: String::new(),
            result_ms: 0.0,
            result_error: None,
            last_search_at: None,
            selected: None,
            result_sel: Selection::default(),
            selected_t: None,
            grid_cols: 1,
            scroll_to: None,
            keyboard_moved: false,
            grid_shown: false,
            frame_stats: FrameStats::from_env(),
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
            #[cfg(test)]
            today_override: None,
        };
        app.snapshot = Snapshot::fetch(&app.lib);
        // Unit tests fake an installed model with placeholder files; never feed those to
        // ONNX Runtime.
        if app.snapshot.model_installed && !cfg!(test) {
            // Load the text model in the background so the first search is fast.
            let lib = Arc::clone(&app.lib);
            std::thread::spawn(move || lib.embed_text("warm up").ok());
        }
        // Pick up changes made while the app was closed (and read dates even without a model).
        if !app.snapshot.folders.is_empty() {
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
            taken_range: self.date.range(chrono::Datelike::year(&self.today())),
            sort: self.sort,
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
        self.previews.begin_frame(ctx);
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
                    self.results = Arc::new(h);
                    self.result_error = None;
                }
                Err(e) => self.result_error = Some(e),
            }
            // Keep the selection on the same file across refreshes.
            self.selected = prev.and_then(|p| self.results.iter().position(|h| h.path == p));
            self.selected = self
                .result_sel
                .rebase(self.results.iter().map(|h| h.file_id), self.selected);
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
                self.results = Arc::default();
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
        if let Some((_, at)) = &self.notice {
            ctx.request_repaint_after(Duration::from_secs(6).saturating_sub(at.elapsed()));
        }
        self.poll_favorites();
        self.poll_albums();
        self.update_timeline();
        self.poll_years(jobs);
        self.settle_favorites();
        self.sync_selection();
    }

    /// Keeps each grid's selection consistent with its primary item (nothing is selected
    /// without a primary; the primary is always selected).
    fn sync_selection(&mut self) {
        let primary = self.tl.selected.and_then(|i| self.tl.items.get(i));
        self.tl.sel.sync(primary.map(|it| it.file_id));
        let primary = self.selected.and_then(|i| self.results.get(i));
        self.result_sel.sync(primary.map(|h| h.file_id));
    }

    /// Reads the list of years for the "When" selector in the background, again whenever a job
    /// has finished (files were added, removed or dated).
    fn poll_years(&mut self, jobs: u64) {
        if let Some(rx) = &self.years_rx
            && let Ok(years) = rx.try_recv()
        {
            self.known_years = years;
            self.years_rx = None;
        }
        if self.years_rx.is_none() && self.years_seen != Some(jobs) {
            self.years_seen = Some(jobs);
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (lib, ctx) = (Arc::clone(&self.lib), self.ctx.clone());
            std::thread::spawn(move || {
                if let Ok(years) = lib.years() {
                    let _ = tx.send(years);
                    ctx.request_repaint();
                }
            });
            self.years_rx = Some(rx);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Recollect").strong());
            ui.add_space(12.0);
            let width = (ui.available_width() - 300.0).max(200.0);
            let hint = if self.snapshot.model_installed {
                "Describe what you're looking for, e.g. \"kids on the beach at sunset\""
            } else {
                "Download the model to search"
            };
            let resp = ui.add_enabled(
                self.snapshot.model_installed,
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(hint)
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
        if self.searching() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let year = chrono::Datelike::year(&self.today());
                let mut changed = self.date.ui(ui, &self.known_years, year);
                ui.add_space(12.0);
                changed |= when::sort_ui(ui, &mut self.sort);
                if changed {
                    self.rerun();
                }
            });
        }
    }

    fn library_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Library");
        ui.add_space(4.0);
        self.view_selector(ui);
        ui.add_space(6.0);
        self.albums_section(ui);
        ui.add_space(6.0);
        ui.label(RichText::new("Folders").strong());
        let mut remove = None;
        let mut open_folder = None;
        if self.snapshot.folders.is_empty() {
            ui.label(RichText::new("No folders yet.").weak());
        }
        let labels = folder_labels(self.snapshot.folders.iter().map(|f| f.path.as_path()));
        for (f, name) in self.snapshot.folders.iter().zip(labels) {
            ui.horizontal(|ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Remove from library (files are not touched)")
                    .clicked()
                {
                    remove = Some(f.id);
                }
                if ui
                    .selectable_label(self.tl.view == View::Folder(f.id), name)
                    .on_hover_text(f.path.display().to_string())
                    .clicked()
                {
                    open_folder = Some(f.id);
                }
            });
        }
        if let Some(id) = open_folder {
            self.set_view(View::Folder(id));
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
                !busy && !self.snapshot.folders.is_empty(),
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
        let failed = s.errors;
        self.date_navigator(ui);
        if failed > 0 {
            egui::CollapsingHeader::new(format!("{failed} files could not be read"))
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
                    let model = self.snapshot.model_installed;
                    if let Some(r) = &self.status.last_run {
                        ui.label(run_summary_text(r, waiting, model));
                    } else if self.status.last_error.is_some() {
                        ui.label(format!("Stopped. {waiting} files waiting."));
                    } else if !model && waiting > 0 {
                        ui.label(format!("{}.", waiting_for_model_text(waiting)));
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
                Activity::ReadingDates { done, total } => {
                    ui.spinner();
                    ui.label(format!("Reading dates… {done} / {total}"));
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
                Activity::Scanning { .. }
                    | Activity::ReadingDates { .. }
                    | Activity::LoadingModel
                    | Activity::Indexing(_)
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

    /// Shown above the album while the search model is missing; browsing works without it.
    fn model_banner(&mut self, ui: &mut egui::Ui) {
        let spec = self.lib.model();
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        if self.consent_dismissed && !downloading {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Search is off until the model is downloaded.").weak());
                if ui.button("Download model…").clicked() {
                    self.consent_dismissed = false;
                }
            });
            return;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("One-time setup: download the search model");
            ui.label(format!(
                "Recollect needs an AI model that understands both pictures and text.\n\
                 {} — {} MB, license {}, from huggingface.co/{}.",
                spec.name,
                spec.total_size() / 1_000_000,
                spec.license,
                spec.repo
            ));
            ui.label(RichText::new(
                "This is the only time Recollect uses the internet. Your photos and videos never leave this computer.",
            ).weak());
            ui.add_space(4.0);
            if downloading {
                ui.spinner();
            } else {
                ui.horizontal(|ui| {
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
        ui.add_space(8.0);
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
            if ui.button("Clear search").clicked() {
                self.clear_search();
            }
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
        self.selection_bar_ui(ui);
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
        self.grid_cols = cols;
        self.grid_shown = true;
        let mut clicked: Option<(usize, bool, ClickMods)> = None;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(y) = self.scroll_to.take() {
            area = area.vertical_scroll_offset(y);
        }
        // `show_rows` spaces rows by the row height plus the item spacing of this `ui`.
        let stride = tile + GAP + ui.spacing().item_spacing.y;
        let out = area.show_rows(ui, tile + GAP, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for i in row * cols..((row + 1) * cols).min(self.results.len()) {
                        let resp = self.tile(ui, i, tile);
                        if resp.double_clicked() || resp.clicked() {
                            let mods = ClickMods::of_click(ui.ctx());
                            clicked = Some((i, resp.double_clicked() && mods.is_plain(), mods));
                        }
                    }
                });
                ui.add_space(GAP - ui.spacing().item_spacing.y);
            }
        });
        if let Some(i) = self.selected.filter(|_| self.keyboard_moved) {
            // Rows are virtualized, so the selected tile may not exist on screen yet:
            // scroll by row arithmetic instead of asking the tile to scroll itself.
            self.keyboard_moved = false;
            let view = out.inner_rect.height();
            let offset = out.state.clamped_offset().y;
            self.scroll_to = scroll_offset_for_row(i / cols, stride, tile, view, offset);
            if self.scroll_to.is_some() {
                ui.ctx().request_repaint();
            }
        }
        if let Some((i, open, mods)) = clicked {
            let results = Arc::clone(&self.results);
            self.selected = self.result_sel.click(
                self.selected,
                results.len(),
                &|k| results[k].file_id,
                i,
                mods,
            );
            if let Some(p) = self.selected.and_then(|p| results.get(p)) {
                self.selected_t = p.t_start;
            }
            if open {
                self.open_viewer_results(i);
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
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::CLICK);
        if ui.is_rect_visible(rect) {
            let badge =
                (hit.kind == MediaKind::Video).then(|| fmt_time(hit.t_start.unwrap_or(0.0)));
            let selected = self.result_sel.contains(hit.file_id) || self.selected == Some(i);
            timeline::paint_tile(
                ui,
                &mut self.thumbs,
                &key,
                rect,
                timeline::TileMarks {
                    badge,
                    favorite: marked(&self.fav_marks, hit.file_id, hit.favorite),
                    selected,
                    hovered: resp.hovered(),
                },
            );
        }
        let name = hit
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let label = format!("Result {name}");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
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
            let fav = marked(&self.fav_marks, hit.file_id, hit.favorite);
            let label = if fav { "Unfavorite" } else { "Favorite" };
            if ui.button(label).clicked() {
                self.set_favorite(hit.file_id, !fav);
            }
            if let Some(c) = albums::album_menu(ui, &self.snapshot.albums) {
                self.add_to_album_choice(vec![hit.file_id], c);
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
                    self.results = Arc::default();
                    self.end_viewer();
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

/// "168 files will be searchable after the model download".
fn waiting_for_model_text(waiting: usize) -> String {
    let files = if waiting == 1 { "file" } else { "files" };
    format!("{waiting} {files} will be searchable after the model download")
}

fn run_summary_text(r: &crate::library::RunSummary, waiting: usize, model: bool) -> String {
    let mut parts = Vec::new();
    if r.cancelled {
        parts.push(format!("Stopped. {waiting} files waiting"));
    } else if waiting > 0 && !model {
        // Without a model nothing is embedded, so "still waiting" would suggest a problem.
        parts.push(waiting_for_model_text(waiting));
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

/// Direction of a keyboard move through the results grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nav {
    Left,
    Right,
    Up,
    Down,
}

/// Where the selection goes after an arrow key. Moves clamp at the ends of the list;
/// up/down stay put when there is no row above/below. With nothing selected, any arrow
/// picks the first result.
fn step(selected: Option<usize>, len: usize, cols: usize, key: Nav) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(cur) = selected.filter(|&i| i < len) else {
        return Some(0);
    };
    let cols = cols.max(1);
    Some(match key {
        Nav::Left => cur.saturating_sub(1),
        Nav::Right => (cur + 1).min(len - 1),
        Nav::Up if cur >= cols => cur - cols,
        Nav::Down if cur / cols < (len - 1) / cols => (cur + cols).min(len - 1),
        Nav::Up | Nav::Down => cur,
    })
}

/// True once if `key` was pressed without modifiers this frame (the press is consumed).
fn take_plain_key(ctx: &egui::Context, key: egui::Key) -> bool {
    ctx.input_mut(|i| consume_plain_key(i, key)) > 0
}

/// Removes and counts presses of `key` that carry no modifiers at all.
fn consume_plain_key(i: &mut egui::InputState, key: egui::Key) -> usize {
    let mut count = 0;
    i.events.retain(|e| {
        let hit = matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. }
            if *k == key && !modifiers.any());
        count += hit as usize;
        !hit
    });
    count
}

/// Where the scroll offset should go so that `row` is fully visible, or `None` when it
/// already is. `stride` is the distance between row tops, `tile` the row's height.
fn scroll_offset_for_row(
    row: usize,
    stride: f32,
    tile: f32,
    view: f32,
    offset: f32,
) -> Option<f32> {
    let top = row as f32 * stride;
    if top < offset {
        Some(top)
    } else if top + tile > offset + view {
        Some(top + tile - view)
    } else {
        None
    }
}

impl App {
    /// Arrow keys move the selection, Shift+arrows extend it, Ctrl/Cmd+A selects everything and
    /// Enter opens the primary item. Only active when the results grid was drawn last frame, the
    /// Settings window is closed and no text field has keyboard focus, so typing in the search
    /// box is never hijacked. Keys with other modifiers are ignored.
    fn handle_navigation_keys(&mut self, ctx: &egui::Context, grid_was_shown: bool) {
        if !grid_was_shown
            || self.settings_open
            || self.results.is_empty()
            || ctx.text_edit_focused()
        {
            return;
        }
        let input = selection::read_nav_keys(ctx);
        let results = Arc::clone(&self.results);
        let (len, cols) = (results.len(), self.grid_cols);
        let before = self.selected;
        let next = selection::apply_keys(
            &input,
            &mut self.result_sel,
            before,
            len,
            &|i| results[i].file_id,
            |sel, n| step(sel, len, cols, n),
        );
        if next != before {
            self.selected = next;
            self.selected_t = next.and_then(|i| results[i].t_start);
            self.keyboard_moved = true;
        }
        if take_plain_key(ctx, egui::Key::F) {
            if self.result_sel.is_multi() {
                self.toggle_favorite_of_selection();
            } else if let Some(h) = self.selected.and_then(|i| self.results.get(i)) {
                let (id, fav) = (h.file_id, !marked(&self.fav_marks, h.file_id, h.favorite));
                self.set_favorite(id, fav);
            }
        }
        if input.enter
            && let Some(i) = self.selected.filter(|&i| i < self.results.len())
        {
            self.open_viewer_results(i);
        }
    }

    /// F with several items selected: favorite them all, or unfavorite them all if they
    /// already are.
    pub(super) fn toggle_favorite_of_selection(&mut self) {
        let ids = self.selected_file_ids();
        let all_fav = if self.searching() {
            self.results
                .iter()
                .filter(|h| self.result_sel.contains(h.file_id))
                .all(|h| marked(&self.fav_marks, h.file_id, h.favorite))
        } else {
            self.tl
                .items
                .iter()
                .filter(|it| self.tl.sel.contains(it.file_id))
                .all(|it| marked(&self.fav_marks, it.file_id, it.favorite))
        };
        self.set_favorites(&ids, !all_fav);
    }

    /// Marks (or unmarks) a favorite everywhere it is shown right away, and has it stored in
    /// the background. The mark wins over lists loaded from the library until the write is
    /// confirmed; if the write fails, the mark is taken back (see [`Self::poll_favorites`]).
    fn set_favorite(&mut self, file_id: i64, fav: bool) {
        self.set_favorites(&[file_id], fav);
    }

    /// [`Self::set_favorite`] for several files, patching the lists once.
    pub(super) fn set_favorites(&mut self, file_ids: &[i64], fav: bool) {
        for &file_id in file_ids {
            self.fav_marks.insert(
                file_id,
                FavMark {
                    value: fav,
                    confirmed: false,
                },
            );
        }
        self.patch_lists();
        for &file_id in file_ids {
            if let Some(v) = &mut self.viewer {
                v.set_favorite(file_id, fav);
            }
            if self.tl.view == View::Favorites && !fav {
                self.tl.remove_file(file_id);
            }
            self.lib.queue_favorite(file_id, fav);
        }
    }

    /// Writes the marks into the lists. A list the viewer holds a snapshot of is left alone
    /// (copying it just for a flag could mean hundreds of thousands of items); the grids draw
    /// marks over their lists anyway, and the list is patched once the viewer is gone.
    fn patch_lists(&mut self) {
        let marks = &self.fav_marks;
        if marks.is_empty() {
            return;
        }
        if let Some(items) = Arc::get_mut(&mut self.tl.items) {
            for it in items.iter_mut() {
                if let Some(m) = marks.get(&it.file_id) {
                    it.favorite = m.value;
                }
            }
        }
        if let Some(hits) = Arc::get_mut(&mut self.results) {
            for h in hits.iter_mut() {
                if let Some(m) = marks.get(&h.file_id) {
                    h.favorite = m.value;
                }
            }
        }
    }

    /// Per frame: confirmations and failures of favorite writes, and what the writes changed.
    fn poll_favorites(&mut self) {
        for w in self.lib.take_favorite_results() {
            let pending = self
                .fav_marks
                .get(&w.file_id)
                .is_some_and(|m| m.value == w.fav);
            match w.error {
                None => {
                    if pending && let Some(m) = self.fav_marks.get_mut(&w.file_id) {
                        m.confirmed = true;
                    }
                }
                Some(e) => {
                    self.notify(format!("Could not save the favorite mark: {e}"));
                    // A newer mark for the same file is still on its way: it decides.
                    if pending {
                        // The library still has the earlier state.
                        let value = !w.fav;
                        self.fav_marks.insert(
                            w.file_id,
                            FavMark {
                                value,
                                confirmed: true,
                            },
                        );
                        if let Some(v) = &mut self.viewer {
                            v.set_favorite(w.file_id, value);
                        }
                        // An item taken out of the Favorites list has to come back.
                        if self.tl.view == View::Favorites {
                            self.tl.requested = None;
                        }
                        self.patch_lists();
                    }
                }
            }
        }
        let version = self.lib.favorites_version();
        if version != self.fav_version_seen {
            self.fav_version_seen = version;
            if self.tl.view == View::Favorites {
                // What the view lists has changed. (Search results have no favorites filter or
                // order: patching their flags is all they need.)
                self.tl.requested = None;
            }
        }
    }

    /// Drops confirmed marks once nothing loaded earlier can arrive any more and the lists
    /// carry the values themselves.
    fn settle_favorites(&mut self) {
        if !self.fav_marks.values().any(|m| m.confirmed)
            || self.tl.awaiting.is_some()
            || self.awaiting.is_some()
        {
            return;
        }
        self.patch_lists();
        if Arc::strong_count(&self.tl.items) == 1 && Arc::strong_count(&self.results) == 1 {
            self.fav_marks.retain(|_, m| !m.confirmed);
        }
    }

    /// Draws the whole app into `ui` (separate from `eframe::App` for headless UI tests).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(stats) = &mut self.frame_stats {
            stats.frame(self.lib.is_busy());
            // egui only repaints on events; keep frames coming so there is something to time.
            ctx.request_repaint();
        }
        self.update_state(&ctx);
        if self.dialog_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_album_dialogs();
        }
        if self.viewer_open() {
            // The viewer fills the window: no sidebar, top bar or grid while it is open.
            self.grid_shown = false;
            self.tl.shown = false;
            self.show_viewer(ui);
            self.album_dialogs(&ctx);
            return;
        }
        let grid_was_shown = std::mem::take(&mut self.grid_shown);
        let timeline_was_shown = std::mem::take(&mut self.tl.shown);
        self.handle_navigation_keys(&ctx, grid_was_shown);
        self.handle_timeline_keys(&ctx, timeline_was_shown);
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
        let has_folders = !self.snapshot.folders.is_empty();
        if self.searching() {
            if let Some(i) = self.selected.filter(|&i| i < self.results.len()) {
                let multi = self.result_sel.len();
                egui::Panel::right("detail")
                    .resizable(true)
                    .default_size(340.0)
                    .min_size(260.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if multi >= 2 {
                                self.selection_summary(ui, multi);
                            } else {
                                self.detail_panel(ui, i);
                            }
                        });
                    });
            }
        } else if let Some(i) = self.tl.selected.filter(|&i| i < self.tl.items.len())
            && has_folders
        {
            let multi = self.tl.sel.len();
            egui::Panel::right("detail")
                .resizable(true)
                .default_size(340.0)
                .min_size(260.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if multi >= 2 {
                            self.selection_summary(ui, multi);
                        } else {
                            self.timeline_detail(ui, i);
                        }
                    });
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if !self.snapshot.model_installed {
                self.model_banner(ui);
            }
            if !has_folders {
                self.empty_state(ui, "Add a folder to get started", "Use “Add folder…” on the left. Recollect only reads your files; it never changes them.");
            } else if self.searching() {
                self.results_grid(ui);
            } else {
                self.timeline_view(ui);
            }
        });
        if self.settings_open {
            self.settings_window(&ctx);
        }
        self.album_dialogs(&ctx);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.clear_selection();
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

/// Short sidebar names for library folders: the last path component, with as many parent
/// components added as needed to tell folders with the same name apart ("images/dated" and
/// "videos/dated").
fn folder_labels<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Vec<String> {
    let parts: Vec<(&Path, Vec<String>)> = paths
        .into_iter()
        .map(|p| {
            let names = p
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            (p, names)
        })
        .collect();
    let label = |names: &[String], depth: usize, p: &Path| {
        if names.is_empty() {
            p.display().to_string()
        } else {
            names[names.len().saturating_sub(depth)..].join("/")
        }
    };
    let mut depth = vec![1usize; parts.len()];
    loop {
        let labels: Vec<String> = parts
            .iter()
            .zip(&depth)
            .map(|((p, n), &d)| label(n, d, p))
            .collect();
        let mut changed = false;
        for i in 0..parts.len() {
            let clash = (0..parts.len()).any(|j| j != i && labels[j] == labels[i]);
            if clash && depth[i] < parts[i].1.len() {
                depth[i] += 1;
                changed = true;
            }
        }
        if !changed {
            return labels;
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod nav_tests {
    use super::{Nav, scroll_offset_for_row, step};

    #[test]
    fn scroll_offset_keeps_rows_visible() {
        // 100 rows, 200 px tiles, 4 px item spacing, 500 px viewport.
        let (stride, tile, view) = (212.0, 200.0, 500.0);
        assert_eq!(scroll_offset_for_row(0, stride, tile, view, 0.0), None);
        assert_eq!(scroll_offset_for_row(1, stride, tile, view, 0.0), None);
        // Row 2 ends at 624 > 500: scroll so its bottom edge meets the viewport bottom.
        assert_eq!(
            scroll_offset_for_row(2, stride, tile, view, 0.0),
            Some(124.0)
        );
        // Far down the grid the offset uses the real stride.
        let o = scroll_offset_for_row(99, stride, tile, view, 0.0).unwrap();
        assert_eq!(o, 99.0 * stride + tile - view);
        assert_eq!(scroll_offset_for_row(99, stride, tile, view, o), None);
        // Scrolling back up aligns the row's top with the viewport top.
        assert_eq!(
            scroll_offset_for_row(50, stride, tile, view, o),
            Some(50.0 * stride)
        );
        // Already fully visible in the middle of the list.
        assert_eq!(
            scroll_offset_for_row(50, stride, tile, view, 50.0 * stride - 100.0),
            None
        );
    }

    #[test]
    fn folder_names_get_parents_only_when_needed() {
        use std::path::Path;
        let names = |v: &[&str]| super::folder_labels(v.iter().map(Path::new));
        assert_eq!(names(&["/a/photos", "/b/trips"]), ["photos", "trips"]);
        assert_eq!(
            names(&["/lib/images/dated", "/lib/videos/dated", "/lib/other"]),
            ["images/dated", "videos/dated", "other"]
        );
        // Needs two parents to tell apart; the unrelated folder stays short.
        assert_eq!(
            names(&["/x/a/b/pics", "/x/c/b/pics", "/y/misc"]),
            ["a/b/pics", "c/b/pics", "misc"]
        );
        assert_eq!(names(&["/"]), ["/"]);
        assert_eq!(names(&[]), Vec::<String>::new());
    }

    #[test]
    fn steps_through_the_grid() {
        assert_eq!(step(None, 0, 4, Nav::Right), None);
        assert_eq!(step(None, 5, 4, Nav::Down), Some(0));
        assert_eq!(step(Some(0), 5, 4, Nav::Left), Some(0));
        assert_eq!(step(Some(4), 5, 4, Nav::Right), Some(4));
        assert_eq!(step(Some(1), 5, 4, Nav::Right), Some(2));
        assert_eq!(step(Some(2), 5, 4, Nav::Up), Some(2));
        assert_eq!(step(Some(5), 9, 4, Nav::Up), Some(1));
        // Down from a full row into a short last row clamps to its final tile.
        assert_eq!(step(Some(3), 6, 4, Nav::Down), Some(5));
        // No row below: stay.
        assert_eq!(step(Some(5), 6, 4, Nav::Down), Some(5));
        assert_eq!(step(Some(0), 1, 4, Nav::Down), Some(0));
        assert_eq!(step(Some(9), 3, 0, Nav::Down), Some(0));
    }
}
