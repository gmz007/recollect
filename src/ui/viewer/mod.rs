//! The viewer (lightbox): one photo or video frame as large as fits the window, with an info
//! panel, keyboard navigation through the list it was opened from, and zoom.
//!
//! [`Viewer`] holds only view state and returns [`Action`]s for everything that touches the rest
//! of the app; the `impl App` block at the bottom connects it to the grids. Its parts are drawn
//! by `toolbar`, `canvas` and `info`; `entry` describes the item on screen.
//!
//! The list being viewed is shared with the grid it was opened from (an `Arc`), never copied:
//! entries are built one at a time, for the item on screen and its neighbours. The viewer keeps
//! that snapshot while it is open, so reloads of the grid behind it do not move the photos
//! under the user; favorite marks changed meanwhile are tracked separately.

mod canvas;
mod entry;
mod info;
mod toolbar;

pub(super) use entry::{Source, ViewerEntry};

use super::albums::AlbumChoice;
use super::preview::Previews;
use super::workers::ThumbCache;
use super::{App, Query, consume_plain_key, platform};
use crate::media::MediaKind;
use crate::store::db::AlbumInfo;
use egui::{Color32, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
        // While a text field has focus, a dialog is open or one of the viewer's menus is open,
        // keys belong to them (Escape closes the menu, not the viewer).
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
}

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
mod tests;
