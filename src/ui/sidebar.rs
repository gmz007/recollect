//! The left sidebar: the app mark, views, albums, folders and, at the bottom, a ring that
//! shows what Recollect is busy with.

use super::folders::StopFolder;
use super::style::{self, Palette, icons};
use super::timeline::{View, thousands};
use super::workers::ThumbCache;
use super::{App, folder_labels};
use crate::library::Activity;
use crate::model::registry::megabytes;
use crate::thumbs::ThumbKey;
use egui::{Color32, CornerRadius, Margin, Pos2, Rect, Response, RichText, Sense, Vec2};
use egui_material_icons::MaterialIcon;

/// One line of the activity card: what is happening in plain words and, when it is known, how
/// far along it is (0 to 1).
fn activity_text(a: &Activity) -> Option<(String, Option<f32>)> {
    Some(match a {
        Activity::Idle => return None,
        Activity::Downloading { done, total, .. } => (
            format!(
                "Downloading smart search… {} of {} MB",
                thousands(usize::try_from(megabytes(*done)).unwrap_or(usize::MAX)),
                thousands(usize::try_from(megabytes(*total)).unwrap_or(usize::MAX))
            ),
            Some(*done as f32 / (*total).max(1) as f32),
        ),
        Activity::Scanning { .. } => ("Looking for new photos…".to_string(), None),
        Activity::ReadingDates { done, total } => (
            format!(
                "Reading photo dates… {} of {}",
                thousands(*done),
                thousands(*total)
            ),
            Some(*done as f32 / (*total).max(1) as f32),
        ),
        Activity::LoadingModel => ("Getting smart search ready…".to_string(), None),
        Activity::SwitchingModel => ("Switching smart search…".to_string(), None),
        Activity::Indexing(p) => {
            let f = p.done as f32 / p.total.max(1) as f32;
            (
                format!(
                    "Getting your photos ready for search… {} %",
                    (f * 100.0).floor() as u32
                ),
                Some(f),
            )
        }
    })
}

/// Width of the sidebar.
const SIDEBAR_WIDTH: f32 = 216.0;
/// Height of a sidebar row, and of one with a second line.
const ROW_HEIGHT: f32 = 32.0;
const TWO_LINE_ROW_HEIGHT: f32 = 44.0;
/// Side of the album cover in a row.
const COVER: f32 = 20.0;
/// Left edge of a row's label, from the row's left edge.
const LABEL_X: f32 = 38.0;

/// What a sidebar row shows in front of its label.
pub(super) enum Leading<'a> {
    Icon(MaterialIcon),
    /// An album cover; a neutral square while there is none or it is loading.
    Cover(Option<&'a ThumbKey>),
}

/// A sidebar row.
pub(super) struct Row<'a> {
    pub leading: Leading<'a>,
    pub text: &'a str,
    /// Second line in small muted text (a disconnected drive).
    pub note: Option<&'a str>,
    /// What assistive technology reads.
    pub accessible: &'a str,
    pub selected: bool,
    /// Space kept free at the right (for a button drawn over the row).
    pub reserve_right: f32,
    /// Small muted text at the right (the path of a suggested folder).
    pub trailing: Option<&'a str>,
    /// The label is drawn muted (a folder that is not connected).
    pub dimmed: bool,
}

impl<'a> Row<'a> {
    pub(super) fn new(leading: Leading<'a>, text: &'a str, selected: bool) -> Self {
        Self {
            leading,
            text,
            note: None,
            accessible: text,
            selected,
            reserve_right: 0.0,
            trailing: None,
            dimmed: false,
        }
    }
}

/// A row with an icon and a label, for lists in sheets and dialogs. `trailing` is small muted
/// text at the right.
pub(super) fn plain_row(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    text: &str,
    accessible: &str,
    trailing: Option<&str>,
    selected: bool,
) -> (Rect, Response) {
    let mut row = Row::new(Leading::Icon(icon), text, selected);
    row.accessible = accessible;
    row.trailing = trailing;
    nav_row(ui, None, &row)
}

/// A full-width sidebar row. Returns the row's rectangle and response.
pub(super) fn nav_row(
    ui: &mut egui::Ui,
    thumbs: Option<&mut ThumbCache>,
    row: &Row<'_>,
) -> (Rect, Response) {
    let height = if row.note.is_some() {
        TWO_LINE_ROW_HEIGHT
    } else {
        ROW_HEIGHT
    };
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if ui.is_rect_visible(rect) {
        paint_nav_row_background(ui, rect, resp.hovered(), row.selected);
        paint_leading(ui, thumbs, rect, &row.leading);
        paint_nav_row_text(ui, rect, row);
    }
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, row.selected, row.accessible)
    });
    (rect, resp.on_hover_cursor(egui::CursorIcon::PointingHand))
}

/// The row's background: grey when selected (never the accent), lighter grey on hover.
fn paint_nav_row_background(ui: &egui::Ui, rect: Rect, hovered: bool, selected: bool) {
    let pal = Palette::of(ui.ctx());
    let radius = CornerRadius::same(style::CONTROL_RADIUS);
    if selected {
        ui.painter().rect_filled(rect, radius, pal.sel);
    } else if hovered {
        ui.painter().rect_filled(rect, radius, pal.hover);
    }
}

/// The icon or the album cover at the row's left.
fn paint_leading(
    ui: &egui::Ui,
    thumbs: Option<&mut ThumbCache>,
    rect: Rect,
    leading: &Leading<'_>,
) {
    let pal = Palette::of(ui.ctx());
    let centre = Pos2::new(rect.min.x + 20.0, rect.center().y);
    match leading {
        Leading::Icon(icon) => {
            let icon = icon.outlined();
            ui.painter().text(
                centre,
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                style::icon_font_id(icon, 18.0),
                pal.text2,
            );
        }
        Leading::Cover(key) => {
            let cover = Rect::from_center_size(centre, Vec2::splat(COVER));
            let radius = CornerRadius::same(4);
            let texture = key.and_then(|k| thumbs?.get(k));
            if let Some(tex) = texture {
                let shape = egui::epaint::RectShape::filled(cover, radius, Color32::WHITE)
                    .with_texture(tex.id(), super::tiles::square_uv(tex.size_vec2()));
                ui.painter().add(shape);
            } else {
                ui.painter().rect_filled(cover, radius, pal.skel);
            }
        }
    }
}

/// The row's label (and second line), cut short with an ellipsis to fit what is left.
fn paint_nav_row_text(ui: &egui::Ui, rect: Rect, row: &Row<'_>) {
    let pal = Palette::of(ui.ctx());
    let painter = ui.painter();
    let fg = if row.dimmed { pal.muted } else { pal.text };
    let mut right = rect.max.x - 8.0 - row.reserve_right;
    if let Some(t) = row.trailing {
        let g = painter.layout_no_wrap(
            t.to_string(),
            egui::TextStyle::Small.resolve(ui.style()),
            pal.muted,
        );
        right -= g.size().x;
        painter.galley(
            Pos2::new(right, rect.center().y - g.size().y / 2.0),
            g,
            pal.muted,
        );
        right -= 8.0;
    }
    let max_w = (right - (rect.min.x + LABEL_X)).max(20.0);
    let line = |text: &str, font: egui::FontId, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
        job.wrap = egui::text::TextWrapping {
            max_width: max_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        painter.layout_job(job)
    };
    let main = line(row.text, egui::TextStyle::Body.resolve(ui.style()), fg);
    let note = row
        .note
        .map(|n| line(n, egui::TextStyle::Small.resolve(ui.style()), pal.muted));
    let total = main.size().y + note.as_ref().map_or(0.0, |n| n.size().y);
    let x = rect.min.x + LABEL_X;
    let mut y = rect.center().y - total / 2.0;
    let main_height = main.size().y;
    painter.galley(Pos2::new(x, y), main, fg);
    y += main_height;
    if let Some(n) = note {
        painter.galley(Pos2::new(x, y), n, pal.muted);
    }
}

/// A section title (12, muted) with an optional "+" style button on its right. Returns the
/// buttons' responses.
pub(super) fn section_header(
    ui: &mut egui::Ui,
    title: &str,
    buttons: &mut [(MaterialIcon, &str, bool)],
) -> Vec<Response> {
    let pal = Palette::of(ui.ctx());
    let mut out = Vec::new();
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            RichText::new(title)
                .text_style(egui::TextStyle::Small)
                .color(pal.muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (icon, label, enabled) in buttons.iter().rev() {
                ui.add_enabled_ui(*enabled, |ui| {
                    out.push(style::small_icon_button(ui, *icon, label));
                });
            }
        });
    });
    out.reverse();
    out
}

/// A folder's row with the small "x" that stops showing it. A folder whose drive is not
/// connected says so on a second line. Returns the row's response and whether the "x" was
/// clicked.
fn folder_row(
    ui: &mut egui::Ui,
    thumbs: &mut ThumbCache,
    name: &str,
    selected: bool,
    connected: bool,
) -> (Response, bool) {
    let pal = Palette::of(ui.ctx());
    let mut row = Row::new(Leading::Icon(icons::ICON_FOLDER), name, selected);
    row.reserve_right = 28.0;
    if !connected {
        row.leading = Leading::Icon(icons::ICON_FOLDER_OFF);
        row.note = Some("Not connected");
        row.dimmed = true;
    }
    let (rect, resp) = nav_row(ui, Some(thumbs), &row);
    let x_rect = Rect::from_center_size(
        Pos2::new(rect.max.x - 18.0, rect.center().y),
        Vec2::splat(26.0),
    );
    let x = ui.interact(x_rect, resp.id.with("stop"), Sense::click());
    let label = format!("Stop showing {name}");
    x.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    // Also while it has keyboard focus, or Tab would land on an invisible button.
    if ui.rect_contains_pointer(rect) || x.has_focus() {
        ui.painter().text(
            x_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::ICON_CLOSE.codepoint,
            style::icon_font_id(icons::ICON_CLOSE, 16.0),
            if x.hovered() { pal.text } else { pal.muted },
        );
    }
    let x = x.on_hover_text(format!("Stop showing “{name}”"));
    (resp, x.clicked())
}

/// A short name for what is going on, for the foot of the sidebar.
fn short_label(a: &Activity) -> &'static str {
    match a {
        Activity::Idle => "",
        Activity::Downloading { .. } => "Downloading",
        Activity::Scanning { .. } => "Checking photos",
        Activity::ReadingDates { .. } => "Reading dates",
        Activity::LoadingModel | Activity::SwitchingModel => "Getting ready",
        Activity::Indexing(_) => "Preparing search",
    }
}

/// Whether the user can pause what the activity is doing.
fn is_pausable(a: &Activity) -> bool {
    matches!(
        a,
        Activity::Scanning { .. }
            | Activity::ReadingDates { .. }
            | Activity::LoadingModel
            | Activity::Indexing(_)
    )
}

/// The foot's label: "Paused" while the work is paused, else what is going on.
fn row_label(a: &Activity, paused: bool) -> &'static str {
    if paused && is_pausable(a) {
        "Paused"
    } else {
        short_label(a)
    }
}

/// A small ring: the `ring` track with an accent arc for `fraction`, or a turning arc when the
/// progress is not known (and the work is not paused).
fn paint_progress_ring(
    ui: &egui::Ui,
    centre: Pos2,
    radius: f32,
    fraction: Option<f32>,
    paused: bool,
) {
    let pal = Palette::of(ui.ctx());
    let width = 2.5;
    ui.painter()
        .circle_stroke(centre, radius, egui::Stroke::new(width, pal.ring));
    let (start, sweep) = if let Some(f) = fraction {
        (
            -std::f32::consts::FRAC_PI_2,
            f.clamp(0.0, 1.0) * std::f32::consts::TAU,
        )
    } else {
        let t = if paused {
            0.0
        } else {
            ui.ctx().request_repaint();
            ui.input(|i| i.time) as f32
        };
        (t * std::f32::consts::TAU, std::f32::consts::PI * 0.75)
    };
    let steps = 32;
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = start + sweep * i as f32 / steps as f32;
            centre + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect();
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(width, pal.accent),
    ));
}

impl App {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let frame = egui::Frame::new()
            .fill(pal.side)
            .inner_margin(Margin::symmetric(8, 12));
        egui::Panel::left("library")
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .show_separator_line(false)
            .frame(frame)
            .show(ui, |ui| {
                Self::sidebar_header(ui);
                self.sidebar_bottom(ui);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.sidebar_body(ui));
            });
    }

    /// The app mark and name.
    fn sidebar_header(ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.set_height(ROW_HEIGHT);
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(10.0);
            style::app_mark(ui, 20.0, pal.side);
            ui.label(style::semibold("Recollect").size(14.0).color(pal.text));
        });
        ui.add_space(6.0);
    }

    fn sidebar_body(&mut self, ui: &mut egui::Ui) {
        let mut pick = None;
        for (view, icon, label) in [
            (View::Timeline, icons::ICON_PHOTO_LIBRARY, "Photos"),
            (View::RecentlyAdded, icons::ICON_SCHEDULE, "Recently added"),
            (View::Favorites, icons::ICON_FAVORITE, "Favorites"),
        ] {
            let row = Row::new(Leading::Icon(icon), label, self.tl.view == view);
            let (_, r) = nav_row(ui, Some(&mut self.thumbs), &row);
            if r.clicked() {
                pick = Some(view);
            }
        }
        if let Some(v) = pick {
            self.set_view(v);
        }

        self.albums_section(ui);
        self.folders_section(ui);
        self.failed_files(ui);
    }

    fn folders_section(&mut self, ui: &mut egui::Ui) {
        let buttons = section_header(
            ui,
            "Folders",
            &mut [(icons::ICON_ADD, "Add a folder", true)],
        );
        if buttons[0].clicked() {
            self.open_add_folder();
        }
        if self.snapshot.folders.is_empty() {
            let pal = Palette::of(ui.ctx());
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new("No folders yet.").color(pal.muted));
            });
            return;
        }
        let labels = folder_labels(self.snapshot.folders.iter().map(|f| f.path.as_path()));
        let mut open = None;
        let mut stop = None;
        for (f, name) in self.snapshot.folders.iter().zip(labels) {
            let connected = !self.snapshot.missing_folders.contains(&f.id);
            let (resp, stop_clicked) = folder_row(
                ui,
                &mut self.thumbs,
                &name,
                self.tl.view == View::Folder(f.id),
                connected,
            );
            if stop_clicked {
                stop = Some(StopFolder {
                    id: f.id,
                    name: name.clone(),
                });
            } else if resp.on_hover_text(f.path.display().to_string()).clicked() {
                open = Some(f.id);
            }
        }
        if let Some(id) = open {
            self.set_view(View::Folder(id));
        }
        if stop.is_some() {
            self.dialogs.stop_folder = stop;
        }
    }

    /// "N files couldn't be opened", with a way to try them again.
    fn failed_files(&mut self, ui: &mut egui::Ui) {
        let failed = self.snapshot.stats.errors;
        if failed == 0 {
            return;
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new(format!(
            "{} {} couldn't be opened",
            thousands(failed),
            if failed == 1 { "file" } else { "files" }
        ))
        .id_salt("errors")
        .show(ui, |ui| self.failed_files_list(ui, "errors-list", 180.0));
    }

    /// The names of the files that couldn't be opened (the full path and the reason show on
    /// hover) and a "Try again" button; shared by the sidebar and Settings.
    pub(super) fn failed_files_list(&mut self, ui: &mut egui::Ui, id: &str, max_height: f32) {
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(max_height)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (p, e) in &self.snapshot.errors {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.add(egui::Label::new(RichText::new(name).small()).truncate())
                        .on_hover_text(format!("{}\n{e}", p.display()));
                }
            });
        if ui
            .add_enabled(!self.lib.is_busy(), egui::Button::new("Try again"))
            .clicked()
        {
            self.retry_failed();
        }
    }

    /// The foot: what is happening now (a ring with a short label and the percentage; click
    /// for details), or an invitation to turn smart search on, and the privacy line.
    fn sidebar_bottom(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let activity = activity_text(&self.status.activity);
        let offer = activity.is_none()
            && !self.snapshot.model_installed
            && !self.snapshot.folders.is_empty()
            && self.settings.search_prompt_dismissed;
        egui::Panel::bottom("sidebar_bottom")
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ui.add_space(8.0);
                if let Some((text, fraction)) = activity {
                    self.activity_row(ui, &text, fraction);
                } else if offer {
                    let card = egui::Frame::new()
                        .fill(pal.field)
                        .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                        .inner_margin(Margin::same(12));
                    card.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(style::semibold("Smart search is off").size(13.0));
                        ui.add_space(4.0);
                        if ui.button("Turn on").clicked() {
                            self.set_search_prompt_dismissed(false);
                        }
                    });
                }
                ui.add_space(8.0);
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(10, 0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Your photos stay on this computer")
                                .small()
                                .color(pal.muted),
                        );
                    });
            });
    }

    /// The ring, a short label and the percentage; clicking opens the details.
    fn activity_row(&mut self, ui: &mut egui::Ui, text: &str, fraction: Option<f32>) {
        let pal = Palette::of(ui.ctx());
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(style::CONTROL_RADIUS), pal.hover);
        }
        let paused = self.status.paused && is_pausable(&self.status.activity);
        paint_progress_ring(
            ui,
            Pos2::new(rect.min.x + 20.0, rect.center().y),
            7.5,
            fraction,
            paused,
        );
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().text(
            Pos2::new(rect.min.x + LABEL_X, rect.center().y),
            egui::Align2::LEFT_CENTER,
            row_label(&self.status.activity, paused),
            font.clone(),
            pal.text2,
        );
        if let Some(f) = fraction {
            ui.painter().text(
                Pos2::new(rect.max.x - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{}%", (f * 100.0).floor() as u32),
                font,
                pal.muted,
            );
        }
        let accessible = if paused {
            format!("Progress: Paused — {text}")
        } else {
            format!("Progress: {text}")
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &accessible));
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::from_toggle_button_response(&resp)
            .align(egui::RectAlign::TOP_START)
            .width(SIDEBAR_WIDTH - 16.0)
            .show(|ui| self.activity_details(ui, text, fraction));
    }

    /// What is going on in words, with Pause and Stop.
    fn activity_details(&mut self, ui: &mut egui::Ui, text: &str, fraction: Option<f32>) {
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        let pausable = is_pausable(&self.status.activity);
        let paused = self.status.paused;
        let shown = if paused && pausable {
            format!("Paused — {text}")
        } else {
            text.to_string()
        };
        ui.label(shown);
        ui.add_space(4.0);
        let bar = egui::ProgressBar::new(fraction.map_or(0.0, |f| f.clamp(0.0, 1.0)))
            .desired_height(6.0)
            .corner_radius(3.0);
        ui.add(if fraction.is_some() {
            bar
        } else {
            bar.animate(true)
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if pausable {
                let label = if paused { "Resume" } else { "Pause" };
                if ui.button(label).clicked() {
                    self.lib.pause(!paused);
                }
            }
            if ui.button("Stop").clicked() {
                self.lib.cancel();
                if downloading {
                    self.set_search_prompt_dismissed(true);
                }
            }
        });
    }

    /// Marks the files that couldn't be opened for another try and starts looking at them.
    fn retry_failed(&mut self) {
        match self.lib.retry_errors() {
            Ok(_) => self.start_indexing(),
            Err(_) => self.notify_error("Couldn't try those files again. Please try in a moment."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::IndexProgress;

    #[test]
    fn activity_is_described_in_plain_words() {
        assert_eq!(activity_text(&Activity::Idle), None);
        assert_eq!(row_label(&Activity::LoadingModel, true), "Paused");
        assert_eq!(row_label(&Activity::LoadingModel, false), "Getting ready");
        // A download cannot be paused, so the flag does not change its label.
        let dl = Activity::Downloading {
            done: 0,
            total: 1,
            model: String::new(),
        };
        assert_eq!(row_label(&dl, true), "Downloading");
        let scanning = Activity::Scanning {
            folder: "/x".into(),
        };
        assert_eq!(
            activity_text(&scanning).unwrap().0,
            "Looking for new photos…"
        );
        let dates = Activity::ReadingDates {
            done: 1234,
            total: 5000,
        };
        assert_eq!(
            activity_text(&dates).unwrap().0,
            "Reading photo dates… 1,234 of 5,000"
        );
        let idx = Activity::Indexing(IndexProgress {
            done: 45,
            total: 100,
            ..Default::default()
        });
        let (t, f) = activity_text(&idx).unwrap();
        assert_eq!(t, "Getting your photos ready for search… 45 %");
        assert_eq!(f, Some(0.45));
        let dl = Activity::Downloading {
            model: "m".into(),
            done: 120_000_000,
            total: 485_000_000,
        };
        assert_eq!(
            activity_text(&dl).unwrap().0,
            "Downloading smart search… 120 of 485 MB"
        );
    }
}
