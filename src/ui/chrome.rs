//! The window's frame around the grids: the top bar, the view header, the "turn on smart
//! search" card, the help window and the toast notifications.

use super::style::{self, Palette, icons};
use super::timeline::{View, thousands};
use super::{App, NoticeAction};
use crate::library::Activity;
use crate::media::MediaKind;
use egui::{CornerRadius, Margin, Pos2, RichText, Sense, Stroke, Vec2};
use std::time::Instant;

/// The widest the search field gets, in points.
const SEARCH_MAX_WIDTH: f32 = 520.0;
/// Height of the search field.
const SEARCH_HEIGHT: f32 = 34.0;
/// Size of the heading of the search results ("36 results").
const HEADING_SIZE: f32 = 15.0;
/// Size of the quiet note beside the filters.
const NOTE_SIZE: f32 = 12.5;
/// Placeholder of the search field while smart search is on.
const SEARCH_HINT: &str = "Describe a moment…";
/// What assistive technology reads for the search field while smart search is off.
pub(super) const SEARCH_OFF_LABEL: &str =
    "Search your photos (smart search is off, press to turn it on)";
/// Shown in place of the search field while smart search is not on.
const SEARCH_OFF_HINT: &str = "Turn on smart search to find photos by describing them";

/// "1,204 photos and 37 videos", "1 photo", "12 videos"; empty when there is nothing.
fn count_text(photos: usize, videos: usize) -> String {
    let p = |n: usize| {
        format!(
            "{} {}",
            thousands(n),
            if n == 1 { "photo" } else { "photos" }
        )
    };
    let v = |n: usize| {
        format!(
            "{} {}",
            thousands(n),
            if n == 1 { "video" } else { "videos" }
        )
    };
    match (photos, videos) {
        (0, 0) => String::new(),
        (p_, 0) => p(p_),
        (0, v_) => v(v_),
        (p_, v_) => format!("{} and {}", p(p_), v(v_)),
    }
}

/// Under an album's name: "214 photos, 3 videos · 2019 – 2026". The years are left out when
/// they are not known, and a single year stands alone.
fn album_subtitle(photos: usize, videos: usize, years: Option<(i32, i32)>) -> String {
    let n = |n: usize, one: &str, many: &str| {
        format!("{} {}", thousands(n), if n == 1 { one } else { many })
    };
    let mut parts = Vec::new();
    if photos > 0 {
        parts.push(n(photos, "photo", "photos"));
    }
    if videos > 0 {
        parts.push(n(videos, "video", "videos"));
    }
    let text = if parts.is_empty() {
        "No photos yet".to_string()
    } else {
        parts.join(", ")
    };
    match years {
        Some((from, to)) if from == to => format!("{text} · {from}"),
        Some((from, to)) => format!("{text} · {from} – {to}"),
        None => text,
    }
}

/// What happened in the search field's text box this frame.
#[derive(Default)]
struct SearchEdit {
    has_focus: bool,
    cleared: bool,
}

impl App {
    /// The top bar: the search field in the middle, Help and Settings on the right.
    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        // The sheets must not stack on top of a dialog, Settings or a menu.
        let sheets_allowed = !(self.dialogs.settings_open
            || self.dialog_open()
            || egui::Popup::is_any_open(ui.ctx()));
        if sheets_allowed && ui.input(|i| i.key_pressed(egui::Key::F1)) {
            self.dialogs.help_open = true;
        }
        if sheets_allowed && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Comma)) {
            self.open_settings();
        }
        let total = ui.available_width();
        let icons_w = 2.0 * style::ICON_BUTTON + 4.0;
        // Centred in the bar, and narrower when the window is.
        let field_w = (total - 2.0 * (icons_w + 16.0)).clamp(180.0, SEARCH_MAX_WIDTH);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(((total - field_w) / 2.0).max(0.0));
            self.search_field(ui, field_w);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let settings = format!("Settings ({}+,)", style::command_key());
                if style::top_bar_button(ui, icons::ICON_SETTINGS.outlined(), "Settings", &settings)
                    .clicked()
                {
                    self.open_settings();
                }
                if style::top_bar_button(ui, icons::ICON_HELP.outlined(), "Help", "Help (F1)")
                    .clicked()
                {
                    self.dialogs.help_open = true;
                }
            });
        });
    }

    /// The search field while smart search is off: a prompt that opens Settings.
    fn search_field_off(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = Palette::of(ui.ctx());
        let radius = CornerRadius::same(style::FIELD_RADIUS);
        // Searching needs smart search: the field invites turning it on instead.
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, SEARCH_HEIGHT), Sense::click());
        ui.painter().rect_filled(rect, radius, pal.field);
        ui.painter().text(
            Pos2::new(rect.min.x + 20.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icons::ICON_SEARCH.codepoint,
            style::icon_font_id(icons::ICON_SEARCH, 16.0),
            pal.text2,
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            SEARCH_OFF_HINT.to_string(),
            egui::TextStyle::Body.resolve(ui.style()),
            pal.text2,
        );
        job.wrap.max_width = width - 44.0;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter().galley(
            Pos2::new(rect.min.x + 38.0, rect.center().y - g.size().y / 2.0),
            g,
            pal.text2,
        );
        self.search_help.field = None;
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, SEARCH_OFF_LABEL));
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.open_settings_at(super::settings::Section::SmartSearch);
        }
    }

    fn search_field(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = Palette::of(ui.ctx());
        let radius = CornerRadius::same(style::FIELD_RADIUS);
        if !self.snapshot.model_installed {
            self.search_field_off(ui, width);
            return;
        }
        let id = egui::Id::unique("search");
        let focused = ui.memory(|m| m.has_focus(id));
        let frame = egui::Frame::new()
            .fill(pal.field)
            .stroke(if focused {
                Stroke::new(1.5, pal.accent)
            } else {
                Stroke::NONE
            })
            .corner_radius(radius)
            .inner_margin(Margin::symmetric(10, 0));
        let mut edit = SearchEdit::default();
        let shown = frame.show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.horizontal(|ui| edit = self.search_edit_row(ui, id, width));
        });
        // A click anywhere on the field (icon, padding) lands in the text box. (Asked for after the
        // text box was drawn: a click beside it makes it let go of the focus first.)
        let clicked_on_field = ui.input(|i| i.pointer.primary_clicked())
            && ui.rect_contains_pointer(shown.response.rect);
        if clicked_on_field {
            ui.memory_mut(|m| m.request_focus(id));
            edit.has_focus = true;
        }
        self.search_help.field = Some(shown.response.rect);
        self.search_help.focused = edit.has_focus;
        if edit.has_focus {
            self.search_help.suppressed = false;
        }
        if edit.cleared {
            self.clear_search();
        }
    }

    /// The icon, the text box and the clear button inside the search field's frame.
    fn search_edit_row(&mut self, ui: &mut egui::Ui, id: egui::Id, width: f32) -> SearchEdit {
        let pal = Palette::of(ui.ctx());
        let mut edit = SearchEdit::default();
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(style::icon_text(icons::ICON_SEARCH, 16.0).color(pal.muted));
        let edit_w = width - 20.0 - 28.0 - if self.query.is_empty() { 0.0 } else { 30.0 };
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(id)
                .frame(egui::Frame::NONE)
                .hint_text(SEARCH_HINT)
                // Typed text, placeholder and cursor share one vertical centre
                // with the search icon.
                .align(egui::Align2::LEFT_CENTER)
                .min_size(Vec2::new(0.0, SEARCH_HEIGHT))
                .desired_width(edit_w),
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Search your photos")
        });
        if resp.changed() {
            self.query_changed_at = Some(Instant::now());
        }
        // (Asked of the memory, like the frame's border: `Response::has_focus` is also
        // false when the window itself is not the active one.)
        edit.has_focus = ui.memory(|m| m.has_focus(id));
        self.search_field_keys(ui, &resp);
        if !self.query.is_empty()
            && style::icon_button(ui, icons::ICON_CLOSE, "Clear search").clicked()
        {
            edit.cleared = true;
        }
        edit
    }

    /// Enter searches now, Tab moves to the suggestions and Ctrl/Cmd+F focuses the field.
    fn search_field_keys(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.search_now();
        }
        // Tab moves into the suggestions under the field, when they are showing.
        if resp.lost_focus()
            && self.search_help.dropdown.is_some()
            && ui.input(|i| i.key_pressed(egui::Key::Tab) && !i.modifiers.any())
        {
            self.search_help.want_chips = true;
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            resp.request_focus();
        }
    }

    /// The All / Photos / Videos chips of the search results.
    fn type_chips(&mut self, ui: &mut egui::Ui) -> bool {
        let before = self.kind;
        for (kind, text, accessible) in [
            (None, "All", "Everything"),
            (Some(MediaKind::Image), "Photos", "Photos only"),
            (Some(MediaKind::Video), "Videos", "Videos only"),
        ] {
            if style::filter_chip(ui, text, accessible, self.kind == kind) {
                self.kind = kind;
            }
        }
        before != self.kind
    }

    /// Title of the view the timeline area shows.
    pub(super) fn view_title(&self) -> String {
        match self.tl.view {
            View::Timeline => "Photos".to_string(),
            View::RecentlyAdded => "Recently added".to_string(),
            View::Favorites => "Favorites".to_string(),
            View::Folder(id) => {
                let labels =
                    super::folder_labels(self.snapshot.folders.iter().map(|f| f.path.as_path()));
                self.snapshot
                    .folders
                    .iter()
                    .zip(labels)
                    .find(|(f, _)| f.id == id)
                    .map_or_else(|| "Folder".to_string(), |(_, l)| l)
            }
            View::Album(id) => {
                super::albums::short_name(self.album_name(id).unwrap_or("Album"), 60)
            }
        }
    }

    /// The title of an album or folder with its count (and the album's actions); for the other
    /// views only a problem with loading, if there is one.
    pub(super) fn view_header(&mut self, ui: &mut egui::Ui) {
        let album = match self.tl.view {
            View::Album(id) => Some(id),
            _ => None,
        };
        if album.is_none() && !matches!(self.tl.view, View::Folder(_)) {
            if self.tl.error.is_some() {
                let pal = Palette::of(ui.ctx());
                ui.colored_label(
                    pal.danger,
                    "Couldn't load your photos. Please try again in a moment.",
                );
                ui.add_space(8.0);
            }
            return;
        }
        let title = self.view_title();
        let subtitle = if album.is_some() {
            album_subtitle(self.tl.n_photos, self.tl.n_videos, self.tl.years)
        } else {
            count_text(self.tl.n_photos, self.tl.n_videos)
        };
        ui.horizontal(|ui| {
            self.view_heading(ui, &title, &subtitle);
            if let Some(id) = album {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    self.album_actions(ui, id);
                });
            }
        });
        ui.add_space(8.0);
    }

    /// The view's title and under it the count (or the load error).
    fn view_heading(&mut self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        let pal = Palette::of(ui.ctx());
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).text_style(egui::TextStyle::Heading));
                if self.tl.awaiting.is_some() {
                    ui.spinner();
                }
            });
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).color(pal.text2));
            } else if self.tl.error.is_some() {
                ui.colored_label(
                    pal.danger,
                    "Couldn't load your photos. Please try again in a moment.",
                );
            }
        });
    }

    /// The heading of the results: how many are shown (after the filters), or "Similar
    /// photos". `None` while the search failed and there is nothing to count.
    fn results_heading(&self) -> Option<String> {
        if matches!(self.res.current, Some(super::Query::Similar { .. })) {
            return Some("Similar photos".to_string());
        }
        let n = self.res.hits.len();
        if n == 0 && self.res.awaiting.is_some() {
            return Some("Searching…".to_string());
        }
        if n == 0 && self.res.error.is_some() {
            return None;
        }
        Some(format!(
            "{} {}",
            thousands(n),
            if n == 1 { "result" } else { "results" }
        ))
    }

    /// While photos are still being made searchable: how many are left, so that a short list
    /// of results is not taken for the whole answer.
    fn preparing_note(&self) -> Option<String> {
        let Activity::Indexing(p) = &self.status.activity else {
            return None;
        };
        let left = p.total.saturating_sub(p.done);
        (left > 0).then(|| {
            format!(
                "Still getting {} {} ready \u{2014} results may grow",
                thousands(left),
                if left == 1 { "photo" } else { "photos" }
            )
        })
    }

    /// The quiet note at the right of the filters.
    fn preparing_note_ui(ui: &mut egui::Ui, note: &str) {
        let pal = Palette::of(ui.ctx());
        ui.add(egui::Label::new(RichText::new(note).size(NOTE_SIZE).color(pal.muted)).truncate());
        ui.label(style::icon_text(icons::ICON_INFO.outlined(), NOTE_SIZE + 1.0).color(pal.muted));
    }

    /// Header of the search results: how many there are, and chips that narrow and order
    /// them (type, date, sort).
    pub(super) fn results_header(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let year = chrono::Datelike::year(&self.today());
        let mut changed = false;
        ui.horizontal(|ui| {
            if let Some(title) = self.results_heading() {
                ui.add(egui::Label::new(
                    style::semibold(title).size(HEADING_SIZE).color(pal.text),
                ));
                // What the count is for, quieter: the words can scroll out of the field.
                if let Some(super::Query::Text(q)) = &self.res.current {
                    ui.add(
                        egui::Label::new(RichText::new(format!("for “{q}”")).color(pal.muted))
                            .truncate(),
                    );
                }
            }
            if self.res.awaiting.is_some() {
                ui.spinner();
            }
            // An empty result list shows the problem itself; don't say it twice.
            if let Some(e) = (self.res.error.as_ref()).filter(|_| !self.res.hits.is_empty()) {
                ui.colored_label(pal.danger, &e.text);
            }
        });
        ui.add_space(6.0);
        let note = self.preparing_note();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            changed |= self.type_chips(ui);
            ui.add_space(4.0);
            changed |= self.date.ui(ui, &self.known_years, year);
            changed |= super::when::sort_ui(ui, &mut self.sort);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The search field has its own clear button while it holds text; without text (results
                // of "similar photos") this is the only way out.
                if self.query.is_empty() {
                    let r = ui.button("Clear");
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, "Clear search")
                    });
                    if r.clicked() {
                        self.clear_search();
                    }
                }
                if let Some(note) = &note {
                    Self::preparing_note_ui(ui, note);
                }
            });
        });
        if changed {
            self.rerun();
        }
        ui.add_space(8.0);
    }

    /// Whether the banner that offers smart search is to be shown. It takes the place of any tip:
    /// only one banner is shown above the grid at a time.
    pub(super) fn search_offer_visible(&self) -> bool {
        !(self.settings.search_prompt_dismissed
            || self.snapshot.model_installed
            || matches!(
                self.status.activity,
                crate::library::Activity::Downloading { .. }
            ))
    }

    /// One line above the grid that offers smart search until the user decides. "Not now" is
    /// remembered; after that only the small card in the sidebar offers it.
    pub(super) fn search_banner(&mut self, ui: &mut egui::Ui) {
        if !self.search_offer_visible() {
            return;
        }
        let pal = Palette::of(ui.ctx());
        let spec = self.lib.model();
        let (mut turn_on, mut not_now, mut learn_more) = (false, false, false);
        egui::Frame::new()
            .fill(pal.side)
            .stroke(Stroke::new(1.0, pal.line))
            .corner_radius(CornerRadius::same(style::CARD_RADIUS))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(style::icon_text(icons::ICON_AUTO_AWESOME, 20.0).color(pal.accent));
                    ui.label(SEARCH_OFF_HINT);
                    // The buttons and the link wrap together, so a large text size never
                    // splits them across lines one by one.
                    ui.horizontal(|ui| {
                        let on = egui::Button::new(RichText::new("Turn on").color(pal.onaccent))
                            .fill(pal.accent);
                        turn_on = ui
                            .add(on)
                            .on_hover_text(format!(
                                "A one-time download of {} MB. Search then works offline, \
                                 and your photos never leave this computer.",
                                spec.size_mb()
                            ))
                            .clicked();
                        not_now = ui.button("Not now").clicked();
                        learn_more = ui.link("Learn more").clicked();
                    });
                });
            });
        if turn_on {
            self.turn_on_search();
        }
        if not_now {
            self.set_search_prompt_dismissed(true);
        }
        if learn_more {
            self.open_settings_at(super::settings::Section::SmartSearch);
        }
        ui.add_space(10.0);
    }

    /// Starts the one-time download of smart search.
    pub(super) fn turn_on_search(&mut self) {
        if let Err(e) = self.lib.start_download() {
            self.notify_problem(
                "Couldn't start the download. Please try again in a moment.",
                format!("{e:#}"),
                Some(NoticeAction::Download),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{album_subtitle, count_text};

    #[test]
    fn an_albums_subtitle_counts_and_gives_the_years() {
        assert_eq!(
            album_subtitle(214, 3, Some((2019, 2026))),
            "214 photos, 3 videos · 2019 – 2026"
        );
        assert_eq!(album_subtitle(1, 0, Some((2021, 2021))), "1 photo · 2021");
        assert_eq!(album_subtitle(0, 12, None), "12 videos");
        assert_eq!(album_subtitle(0, 0, None), "No photos yet");
    }

    #[test]
    fn counts_read_naturally() {
        assert_eq!(count_text(1204, 37), "1,204 photos and 37 videos");
        assert_eq!(count_text(1, 1), "1 photo and 1 video");
        assert_eq!(count_text(12, 0), "12 photos");
        assert_eq!(count_text(0, 1), "1 video");
        assert_eq!(count_text(0, 0), "");
    }
}
