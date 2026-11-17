//! Guidance for people who are new to the app: the shared "nothing here yet" screen, the search
//! help under the search field, the one-time tips and the plain-language wording of failures.

use super::style::{self, Palette, icons};
use super::timeline::View;
use super::{App, NoticeAction};
use crate::library::{ErrorKind, SearchError};
use crate::store::db::{AlbumError, MAX_ALBUM_NAME_CHARS};
use egui::{CornerRadius, Margin, Pos2, Rect, RichText, Stroke, Vec2};
use egui_material_icons::MaterialIcon;
use std::time::Instant;

/// Searches offered as examples when the search field is empty.
pub(super) const EXAMPLE_SEARCHES: [&str; 6] = [
    "kids on the beach",
    "birthday cake",
    "snow",
    "dog",
    "sunset",
    "a red car",
];

/// How many recent searches are kept and shown.
pub(super) const MAX_RECENT: usize = 8;

// --- Wording of failures ---------------------------------------------------------------

/// What to tell the user about a failure the library recorded, and the button that tries again
/// when there is one. The technical text goes under "Details" in the toast.
pub(super) fn describe_failure(kind: ErrorKind) -> (String, Option<NoticeAction>) {
    match kind {
        ErrorKind::Download => (
            "Couldn't download smart search. Check your internet connection and try again.".into(),
            Some(NoticeAction::Download),
        ),
        ErrorKind::AddFolder => (
            "Couldn't add that folder. Check that it still exists and that you are allowed to \
             open it, then try again."
                .into(),
            None,
        ),
        ErrorKind::RemoveFolder => (
            "Couldn't stop showing that folder. Please try again in a moment.".into(),
            None,
        ),
        ErrorKind::Scan => (
            "Couldn't finish looking through your photos. Please try again.".into(),
            Some(NoticeAction::Scan),
        ),
        ErrorKind::SwitchModel => (
            "Couldn't change smart search. Please try again in a moment.".into(),
            None,
        ),
        ErrorKind::Other => (
            "Something went wrong. Please try again, and restart Recollect if it keeps \
             happening."
                .into(),
            None,
        ),
    }
}

/// A search that could not run, in plain words.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SearchProblem {
    pub text: String,
    /// Whether trying again can help.
    pub can_retry: bool,
}

/// The sentence shown when a search could not run (the reason is only logged).
pub(super) fn search_problem(kind: SearchError) -> SearchProblem {
    let (text, can_retry) = match kind {
        SearchError::ModelMissing => ("Search isn't ready yet. Turn on smart search first.", true),
        SearchError::NotReady => (
            "Recollect is still getting your photos ready. Please try again in a moment.",
            true,
        ),
        SearchError::ItemGone => ("That photo is no longer in your library.", false),
        SearchError::Other => (
            "Couldn't search just now. Please try again in a moment.",
            true,
        ),
    };
    SearchProblem {
        text: text.into(),
        can_retry,
    }
}

/// An album problem as a sentence for the toast plus, when it was not one the library
/// refuses on purpose, the text for "Details".
pub(super) fn album_problem(kind: Option<&AlbumError>, reason: &str) -> (String, Option<String>) {
    match kind {
        Some(AlbumError::NameTaken(other)) => (
            format!("An album named “{other}” already exists. Choose a different name."),
            None,
        ),
        Some(AlbumError::NameEmpty) => (
            "An album needs a name. Type a name and try again.".into(),
            None,
        ),
        Some(AlbumError::TooLong) => (
            format!(
                "Album names can have at most {MAX_ALBUM_NAME_CHARS} characters. Please shorten it."
            ),
            None,
        ),
        Some(AlbumError::Gone) => ("That album no longer exists.".into(), None),
        None => (
            "Couldn't save that album change. Nothing was changed. Please try again.".into(),
            Some(reason.to_string()),
        ),
    }
}

// --- Empty states ----------------------------------------------------------------------

/// A piece of the sentence of an empty state.
pub(super) enum Part<'a> {
    Text(&'a str),
    /// The heart icon (the symbol itself may be missing from the text font).
    Heart,
}

/// The sentence of an empty-state screen laid out for display, and the same sentence in
/// plain text for assistive technology (the heart named).
fn sentence_job(
    ui: &egui::Ui,
    sentence: &[Part<'_>],
    pal: &Palette,
) -> (egui::text::LayoutJob, String) {
    let color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width().min(460.0);
    job.halign = egui::Align::Center;
    let mut plain = String::new();
    let body = egui::TextStyle::Body.resolve(ui.style());
    let font = egui::FontId::new(body.size + 1.5, body.family);
    for part in sentence {
        match part {
            Part::Text(t) => {
                job.append(
                    t,
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color,
                        ..Default::default()
                    },
                );
                plain.push_str(t);
            }
            Part::Heart => {
                job.append(
                    icons::ICON_FAVORITE.codepoint,
                    0.0,
                    egui::TextFormat {
                        font_id: style::icon_font_id(icons::ICON_FAVORITE, 20.0),
                        color: pal.text,
                        valign: egui::Align::Center,
                        ..Default::default()
                    },
                );
                plain.push_str("heart");
            }
        }
    }
    (job, plain)
}

/// An [`empty_state`] whose sentence is plain text.
fn empty_text(ui: &mut egui::Ui, icon: MaterialIcon, text: &str, button: &str) -> bool {
    empty_state(ui, icon, &[Part::Text(text)], button)
}

/// The screen for a view with nothing in it: an icon, one sentence and one main button.
/// Returns true when the button was clicked.
fn empty_state(ui: &mut egui::Ui, icon: MaterialIcon, sentence: &[Part<'_>], button: &str) -> bool {
    let pal = Palette::of(ui.ctx());
    let (job, plain) = sentence_job(ui, sentence, &pal);
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).min(140.0));
        ui.label(style::icon_text(icon, 56.0).color(pal.text2));
        ui.add_space(12.0);
        let r = ui.add(egui::Label::new(job));
        // Assistive technology reads the whole sentence, with the heart named.
        ui.ctx().accesskit_node_builder(r.id, |b| {
            b.set_value(plain.clone());
            b.set_label(plain.clone());
        });
        ui.add_space(16.0);
        clicked = style::primary_button(ui, button).clicked();
    });
    clicked
}

impl App {
    /// The view of a library without folders.
    pub(super) fn no_folders_state(&mut self, ui: &mut egui::Ui) {
        if empty_state(
            ui,
            icons::ICON_FOLDER_OPEN,
            &[Part::Text(
                "Add a folder with your photos and videos to get started.",
            )],
            "Add a folder",
        ) {
            self.open_add_folder();
        }
    }

    /// What the album grids show when the current view has no items.
    pub(super) fn timeline_empty_state(&mut self, ui: &mut egui::Ui) {
        let go_photos = match self.tl.view {
            View::Album(_) => empty_text(
                ui,
                icons::ICON_PHOTO_ALBUM,
                "This album is empty. Select photos and choose Add to album.",
                "Go to Photos",
            ),
            View::Favorites => empty_state(
                ui,
                icons::ICON_FAVORITE,
                &[
                    Part::Text("Click the "),
                    Part::Heart,
                    Part::Text(" on any photo to add it here."),
                ],
                "Go to Photos",
            ),
            View::RecentlyAdded => {
                let text = "Photos and videos you add to your folders show up here.";
                if empty_text(ui, icons::ICON_SCHEDULE, text, "Check for new photos") {
                    self.start_indexing();
                }
                false
            }
            View::Folder(_) => {
                let text = "There are no photos or videos in this folder.";
                if empty_text(ui, icons::ICON_FOLDER_OPEN, text, "Check for new photos") {
                    self.start_indexing();
                }
                false
            }
            View::Timeline => {
                let text = "No photos or videos yet. Add a folder, or give Recollect a moment to \
                            look through the ones you have.";
                if empty_text(ui, icons::ICON_PHOTO_LIBRARY, text, "Add a folder") {
                    self.open_add_folder();
                }
                false
            }
        };
        if go_photos {
            self.set_view(View::Timeline);
        }
    }

    /// What the search results show when there is nothing to list.
    pub(super) fn results_empty_state(&mut self, ui: &mut egui::Ui) {
        if self.res.awaiting.is_some() {
            return;
        }
        if let Some(e) = self.res.error.clone() {
            self.search_problem_state(ui, &e);
            return;
        }
        if self.res.label.is_empty() {
            return;
        }
        self.no_match_state(ui);
    }

    /// The search could not run: say why and offer the way out.
    fn search_problem_state(&mut self, ui: &mut egui::Ui, e: &SearchProblem) {
        if !self.snapshot.model_installed {
            let text = "Smart search is off. Turn it on to find photos by describing them.";
            if empty_text(ui, icons::ICON_AUTO_AWESOME, text, "Turn on smart search") {
                self.open_settings_at(super::settings::Section::SmartSearch);
            }
        } else if e.can_retry {
            if empty_text(ui, icons::ICON_SEARCH_OFF, &e.text, "Try again") {
                self.rerun();
            }
        } else if empty_text(ui, icons::ICON_SEARCH_OFF, &e.text, "Start over") {
            self.clear_search();
        }
    }

    /// The search ran and found nothing. With a filter on, the way out is clearing it;
    /// otherwise the hint is to describe the moment in other words.
    fn no_match_state(&mut self, ui: &mut egui::Ui) {
        let filtered = self.kind.is_some() || self.date != super::when::DateFilter::default();
        let sentence = match (&self.res.current, filtered) {
            (Some(super::Query::Similar { .. }), _) => {
                "No similar photos were found. Try another photo.".to_string()
            }
            (_, true) => "Nothing matched those filters. Try a different time or type, or \
                          describe the moment in other words."
                .to_string(),
            (_, false) => format!(
                "No photos match \u{201c}{}\u{201d}. Try describing the moment in other words.",
                super::albums::short_name(&self.res.label, 40)
            ),
        };
        if no_match_view(ui, &sentence, filtered.then_some("Clear filters")) {
            self.clear_filters();
        }
    }

    /// Back to all types, any time and best match first, and search again.
    pub(super) fn clear_filters(&mut self) {
        self.kind = None;
        self.date = super::when::DateFilter::default();
        self.sort = crate::search::SearchSort::default();
        self.rerun();
    }
}

/// The screen for a search that found nothing: an icon, one sentence and, when there is a way
/// out, a quiet button. Returns true when the button was clicked.
fn no_match_view(ui: &mut egui::Ui, sentence: &str, button: Option<&str>) -> bool {
    let pal = Palette::of(ui.ctx());
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).min(140.0));
        ui.label(style::icon_text(icons::ICON_IMAGE_SEARCH, 56.0).color(pal.muted));
        ui.add_space(12.0);
        ui.scope(|ui| {
            ui.set_max_width(360.0);
            ui.label(RichText::new(sentence).color(pal.text2));
        });
        if let Some(button) = button {
            ui.add_space(12.0);
            clicked = ui.button(button).clicked();
        }
    });
    clicked
}

// --- Search help -----------------------------------------------------------------------

/// Where the search field is and whether its help is showing (from the previous frame).
#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent per-frame UI flags, each set and cleared by a different event"
)]
pub(super) struct SearchHelp {
    pub field: Option<Rect>,
    pub focused: bool,
    pub dropdown: Option<Rect>,
    /// Tab was pressed in the field: move into the suggestions.
    pub want_chips: bool,
    /// Keyboard focus is on one of the suggestions (from the previous frame).
    pub inside: bool,
    /// Escape closed the suggestions; they stay closed until the field is used again.
    pub suppressed: bool,
}

/// Height of a row of the suggestions.
const SUGGESTION_ROW: f32 = 32.0;

/// One row of the suggestions: a muted icon and the text, with a hover fill. `name` is what
/// assistive technology reads.
fn suggestion_row(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    text: &str,
    name: &str,
    ids: &mut Vec<egui::Id>,
) -> bool {
    let pal = Palette::of(ui.ctx());
    let (rect, r) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SUGGESTION_ROW),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let fill = if r.is_pointer_button_down_on() {
            pal.sel
        } else if r.hovered() || r.has_focus() {
            pal.hover
        } else {
            egui::Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(style::CONTROL_RADIUS), fill);
        painter.text(
            Pos2::new(rect.min.x + 18.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon.codepoint,
            style::icon_font_id(icon, 16.0),
            pal.muted,
        );
        painter.text(
            Pos2::new(rect.min.x + 36.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            egui::TextStyle::Body.resolve(ui.style()),
            pal.text,
        );
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    ids.push(r.id);
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// What the user picked in the search help.
#[derive(Default)]
struct HelpPicks {
    /// A suggestion to search for.
    chosen: Option<String>,
    /// "Clear recent searches" was clicked.
    clear: bool,
}

/// The sections of the search help: recent searches, examples and the privacy note.
/// `ids` collects the suggestions' ids for keyboard movement.
fn help_sections(
    ui: &mut egui::Ui,
    pal: &Palette,
    recent: &[String],
    ids: &mut Vec<egui::Id>,
) -> HelpPicks {
    let mut picks = HelpPicks::default();
    style::menu_body(ui);
    if !recent.is_empty() {
        help_heading(ui, pal, "Recent", 4.0);
        for q in recent {
            let name = format!("Recent search: {q}");
            if suggestion_row(ui, icons::ICON_HISTORY, q, &name, ids) {
                picks.chosen = Some(q.clone());
            }
        }
    }
    help_heading(ui, pal, "Try", if recent.is_empty() { 4.0 } else { 10.0 });
    for q in EXAMPLE_SEARCHES {
        if suggestion_row(ui, icons::ICON_SEARCH, q, q, ids) {
            picks.chosen = Some(q.to_string());
        }
    }
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new("Searches never leave this computer.")
                .size(12.0)
                .color(pal.muted),
        );
        if !recent.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                picks.clear = ui
                    .add(egui::Button::new(
                        RichText::new("Clear recent searches")
                            .size(12.0)
                            .color(pal.accent),
                    ))
                    .clicked();
            });
        }
    });
    picks
}

/// A small heading of the search help, `above` points below the previous row.
fn help_heading(ui: &mut egui::Ui, pal: &Palette, text: &str, above: f32) {
    ui.add_space(above);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(RichText::new(text).size(12.0).color(pal.muted));
    });
}

impl App {
    /// Remembers a search the user ran (newest first, no repeats).
    pub(super) fn remember_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() {
            return;
        }
        let lower = q.to_lowercase();
        let mut list = self.settings.recent_searches.clone();
        if list.first().is_some_and(|f| f.to_lowercase() == lower) {
            return;
        }
        list.retain(|r| r.to_lowercase() != lower);
        list.insert(0, q.to_string());
        list.truncate(MAX_RECENT);
        self.change_settings(|s| s.recent_searches = list);
    }

    /// Runs a search the user picked from the help (or any text) right away.
    fn search_for(&mut self, text: &str) {
        self.query = text.to_string();
        self.search_now();
    }

    /// Makes the typed query count as settled, so the search starts on the next frame instead
    /// of after the typing pause. (Right after boot the clock may not reach back that far; the
    /// search then simply waits out the pause.)
    pub(super) fn search_now(&mut self) {
        self.query_changed_at = Some(
            Instant::now()
                .checked_sub(super::results::DEBOUNCE)
                .unwrap_or_else(Instant::now),
        );
    }

    /// Keyboard use of the suggestions: Tab from the field goes to the first one, Up/Down (or
    /// Left/Right) move between them, Enter runs one (a focused button does that itself) and
    /// Escape closes them.
    fn keyboard_in_suggestions(&mut self, ctx: &egui::Context, ids: &[egui::Id]) {
        let focused = ctx.memory(egui::Memory::focused);
        let current = focused.and_then(|f| ids.iter().position(|i| *i == f));
        let pressed = |k: egui::Key| ctx.input(|i| i.key_pressed(k) && !i.modifiers.any());
        let mut target = None;
        if std::mem::take(&mut self.search_help.want_chips) {
            target = ids.first().copied();
        } else if let Some(i) = current {
            if pressed(egui::Key::ArrowDown) || pressed(egui::Key::ArrowRight) {
                target = ids.get((i + 1).min(ids.len() - 1)).copied();
            } else if pressed(egui::Key::ArrowUp) || pressed(egui::Key::ArrowLeft) {
                target = ids.get(i.saturating_sub(1)).copied();
            }
        }
        if let Some(id) = target {
            ctx.memory_mut(|m| {
                // Replaces what the built-in Tab / arrow focus movement chose this frame.
                m.move_focus(egui::FocusDirection::None);
                m.request_focus(id);
            });
            ctx.request_repaint();
        }
        if pressed(egui::Key::Escape) {
            self.search_help.suppressed = true;
        }
        self.search_help.inside = target.is_some() || current.is_some();
    }

    /// Recent and example searches under the search field while it is focused and empty.
    pub(super) fn search_help_dropdown(&mut self, ctx: &egui::Context) {
        let pointer_inside = self
            .search_help
            .dropdown
            .zip(ctx.pointer_latest_pos())
            .is_some_and(|(r, p)| r.contains(p));
        let show = self.snapshot.model_installed
            && self.query.is_empty()
            && !self.dialogs.settings_open
            && !self.dialog_open()
            && !self.search_help.suppressed
            && (self.search_help.focused
                || pointer_inside
                || self.search_help.inside
                || self.search_help.want_chips);
        let Some(field) = self.search_help.field.filter(|_| show) else {
            self.search_help.dropdown = None;
            self.search_help.inside = false;
            self.search_help.want_chips = false;
            return;
        };
        let mut ids: Vec<egui::Id> = Vec::new();
        let pal = Palette::of(ctx);
        let recent = self.settings.recent_searches.clone();
        let mut picks = HelpPicks::default();
        let out = egui::Area::new(egui::Id::unique("search-help"))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::new(field.min.x, field.max.y + 6.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.pop)
                    .stroke(Stroke::new(1.0, pal.line))
                    .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                    .inner_margin(Margin::same(6))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.set_width(field.width() - 12.0);
                        picks = help_sections(ui, &pal, &recent, &mut ids);
                    });
            });
        self.search_help.dropdown = Some(out.response.rect);
        self.keyboard_in_suggestions(ctx, &ids);
        if picks.clear {
            self.change_settings(|s| s.recent_searches.clear());
        }
        if let Some(q) = picks.chosen {
            self.search_for(&q);
        }
    }
}

// --- One-time tips ---------------------------------------------------------------------

/// A short hint shown once, at the moment it is useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tip {
    Search,
    Select,
    Album,
}

impl Tip {
    pub(super) fn id(self) -> &'static str {
        match self {
            Tip::Search => "search",
            Tip::Select => "select",
            Tip::Album => "album",
        }
    }

    pub(super) fn text(self) -> &'static str {
        match self {
            Tip::Search => "Try searching for something you remember, like “sunset at the beach”.",
            Tip::Select => {
                "Hover a photo and click the circle in its corner to select it. Then click more \
                 photos, or Shift-click to select everything in between."
            }
            Tip::Album => {
                "Add photos to this album from Photos: select them and choose Add to album."
            }
        }
    }
}

impl App {
    /// The tip to show now, if any. Only one at a time, and only on a grid with photos in it.
    fn pending_tip(&self) -> Option<Tip> {
        if self.tl.grid.picking
            || !self.tl.loaded
            || self.tl.items.is_empty()
            || self.search_offer_visible()
        {
            return None;
        }
        let todo = |t: Tip| !self.settings.tips_dismissed.iter().any(|d| d == t.id());
        let in_album = matches!(self.tl.view, View::Album(_));
        [Tip::Album, Tip::Search, Tip::Select]
            .into_iter()
            .filter(|t| todo(*t))
            .find(|t| match t {
                Tip::Album => in_album,
                Tip::Search => !in_album && self.snapshot.model_installed,
                Tip::Select => self.tl.items.len() >= 2,
            })
    }

    /// The tip card above the photos.
    pub(super) fn tip_card(&mut self, ui: &mut egui::Ui) {
        let Some(tip) = self.pending_tip() else {
            return;
        };
        let pal = Palette::of(ui.ctx());
        let mut dismiss = false;
        egui::Frame::new()
            .fill(pal.accent.gamma_multiply(0.14))
            .stroke(Stroke::new(1.0, pal.accent.gamma_multiply(0.5)))
            .corner_radius(CornerRadius::same(style::CARD_RADIUS))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(style::icon_text(icons::ICON_LIGHTBULB, 22.0).color(pal.accent));
                    ui.label(tip.text());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dismiss = ui.button("Got it").clicked();
                    });
                });
            });
        ui.add_space(8.0);
        if dismiss {
            self.dismiss_tip(tip);
        }
    }

    fn dismiss_tip(&mut self, tip: Tip) {
        let id = tip.id().to_string();
        self.change_settings(|s| {
            if !s.tips_dismissed.contains(&id) {
                s.tips_dismissed.push(id);
            }
        });
    }
}
