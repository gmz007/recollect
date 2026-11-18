//! Adding a folder to the library (suggestions, the picker, a typed path) and the
//! confirmation for taking one out.

use super::App;
use super::dialogs::{Confirmation, confirm_dialog, modal_window};
use super::sidebar::plain_row;
use super::style::{self, icons};
use egui::RichText;
use std::path::PathBuf;

/// State of the "Add a folder" dialog.
#[derive(Debug, Default)]
pub(super) struct AddFolder {
    pub(super) typed: String,
    /// Standard folders that are not in the library yet, worked out once when the dialog opens
    /// (looking them up touches the disk).
    pub(super) suggestions: Vec<(String, PathBuf)>,
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
fn folder_suggestions(
    candidates: Vec<(String, PathBuf)>,
    added: &[PathBuf],
) -> Vec<(String, PathBuf)> {
    candidates
        .into_iter()
        .filter(|(_, c)| !added.iter().any(|a| c.starts_with(a)))
        .collect()
}

/// The system's folder picker; `None` when it was closed without a choice.
pub(super) fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose a folder with photos and videos")
        .pick_folder()
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

    /// Takes a folder out of the library (its files are not touched).
    pub(super) fn stop_showing_folder(&mut self, id: i64) {
        // Cancels any running job first; runs in the background.
        self.lib.start_remove_folder(id);
        self.snapshot.folders.retain(|f| f.id != id);
        // A fetch already in flight may predate the removal: fetch again after it.
        self.snapshot_read.mark_again();
    }

    pub(super) fn add_folder_dialog(&mut self, ctx: &egui::Context) {
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
                self.stop_showing_folder(s.id);
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
    pub(super) chosen: Option<PathBuf>,
    /// The typed text is not a usable folder path.
    pub(super) bad_path: bool,
    pub(super) close: bool,
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
    if style::icon_label_button(ui, icons::ICON_FOLDER_OPEN, "Choose a folder…").clicked() {
        chosen = pick_folder();
    }
    if !suggestions.is_empty() {
        ui.add_space(12.0);
        ui.label(style::semibold("Suggestions").size(14.0));
        for (name, path) in suggestions {
            let (_, r) = plain_row(
                ui,
                icons::ICON_FOLDER,
                name,
                &format!("Add {name}"),
                Some(&path.display().to_string()),
                false,
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
}
