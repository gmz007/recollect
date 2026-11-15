//! The left sidebar: views, albums, folders, the date jumper and, at the bottom, a card that
//! says what Recollect is busy with.

use super::folders::StopFolder;
use super::style::{self, Palette, icons};
use super::timeline::{View, thousands};
use super::{App, folder_labels};
use crate::library::Activity;
use crate::model::registry::megabytes;
use egui::{CornerRadius, Margin, Pos2, Rect, Response, RichText, Sense, Vec2};
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

/// A full-width sidebar row. Returns the row's rectangle and response.
pub(super) fn nav_row(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    text: &str,
    accessible: &str,
    trailing: Option<&str>,
    selected: bool,
    reserve_right: f32,
) -> (Rect, Response) {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if ui.is_rect_visible(rect) {
        paint_nav_row_icon(ui, rect, resp.hovered(), icon, selected);
        paint_nav_row_text(ui, rect, text, trailing, selected, reserve_right);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, accessible));
    (rect, resp.on_hover_cursor(egui::CursorIcon::PointingHand))
}

/// The row's background (selected or hovered) and its icon.
fn paint_nav_row_icon(
    ui: &egui::Ui,
    rect: Rect,
    hovered: bool,
    icon: MaterialIcon,
    selected: bool,
) {
    let pal = Palette::of(ui.ctx());
    let painter = ui.painter();
    let radius = CornerRadius::same(style::CONTROL_RADIUS);
    if selected {
        painter.rect_filled(rect, radius, pal.accent.gamma_multiply(0.22));
    } else if hovered {
        painter.rect_filled(rect, radius, pal.raised);
    }
    painter.text(
        Pos2::new(rect.min.x + 20.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        icon.codepoint,
        style::icon_font_id(icon, 20.0),
        if selected {
            pal.accent
        } else {
            pal.text_secondary
        },
    );
}

/// The row's trailing text, right-aligned, and its label, cut short with an ellipsis to fit
/// what is left.
fn paint_nav_row_text(
    ui: &egui::Ui,
    rect: Rect,
    text: &str,
    trailing: Option<&str>,
    selected: bool,
    reserve_right: f32,
) {
    let pal = Palette::of(ui.ctx());
    let painter = ui.painter();
    let fg = if selected { pal.accent } else { pal.text };
    let mut right = rect.max.x - 10.0 - reserve_right;
    if let Some(t) = trailing {
        let g = painter.layout_no_wrap(
            t.to_string(),
            egui::TextStyle::Small.resolve(ui.style()),
            pal.text_secondary,
        );
        right -= g.size().x;
        painter.galley(
            Pos2::new(right, rect.center().y - g.size().y / 2.0),
            g,
            pal.text_secondary,
        );
        right -= 8.0;
    }
    let max_w = (right - (rect.min.x + 38.0)).max(20.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_string(),
        egui::TextStyle::Body.resolve(ui.style()),
        fg,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_w,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let g = painter.layout_job(job);
    painter.galley(
        Pos2::new(rect.min.x + 38.0, rect.center().y - g.size().y / 2.0),
        g,
        fg,
    );
}

/// A section title with an optional icon button on its right. Returns the button's response.
pub(super) fn section_header(
    ui: &mut egui::Ui,
    title: &str,
    buttons: &mut [(MaterialIcon, &str, bool)],
) -> Vec<Response> {
    let pal = Palette::of(ui.ctx());
    let mut out = Vec::new();
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(style::semibold(title).size(13.5).color(pal.text_secondary));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (icon, label, enabled) in buttons.iter().rev() {
                ui.add_enabled_ui(*enabled, |ui| {
                    out.push(style::icon_button(ui, *icon, label));
                });
            }
        });
    });
    out.reverse();
    out
}

/// A folder's row with the small "x" that stops showing it. Returns the row's response and
/// whether the "x" was clicked.
fn folder_row(ui: &mut egui::Ui, pal: Palette, name: &str, selected: bool) -> (Response, bool) {
    let (rect, resp) = nav_row(ui, icons::ICON_FOLDER, name, name, None, selected, 28.0);
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
            if x.hovered() {
                pal.text
            } else {
                pal.text_secondary
            },
        );
    }
    let x = x.on_hover_text(format!("Stop showing “{name}”"));
    (resp, x.clicked())
}

/// The thin progress bar of the activity card: filled to `fraction`, or animated when unknown.
fn activity_progress(ui: &mut egui::Ui, fraction: Option<f32>) {
    ui.add_space(4.0);
    let bar = egui::ProgressBar::new(fraction.map_or(0.0, |f| f.clamp(0.0, 1.0)))
        .desired_height(6.0)
        .corner_radius(3.0);
    ui.add(if fraction.is_some() {
        bar
    } else {
        bar.animate(true)
    });
}

impl App {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let frame = egui::Frame::new()
            .fill(pal.surface)
            .inner_margin(Margin::symmetric(10, 12));
        egui::Panel::left("library")
            .resizable(true)
            .default_size(250.0)
            .min_size(210.0)
            .frame(frame)
            .show(ui, |ui| {
                self.sidebar_bottom(ui);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.sidebar_body(ui));
            });
    }

    fn sidebar_body(&mut self, ui: &mut egui::Ui) {
        let mut pick = None;
        for (view, icon, label) in [
            (View::Timeline, icons::ICON_PHOTO_LIBRARY, "Photos"),
            (View::RecentlyAdded, icons::ICON_SCHEDULE, "Recently added"),
            (View::Favorites, icons::ICON_STAR, "Favorites"),
        ] {
            let (_, r) = nav_row(ui, icon, label, label, None, self.tl.view == view, 0.0);
            if r.clicked() {
                pick = Some(view);
            }
        }
        if let Some(v) = pick {
            self.set_view(v);
        }

        self.albums_section(ui);
        self.folders_section(ui);
        ui.add_space(10.0);
        self.date_navigator(ui);
        self.failed_files(ui);
    }

    fn folders_section(&mut self, ui: &mut egui::Ui) {
        if !self.folders_header(ui) {
            ui.label(RichText::new("No folders yet.").weak());
            return;
        }
        let pal = Palette::of(ui.ctx());
        let labels = folder_labels(self.snapshot.folders.iter().map(|f| f.path.as_path()));
        let mut open = None;
        let mut stop = None;
        for (f, name) in self.snapshot.folders.iter().zip(labels) {
            let (resp, stop_clicked) =
                folder_row(ui, pal, &name, self.tl.view == View::Folder(f.id));
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

    /// The "Folders" title with its refresh and add buttons. Returns whether there are folders.
    fn folders_header(&mut self, ui: &mut egui::Ui) -> bool {
        let busy = self.lib.is_busy();
        let has_folders = !self.snapshot.folders.is_empty();
        let buttons = section_header(
            ui,
            "Folders",
            &mut [
                (
                    icons::ICON_REFRESH,
                    "Check for new photos",
                    !busy && has_folders,
                ),
                (icons::ICON_ADD, "Add a folder", true),
            ],
        );
        if buttons[0].clicked() {
            self.start_indexing();
        }
        if buttons[1].clicked() {
            self.open_add_folder();
        }
        has_folders
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

    /// The card at the bottom: what is happening now, or an invitation to turn smart search on.
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
                    self.activity_card(ui, &text, fraction);
                } else if offer {
                    let card = egui::Frame::new()
                        .fill(pal.raised)
                        .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                        .inner_margin(Margin::same(12));
                    card.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(style::semibold("Smart search is off").size(14.0));
                        ui.add_space(4.0);
                        if ui.button("Turn on").clicked() {
                            self.set_search_prompt_dismissed(false);
                        }
                    });
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Your photos stay on this computer")
                        .small()
                        .color(pal.text_secondary),
                );
            });
    }

    fn activity_card(&mut self, ui: &mut egui::Ui, text: &str, fraction: Option<f32>) {
        let pal = Palette::of(ui.ctx());
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        let pausable = matches!(
            self.status.activity,
            Activity::Scanning { .. }
                | Activity::ReadingDates { .. }
                | Activity::LoadingModel
                | Activity::Indexing(_)
        );
        let paused = self.status.paused;
        let card = egui::Frame::new()
            .fill(pal.raised)
            .corner_radius(CornerRadius::same(style::CARD_RADIUS))
            .inner_margin(Margin::same(12));
        card.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let shown = if paused && pausable {
                format!("Paused — {text}")
            } else {
                text.to_string()
            };
            ui.label(shown);
            activity_progress(ui, fraction);
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
