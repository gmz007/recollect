//! The egui desktop app. All work happens on background threads; this module only renders
//! state and sends commands to the [`Library`].

mod albums;
mod chrome;
mod frame_stats;
mod guide;
mod help;
mod platform;
mod preview;
mod selection;
mod settings;
mod sidebar;
mod style;
mod tiles;
mod timeline;
mod viewer;
mod welcome;
mod when;
mod workers;

use crate::Library;
use crate::config::Settings;
use crate::library::{Activity, Status};
use crate::media::MediaKind;
use crate::search::{SearchHit, SearchOptions, SearchSort};
use crate::store::db::{AlbumInfo, Folder, Stats};
use crate::thumbs::ThumbKey;
use egui::{RichText, Vec2};
use frame_stats::FrameStats;
use preview::Previews;
use selection::{ClickMods, Selection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiles::Tile;
use timeline::{TimelineState, View};
use viewer::Viewer;
use when::DateFilter;
pub use workers::Query;
use workers::{SearchWorker, ThumbCache};

const TILE: f32 = 180.0;
const GAP: f32 = 4.0;
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

/// Short messages shown as toasts. An error stays visible until the next error or
/// [`Notices::ERROR_FOR`]; later plain messages do not push it out (they show once it is gone).
#[derive(Default)]
struct Notices {
    items: Vec<Notice>,
}

struct Notice {
    text: String,
    at: Instant,
    error: bool,
    /// Technical text, shown only when the user unfolds "Details".
    details: Option<String>,
    /// A button that tries the failed thing again.
    action: Option<NoticeAction>,
}

/// What the "Try again" button of an error does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeAction {
    Download,
    Scan,
}

impl Notices {
    const INFO_FOR: Duration = Duration::from_secs(5);
    const ERROR_FOR: Duration = Duration::from_secs(10);
    /// More than this many pending messages are never useful.
    const MAX: usize = 6;

    fn ttl(n: &Notice) -> Duration {
        if n.error {
            Self::ERROR_FOR
        } else {
            Self::INFO_FOR
        }
    }

    fn push(&mut self, text: String, error: bool, now: Instant) {
        self.push_with(text, error, now, None, None);
    }

    fn push_with(
        &mut self,
        text: String,
        error: bool,
        now: Instant,
        details: Option<String>,
        action: Option<NoticeAction>,
    ) {
        self.expire(now);
        if error {
            // A newer error replaces the older ones.
            self.items.retain(|n| !n.error);
        }
        self.items.push(Notice {
            text,
            at: now,
            error,
            details,
            action,
        });
        while self.items.len() > Self::MAX {
            self.items.remove(0);
        }
    }

    fn expire(&mut self, now: Instant) {
        self.items
            .retain(|n| now.saturating_duration_since(n.at) < Self::ttl(n));
    }

    /// What to show: the error if there is one, else the latest message.
    #[cfg(test)]
    fn current(&self) -> Option<&str> {
        self.items
            .iter()
            .find(|n| n.error)
            .or(self.items.last())
            .map(|n| n.text.as_str())
    }

    /// The notice [`Notices::current`] shows.
    fn current_notice(&self) -> Option<&Notice> {
        self.items.iter().find(|n| n.error).or(self.items.last())
    }

    /// Time until the next message disappears (to schedule a repaint).
    fn next_expiry(&self, now: Instant) -> Option<Duration> {
        self.items
            .iter()
            .map(|n| Self::ttl(n).saturating_sub(now.saturating_duration_since(n.at)))
            .min()
    }
}

/// Cached library facts, refreshed a few times per second rather than every frame.
#[derive(Default)]
struct Snapshot {
    stats: Stats,
    folders: Vec<Folder>,
    errors: Vec<(PathBuf, String)>,
    model_installed: bool,
    /// Ids of the smart-search choices whose files are on this computer.
    installed_models: Vec<&'static str>,
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
            installed_models: crate::model::registry::MODELS
                .iter()
                .filter(|m| crate::model::download::is_installed(&lib.paths().models_dir(), m))
                .map(|m| m.id)
                .collect(),
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
    /// The albums with counts and covers; read in the background when the library's album
    /// version moves or a job finishes (not part of the periodic snapshot).
    albums: Vec<AlbumInfo>,
    albums_rx: Option<crossbeam_channel::Receiver<anyhow::Result<Vec<AlbumInfo>>>>,
    albums_again: bool,
    /// A name being typed for a new album, a rename and a pending deletion.
    new_album: Option<albums::NewAlbum>,
    rename_album: Option<albums::RenameAlbum>,
    delete_album: Option<albums::DeleteAlbum>,
    current: Option<Query>,
    awaiting: Option<u64>,
    /// Shared with the viewer while it is open.
    results: Arc<Vec<SearchHit>>,
    result_label: String,
    result_error: Option<guide::SearchProblem>,
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
    /// Selection mode: the user picked items with a circle or a modifier key, so plain clicks
    /// toggle items instead of opening them and the selection bar is shown.
    picking: bool,
    /// Selection mode of the results grid (the timeline has its own, `picking`).
    result_picking: bool,
    add_folder_dialog: Option<sidebar::AddFolder>,
    stop_folder: Option<sidebar::StopFolder>,
    help_open: bool,
    help_topic: help::HelpTopic,
    /// The first-run welcome flow, while it is still to be shown.
    welcome: Option<welcome::Welcome>,
    /// Search help (examples and recent searches) under the search field.
    search_help: guide::SearchHelp,
    /// `Status::error_seq` of the last error that was shown as a toast.
    error_seq_seen: u64,
    notices: Notices,
    snapshot: Snapshot,
    snapshot_rx: Option<crossbeam_channel::Receiver<Snapshot>>,
    snapshot_again: bool,
    ctx: egui::Context,
    status: Status,
    was_busy: bool,
    jobs_seen: u64,
    last_prune: Instant,
    /// A copy of the library's settings, refreshed when they change.
    settings: Settings,
    settings_seen: u64,
    settings_open: bool,
    settings_section: settings::Section,
    /// A smart-search choice waiting for the user to confirm the switch.
    model_confirm: Option<&'static str>,
    /// Settings > Library lists the files that couldn't be opened.
    failed_open: bool,
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
            albums: Vec::new(),
            albums_rx: None,
            albums_again: false,
            new_album: None,
            rename_album: None,
            delete_album: None,
            current: None,
            awaiting: None,
            results: Arc::default(),
            result_label: String::new(),
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
            picking: false,
            result_picking: false,
            add_folder_dialog: None,
            stop_folder: None,
            help_open: false,
            help_topic: help::HelpTopic::default(),
            welcome: None,
            search_help: guide::SearchHelp::default(),
            error_seq_seen: 0,
            notices: Notices::default(),
            snapshot: Snapshot::default(),
            snapshot_rx: None,
            snapshot_again: false,
            ctx: ctx.clone(),
            status: Status::default(),
            was_busy: false,
            jobs_seen: 0,
            last_prune: Instant::now(),
            settings: Settings::default(),
            settings_seen: 0,
            settings_open: false,
            settings_section: settings::Section::default(),
            model_confirm: None,
            failed_open: false,
            #[cfg(test)]
            today_override: None,
        };
        app.settings = app.lib.settings();
        app.settings_seen = app.lib.settings_version();
        style::apply(ctx, app.settings.theme, app.settings.text_size);
        app.snapshot = Snapshot::fetch(&app.lib);
        if !app.settings.welcome_done {
            if app.snapshot.folders.is_empty() {
                app.welcome = Some(welcome::Welcome::new(app.lib.model().id));
            } else {
                // Someone who already uses the app never sees the welcome.
                app.change_settings(|s| s.welcome_done = true);
            }
        }
        app.albums = app.lib.albums().unwrap_or_default();
        app.albums_version_seen = app.lib.albums_version();
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
        self.notices.push(msg.into(), false, Instant::now());
    }

    /// Like [`Self::notify`] for failures: stays visible longer and is not pushed out by the
    /// messages that follow.
    fn notify_error(&mut self, msg: impl Into<String>) {
        self.notices.push(msg.into(), true, Instant::now());
    }

    /// An error in two parts: what happened and what to do (`msg`), and the technical text
    /// that goes under "Details". `action` adds a "Try again" button.
    fn notify_problem(
        &mut self,
        msg: impl Into<String>,
        details: impl ToString,
        action: Option<NoticeAction>,
    ) {
        let details = details.to_string();
        self.notices.push_with(
            msg.into(),
            true,
            Instant::now(),
            (!details.is_empty()).then_some(details),
            action,
        );
    }

    /// Runs what a toast's "Try again" button stands for.
    fn retry(&mut self, action: NoticeAction) {
        match action {
            NoticeAction::Download => self.turn_on_search(),
            NoticeAction::Scan => self.start_indexing(),
        }
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

    fn submit(&mut self, q: Query) {
        let opts = SearchOptions {
            limit: 300,
            kind: self.kind,
            taken_range: self.date.range(chrono::Datelike::year(&self.today())),
            sort: self.sort,
        };
        if self.current.is_none() {
            // Starting a search leaves the timeline's selection behind.
            self.tl.sel.clear();
            self.tl.selected = None;
            self.picking = false;
        }
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
                Err(e) => {
                    tracing::warn!("search failed: {}", e.detail);
                    self.result_error = Some(guide::search_problem(e.kind));
                }
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
                // A search the person asked for (Enter, or a pause in typing): worth
                // remembering. Leaving the field or pressing Escape never gets here.
                self.remember_search(&q);
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
        let run_ended = self.was_busy && !busy;
        self.was_busy = busy;
        if run_ended || jobs != self.jobs_seen {
            self.jobs_seen = jobs;
            self.refresh_snapshot(true);
            self.refresh_albums();
            self.rerun();
        }
        if self.last_prune.elapsed() > Duration::from_secs(600) {
            self.last_prune = Instant::now();
            prune_thumbnails(&self.lib);
        }
        self.report_status(run_ended);
        let now = Instant::now();
        self.notices.expire(now);
        if let Some(left) = self.notices.next_expiry(now) {
            ctx.request_repaint_after(left);
        }
        self.poll_favorites();
        self.poll_albums();
        self.update_timeline();
        self.poll_years(jobs);
        self.settle_favorites();
        self.sync_selection();
    }

    /// Turns what the library reports into toasts: each failure once (also when the same
    /// message comes again), and folders that could not be reached when a run has just ended.
    fn report_status(&mut self, run_ended: bool) {
        if self.status.error_seq != self.error_seq_seen {
            self.error_seq_seen = self.status.error_seq;
            if let Some(e) = self.status.last_error.clone() {
                let (text, action) = guide::describe_failure(self.status.last_error_kind);
                self.notify_problem(text, e, action);
            }
        }
        if run_ended
            && self
                .status
                .last_run
                .as_ref()
                .is_some_and(|r| !r.skipped_folders.is_empty())
        {
            self.notify("Some folders couldn't be reached, so they were left as they were.");
        }
    }

    /// Keeps each grid's selection consistent with its primary item (nothing is selected
    /// without a primary; the primary is always selected).
    fn sync_selection(&mut self) {
        let primary = self.tl.selected.and_then(|i| self.tl.items.get(i));
        self.tl.sel.sync(primary.map(|it| it.file_id));
        let primary = self.selected.and_then(|i| self.results.get(i));
        self.result_sel.sync(primary.map(|h| h.file_id));
        if self.tl.sel.is_multi() {
            self.picking = true;
        } else if self.tl.sel.is_empty() {
            self.picking = false;
        }
        if self.result_sel.is_multi() {
            self.result_picking = true;
        } else if self.result_sel.is_empty() {
            self.result_picking = false;
        }
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

    fn results_grid(&mut self, ui: &mut egui::Ui) {
        if !self.selection_bar_ui(ui) {
            self.results_header(ui);
        }
        if self.results.is_empty() {
            self.results_empty_state(ui);
            return;
        }
        let avail = ui.available_width() - 12.0; // leave room for the scroll bar
        let cols = (((avail + GAP) / (TILE + GAP)).floor() as usize).max(1);
        let tile = ((avail - GAP * (cols - 1) as f32) / cols as f32).max(TILE * 0.6);
        let rows = self.results.len().div_ceil(cols);
        self.grid_cols = cols;
        self.grid_shown = true;
        let stride = tile + GAP;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("results")
            .auto_shrink([false, false]);
        if let Some(y) = self.scroll_to.take() {
            area = area.vertical_scroll_offset(y);
        }
        let results = Arc::clone(&self.results);
        let mut event: Option<(usize, tiles::TileEvent, ClickMods)> = None;
        let mut menu = None;
        let mut right_clicked = None;
        let out = area.show_viewport(ui, |ui, viewport| {
            let (origin, _) = ui
                .allocate_exact_size(Vec2::new(avail, rows as f32 * stride), egui::Sense::hover());
            let first = (viewport.min.y / stride).floor().max(0.0) as usize;
            let last = ((viewport.max.y / stride).ceil() as usize + 1).min(rows);
            for row in first..last {
                for i in row * cols..((row + 1) * cols).min(results.len()) {
                    let hit = &results[i];
                    let x = origin.min.x + (i - row * cols) as f32 * (tile + GAP);
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(x, origin.min.y + row as f32 * stride),
                        Vec2::splat(tile),
                    );
                    let key = Self::thumb_key(hit, None);
                    let name = hit
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let in_sel = self.result_sel.contains(hit.file_id);
                    let checked = in_sel && (self.result_picking || self.result_sel.is_multi());
                    let t = Tile {
                        key: &key,
                        name: &name,
                        role: "Result",
                        badge: (hit.kind == MediaKind::Video)
                            .then(|| format!("at {}", fmt_time(hit.t_start.unwrap_or(0.0)))),
                        favorite: marked(&self.fav_marks, hit.file_id, hit.favorite),
                        checked,
                        focused: self.selected == Some(i) && !checked,
                        picking: self.result_picking,
                        selectable: true,
                        selected: in_sel || self.selected == Some(i),
                        tip: &|| name.clone(),
                    };
                    let id = ui.make_persistent_id(("result", i));
                    let o = tiles::show(ui, &mut self.thumbs, id, rect, &t);
                    if let Some((ev, mods)) = o.event {
                        event = Some((i, ev, mods));
                    }
                    if o.resp.secondary_clicked() {
                        right_clicked = Some(i);
                    }
                    o.resp.context_menu(|ui| {
                        menu = albums::tile_menu(ui, &self.albums, false).map(|m| (i, m))
                    });
                }
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
        if let Some(i) = right_clicked {
            self.select_for_menu(i);
        }
        if let Some((i, m)) = menu {
            self.apply_tile_menu(i, m);
        }
        if let Some((i, ev, mods)) = event {
            match tiles::click_plan(ev, mods, self.result_picking) {
                tiles::ClickPlan::Favorite => {
                    let h = &results[i];
                    let fav = !marked(&self.fav_marks, h.file_id, h.favorite);
                    self.set_favorite(h.file_id, fav);
                }
                tiles::ClickPlan::Select { mods, fresh } => {
                    if fresh {
                        self.result_sel.clear();
                        self.selected = None;
                    }
                    self.selected = self.result_sel.click(
                        self.selected,
                        results.len(),
                        &|k| results[k].file_id,
                        i,
                        mods,
                    );
                    self.result_picking = !self.result_sel.is_empty();
                    self.selected_t = self.selected.and_then(|p| results[p].t_start);
                }
                tiles::ClickPlan::Open => {
                    self.selected = self.result_sel.click(
                        self.selected,
                        results.len(),
                        &|k| results[k].file_id,
                        i,
                        ClickMods::default(),
                    );
                    self.selected_t = results[i].t_start;
                    self.open_viewer_results(i);
                }
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
    /// Settings window and the album dialogs are closed and no text field has keyboard focus, so typing in the search
    /// box is never hijacked. Keys with other modifiers are ignored.
    fn handle_navigation_keys(&mut self, ctx: &egui::Context, grid_was_shown: bool) {
        if !grid_was_shown || self.results.is_empty() || self.keys_blocked(ctx) {
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
            self.lib.queue_favorite(file_id, fav);
        }
        if self.tl.view == View::Favorites && !fav {
            self.tl
                .remove_files(&file_ids.iter().copied().collect::<HashSet<i64>>());
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
    /// Failures are handled as one batch: one notice and one pass over the lists.
    fn poll_favorites(&mut self) {
        let mut failed = 0;
        let mut first_error = String::new();
        let mut reverted = false;
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
                    failed += 1;
                    if first_error.is_empty() {
                        first_error = e;
                    }
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
                        reverted = true;
                    }
                }
            }
        }
        if failed == 1 {
            tracing::warn!("could not save a favorite mark: {first_error}");
            self.notify_error("Couldn't save your favorite. Try again in a moment.");
        } else if failed > 1 {
            self.notify_error(format!(
                "Couldn't save favorites for {failed} items. Try again in a moment."
            ));
        }
        if reverted {
            // An item taken out of the Favorites list has to come back.
            if self.tl.view == View::Favorites {
                self.tl.requested = None;
            }
            self.patch_lists();
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
        if let Some(stats) = &mut self.frame_stats {
            stats.frame(self.lib.is_busy());
            // egui only repaints on events; keep frames coming so there is something to time.
            ctx.request_repaint();
        }
        if self.refresh_settings(&ctx) {
            // This frame's root ui was created with the old style.
            ui.set_style(ctx.global_style());
        }
        self.update_state(&ctx);
        if (self.dialog_open() || self.settings_open)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            // The topmost thing closes first: a question, otherwise the Settings sheet.
            if self.dialog_open() {
                self.cancel_album_dialogs();
            } else {
                self.settings_open = false;
            }
        }
        if self.viewer_open() {
            // The viewer fills the window: no sidebar, top bar or grid while it is open.
            self.grid_shown = false;
            self.tl.shown = false;
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
        let grid_was_shown = std::mem::take(&mut self.grid_shown);
        let timeline_was_shown = std::mem::take(&mut self.tl.shown);
        self.handle_navigation_keys(&ctx, grid_was_shown);
        self.handle_timeline_keys(&ctx, timeline_was_shown);
        let pal = style::Palette::of(&ctx);
        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(pal.surface)
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ui, |ui| self.top_bar(ui));
        self.sidebar(ui);
        let has_folders = !self.snapshot.folders.is_empty();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.background)
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| {
                if !has_folders {
                    self.no_folders_state(ui);
                } else {
                    self.search_banner(ui);
                    if self.searching() {
                        self.results_grid(ui);
                    } else {
                        self.timeline_view(ui);
                    }
                }
            });
        if self.settings_open {
            self.settings_sheet(&ctx);
        }
        self.model_confirm_dialog(&ctx);
        if self.help_open {
            self.help_sheet(&ctx);
        }
        self.search_help_dropdown(&ctx);
        self.album_dialogs(&ctx);
        self.folder_dialogs(&ctx);
        self.toasts(&ctx);
        if !self.keys_blocked(&ctx) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.clear_selection();
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
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
mod nav_tests {
    use super::{Nav, Notices, scroll_offset_for_row, step};
    use std::time::{Duration, Instant};

    #[test]
    fn an_error_is_not_swallowed_by_later_messages() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut n = Notices::default();
        n.push("could not save".into(), true, t0);
        n.push("Added 2 items".into(), false, at(1));
        assert_eq!(n.current(), Some("could not save"));
        // A newer error replaces the older one.
        n.push("disk full".into(), true, at(2));
        assert_eq!(n.current(), Some("disk full"));
        // Plain messages expire sooner than errors; the error outlives them.
        n.expire(at(7));
        assert_eq!(n.current(), Some("disk full"));
        assert_eq!(n.next_expiry(at(7)), Some(Duration::from_secs(5)));
        // After the error's time the message is gone.
        n.expire(at(13));
        assert_eq!(n.current(), None);
        // With no error, the latest message shows.
        n.push("one".into(), false, at(20));
        n.push("two".into(), false, at(21));
        assert_eq!(n.current(), Some("two"));
    }

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
