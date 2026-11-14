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
    /// The star icon (the symbol itself may be missing from the text font).
    Star,
}

/// The screen for a view with nothing in it: an icon, one sentence and one main button.
/// Returns true when the button was clicked.
pub(super) fn empty_state(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    sentence: &[Part<'_>],
    button: &str,
) -> bool {
    let pal = Palette::of(ui.ctx());
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
            Part::Star => {
                job.append(
                    icons::ICON_STAR.codepoint,
                    0.0,
                    egui::TextFormat {
                        font_id: style::icon_font_id(icons::ICON_STAR, 20.0),
                        color: pal.favorite,
                        valign: egui::Align::Center,
                        ..Default::default()
                    },
                );
                plain.push_str("star");
            }
        }
    }
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).min(140.0));
        ui.label(style::icon_text(icon, 56.0).color(pal.text_secondary));
        ui.add_space(12.0);
        let r = ui.add(egui::Label::new(job));
        // Assistive technology reads the whole sentence, with the star named.
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
            View::Album(_) => empty_state(
                ui,
                icons::ICON_PHOTO_ALBUM,
                &[Part::Text(
                    "This album is empty. Select photos and choose Add to album.",
                )],
                "Go to Photos",
            ),
            View::Favorites => empty_state(
                ui,
                icons::ICON_STAR,
                &[
                    Part::Text("Click the "),
                    Part::Star,
                    Part::Text(" on any photo to add it here."),
                ],
                "Go to Photos",
            ),
            View::RecentlyAdded => {
                if empty_state(
                    ui,
                    icons::ICON_SCHEDULE,
                    &[Part::Text(
                        "Photos and videos you add to your folders show up here.",
                    )],
                    "Check for new photos",
                ) {
                    self.start_indexing();
                }
                false
            }
            View::Folder(_) => {
                if empty_state(
                    ui,
                    icons::ICON_FOLDER_OPEN,
                    &[Part::Text("There are no photos or videos in this folder.")],
                    "Check for new photos",
                ) {
                    self.start_indexing();
                }
                false
            }
            View::Timeline => {
                if empty_state(
                    ui,
                    icons::ICON_PHOTO_LIBRARY,
                    &[Part::Text(
                        "No photos or videos yet. Add a folder, or give Recollect a moment to \
                         look through the ones you have.",
                    )],
                    "Add a folder",
                ) {
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
            if !self.snapshot.model_installed {
                if empty_state(
                    ui,
                    icons::ICON_AUTO_AWESOME,
                    &[Part::Text(
                        "Smart search is off. Turn it on to find photos by describing them.",
                    )],
                    "Turn on smart search",
                ) {
                    self.open_settings_at(super::settings::Section::SmartSearch);
                }
            } else if e.can_retry {
                if empty_state(
                    ui,
                    icons::ICON_SEARCH_OFF,
                    &[Part::Text(&e.text)],
                    "Try again",
                ) {
                    self.rerun();
                }
            } else if empty_state(
                ui,
                icons::ICON_SEARCH_OFF,
                &[Part::Text(&e.text)],
                "Start over",
            ) {
                self.clear_search();
            }
            return;
        }
        if self.res.label.is_empty() {
            return;
        }
        let date_set = self.date != super::when::DateFilter::default();
        let sentence = format!(
            "No photos match “{}”. Try different words{}.",
            super::albums::short_name(&self.res.label, 40),
            if date_set {
                ", or check the When filter"
            } else {
                ""
            }
        );
        let button = if date_set {
            "Clear the date filter"
        } else {
            "Start over"
        };
        if empty_state(
            ui,
            icons::ICON_IMAGE_SEARCH,
            &[Part::Text(&sentence)],
            button,
        ) {
            if date_set {
                self.date = super::when::DateFilter::default();
                self.rerun();
            } else {
                self.clear_search();
            }
        }
    }
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

/// One small clickable suggestion.
fn suggestion_chip(
    ui: &mut egui::Ui,
    icon: Option<MaterialIcon>,
    text: &str,
    name: &str,
    ids: &mut Vec<egui::Id>,
) -> bool {
    let pal = Palette::of(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    if let Some(icon) = icon {
        job.append(
            icon.codepoint,
            0.0,
            egui::TextFormat {
                font_id: style::icon_font_id(icon, 16.0),
                color: pal.text_secondary,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
    }
    job.append(
        text,
        if icon.is_some() { 6.0 } else { 0.0 },
        egui::TextFormat {
            font_id: egui::TextStyle::Body.resolve(ui.style()),
            color: pal.text,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    let b = egui::Button::new(job)
        .fill(pal.raised)
        .corner_radius(CornerRadius::same(16))
        .min_size(Vec2::new(0.0, 30.0));
    let r = ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    ids.push(r.id);
    r.clicked()
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
    pub(super) fn search_for(&mut self, text: &str) {
        self.query = text.to_string();
        self.search_now();
    }

    /// Makes the typed query count as settled, so the search starts on the next frame instead
    /// of after the typing pause. (Right after boot the clock may not reach back that far; the
    /// search then simply waits out the pause.)
    pub(super) fn search_now(&mut self) {
        self.query_changed_at = Some(
            Instant::now()
                .checked_sub(super::DEBOUNCE)
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

    /// Examples and recent searches under the search field while it is focused and empty.
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
        let (mut chosen, mut clear) = (None::<String>, false);
        let out = egui::Area::new(egui::Id::unique("search-help"))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::new(field.min.x, field.max.y + 6.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.surface)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.set_width(field.width().max(480.0) - 28.0);
                        ui.label(
                            style::semibold("Try searching for")
                                .size(13.5)
                                .color(pal.text_secondary),
                        );
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                            for q in EXAMPLE_SEARCHES {
                                if suggestion_chip(ui, None, q, q, &mut ids) {
                                    chosen = Some(q.to_string());
                                }
                            }
                        });
                        if !recent.is_empty() {
                            ui.add_space(10.0);
                            ui.label(
                                style::semibold("Recent searches")
                                    .size(13.5)
                                    .color(pal.text_secondary),
                            );
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                                for q in &recent {
                                    let name = format!("Recent search: {q}");
                                    if suggestion_chip(
                                        ui,
                                        Some(icons::ICON_HISTORY),
                                        q,
                                        &name,
                                        &mut ids,
                                    ) {
                                        chosen = Some(q.clone());
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            clear = ui
                                .add(egui::Button::new(
                                    RichText::new("Clear recent searches").color(pal.accent),
                                ))
                                .clicked();
                        }
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Your searches stay on this computer.")
                                .small()
                                .color(pal.text_secondary),
                        );
                    });
            });
        self.search_help.dropdown = Some(out.response.rect);
        self.keyboard_in_suggestions(ctx, &ids);
        if clear {
            self.change_settings(|s| s.recent_searches.clear());
        }
        if let Some(q) = chosen {
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
            Tip::Select => "Hover a photo and click the circle to select several.",
            Tip::Album => {
                "Add photos to this album from Photos: select them and choose Add to album."
            }
        }
    }
}

impl App {
    /// The tip to show now, if any. Only one at a time, and only on a grid with photos in it.
    pub(super) fn pending_tip(&self) -> Option<Tip> {
        if self.tl.grid.picking || !self.tl.loaded || self.tl.items.is_empty() {
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

    pub(super) fn dismiss_tip(&mut self, tip: Tip) {
        let id = tip.id().to_string();
        self.change_settings(|s| {
            if !s.tips_dismissed.contains(&id) {
                s.tips_dismissed.push(id);
            }
        });
    }
}
