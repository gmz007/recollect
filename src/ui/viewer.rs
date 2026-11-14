//! The viewer (lightbox): one photo or video frame as large as fits the window, with an info
//! panel, keyboard navigation through the list it was opened from, and zoom.
//!
//! [`Viewer`] holds only view state and returns [`Action`]s for everything that touches the rest
//! of the app; the `impl App` block at the bottom connects it to the grids.
//!
//! The list being viewed is shared with the grid it was opened from (an `Arc`), never copied:
//! entries are built one at a time, for the item on screen and its neighbours. The viewer keeps
//! that snapshot while it is open, so reloads of the grid behind it do not move the photos
//! under the user; favorite marks changed meanwhile are tracked separately.

use super::albums::AlbumChoice;
use super::preview::{self, Preview, PreviewKey, Previews};
use super::style::{self, icons};
use super::timeline::{DATE_FROM_FILE, fmt_date};
use super::workers::ThumbCache;
use super::{App, Query, consume_plain_key, fmt_time, platform};
use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::search::SearchHit;
use crate::store::db::{AlbumInfo, FileDetails, TimelineItem};
use crate::thumbs::ThumbKey;
use egui::{Color32, Pos2, Rect, RichText, Sense, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Height of the viewer's toolbar row.
const BAR_H: f32 = 34.0;

/// Whether the toolbar buttons carry a word next to their icon: only when the window is wide
/// enough (narrower windows show the icons alone, with tooltips).
pub(super) fn toolbar_shows_text(window_width: f32) -> bool {
    window_width >= 1000.0
}

/// Where the "3 of 40" label starts (its left edge), or `None` when there is no room for it
/// between the buttons on the left (ending at `left_end`) and on the right (starting at
/// `right_start`). It prefers the middle of the bar, else sits right beside the buttons.
fn position_slot(centre: f32, left_end: f32, right_start: f32, width: f32) -> Option<f32> {
    const AIR: f32 = 12.0;
    let centred = centre - width / 2.0;
    let beside = right_start - AIR - width;
    if centred >= left_end + AIR && centred + width <= right_start - AIR {
        Some(centred)
    } else if beside >= left_end + AIR {
        Some(beside)
    } else {
        None
    }
}

/// A matching moment of a video (from a search hit).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MomentChip {
    pub item_id: i64,
    pub t_start: f64,
    pub t_end: f64,
    pub score: f32,
}

/// One item of the list being viewed, from a timeline row or a search hit.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ViewerEntry {
    pub file_id: i64,
    /// A searchable item of the file, if it is indexed (what "Find similar" starts from).
    pub item_id: Option<i64>,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub mtime: i64,
    pub taken_at: i64,
    /// Where `taken_at` comes from, when the list knew it (search hits do not).
    pub date_source: Option<DateSource>,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub favorite: bool,
    pub camera: Option<String>,
    /// Video position shown, in seconds: the matching moment of a search hit.
    pub t: Option<f64>,
    /// All matching moments (best first) when opened from a search hit.
    pub moments: Vec<MomentChip>,
}

impl ViewerEntry {
    pub fn from_timeline(it: &TimelineItem) -> Self {
        Self {
            file_id: it.file_id,
            item_id: it.item_id,
            path: it.path.clone(),
            kind: it.kind,
            mtime: it.mtime,
            taken_at: it.taken_at,
            date_source: Some(it.date_source),
            duration: it.duration,
            width: it.width,
            height: it.height,
            favorite: it.favorite,
            camera: it.camera.clone(),
            t: None,
            moments: Vec::new(),
        }
    }

    pub fn from_hit(h: &SearchHit) -> Self {
        let mut moments = Vec::new();
        if h.kind == MediaKind::Video {
            moments.push(MomentChip {
                item_id: h.item_id,
                t_start: h.t_start.unwrap_or(0.0),
                t_end: h.t_end.or(h.t_start).unwrap_or(0.0),
                score: h.score,
            });
            moments.extend(h.moments.iter().map(|m| MomentChip {
                item_id: m.item_id,
                t_start: m.t_start,
                t_end: m.t_end,
                score: m.score,
            }));
        }
        Self {
            file_id: h.file_id,
            item_id: Some(h.item_id),
            path: h.path.clone(),
            kind: h.kind,
            mtime: h.mtime,
            taken_at: h.taken_at,
            date_source: None,
            duration: h.duration,
            width: h.width,
            height: h.height,
            favorite: h.favorite,
            camera: None,
            t: h.t_start,
            moments,
        }
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Position of the frame shown for a video: the chosen moment, else 10 % of the length
    /// (at most 5 s), like the timeline thumbnail.
    fn frame_time(&self) -> Option<f64> {
        (self.kind == MediaKind::Video).then(|| {
            self.t
                .unwrap_or_else(|| (self.duration.unwrap_or(0.0) * 0.1).clamp(0.0, 5.0))
        })
    }

    fn thumb_key(&self) -> ThumbKey {
        ThumbKey {
            path: self.path.clone(),
            mtime: self.mtime,
            t_ms: self.frame_time().map(super::to_millis),
        }
    }

    fn dims(&self) -> Option<(u32, u32)> {
        self.width.zip(self.height)
    }

    fn preview_key(&self, side: u32, zoom: bool) -> PreviewKey {
        PreviewKey {
            path: self.path.clone(),
            mtime: self.mtime,
            t_ms: self.frame_time().map(super::to_millis),
            side,
            min_side: preview::min_side_for(self.dims(), side),
            zoom,
        }
    }

    /// The item "Find similar" should start from: the chosen moment's, for videos.
    fn similar_item(&self) -> Option<i64> {
        self.moments
            .iter()
            .find(|m| Some(m.t_start) == self.t)
            .map(|m| m.item_id)
            .or(self.item_id)
    }
}

/// What the viewer asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Action {
    Close,
    Favorite(i64, bool),
    AddToAlbum(i64, AlbumChoice),
    Open(PathBuf),
    Reveal(PathBuf),
    PlayAt(PathBuf, f64),
    FindSimilar { item_id: i64, label: String },
}

/// The list a viewer was opened on.
#[derive(Clone)]
pub(super) enum Source {
    /// The album (shared with the timeline).
    Timeline(Arc<Vec<TimelineItem>>),
    /// Search results (shared with the results grid).
    Results(Arc<Vec<SearchHit>>),
    /// A short list built for the occasion (the "On this day" strip).
    Entries(Vec<ViewerEntry>),
}

impl Source {
    fn len(&self) -> usize {
        match self {
            Source::Timeline(v) => v.len(),
            Source::Results(v) => v.len(),
            Source::Entries(v) => v.len(),
        }
    }

    /// The entry at `i` (`i < len`), built on demand.
    fn entry(&self, i: usize) -> ViewerEntry {
        match self {
            Source::Timeline(v) => ViewerEntry::from_timeline(&v[i]),
            Source::Results(v) => ViewerEntry::from_hit(&v[i]),
            Source::Entries(v) => v[i].clone(),
        }
    }
}

pub(super) struct Viewer {
    source: Source,
    index: usize,
    /// The entry at `index`.
    current: ViewerEntry,
    pub info: bool,
    /// Showing the image at 100 % (one image pixel per screen pixel) instead of fitted.
    zoom: bool,
    /// Offset of the image centre from the centre of the view while zoomed.
    pan: Vec2,
    /// Favorite marks that differ from the snapshot (file id to value).
    marks: HashMap<i64, bool>,
    /// Video moments picked in the info panel (index to seconds).
    moments: HashMap<usize, f64>,
}

/// What the app tells the viewer each frame.
pub struct ViewerEnv<'a> {
    pub can_find_similar: bool,
    pub albums: &'a [AlbumInfo],
    /// False while a text field or a dialog has the keyboard.
    pub keys: bool,
}

/// What the canvas draws for the current item.
struct Shown {
    tex: Option<egui::TextureHandle>,
    /// Only the thumbnail is available so far, and the preview is on its way.
    loading: bool,
    failed: bool,
}

impl Viewer {
    pub fn new(source: Source, index: usize, info: bool) -> Self {
        debug_assert!(source.len() > 0, "a viewer needs something to show");
        let index = index.min(source.len().saturating_sub(1));
        let current = source.entry(index);
        Self {
            source,
            index,
            current,
            info,
            zoom: false,
            pan: Vec2::ZERO,
            marks: HashMap::new(),
            moments: HashMap::new(),
        }
    }

    /// Starts with these favorite marks applied on top of the list's own.
    pub fn with_marks(mut self, marks: HashMap<i64, bool>) -> Self {
        self.marks = marks;
        self.current = self.entry_at(self.index);
        self
    }

    /// Starts the item at `index` at video position `t`.
    pub fn with_moment(mut self, index: usize, t: f64) -> Self {
        self.moments.insert(index, t);
        self.current = self.entry_at(self.index);
        self
    }

    fn len(&self) -> usize {
        self.source.len()
    }

    /// The entry at `i` with this viewer's own changes applied.
    fn entry_at(&self, i: usize) -> ViewerEntry {
        let mut e = self.source.entry(i);
        if let Some(&fav) = self.marks.get(&e.file_id) {
            e.favorite = fav;
        }
        if let Some(&t) = self.moments.get(&i) {
            e.t = Some(t);
        }
        e
    }

    pub fn current(&self) -> &ViewerEntry {
        &self.current
    }

    /// Keeps the favorite mark of every entry for `file_id` in line with the library.
    pub fn set_favorite(&mut self, file_id: i64, fav: bool) {
        self.marks.insert(file_id, fav);
        if self.current.file_id == file_id {
            self.current.favorite = fav;
        }
    }

    fn go(&mut self, to: usize) {
        let to = to.min(self.len() - 1);
        if to != self.index {
            self.index = to;
            self.current = self.entry_at(to);
            self.zoom = false;
            self.pan = Vec2::ZERO;
        }
    }

    fn toggle_zoom(&mut self) {
        if self.current().kind == MediaKind::Image {
            self.zoom = !self.zoom;
            self.pan = Vec2::ZERO;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use egui::Key;
        let n = self.len();
        let (esc, left, right, home, end, info, zoom, fav) = ctx.input_mut(|i| {
            (
                consume_plain_key(i, Key::Escape),
                consume_plain_key(i, Key::ArrowLeft),
                consume_plain_key(i, Key::ArrowRight),
                consume_plain_key(i, Key::Home),
                consume_plain_key(i, Key::End),
                consume_plain_key(i, Key::I),
                consume_plain_key(i, Key::Z),
                consume_plain_key(i, Key::F),
            )
        });
        if esc > 0 {
            actions.push(Action::Close);
        }
        let last = n.saturating_sub(1);
        let mut target = (self.index + right).saturating_sub(left);
        if home > 0 {
            target = 0;
        }
        if end > 0 {
            target = last;
        }
        self.go(target.min(last));
        if left + right + home + end > 0
            && let Some(id) = ctx.memory(egui::Memory::focused)
        {
            // A focused button would otherwise also move with the arrow keys.
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        if info % 2 == 1 {
            self.info = !self.info;
        }
        if zoom % 2 == 1 {
            self.toggle_zoom();
        }
        if fav % 2 == 1 {
            let e = self.current();
            actions.push(Action::Favorite(e.file_id, !e.favorite));
        }
    }

    /// Draws the viewer over the whole window and returns what the user asked for.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        previews: &mut Previews,
        thumbs: &mut ThumbCache,
        env: &ViewerEnv<'_>,
    ) -> Vec<Action> {
        let ViewerEnv {
            can_find_similar,
            albums,
            keys,
        } = *env;
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();
        // While a text field has focus or a dialog is open, keys belong to them.
        if keys {
            self.handle_keys(&ctx, &mut actions);
        }
        egui::Panel::top("viewer_top").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui, albums, &mut actions);
            ui.add_space(6.0);
        });
        if self.info {
            egui::Panel::right("viewer_info")
                .resizable(true)
                .default_size(300.0)
                .min_size(240.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.info_panel(ui, previews, can_find_similar, &mut actions);
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_gray(16)))
            .show(ui, |ui| self.canvas(ui, previews, thumbs));
        actions
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, albums: &[AlbumInfo], actions: &mut Vec<Action>) {
        let pal = style::Palette::of(ui.ctx());
        let text = toolbar_shows_text(ui.ctx().content_rect().width());
        let width = ui.available_width();
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, BAR_H), Sense::hover());
        // Keep clear of the window's edges.
        let row = Rect::from_min_max(row.min + Vec2::new(8.0, 0.0), row.max - Vec2::new(8.0, 0.0));
        // The right-hand buttons first, so the left side knows how much room is left.
        let right = ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = if text { 10.0 } else { 4.0 };
                self.right_buttons(ui, albums, actions, text);
            },
        );
        let right_start = right.response.rect.min.x;
        let pos = format!("{} of {}", self.index + 1, self.len());
        let pos_w = ui
            .painter()
            .layout_no_wrap(
                pos.clone(),
                egui::TextStyle::Body.resolve(ui.style()),
                pal.text_secondary,
            )
            .size()
            .x;
        // Room for the name: what is left of the bar, minus the position and some air.
        let left_max = (right_start - 8.0 - row.min.x).max(0.0);
        let left_rect = Rect::from_min_size(row.min, Vec2::new(left_max, BAR_H));
        let left = ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if style::toolbar_button(
                    ui,
                    icons::ICON_ARROW_BACK,
                    text.then_some("Back"),
                    "Back",
                    "Back",
                    None,
                    false,
                )
                .clicked()
                {
                    actions.push(Action::Close);
                }
                // The name is shown once: here while the info panel is closed, there while open.
                if !self.info {
                    ui.add_space(8.0);
                    let room = (ui.available_width() - pos_w - 24.0)
                        .min(row.width() * 0.3)
                        .max(0.0);
                    if room > 40.0 {
                        ui.scope(|ui| {
                            ui.set_max_width(room);
                            ui.add(
                                egui::Label::new(style::semibold(self.current().name())).truncate(),
                            );
                        });
                    }
                }
            },
        );
        let left_end = left.response.rect.max.x;
        // The position sits in the middle of the bar when there is room there, else right
        // beside the buttons, else it is left out. It is plain text: never a click target.
        let x = position_slot(row.center().x, left_end, right_start, pos_w);
        if let Some(x) = x {
            let rect = Rect::from_min_size(
                Pos2::new(x, row.center().y - 11.0),
                Vec2::new(pos_w + 2.0, 22.0),
            );
            ui.put(
                rect,
                egui::Label::new(RichText::new(pos).color(pal.text_secondary)).selectable(false),
            );
        }
    }

    /// The buttons on the right of the toolbar, laid out right to left. With `text`, each has a
    /// short word next to its icon.
    fn right_buttons(
        &mut self,
        ui: &mut egui::Ui,
        albums: &[AlbumInfo],
        actions: &mut Vec<Action>,
        text: bool,
    ) {
        let word = |w: &'static str| text.then_some(w);
        if self.current().kind == MediaKind::Image {
            let (icon, label) = if self.zoom {
                (icons::ICON_ZOOM_OUT, "Fit to window")
            } else {
                (icons::ICON_ZOOM_IN, "Zoom to 100%")
            };
            let tip = if self.zoom {
                label.to_string()
            } else {
                "Zoom to 100% (one image pixel per screen pixel; photos larger than \
                 4096 px on a side are scaled down to that first)"
                    .to_string()
            };
            let short = if self.zoom { "Fit" } else { "Zoom" };
            if style::toolbar_button(ui, icon, word(short), label, &tip, None, self.zoom).clicked()
            {
                self.toggle_zoom();
            }
        }
        let path = self.current().path.clone();
        if style::toolbar_button(
            ui,
            icons::ICON_FOLDER_OPEN,
            word("Show in folder"),
            "Show in folder",
            "Show in folder",
            None,
            false,
        )
        .clicked()
        {
            actions.push(Action::Reveal(path.clone()));
        }
        if style::toolbar_button(
            ui,
            icons::ICON_OPEN_IN_NEW,
            word("Open"),
            "Open with default app",
            "Open with default app",
            None,
            false,
        )
        .clicked()
        {
            actions.push(Action::Open(path));
        }
        if style::toolbar_button(
            ui,
            icons::ICON_INFO,
            word("Info"),
            "Info",
            "Photo details",
            None,
            self.info,
        )
        .clicked()
        {
            self.info = !self.info;
        }
        if let Some(c) = super::albums::album_menu_icon(ui, albums, word("Add to album")) {
            actions.push(Action::AddToAlbum(self.current().file_id, c));
        }
        self.favorite_button(ui, actions, text);
        if self.current().kind == MediaKind::Video {
            self.play_button(ui, actions);
        }
    }

    /// The accent button that plays the current video from the chosen moment.
    fn play_button(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let pal = style::Palette::of(ui.ctx());
        let e = self.current().clone();
        ui.add_space(8.0);
        let t = e.t.unwrap_or(0.0);
        let label = format!("Play at {}", fmt_time(t));
        let mut job = egui::text::LayoutJob::default();
        for (text, font, gap) in [
            (
                icons::ICON_PLAY_ARROW.codepoint,
                style::icon_font_id(icons::ICON_PLAY_ARROW, 20.0),
                0.0,
            ),
            (
                label.as_str(),
                egui::TextStyle::Button.resolve(ui.style()),
                4.0,
            ),
        ] {
            job.append(
                text,
                gap,
                egui::TextFormat {
                    font_id: font,
                    color: pal.on_accent,
                    valign: egui::Align::Center,
                    ..Default::default()
                },
            );
        }
        let b = egui::Button::new(job).fill(pal.accent);
        let r = ui.add(b);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if r.on_hover_text("Play this video").clicked() {
            actions.push(Action::PlayAt(e.path, t));
        }
    }

    fn favorite_button(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, text: bool) {
        let e = self.current();
        let pal = style::Palette::of(ui.ctx());
        let (icon, label, short, color) = if e.favorite {
            (
                icons::ICON_STAR,
                "Unfavorite",
                "Favorited",
                Some(pal.favorite),
            )
        } else {
            (icons::ICON_STAR.outlined(), "Favorite", "Favorite", None)
        };
        let tip = if e.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        if style::toolbar_button(ui, icon, text.then_some(short), label, tip, color, false)
            .clicked()
        {
            actions.push(Action::Favorite(e.file_id, !e.favorite));
        }
    }

    fn info_panel(
        &mut self,
        ui: &mut egui::Ui,
        previews: &mut Previews,
        can_find_similar: bool,
        actions: &mut Vec<Action>,
    ) {
        let e = self.current.clone();
        let details: Option<FileDetails> = previews.details(e.file_id).cloned();
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(e.name()).strong());
            if e.favorite {
                ui.label(
                    style::icon_text(icons::ICON_STAR, 16.0)
                        .color(style::Palette::of(ui.ctx()).favorite),
                );
            }
        });
        // The folder's name; the whole path shows on hover.
        if let Some(dir) = e.path.parent() {
            let name = dir.file_name().map_or_else(
                || dir.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            ui.label(RichText::new(name).small().weak())
                .on_hover_text(dir.display().to_string());
        }
        ui.add_space(6.0);
        let taken = details.as_ref().map_or(e.taken_at, |d| d.taken_at);
        let source = details.as_ref().map(|d| d.date_source).or(e.date_source);
        ui.label(fmt_date(taken));
        if source == Some(DateSource::File) {
            ui.label(RichText::new(DATE_FROM_FILE).small().weak());
        }
        let camera = details
            .as_ref()
            .and_then(|d| d.camera.clone())
            .or(e.camera.clone());
        if let Some(c) = camera {
            ui.label(RichText::new(c).small());
        }
        if let Some((w, h)) = e.dims() {
            ui.label(format!("{w}×{h}"));
        }
        if let Some(s) = details.as_ref().and_then(|d| d.size) {
            ui.label(fmt_size(s));
        }
        if let Some(d) = e.duration {
            ui.label(format!("length {}", fmt_time(d)));
        }
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

    /// Asks for what the canvas needs (and forgets what it no longer wants) and picks the
    /// texture to draw.
    fn pick_texture(&self, previews: &mut Previews, thumbs: &mut ThumbCache, fit: u32) -> Shown {
        let e = &self.current;
        // The neighbours are prefetched; the item on screen is asked for urgently every frame,
        // so it is decoded before them even if they were queued first (the fitted preview
        // last of all, hence first in line). Whatever is no longer wanted is forgotten.
        let mut wanted = Vec::new();
        for i in [self.index + 1, self.index.wrapping_sub(1)] {
            if i < self.len() {
                let k = self.entry_at(i).preview_key(fit, false);
                previews.get(&k);
                wanted.push(k);
            }
        }
        let fit_key = e.preview_key(fit, false);
        let zoom_key = (self.zoom && e.kind == MediaKind::Image)
            .then(|| {
                let longest = e.dims().map_or(preview::MAX_ZOOM_SIDE, |(w, h)| w.max(h));
                longest.min(preview::MAX_ZOOM_SIDE)
            })
            .filter(|&s| s > fit)
            .map(|s| e.preview_key(s, true));
        let mut tex = None;
        let mut failed = false;
        if let Some(k) = &zoom_key {
            wanted.push(k.clone());
        }
        wanted.push(fit_key.clone());
        if let Some(k) = &zoom_key {
            previews.get_urgent(k);
        }
        match previews.get_urgent(&fit_key) {
            Preview::Ready(t) => tex = Some(t.clone()),
            Preview::Failed => failed = true,
            Preview::Loading => {}
        }
        if let Some(k) = &zoom_key
            && let Preview::Ready(t) = previews.get(k)
        {
            tex = Some(t.clone());
        }
        previews.retain_queued(&wanted);
        let mut loading = false;
        if tex.is_none() {
            // Until the preview arrives, the (already cached) thumbnail stands in for it.
            tex = thumbs.get(&e.thumb_key()).cloned();
            loading = !failed;
        }
        Shown {
            tex,
            loading,
            failed,
        }
    }

    /// The media area: preview (or the thumbnail until it is ready), zoom and previous / next.
    fn canvas(&mut self, ui: &mut egui::Ui, previews: &mut Previews, thumbs: &mut ThumbCache) {
        let ctx = ui.ctx().clone();
        let area = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(area, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Viewer canvas"));
        if resp.double_clicked() {
            self.toggle_zoom();
        }
        let ppp = ctx.pixels_per_point();
        let fit = preview::fit_side(ctx.content_rect().size(), ppp);
        let e = self.current.clone();
        let Shown {
            tex,
            loading,
            failed,
        } = self.pick_texture(previews, thumbs, fit);
        let painter = ui.painter_at(area);
        if let Some(tex) = tex {
            let native = tex.size_vec2();
            let size = if self.zoom && !loading {
                e.dims()
                    .map_or(native, |(w, h)| Vec2::new(w as f32, h as f32))
                    / ppp
            } else {
                native * (area.width() / native.x).min(area.height() / native.y)
            };
            if self.zoom && resp.dragged() {
                self.pan += resp.drag_delta();
            }
            let slack = ((size - area.size()) / 2.0).max(Vec2::ZERO);
            self.pan = self.pan.clamp(-slack, slack);
            let centre = if self.zoom {
                area.center() + self.pan
            } else {
                area.center()
            };
            painter.image(
                tex.id(),
                Rect::from_center_size(centre, size),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            if self.zoom && !loading {
                ui.output_mut(|o| {
                    o.cursor_icon = if resp.dragged() {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    }
                });
            }
        }
        if loading {
            ui.put(
                Rect::from_center_size(area.center(), Vec2::splat(28.0)),
                egui::Spinner::new(),
            );
        } else if failed {
            painter.text(
                area.center() + Vec2::new(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Preview unavailable",
                egui::FontId::proportional(15.0),
                Color32::from_gray(180),
            );
        }
        // Previous / next, drawn over the media.
        let mid = area.center().y;
        let prev = Rect::from_center_size(Pos2::new(area.left() + 34.0, mid), Vec2::splat(44.0));
        let next = Rect::from_center_size(Pos2::new(area.right() - 34.0, mid), Vec2::splat(44.0));
        if self.index > 0 && nav_button(ui, prev, "Previous", -1.0) {
            self.go(self.index - 1);
        }
        if self.index + 1 < self.len() && nav_button(ui, next, "Next", 1.0) {
            self.go(self.index + 1);
        }
    }
}

/// A round button with a chevron; `dir` is -1 for "previous" and 1 for "next".
fn nav_button(ui: &mut egui::Ui, rect: Rect, label: &str, dir: f32) -> bool {
    let resp = ui.interact(
        rect,
        ui.make_persistent_id(("viewer-nav", label)),
        Sense::click(),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let alpha = if resp.hovered() { 200 } else { 110 };
    let p = ui.painter();
    p.circle_filled(
        rect.center(),
        rect.width() / 2.0,
        Color32::from_black_alpha(alpha),
    );
    let c = rect.center();
    let stroke = egui::Stroke::new(3.0, Color32::WHITE);
    let (tip, tail) = (
        c + Vec2::new(5.0 * dir, 0.0),
        c + Vec2::new(-4.0 * dir, 0.0),
    );
    p.line_segment([tip, tail + Vec2::new(0.0, -11.0)], stroke);
    p.line_segment([tip, tail + Vec2::new(0.0, 11.0)], stroke);
    resp.on_hover_text(label).clicked()
}

/// `3.2 MB`, `812 KB`, `2.0 GB`.
fn fmt_size(bytes: u64) -> String {
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

// ---------------------------------------------------------------------------------------------
// App wiring
// ---------------------------------------------------------------------------------------------

impl App {
    pub(super) fn viewer_open(&self) -> bool {
        self.viewer.is_some()
    }

    /// Opens a viewer on `source` at `index`, at video position `moment` if given.
    fn start_viewer(&mut self, source: Source, index: usize, moment: Option<f64>) {
        if source.len() == 0 {
            return;
        }
        let marks = self
            .fav_marks
            .iter()
            .map(|(id, m)| (*id, m.value))
            .collect();
        let mut v = Viewer::new(source, index, self.viewer_info).with_marks(marks);
        if let Some(t) = moment {
            v = v.with_moment(index, t);
        }
        self.viewer = Some(v);
    }

    /// Opens the viewer on the whole album list at `i`. The list is shared, not copied.
    pub(super) fn open_viewer_timeline(&mut self, i: usize) {
        self.tl.grid.selected = Some(i);
        self.start_viewer(Source::Timeline(Arc::clone(&self.tl.items)), i, None);
    }

    /// Opens the viewer on the search results at `i`. The list is shared, not copied.
    pub(super) fn open_viewer_results(&mut self, i: usize) {
        let moment = self
            .res
            .selected_t
            .filter(|_| self.res.grid.selected == Some(i))
            .filter(|_| {
                self.res
                    .hits
                    .get(i)
                    .is_some_and(|h| h.kind == MediaKind::Video)
            });
        self.res.grid.selected = Some(i);
        self.start_viewer(Source::Results(Arc::clone(&self.res.hits)), i, moment);
    }

    /// Opens the viewer on a subset of the album (e.g. the "On this day" strip).
    pub(super) fn open_viewer_subset(&mut self, items: &[usize], pos: usize) {
        let entries = items
            .iter()
            .filter_map(|&i| self.tl.items.get(i))
            .map(ViewerEntry::from_timeline)
            .collect();
        self.start_viewer(Source::Entries(entries), pos, None);
    }

    /// Draws the viewer instead of the album and applies what it asks for.
    pub(super) fn show_viewer(&mut self, ui: &mut egui::Ui) {
        let Some(mut viewer) = self.viewer.take() else {
            return;
        };
        let can_similar = self.snapshot.model_installed;
        // Details (file size, camera) are re-read once the library has changed under them.
        self.previews
            .sync_details((self.tl.generation, self.lib.jobs_finished()));
        let keys = !self.keys_blocked(ui.ctx());
        let actions = viewer.show(
            ui,
            &mut self.previews,
            &mut self.thumbs,
            &ViewerEnv {
                can_find_similar: can_similar,
                albums: &self.albums,
                keys,
            },
        );
        self.viewer = Some(viewer);
        for a in actions {
            match a {
                Action::Close => self.close_viewer(),
                Action::Favorite(id, fav) => self.set_favorite(id, fav),
                Action::AddToAlbum(id, c) => self.add_to_album_choice(vec![id], c),
                Action::Open(p) => self.report(
                    platform::open(&p),
                    "Couldn't open that file. It may have been moved or deleted; check \
                     that it is still in its folder.",
                ),
                Action::Reveal(p) => self.report(
                    platform::reveal(&p),
                    "Couldn't show that file in its folder. It may have been moved or \
                     deleted.",
                ),
                Action::PlayAt(p, t) => self.report(
                    platform::open_video_at(&p, t),
                    "Couldn't play that video. Install a video player such as VLC, or \
                     open the file from its folder.",
                ),
                Action::FindSimilar { item_id, label } => {
                    self.end_viewer();
                    self.tl.grid.selected = None;
                    self.query.clear();
                    self.submit(Query::Similar { item_id, label });
                }
            }
        }
    }

    /// Shows `message` (with the technical detail behind "details") if `result` failed.
    fn report<T>(&mut self, result: anyhow::Result<T>, message: &str) {
        if let Err(e) = result {
            self.notify_problem(message, format!("{e:#}"), None);
        }
    }

    /// Takes the viewer down for any reason: remembers whether its info panel was open and
    /// frees the previews and details it was holding. Returns it for whoever wants to know
    /// where it was.
    pub(super) fn end_viewer(&mut self) -> Option<Viewer> {
        let v = self.viewer.take()?;
        self.viewer_info = v.info;
        self.previews.clear();
        Some(v)
    }

    /// Back to the grid with the item that was being viewed selected and scrolled into view.
    pub(super) fn close_viewer(&mut self) {
        let Some(v) = self.end_viewer() else {
            return;
        };
        let e = v.current();
        let Some(i) = self.listed().position(|(id, _)| id == e.file_id) else {
            return;
        };
        let searching = self.searching();
        let grid = self.active_grid();
        grid.selected = Some(i);
        grid.keyboard_moved = true;
        if searching {
            self.res.selected_t = e.t.or(self.res.hits[i].t_start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(2048), "2 KB");
        assert_eq!(fmt_size(3_355_443), "3.2 MB");
        assert_eq!(fmt_size(3 << 30), "3.0 GB");
    }

    fn entry(kind: MediaKind) -> ViewerEntry {
        ViewerEntry {
            file_id: 1,
            item_id: Some(10),
            path: "/x/clip.mp4".into(),
            kind,
            mtime: 5,
            taken_at: 0,
            date_source: None,
            duration: Some(100.0),
            width: Some(1920),
            height: Some(1080),
            favorite: false,
            camera: None,
            t: None,
            moments: Vec::new(),
        }
    }

    #[test]
    fn video_frames_follow_the_moment_or_the_thumbnail_rule() {
        let mut v = entry(MediaKind::Video);
        assert_eq!(v.frame_time(), Some(5.0)); // 10 % of 100 s, capped at 5 s
        v.duration = Some(20.0);
        assert_eq!(v.frame_time(), Some(2.0));
        v.t = Some(42.5);
        assert_eq!(v.frame_time(), Some(42.5));
        assert_eq!(v.preview_key(1024, false).t_ms, Some(42_500));
        assert_eq!(v.thumb_key().t_ms, Some(42_500));
        assert_eq!(entry(MediaKind::Image).frame_time(), None);
    }

    #[test]
    fn find_similar_starts_from_the_chosen_moment() {
        let mut v = entry(MediaKind::Video);
        v.moments = vec![
            MomentChip {
                item_id: 10,
                t_start: 4.0,
                t_end: 6.0,
                score: 0.3,
            },
            MomentChip {
                item_id: 11,
                t_start: 60.0,
                t_end: 62.0,
                score: 0.2,
            },
        ];
        v.t = Some(60.0);
        assert_eq!(v.similar_item(), Some(11));
        v.t = Some(4.0);
        assert_eq!(v.similar_item(), Some(10));
        v.t = None;
        assert_eq!(v.similar_item(), Some(10));
    }

    #[test]
    fn navigation_stays_inside_the_list() {
        let mut v = Viewer::new(Source::Entries(vec![entry(MediaKind::Image); 3]), 1, true);
        v.zoom = true;
        v.go(2);
        assert_eq!(v.index, 2);
        assert!(!v.zoom, "moving on resets the zoom");
        v.go(99);
        assert_eq!(v.index, 2);
        v.go(0);
        assert_eq!(v.index, 0);
        let one = Source::Entries(vec![entry(MediaKind::Image)]);
        assert_eq!(Viewer::new(one, 7, true).index, 0);
    }

    fn timeline_item(i: i64) -> TimelineItem {
        TimelineItem {
            file_id: i,
            item_id: None,
            path: format!("/p/{i}.jpg").into(),
            kind: MediaKind::Image,
            mtime: 0,
            taken_at: i,
            date_source: DateSource::Exif,
            duration: None,
            width: None,
            height: None,
            favorite: false,
            camera: None,
        }
    }

    #[test]
    fn a_huge_list_is_shared_not_copied() {
        let items: Arc<Vec<TimelineItem>> = Arc::new((0..200_000).map(timeline_item).collect());
        let mut v = Viewer::new(Source::Timeline(Arc::clone(&items)), 123_456, true);
        // The viewer holds the very same list; building the entry for the item on screen is
        // all it did per item.
        assert_eq!(Arc::strong_count(&items), 2);
        assert_eq!(v.current().file_id, 123_456);
        assert_eq!(v.len(), 200_000);
        v.go(7);
        assert_eq!(v.current().path, PathBuf::from("/p/7.jpg"));
        assert_eq!(Arc::strong_count(&items), 2);
        drop(v);
        assert_eq!(Arc::strong_count(&items), 1);
    }

    #[test]
    fn favorite_marks_and_picked_moments_follow_the_viewer_across_entries() {
        let items: Arc<Vec<TimelineItem>> = Arc::new((0..3).map(timeline_item).collect());
        let mut v = Viewer::new(Source::Timeline(items), 0, true)
            .with_marks(HashMap::from([(2, true)]))
            .with_moment(1, 9.0);
        assert!(!v.current().favorite);
        v.set_favorite(0, true);
        assert!(v.current().favorite);
        v.go(1);
        assert_eq!(v.current().t, Some(9.0));
        v.go(2);
        assert!(v.current().favorite, "the mark given at the start applies");
        v.go(0);
        assert!(v.current().favorite, "and the one set while open");
    }

    #[test]
    fn the_position_is_centred_beside_the_buttons_or_left_out() {
        // Plenty of room: in the middle.
        assert_eq!(position_slot(500.0, 100.0, 900.0, 60.0), Some(470.0));
        // The right-hand buttons reach past the middle: it moves next to them.
        assert_eq!(position_slot(500.0, 100.0, 520.0, 60.0), Some(448.0));
        // No gap wide enough: hidden rather than overlapping a button.
        assert_eq!(position_slot(500.0, 100.0, 150.0, 60.0), None);
        // Whatever the answer, it never touches either side.
        for right in (100..1000).step_by(7) {
            if let Some(x) = position_slot(500.0, 100.0, right as f32, 60.0) {
                assert!(x >= 112.0 && x + 60.0 <= right as f32 - 12.0);
            }
        }
    }

    #[test]
    fn words_next_to_the_icons_need_a_wide_window() {
        assert!(!toolbar_shows_text(999.0));
        assert!(toolbar_shows_text(1000.0));
    }
}
