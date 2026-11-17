//! The egui desktop app. All work happens on background threads; this module only renders
//! state and sends commands to the [`Library`].
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "layout and display math on small floats; saturating conversion is intended"
)]

mod albums;
mod bg;
mod chrome;
mod dialogs;
mod favorites;
mod folders;
mod frame_stats;
mod grid;
mod guide;
mod help;
mod notices;
mod photo_size;
mod platform;
mod preview;
mod results;
mod ruler;
mod selection;
mod selection_bar;
mod settings;
mod sidebar;
mod snapshot;
mod style;
mod texture_loader;
mod tiles;
mod timeline;
mod toast;
mod viewer;
mod welcome;
mod when;
mod workers;

use crate::Library;
use crate::config::Settings;
use crate::library::{Activity, Status};
use crate::media::MediaKind;
use crate::search::SearchSort;
use crate::store::db::AlbumInfo;
use bg::BgRead;
use dialogs::Dialogs;
use egui::RichText;
use favorites::{FavMark, marked};
use frame_stats::FrameStats;
use grid::{
    ClickOutcome, GridEvents, GridState, Nav, consume_plain_key, scroll_offset_for_row, step,
};
use notices::{NoticeAction, Notices};
use preview::Previews;
use results::ResultsState;
use snapshot::Snapshot;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use style::PANEL_MARGIN;
use timeline::TimelineState;
use viewer::Viewer;
use when::DateFilter;
pub use workers::Query;
use workers::{SearchWorker, ThumbCache};

/// Height of the top bar.
const TOP_BAR_HEIGHT: f32 = 52.0;
/// How often to look at the library while it is busy.
const BUSY_POLL: Duration = Duration::from_millis(500);
/// How old the results may get while files are being indexed before they are refreshed.
const RESULTS_REFRESH: Duration = Duration::from_secs(3);
/// How often old thumbnails are pruned.
const PRUNE_EVERY: Duration = Duration::from_secs(600);

pub fn run(lib: Arc<Library>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Recollect")
            .with_app_id("recollect")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Recollect",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, lib)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}

/// The window icon (the app mark on a rounded square).
fn window_icon() -> egui::IconData {
    let png = include_bytes!("../../assets/icon/recollect-256.png");
    match image::load_from_memory(png) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            egui::IconData {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            }
        }
        Err(_) => egui::IconData::default(),
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
    years_read: BgRead<Vec<i32>>,
    /// `jobs_finished` as of the last years request.
    years_seen: Option<u64>,
    /// Favorite marks waiting for the writer or for lists to catch up.
    fav_marks: HashMap<i64, FavMark>,
    fav_version_seen: u64,
    albums_version_seen: u64,
    /// The albums with counts and covers; read in the background when the library's album
    /// version moves or a job finishes (not part of the periodic snapshot).
    albums: Vec<AlbumInfo>,
    albums_read: BgRead<anyhow::Result<Vec<AlbumInfo>>>,
    /// The search and its results; shown instead of the timeline while a search is active.
    res: ResultsState,
    frame_stats: Option<FrameStats>,
    dialogs: Dialogs,
    /// The first-run welcome flow, while it is still to be shown.
    welcome: Option<welcome::Welcome>,
    /// Search help (examples and recent searches) under the search field.
    search_help: guide::SearchHelp,
    /// `Status::error_seq` of the last error that was shown as a toast.
    error_seq_seen: u64,
    /// The last finished run that was reported (see `Status::run_seq`).
    run_seq_seen: u64,
    notices: Notices,
    /// Tickets of album writes that undo something: they are not confirmed again.
    silent_album_writes: HashSet<u64>,
    /// Bottom centre of the grid area, where the selection bar and confirmations float
    /// (set while a grid is drawn, used once per frame).
    pill_anchor: Option<egui::Pos2>,
    /// Where the selection bar was drawn this frame, if it was (a toast then floats above it).
    bar_anchor: Option<egui::Pos2>,
    snapshot: Snapshot,
    snapshot_read: BgRead<Snapshot>,
    /// Folders that cannot be read now (an unplugged drive), found by a background check that
    /// runs when the folders change and then every half minute.
    missing_folders: Vec<i64>,
    folder_check: BgRead<Vec<i64>>,
    /// The folders the last check was started for, and when.
    folders_checked: Option<(Vec<i64>, Instant)>,
    /// When the photo size last changed and whether it is saved.
    size_change: photo_size::SizeChange,
    ctx: egui::Context,
    status: Status,
    was_busy: bool,
    jobs_seen: u64,
    last_prune: Instant,
    /// A copy of the library's settings, refreshed when they change.
    settings: Settings,
    settings_seen: u64,
    /// Settings > Library lists the files that couldn't be opened.
    failed_open: bool,
    #[cfg(test)]
    today_override: Option<chrono::NaiveDate>,
}

/// Whether the timed refresh of the results should start a new search: the last answer
/// arrived a while ago and no search is awaited. Refreshing while an answer is still awaited
/// would drop that answer every time, so a slow search would never show anything; counting
/// from the answer (not from when the search was asked) leaves a slow search's time to the
/// indexer instead of running the next one right behind it.
fn results_refresh_due(last_answer_at: Option<Instant>, awaiting: Option<u64>) -> bool {
    awaiting.is_none() && last_answer_at.is_some_and(|t| t.elapsed() > RESULTS_REFRESH)
}

/// The background workers and readers the app starts with.
struct Workers {
    search: SearchWorker,
    thumbs: ThumbCache,
    tl: TimelineState,
    previews: Previews,
    years_read: BgRead<Vec<i32>>,
    albums_read: BgRead<anyhow::Result<Vec<AlbumInfo>>>,
    snapshot_read: BgRead<Snapshot>,
    folder_check: BgRead<Vec<i64>>,
}

impl Workers {
    fn start(ctx: &egui::Context, lib: &Arc<Library>) -> Self {
        let c = ctx.clone();
        lib.set_notifier(move || c.request_repaint());
        let c = ctx.clone();
        let search = SearchWorker::new(Arc::clone(lib), move || c.request_repaint());
        let thumbs = ThumbCache::new(ctx, &lib.paths().thumbs_dir(), 3);
        prune_thumbnails(lib);
        let tl = TimelineState::new(Arc::clone(lib), ctx);
        let previews = Previews::new(ctx, Arc::clone(lib), 2);
        let years_read = BgRead::new(ctx, "years", {
            let lib = Arc::clone(lib);
            move || lib.years().ok()
        });
        let albums_read = BgRead::new(ctx, "albums", {
            let lib = Arc::clone(lib);
            move || Some(lib.albums())
        });
        let snapshot_read = BgRead::new(ctx, "snapshot", {
            let lib = Arc::clone(lib);
            move || Some(Snapshot::fetch(&lib))
        });
        let folder_check = BgRead::new(ctx, "folder-check", {
            let lib = Arc::clone(lib);
            move || snapshot::missing_folders(&lib)
        });
        Self {
            search,
            thumbs,
            tl,
            previews,
            years_read,
            albums_read,
            snapshot_read,
            folder_check,
        }
    }
}

impl App {
    pub fn new(ctx: &egui::Context, lib: Arc<Library>) -> Self {
        let workers = Workers::start(ctx, &lib);
        let mut app = Self::blank(ctx, lib, workers);
        app.load_initial_state(ctx);
        app
    }

    /// The state before anything is read from the library.
    fn blank(ctx: &egui::Context, lib: Arc<Library>, w: Workers) -> Self {
        Self {
            lib,
            search: w.search,
            thumbs: w.thumbs,
            tl: w.tl,
            viewer: None,
            viewer_info: true,
            previews: w.previews,
            query: String::new(),
            query_changed_at: None,
            kind: None,
            date: DateFilter::default(),
            sort: SearchSort::Relevance,
            known_years: Vec::new(),
            years_read: w.years_read,
            years_seen: None,
            fav_marks: HashMap::new(),
            fav_version_seen: 0,
            albums_version_seen: 0,
            albums: Vec::new(),
            albums_read: w.albums_read,
            res: ResultsState::default(),
            frame_stats: FrameStats::from_env(),
            dialogs: Dialogs::default(),
            welcome: None,
            search_help: guide::SearchHelp::default(),
            error_seq_seen: 0,
            run_seq_seen: 0,
            notices: Notices::default(),
            silent_album_writes: HashSet::new(),
            pill_anchor: None,
            bar_anchor: None,
            snapshot: Snapshot::default(),
            snapshot_read: w.snapshot_read,
            missing_folders: Vec::new(),
            folder_check: w.folder_check,
            folders_checked: None,
            size_change: photo_size::SizeChange::default(),
            ctx: ctx.clone(),
            status: Status::default(),
            was_busy: false,
            jobs_seen: 0,
            last_prune: Instant::now(),
            settings: Settings::default(),
            settings_seen: 0,
            failed_open: false,
            #[cfg(test)]
            today_override: None,
        }
    }

    /// Reads settings, folders and albums, decides on the welcome flow and starts the
    /// background work that follows from them.
    fn load_initial_state(&mut self, ctx: &egui::Context) {
        self.settings = self.lib.settings();
        self.settings_seen = self.lib.settings_version();
        self.tl.tile_size = crate::config::clamp_photo_size(self.settings.photo_size);
        style::apply(ctx, self.settings.theme, self.settings.text_size);
        self.snapshot = Snapshot::fetch(&self.lib);
        if !self.settings.welcome_done {
            if self.snapshot.folders.is_empty() {
                self.welcome = Some(welcome::Welcome::new(self.lib.model().id));
            } else {
                // Someone who already uses the app never sees the welcome.
                self.change_settings(|s| s.welcome_done = true);
            }
        }
        self.albums = self.lib.albums().unwrap_or_default();
        self.albums_version_seen = self.lib.albums_version();
        // Unit tests fake an installed model with placeholder files; never feed those to
        // ONNX Runtime.
        if self.snapshot.model_installed && !cfg!(test) {
            // Load the text model in the background so the first search is fast.
            let lib = Arc::clone(&self.lib);
            std::thread::spawn(move || lib.embed_text("warm up").ok());
        }
        // Pick up changes made while the app was closed (and read dates even without a model).
        if !self.snapshot.folders.is_empty() {
            self.start_indexing();
        }
    }

    /// Looks for new photos, or as soon as the running task ends; says so when it has to wait.
    fn start_indexing(&mut self) {
        match self.lib.request_scan() {
            Ok(crate::library::Request::Started) => {}
            Ok(crate::library::Request::Queued) => self
                .notify("Recollect will check for new photos when it finishes the current task."),
            Err(e) => self.notify_problem(
                "Couldn't look for new photos just now. Please try again in a moment.",
                format!("{e:#}"),
                Some(NoticeAction::Scan),
            ),
        }
    }

    /// Per-frame bookkeeping: results, status transitions, debounce, live refresh.
    fn update_state(&mut self, ctx: &egui::Context) {
        self.thumbs.begin_frame(ctx);
        self.previews.begin_frame(ctx);
        self.poll_snapshot();
        self.poll_folder_check(ctx);
        self.status = self.lib.status();
        self.poll_search();
        self.poll_debounce(ctx);
        let jobs = self.poll_library(ctx);
        self.report_status_and_notices(ctx);
        self.poll_favorites();
        self.poll_albums();
        self.update_timeline();
        self.poll_years(jobs);
        self.settle_favorites();
        self.sync_selection();
    }

    /// Follows what the library is doing: live refreshes while it works, and a full reload
    /// when a job or run ends. Returns the number of jobs finished so far.
    fn poll_library(&mut self, ctx: &egui::Context) -> u64 {
        let busy = self.lib.is_busy();
        if busy {
            self.refresh_snapshot(false);
            ctx.request_repaint_after(BUSY_POLL);
            // Results grow while indexing: refresh them every few seconds.
            if matches!(self.status.activity, Activity::Indexing(_))
                && results_refresh_due(self.res.last_answer_at, self.res.awaiting)
            {
                self.rerun();
            }
        }
        let jobs = self.lib.jobs_finished();
        let run_ended = self.was_busy && !busy;
        self.was_busy = busy;
        if run_ended || jobs != self.jobs_seen {
            self.jobs_seen = jobs;
            self.refresh_snapshot(true);
            self.refresh_albums();
            self.rerun();
        }
        if self.last_prune.elapsed() > PRUNE_EVERY {
            self.last_prune = Instant::now();
            prune_thumbnails(&self.lib);
        }
        jobs
    }

    /// Keeps each grid's selection consistent with its primary item (nothing is selected
    /// without a primary; the primary is always selected).
    fn sync_selection(&mut self) {
        let primary = self.tl.grid.selected.and_then(|i| self.tl.items.get(i));
        self.tl.grid.sync(primary.map(|it| it.file_id));
        let primary = self.res.grid.selected.and_then(|i| self.res.hits.get(i));
        self.res.grid.sync(primary.map(|h| h.file_id));
    }

    /// Reads the list of years for the "When" selector in the background, again whenever a job
    /// has finished (files were added, removed or dated).
    fn poll_years(&mut self, jobs: u64) {
        if let Some(years) = self.years_read.poll() {
            self.known_years = years;
        }
        // A failed lookup leaves nothing pending: the next change retries.
        if !self.years_read.is_pending() && self.years_seen != Some(jobs) {
            self.years_seen = Some(jobs);
            self.years_read.request();
        }
    }

    /// Picks up settings changes (a counter read, copying only on a change) and applies the
    /// style. Applying is a no-op unless the appearance changed or this is the first frame of
    /// the egui context. Returns true when the style changed.
    fn refresh_settings(&mut self, ctx: &egui::Context) -> bool {
        let version = self.lib.settings_version();
        if version != self.settings_seen {
            self.settings_seen = version;
            self.settings = self.lib.settings();
        }
        style::apply(ctx, self.settings.theme, self.settings.text_size)
    }

    /// Draws the whole app into `ui` (separate from `eframe::App` for headless UI tests).
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.begin_frame(ui, &ctx);
        self.update_state(&ctx);
        self.close_topmost_on_escape(&ctx);
        if self.viewer_open() {
            // The viewer fills the window: no sidebar, top bar or grid while it is open.
            self.res.grid.shown = false;
            self.tl.grid.shown = false;
            self.show_viewer(ui);
            self.album_dialogs(&ctx);
            self.toasts(&ctx);
            return;
        }
        if self.welcome_active() {
            self.welcome_screen(ui);
            self.toasts(&ctx);
            return;
        }
        let grid_was_shown = std::mem::take(&mut self.res.grid.shown);
        let timeline_was_shown = std::mem::take(&mut self.tl.grid.shown);
        self.handle_navigation_keys(&ctx, grid_was_shown);
        self.handle_timeline_keys(&ctx, timeline_was_shown);
        self.handle_photo_size(&ctx, grid_was_shown || timeline_was_shown);
        self.show_panels(ui, &ctx);
        self.show_overlays(&ctx);
        if !self.keys_blocked(&ctx) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.clear_selection();
        }
    }

    /// Frame statistics and the style for this frame.
    fn begin_frame(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(stats) = &mut self.frame_stats {
            stats.frame(self.lib.is_busy());
            // egui only repaints on events; keep frames coming so there is something to time.
            ctx.request_repaint();
        }
        if self.refresh_settings(ctx) {
            // This frame's root ui was created with the old style.
            ui.set_style(ctx.global_style());
        }
    }

    /// Escape closes the topmost thing first: a question, otherwise the Settings sheet.
    fn close_topmost_on_escape(&mut self, ctx: &egui::Context) {
        if (self.dialog_open() || self.dialogs.settings_open)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.dialog_open() {
                self.dialogs.cancel();
            } else {
                self.dialogs.settings_open = false;
            }
        }
    }

    /// The top bar, the sidebar and the central panel.
    fn show_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = style::Palette::of(ctx);
        // The sidebar runs the full height; the top bar sits above the content only.
        self.sidebar(ui);
        egui::Panel::top("top")
            .exact_size(TOP_BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .show(ui, |ui| self.top_bar(ui));
        self.ruler_panel(ui);
        let has_folders = !self.snapshot.folders.is_empty();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(PANEL_MARGIN.0, PANEL_MARGIN.1)),
            )
            .show(ui, |ui| {
                if has_folders {
                    self.search_banner(ui);
                    if self.searching() {
                        self.results_grid(ui);
                    } else {
                        self.timeline_view(ui);
                    }
                } else {
                    self.no_folders_state(ui);
                }
            });
    }

    /// Sheets, dialogs, the search help and the toasts, drawn over the panels.
    fn show_overlays(&mut self, ctx: &egui::Context) {
        if self.dialogs.settings_open {
            self.settings_sheet(ctx);
        }
        self.model_confirm_dialog(ctx);
        if self.dialogs.help_open {
            self.help_sheet(ctx);
        }
        self.search_help_dropdown(ctx);
        self.album_dialogs(ctx);
        self.folder_dialogs(ctx);
        self.photo_size_chip(ctx);
        self.toasts(ctx);
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

/// A time in seconds as whole milliseconds; a float beyond the integer range saturates.
#[expect(
    clippy::cast_possible_truncation,
    reason = "saturating float-to-integer conversion is the intent"
)]
pub(super) fn to_millis(seconds: f64) -> i64 {
    (seconds * 1000.0) as i64
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

impl Drop for App {
    fn drop(&mut self) {
        // A photo size changed a moment ago is kept.
        if self.size_change.is_unsaved() {
            let size = self.tl.tile_size;
            let _ = self.lib.update_settings(|s| s.photo_size = size);
        }
        // Closing the window stops background work cleanly (the database stays consistent
        // either way, but this avoids killing a write mid-transaction).
        // Writes still queued (favorites, album changes) would be lost with the library, so
        // let them finish first; bounded in case the database is stuck.
        self.lib.wait_writes_for(Duration::from_secs(3));
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
mod label_tests {
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
}
