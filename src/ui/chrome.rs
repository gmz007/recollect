//! The window's frame around the grids: the top bar, the view header, the "turn on smart
//! search" card, the help window and the toast notifications.

use super::style::{self, Palette, icons};
use super::timeline::{Group, View, thousands};
use super::{App, NoticeAction, Notices};
use crate::media::MediaKind;
use egui::{CornerRadius, Margin, Pos2, RichText, Sense, Stroke, Vec2};
use egui_material_icons::MaterialIcon;
use std::time::Instant;

/// Placeholder of the search field.
pub(super) const SEARCH_HINT: &str = "Search your photos — try “dog on the beach”";
/// What assistive technology reads for the search field while smart search is off.
pub(super) const SEARCH_OFF_LABEL: &str =
    "Search your photos (smart search is off, press to turn it on)";
/// Shown in place of the search field while smart search is not on.
pub(super) const SEARCH_OFF_HINT: &str = "Turn on smart search to find photos by describing them";

/// "1,204 photos and 37 videos", "1 photo", "12 videos"; empty when there is nothing.
pub(super) fn count_text(photos: usize, videos: usize) -> String {
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

/// One segment of the photo/video filter. Returns true when clicked.
fn segment(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    text: &str,
    accessible: &str,
    selected: bool,
) -> bool {
    let pal = Palette::of(ui.ctx());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, pal.text);
    let size = Vec2::new(galley.size().x + 44.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let fg = if selected { pal.on_accent } else { pal.text };
        if selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(7), pal.accent);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(7), pal.border);
        }
        ui.painter().text(
            Pos2::new(rect.min.x + 17.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon.codepoint,
            style::icon_font_id(icon, 17.0),
            fg,
        );
        let g = ui.painter().layout_no_wrap(
            text.to_string(),
            egui::TextStyle::Body.resolve(ui.style()),
            fg,
        );
        ui.painter().galley(
            Pos2::new(rect.min.x + 31.0, rect.center().y - g.size().y / 2.0),
            g,
            fg,
        );
    }
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, accessible)
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

impl App {
    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(style::icon_text(icons::ICON_PHOTO_LIBRARY, 26.0).color(pal.accent));
            ui.label(style::semibold("Recollect").size(20.0));
            let total = ui.available_width();
            let (right_w, seg_w) = (96.0, 236.0);
            let field_w = (total - right_w - seg_w - 120.0).clamp(220.0, 520.0);
            let lead = ((total - right_w - field_w - seg_w - 12.0) / 2.0).max(8.0);
            ui.add_space(lead);
            self.search_field(ui, field_w);
            self.type_filter(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_button_framed(ui, icons::ICON_SETTINGS, "Settings").clicked() {
                    self.open_settings();
                }
                if style::icon_button_framed(ui, icons::ICON_HELP, "Help").clicked() {
                    self.help_open = true;
                }
            });
        });
    }

    fn search_field(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = Palette::of(ui.ctx());
        let radius = CornerRadius::same(style::CONTROL_RADIUS);
        if !self.snapshot.model_installed {
            // Searching needs smart search: the field invites turning it on instead.
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
            ui.painter().rect_filled(rect, radius, pal.raised);
            ui.painter().text(
                Pos2::new(rect.min.x + 20.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                icons::ICON_SEARCH.codepoint,
                style::icon_font_id(icons::ICON_SEARCH, 20.0),
                pal.text_secondary,
            );
            let mut job = egui::text::LayoutJob::simple_singleline(
                SEARCH_OFF_HINT.to_string(),
                egui::TextStyle::Body.resolve(ui.style()),
                pal.text_secondary,
            );
            job.wrap.max_width = width - 44.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(
                Pos2::new(rect.min.x + 38.0, rect.center().y - g.size().y / 2.0),
                g,
                pal.text_secondary,
            );
            self.search_help.field = None;
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, SEARCH_OFF_LABEL)
            });
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.open_settings_at(super::settings::Section::SmartSearch);
            }
            return;
        }
        let id = egui::Id::unique("search");
        let focused = ui.memory(|m| m.has_focus(id));
        let frame = egui::Frame::new()
            .fill(pal.raised)
            .stroke(if focused {
                Stroke::new(1.5, pal.accent)
            } else {
                Stroke::NONE
            })
            .corner_radius(radius)
            .inner_margin(Margin::symmetric(10, 2));
        let mut cleared = false;
        let mut has_focus = false;
        let shown = frame.show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(style::icon_text(icons::ICON_SEARCH, 20.0).color(pal.text_secondary));
                let edit_w = width - 20.0 - 28.0 - if self.query.is_empty() { 0.0 } else { 30.0 };
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .hint_text(SEARCH_HINT)
                        // Typed text, placeholder and cursor share one vertical centre
                        // with the search icon.
                        .align(egui::Align2::LEFT_CENTER)
                        .min_size(Vec2::new(0.0, 30.0))
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
                has_focus = ui.memory(|m| m.has_focus(id));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.query_changed_at = Some(Instant::now() - super::DEBOUNCE);
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
                if !self.query.is_empty()
                    && style::icon_button(ui, icons::ICON_CLOSE, "Clear search").clicked()
                {
                    cleared = true;
                }
            });
        });
        self.search_help.field = Some(shown.response.rect);
        self.search_help.focused = has_focus;
        if has_focus {
            self.search_help.suppressed = false;
        }
        if cleared {
            self.clear_search();
        }
    }

    fn type_filter(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let before = self.kind;
        egui::Frame::new()
            .fill(pal.raised)
            .corner_radius(CornerRadius::same(style::CONTROL_RADIUS))
            .inner_margin(Margin::same(2))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal(|ui| {
                    if segment(
                        ui,
                        icons::ICON_APPS,
                        "All",
                        "Everything",
                        self.kind.is_none(),
                    ) {
                        self.kind = None;
                    }
                    if segment(
                        ui,
                        icons::ICON_PHOTO,
                        "Photos",
                        "Photos only",
                        self.kind == Some(MediaKind::Image),
                    ) {
                        self.kind = Some(MediaKind::Image);
                    }
                    if segment(
                        ui,
                        icons::ICON_MOVIE,
                        "Videos",
                        "Videos only",
                        self.kind == Some(MediaKind::Video),
                    ) {
                        self.kind = Some(MediaKind::Video);
                    }
                });
            });
        if before != self.kind {
            self.rerun();
        }
    }

    /// Title of the view the timeline area shows.
    fn view_title(&self) -> String {
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
                    .map(|(_, l)| l)
                    .unwrap_or_else(|| "Folder".to_string())
            }
            View::Album(id) => {
                super::albums::short_name(self.album_name(id).unwrap_or("Album"), 60)
            }
        }
    }

    /// Title, count and the compact controls above the grid of a timeline view.
    pub(super) fn view_header(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let title = self.view_title();
        let subtitle = count_text(self.tl.n_photos, self.tl.n_videos);
        let album = match self.tl.view {
            View::Album(id) => Some(id),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&title).text_style(egui::TextStyle::Heading));
                    if let Some(id) = album {
                        self.album_actions(ui, id);
                    }
                    if self.tl.awaiting.is_some() {
                        ui.spinner();
                    }
                });
                if !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).color(pal.text_secondary));
                } else if self.tl.error.is_some() {
                    ui.colored_label(
                        pal.error,
                        "Couldn't load your photos. Please try again in a moment.",
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(style::icon_text(icons::ICON_IMAGE, 22.0).color(pal.text_secondary));
                style::slider_scope(ui, |ui| {
                    let r = ui
                        .add(
                            egui::Slider::new(&mut self.tl.tile_size, 96.0..=320.0)
                                .show_value(false)
                                .trailing_fill(true),
                        )
                        .on_hover_text("Thumbnail size");
                    let v = self.tl.tile_size as f64;
                    r.widget_info(|| egui::WidgetInfo::slider(true, v, "Thumbnail size"));
                });
                ui.label(style::icon_text(icons::ICON_IMAGE, 14.0).color(pal.text_secondary));
                ui.label(RichText::new("Size").color(pal.text_secondary));
                ui.add_space(8.0);
                if self.tl.headers() {
                    let label = if self.tl.newest_first {
                        "Newest first"
                    } else {
                        "Oldest first"
                    };
                    style::chip(ui, &format!("Sort: {label}"), label, |ui| {
                        ui.selectable_value(&mut self.tl.newest_first, true, "Newest first");
                        ui.selectable_value(&mut self.tl.newest_first, false, "Oldest first");
                    });
                    let current = match self.tl.group {
                        Group::Day => "Days",
                        Group::Month => "Months",
                        Group::Year => "Years",
                    };
                    let job = style::dropdown_job(ui, current);
                    let r = ui.menu_button(job, |ui| {
                        for (g, name) in [
                            (Group::Day, "Days"),
                            (Group::Month, "Months"),
                            (Group::Year, "Years"),
                        ] {
                            ui.selectable_value(&mut self.tl.group, g, name);
                        }
                    });
                    r.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, current)
                    });
                }
            });
        });
        ui.add_space(8.0);
    }

    /// Header of the search results: what was asked, how many were found, and chips that
    /// narrow and order them. (The photo/video filter lives in the top bar.)
    pub(super) fn results_header(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let title = match &self.current {
            Some(super::Query::Similar { label, .. }) => format!("Photos similar to {label}"),
            Some(super::Query::Text(q)) => format!("Results for “{q}”"),
            None => format!("Results for “{}”", self.query.trim()),
        };
        let photos = self
            .results
            .iter()
            .filter(|h| h.kind == MediaKind::Image)
            .count();
        let videos = self.results.len() - photos;
        let subtitle = count_text(photos, videos);
        let year = chrono::Datelike::year(&self.today());
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(title).text_style(egui::TextStyle::Heading))
                            .truncate(),
                    );
                    if self.awaiting.is_some() {
                        ui.spinner();
                    }
                });
                // An empty result list shows the problem itself; don't say it twice.
                if let Some(e) = self
                    .result_error
                    .as_ref()
                    .filter(|_| !self.results.is_empty())
                {
                    ui.colored_label(pal.error, &e.text);
                } else if self.result_error.is_none() && !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).color(pal.text_secondary));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if style::icon_button_framed(ui, icons::ICON_CLOSE, "Clear search").clicked() {
                    self.clear_search();
                }
                changed |= super::when::sort_ui(ui, &mut self.sort);
                changed |= self.date.ui(ui, &self.known_years, year);
            });
        });
        if changed {
            self.rerun();
        }
        ui.add_space(8.0);
    }

    /// One line above the grid that offers smart search until the user decides. "Not now" is
    /// remembered; after that only the small card in the sidebar offers it.
    pub(super) fn search_banner(&mut self, ui: &mut egui::Ui) {
        if self.settings.search_prompt_dismissed
            || self.snapshot.model_installed
            || matches!(
                self.status.activity,
                crate::library::Activity::Downloading { .. }
            )
        {
            return;
        }
        let pal = Palette::of(ui.ctx());
        let spec = self.lib.model();
        let (mut turn_on, mut not_now, mut learn_more) = (false, false, false);
        egui::Frame::new()
            .fill(pal.surface)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(CornerRadius::same(style::CARD_RADIUS))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(style::icon_text(icons::ICON_AUTO_AWESOME, 20.0).color(pal.accent));
                    ui.label(SEARCH_OFF_HINT);
                    let on = egui::Button::new(RichText::new("Turn on").color(pal.on_accent))
                        .fill(pal.accent);
                    turn_on = ui
                        .add(on)
                        .on_hover_text(format!(
                            "A one-time download of {} MB. Search then works offline, and \
                             your photos never leave this computer.",
                            spec.size_mb()
                        ))
                        .clicked();
                    not_now = ui.button("Not now").clicked();
                    learn_more = ui.link("Learn more").clicked();
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

    /// The current notice as a card at the bottom of the window.
    pub(super) fn toasts(&mut self, ctx: &egui::Context) {
        let Some((text, error, details, action)) = self
            .notices
            .current_notice()
            .map(|n| (n.text.clone(), n.error, n.details.clone(), n.action))
        else {
            return;
        };
        let pal = Palette::of(ctx);
        let mut dismiss = false;
        let mut retry = None;
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.raised)
                    .stroke(Stroke::new(1.0, if error { pal.error } else { pal.border }))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(16, 10))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.horizontal(|ui| {
                            let (icon, color) = if error {
                                (icons::ICON_ERROR, pal.error)
                            } else {
                                (icons::ICON_CHECK_CIRCLE, pal.accent)
                            };
                            ui.label(style::icon_text(icon, 22.0).color(color));
                            ui.vertical(|ui| {
                                ui.label(&text);
                                if let Some(a) = action
                                    && ui.button("Try again").clicked()
                                {
                                    retry = Some(a);
                                }
                                if let Some(d) = &details {
                                    egui::CollapsingHeader::new("Details")
                                        .id_salt("toast-details")
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(d).small());
                                        });
                                }
                            });
                            if style::icon_button(ui, icons::ICON_CLOSE, "Dismiss").clicked() {
                                dismiss = true;
                            }
                        });
                    });
            });
        if dismiss || retry.is_some() {
            self.notices.dismiss_current();
        }
        if let Some(a) = retry {
            self.retry(a);
        }
    }
}

impl Notices {
    /// Takes the message that [`Notices::current`] shows away (the toast's close button).
    pub(super) fn dismiss_current(&mut self) {
        if self.items.iter().any(|n| n.error) {
            self.items.retain(|n| !n.error);
        } else {
            self.items.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::count_text;

    #[test]
    fn counts_read_naturally() {
        assert_eq!(count_text(1204, 37), "1,204 photos and 37 videos");
        assert_eq!(count_text(1, 1), "1 photo and 1 video");
        assert_eq!(count_text(12, 0), "12 photos");
        assert_eq!(count_text(0, 1), "1 video");
        assert_eq!(count_text(0, 0), "");
    }
}
