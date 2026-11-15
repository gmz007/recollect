//! The left sidebar: views, albums, folders, the date jumper and, at the bottom, a card that
//! says what Recollect is busy with. Also the "Add a folder" dialog and the confirmation for
//! taking a folder out of the library.

use super::albums::{Confirmation, confirm_dialog, modal_window};
use super::style::{self, Palette, icons};
use super::timeline::{View, thousands};
use super::{App, folder_labels};
use crate::library::Activity;
use crate::model::registry::megabytes;
use egui::{CornerRadius, Margin, Pos2, Rect, Response, RichText, Sense, Vec2};
use egui_material_icons::MaterialIcon;
use std::path::PathBuf;

/// State of the "Add a folder" dialog.
#[derive(Debug, Default)]
pub(super) struct AddFolder {
    typed: String,
    /// Standard folders that are not in the library yet, worked out once when the dialog opens
    /// (looking them up touches the disk).
    suggestions: Vec<(String, PathBuf)>,
}

/// A folder waiting for the user to confirm that it stops being shown.
#[derive(Debug, Clone)]
pub(super) struct StopFolder {
    pub id: i64,
    pub name: String,
}

/// Standard folders (Pictures, Videos, Desktop, Downloads) that exist on this computer, with
/// the names the dialog shows.
pub(super) fn standard_folders() -> Vec<(String, PathBuf)> {
    let Some(dirs) = directories::UserDirs::new() else {
        return Vec::new();
    };
    [
        ("Pictures", dirs.picture_dir()),
        ("Videos", dirs.video_dir()),
        ("Desktop", dirs.desktop_dir()),
        ("Downloads", dirs.download_dir()),
    ]
    .into_iter()
    .filter_map(|(name, p)| {
        p.filter(|p| p.is_dir())
            .map(|p| (name.to_string(), p.to_path_buf()))
    })
    .collect()
}

/// The `candidates` that are not in the library yet: not added themselves and not inside a
/// folder that is.
pub(super) fn folder_suggestions(
    candidates: Vec<(String, PathBuf)>,
    added: &[PathBuf],
) -> Vec<(String, PathBuf)> {
    candidates
        .into_iter()
        .filter(|(_, c)| !added.iter().any(|a| c.starts_with(a)))
        .collect()
}

/// Shown when a typed folder path is not an existing folder given in full.
pub(super) const BAD_FOLDER_PATH: &str = "That folder couldn't be found. Type the full path \
     of the folder, or use the folder picker instead.";

/// The folder named by typed text: it must be a full (absolute) path to an existing folder, so
/// that what gets added never depends on where the program happens to be running.
pub(super) fn typed_folder(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text.trim());
    (path.is_absolute() && path.is_dir()).then_some(path)
}

/// `paths` without repeats and without those inside another of them (the parent already
/// covers them), in order.
pub(super) fn without_nested(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .enumerate()
        .filter(|(i, p)| !paths[..*i].contains(p))
        .map(|(_, p)| p)
        .filter(|p| {
            let others: Vec<PathBuf> = paths.iter().filter(|q| q != p).cloned().collect();
            !folder_suggestions(vec![(String::new(), (*p).clone())], &others).is_empty()
        })
        .cloned()
        .collect()
}

/// One line of the activity card: what is happening in plain words and, when it is known, how
/// far along it is (0 to 1).
pub(super) fn activity_text(a: &Activity) -> Option<(String, Option<f32>)> {
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

impl AddFolder {
    /// How many suggestions the dialog lists.
    #[cfg(test)]
    pub(super) fn suggestion_count(&self) -> usize {
        self.suggestions.len()
    }
}

impl App {
    /// Opens the "Add a folder" dialog with suggestions from this computer's standard folders.
    pub(super) fn open_add_folder(&mut self) {
        self.open_add_folder_with(standard_folders());
    }

    /// Opens the dialog suggesting those of `candidates` that are not in the library yet.
    pub(super) fn open_add_folder_with(&mut self, candidates: Vec<(String, PathBuf)>) {
        let added: Vec<PathBuf> = self
            .snapshot
            .folders
            .iter()
            .map(|f| f.path.clone())
            .collect();
        self.dialogs.add_folder_dialog = Some(AddFolder {
            typed: String::new(),
            suggestions: folder_suggestions(candidates, &added),
        });
    }

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
    pub(super) fn retry_failed(&mut self) {
        match self.lib.retry_errors() {
            Ok(_) => self.start_indexing(),
            Err(_) => self.notify_error("Couldn't try those files again. Please try in a moment."),
        }
    }

    /// Adds a folder to the library (it is scanned in the background).
    pub(super) fn add_folder(&mut self, p: PathBuf) {
        self.lib.start_add_folder(p);
        self.snapshot_read.mark_again();
    }

    /// The "Add a folder" dialog and the "Stop showing" confirmation.
    pub(super) fn folder_dialogs(&mut self, ctx: &egui::Context) {
        self.add_folder_dialog(ctx);
        self.stop_folder_dialog(ctx);
    }

    fn add_folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.dialogs.add_folder_dialog.take() else {
            return;
        };
        let suggestions = std::mem::take(&mut dlg.suggestions);
        let mut outcome = AddFolderOutcome::default();
        modal_window(ctx, "Add a folder", |ui| {
            outcome = add_folder_body(ui, &mut dlg.typed, &suggestions);
        });
        if outcome.bad_path {
            self.notify_error(BAD_FOLDER_PATH);
        }
        if let Some(p) = outcome.chosen {
            self.add_folder(p);
        } else if !outcome.close {
            dlg.suggestions = suggestions;
            self.dialogs.add_folder_dialog = Some(dlg);
        }
    }

    fn stop_folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(s) = self.dialogs.stop_folder.clone() else {
            return;
        };
        let choice = confirm_dialog(
            ctx,
            "Stop showing this folder?",
            "Stop showing",
            "Cancel",
            true,
            |ui| {
                ui.label(format!("Stop showing “{}”?", s.name));
                ui.label(RichText::new("Your files stay where they are.").weak());
                ui.add_space(8.0);
            },
        );
        match choice {
            Some(Confirmation::Confirmed) => {
                // Cancels any running job first; runs in the background.
                self.lib.start_remove_folder(s.id);
                self.snapshot.folders.retain(|f| f.id != s.id);
                // A fetch already in flight may predate the removal: fetch again after it.
                self.snapshot_read.mark_again();
                self.dialogs.stop_folder = None;
            }
            Some(Confirmation::Cancelled) => self.dialogs.stop_folder = None,
            None => {}
        }
    }
}

/// What the "Add a folder" dialog asked for in one frame.
#[derive(Debug, Default)]
struct AddFolderOutcome {
    chosen: Option<PathBuf>,
    /// The typed text is not a usable folder path.
    bad_path: bool,
    close: bool,
}

/// What the typed-path row did in one frame.
enum TypedFolder {
    Nothing,
    Bad,
    Folder(PathBuf),
}

/// The contents of the "Add a folder" window.
fn add_folder_body(
    ui: &mut egui::Ui,
    typed: &mut String,
    suggestions: &[(String, PathBuf)],
) -> AddFolderOutcome {
    let mut out = AddFolderOutcome {
        chosen: pick_folder_rows(ui, suggestions),
        ..Default::default()
    };
    let (typed_folder, cancel) = typed_folder_row(ui, typed);
    match typed_folder {
        TypedFolder::Nothing => {}
        TypedFolder::Bad => {
            out.chosen = None;
            out.bad_path = true;
        }
        TypedFolder::Folder(p) => out.chosen = Some(p),
    }
    out.close = cancel;
    out
}

/// The explanation, the folder picker button and the suggested folders. Returns the folder
/// that was chosen, if any.
fn pick_folder_rows(ui: &mut egui::Ui, suggestions: &[(String, PathBuf)]) -> Option<PathBuf> {
    let mut chosen = None;
    ui.set_width(420.0);
    ui.label(
        "Choose where your photos and videos are. Recollect only reads them; \
         it never changes or moves your files.",
    );
    ui.add_space(8.0);
    if style::icon_label_button(ui, icons::ICON_FOLDER_OPEN, "Choose a folder…").clicked()
        && let Some(dir) = rfd::FileDialog::new()
            .set_title("Choose a folder with photos and videos")
            .pick_folder()
    {
        chosen = Some(dir);
    }
    if !suggestions.is_empty() {
        ui.add_space(12.0);
        ui.label(style::semibold("Suggestions").size(14.0));
        for (name, path) in suggestions {
            let (_, r) = nav_row(
                ui,
                icons::ICON_FOLDER,
                name,
                &format!("Add {name}"),
                Some(&path.display().to_string()),
                false,
                0.0,
            );
            if r.clicked() {
                chosen = Some(path.clone());
            }
        }
    }
    chosen
}

/// The "or type a folder path" field with the Add and Cancel buttons. Returns what Add did and
/// whether Cancel was clicked.
fn typed_folder_row(ui: &mut egui::Ui, typed: &mut String) -> (TypedFolder, bool) {
    ui.add_space(12.0);
    ui.label("or type a folder path");
    let r = ui.add(
        egui::TextEdit::singleline(typed)
            .hint_text("For example /home/you/Pictures")
            .desired_width(f32::INFINITY),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Folder path"));
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    let (mut added, mut cancel) = (TypedFolder::Nothing, false);
    ui.horizontal(|ui| {
        let can_add = !typed.trim().is_empty();
        if (ui.add_enabled(can_add, egui::Button::new("Add")).clicked() || enter) && can_add {
            added = typed_folder(typed).map_or(TypedFolder::Bad, TypedFolder::Folder);
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    (added, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::IndexProgress;

    #[test]
    fn a_typed_folder_must_be_a_full_path_to_an_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let full = dir.path().to_str().unwrap();
        assert_eq!(typed_folder(&format!("  {full} ")), Some(dir.path().into()));
        assert_eq!(typed_folder("."), None);
        assert_eq!(typed_folder("Pictures"), None);
        assert_eq!(typed_folder(&format!("{full}/missing")), None);
        assert_eq!(typed_folder(""), None);
    }

    #[test]
    fn suggestions_skip_folders_that_are_already_in_the_library() {
        let std = vec![
            ("Pictures".to_string(), PathBuf::from("/home/a/Pictures")),
            ("Videos".to_string(), PathBuf::from("/home/a/Videos")),
            ("Desktop".to_string(), PathBuf::from("/home/a/Desktop")),
        ];
        let added = [
            PathBuf::from("/home/a/Pictures"),
            PathBuf::from("/home/a/Desktop"),
        ];
        let left = folder_suggestions(std.clone(), &added);
        let names: Vec<_> = left.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Videos"]);
        // A folder that contains them covers them too.
        let all_added = [PathBuf::from("/home/a")];
        assert!(folder_suggestions(std, &all_added).is_empty());
    }

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
