//! The window's frame around the grids: the top bar, the view header, the "turn on smart
//! search" card, the help window and the toast notifications.

use super::style::{self, Palette, icons};
use super::timeline::{Group, View, thousands};
use super::{App, Notices};
use crate::media::MediaKind;
use crate::model::registry;
use egui::{CornerRadius, Margin, Pos2, RichText, Sense, Stroke, Vec2};
use egui_material_icons::MaterialIcon;
use std::time::Instant;

/// Placeholder of the search field.
pub(super) const SEARCH_HINT: &str = "Search your photos — try “dog on the beach”";
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

/// A text with a drop-down arrow, for menu buttons.
fn dropdown_job(ui: &egui::Ui, label: &str) -> egui::text::LayoutJob {
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Button.resolve(ui.style()),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        icons::ICON_ARROW_DROP_DOWN.codepoint,
        2.0,
        egui::TextFormat {
            font_id: style::icon_font_id(icons::ICON_ARROW_DROP_DOWN, 20.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job
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
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, SEARCH_OFF_HINT)
            });
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.open_settings();
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
            .inner_margin(Margin::symmetric(10, 1));
        let mut cleared = false;
        frame.show(ui, |ui| {
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
                        .desired_width(edit_w),
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Search your photos")
                });
                if resp.changed() {
                    self.query_changed_at = Some(Instant::now());
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.query_changed_at = Some(Instant::now() - super::DEBOUNCE);
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
                } else if let Some(e) = &self.tl.error {
                    ui.colored_label(pal.error, format!("Couldn't load your photos. {e}"));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(style::icon_text(icons::ICON_IMAGE, 22.0).color(pal.text_secondary));
                ui.add(
                    egui::Slider::new(&mut self.tl.tile_size, 96.0..=320.0)
                        .show_value(false)
                        .trailing_fill(false),
                )
                .on_hover_text("Thumbnail size");
                ui.label(style::icon_text(icons::ICON_IMAGE, 14.0).color(pal.text_secondary));
                ui.add_space(8.0);
                if self.tl.headers() {
                    let label = if self.tl.newest_first {
                        "Newest first"
                    } else {
                        "Oldest first"
                    };
                    if style::icon_button_framed(ui, icons::ICON_SWAP_VERT, label).clicked() {
                        self.tl.newest_first = !self.tl.newest_first;
                    }
                    let current = match self.tl.group {
                        Group::Day => "Days",
                        Group::Month => "Months",
                        Group::Year => "Years",
                    };
                    let job = dropdown_job(ui, current);
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

    /// Header of the search results.
    pub(super) fn results_header(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let title = match &self.current {
            Some(super::Query::Similar { label, .. }) => format!("Similar to “{label}”"),
            _ => format!("Results for “{}”", self.result_label),
        };
        let photos = self
            .results
            .iter()
            .filter(|h| h.kind == MediaKind::Image)
            .count();
        let videos = self.results.len() - photos;
        let subtitle = count_text(photos, videos);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).text_style(egui::TextStyle::Heading));
                    if self.awaiting.is_some() {
                        ui.spinner();
                    }
                });
                if let Some(e) = &self.result_error {
                    ui.colored_label(pal.error, e);
                } else if !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).color(pal.text_secondary));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_label_button(ui, icons::ICON_CLOSE, "Clear search").clicked() {
                    self.clear_search();
                }
            });
        });
        ui.add_space(4.0);
    }

    /// The card that explains smart search and asks to turn it on, shown above the grid until
    /// the user decides.
    pub(super) fn search_banner(&mut self, ui: &mut egui::Ui) {
        if self.consent_dismissed
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
        let quality = if spec.id == registry::SIGLIP_B16.id {
            " (best quality)"
        } else {
            " (faster, a little less accurate)"
        };
        style::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(style::icon_text(icons::ICON_AUTO_AWESOME, 24.0).color(pal.accent));
                ui.label(style::semibold("Turn on smart search").size(18.0));
            });
            ui.add_space(4.0);
            ui.label(format!(
                "Recollect can find photos when you describe them, like “birthday cake”. \
                 This needs a one-time download of {} MB{quality} and works offline afterwards. \
                 Your photos never leave this computer.",
                spec.total_size() / 1_000_000
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let on = egui::Button::new(RichText::new("Turn on").color(pal.on_accent))
                    .fill(pal.accent);
                if ui.add(on).clicked() {
                    self.turn_on_search();
                }
                if ui.button("Not now").clicked() {
                    self.consent_dismissed = true;
                }
            });
            egui::CollapsingHeader::new("Details")
                .id_salt("banner-details")
                .show(ui, |ui| {
                    ui.label(format!("{}, {} licence", spec.name, spec.license));
                    ui.label(format!("Downloaded once from huggingface.co/{}", spec.repo));
                    if ui.button("Choose another…").clicked() {
                        self.open_settings();
                    }
                });
        });
        ui.add_space(12.0);
    }

    /// Starts the one-time download of smart search.
    pub(super) fn turn_on_search(&mut self) {
        if let Err(e) = self.lib.start_download() {
            self.notify_error(format!("Couldn't start the download. {e:#}"));
        }
    }

    /// A short explanation and the keyboard shortcuts.
    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.help_open;
        egui::Window::new("Help")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(style::semibold("Finding photos").size(16.0));
                ui.add_space(4.0);
                ui.label(
                    "Type a few words about what you remember, like “dog on the beach” or \
                     “birthday cake”, and Recollect shows the photos and videos that look most \
                     like it. Everything happens on this computer, so it keeps working \
                     offline and nothing is uploaded.",
                );
                ui.add_space(12.0);
                ui.label(style::semibold("Keyboard shortcuts").size(16.0));
                ui.add_space(4.0);
                egui::Grid::new("shortcuts")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (keys, what) in SHORTCUTS {
                            ui.label(style::semibold(*keys));
                            ui.label(*what);
                            ui.end_row();
                        }
                    });
            });
        self.help_open = open;
    }

    /// The current notice as a card at the bottom of the window.
    pub(super) fn toasts(&mut self, ctx: &egui::Context) {
        let Some((text, error)) = self
            .notices
            .current()
            .map(|t| (t.to_string(), self.notices.current_is_error()))
        else {
            return;
        };
        let pal = Palette::of(ctx);
        let mut dismiss = false;
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
                            ui.label(&text);
                            if error
                                && style::icon_button(ui, icons::ICON_CLOSE, "Dismiss").clicked()
                            {
                                dismiss = true;
                            }
                        });
                    });
            });
        if dismiss {
            self.notices.dismiss_errors();
        }
    }
}

/// Keys and what they do, for the help window.
const SHORTCUTS: &[(&str, &str)] = &[
    ("Arrow keys", "Move between photos"),
    ("Enter", "Open the photo you are on"),
    ("Esc", "Go back, or clear the selection"),
    ("Ctrl+click (Cmd on Mac)", "Select several photos"),
    ("Shift+click", "Select everything in between"),
    ("Ctrl+A (Cmd on Mac)", "Select everything"),
    ("F", "Mark as favorite, or remove the mark"),
    ("Ctrl+F (Cmd on Mac)", "Go to the search box"),
    (
        "Left and right arrows",
        "Previous and next photo while viewing",
    ),
    ("I", "Show or hide photo details while viewing"),
    ("Z", "Zoom in and out while viewing"),
    ("Home and End", "First and last photo while viewing"),
];

impl Notices {
    /// True when the notice [`Notices::current`] shows is an error.
    pub(super) fn current_is_error(&self) -> bool {
        self.items.iter().any(|n| n.error)
    }

    /// Takes the error away (the toast's close button).
    pub(super) fn dismiss_errors(&mut self) {
        self.items.retain(|n| !n.error);
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
