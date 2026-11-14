//! The first-run welcome: a few full-window screens, one question each, shown the first time
//! Recollect starts with no folders. Nothing is changed until the last screen's button.

use super::App;
use super::settings::{choice_card, quality_blurb, quality_name};
use super::sidebar::{standard_folders, without_nested};
use super::style::{self, Palette, icons};
use super::timeline::View;
use crate::model::registry;
use egui::{Margin, RichText, Sense, Vec2};
use egui_material_icons::MaterialIcon;
use std::path::PathBuf;

/// The screens, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Intro,
    Folders,
    Search,
    Done,
}

impl Step {
    const COUNT: usize = 4;

    fn number(self) -> usize {
        match self {
            Step::Intro => 0,
            Step::Folders => 1,
            Step::Search => 2,
            Step::Done => 3,
        }
    }
}

/// A standard folder offered as a checkbox.
#[derive(Debug, Clone)]
struct Suggestion {
    name: String,
    path: PathBuf,
    checked: bool,
}

/// What the user has chosen so far.
pub(super) struct Welcome {
    step: Step,
    suggestions: Vec<Suggestion>,
    /// Folders added with the picker or by typing a path.
    added: Vec<PathBuf>,
    typed: String,
    /// The smart-search choice (a registry id).
    choice: &'static str,
    /// The user asked for smart search to be downloaded.
    download: bool,
    /// The model that suits this computer, worked out once (asking the graphics hardware is
    /// not something to do every frame).
    recommended: &'static str,
}

impl Welcome {
    /// A welcome offering this computer's standard folders (Pictures and Videos ticked).
    pub(super) fn new(choice: &'static str) -> Self {
        Self::with_suggestions(standard_folders(), choice)
    }

    pub(super) fn with_suggestions(folders: Vec<(String, PathBuf)>, choice: &'static str) -> Self {
        Self {
            step: Step::Intro,
            suggestions: folders
                .into_iter()
                .map(|(name, path)| Suggestion {
                    checked: name == "Pictures" || name == "Videos",
                    name,
                    path,
                })
                .collect(),
            added: Vec::new(),
            typed: String::new(),
            choice,
            download: false,
            recommended: crate::library::default_model_for_hardware(),
        }
    }

    /// Every folder the user has chosen, without repeats and without folders that lie inside
    /// another chosen one (the outer folder already covers them).
    fn folders(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in self
            .suggestions
            .iter()
            .filter(|s| s.checked)
            .map(|s| &s.path)
            .chain(&self.added)
        {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        without_nested(&out)
    }

    #[cfg(test)]
    pub(super) fn add_folder_for_test(&mut self, p: PathBuf) {
        self.added.push(p);
    }

    #[cfg(test)]
    pub(super) fn choose_for_test(&mut self, id: &'static str) {
        self.choice = id;
        self.download = true;
    }

    #[cfg(test)]
    pub(super) fn chosen_folders(&self) -> Vec<PathBuf> {
        self.folders()
    }
}

/// What a screen's buttons asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WelcomeAction {
    Back,
    Next,
    /// "Not now" on the smart-search screen.
    SkipSearch,
    Finish,
    SkipSetup,
}

/// What the last screen says will happen after Start.
fn done_text(has_folders: bool, download: bool) -> String {
    if !has_folders {
        let mut t = String::from(
            "You haven't chosen a folder yet. After you press Start, add one from the left \
             side of the window.",
        );
        if download {
            t.push_str(" Search works once the download finishes.");
        }
        return t;
    }
    let mut t = String::from(
        "After you press Start, Recollect gets your photos ready. Progress shows at the \
         bottom left; you can browse right away.",
    );
    t.push_str(if download {
        " Search works once the download finishes."
    } else {
        " You can turn on smart search any time from Settings."
    });
    t
}

/// A line with an icon in front, for the benefits on the first screen.
fn benefit(ui: &mut egui::Ui, icon: MaterialIcon, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(style::icon_text(icon, 24.0).color(pal.accent));
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(16.0));
    });
    ui.add_space(8.0);
}

/// The row of small dots that shows where the user is.
fn progress_dots(ui: &mut egui::Ui, step: Step) {
    let pal = Palette::of(ui.ctx());
    let (r, resp) =
        ui.allocate_exact_size(Vec2::new(Step::COUNT as f32 * 18.0, 12.0), Sense::hover());
    for i in 0..Step::COUNT {
        let c = egui::pos2(r.min.x + 9.0 + i as f32 * 18.0, r.center().y);
        if i == step.number() {
            ui.painter().circle_filled(c, 5.0, pal.accent);
        } else {
            ui.painter()
                .circle_filled(c, 4.0, pal.text_secondary.gamma_multiply(0.45));
        }
    }
    let label = format!("Step {} of {}", step.number() + 1, Step::COUNT);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
}

/// The folders the user picked or typed so far, each with a button to take it back.
fn welcome_added_folders(ui: &mut egui::Ui, w: &mut Welcome) {
    if w.added.is_empty() {
        return;
    }
    let pal = Palette::of(ui.ctx());
    ui.add_space(10.0);
    ui.label(style::semibold("Folders you added").size(14.0));
    let mut remove = None;
    for (i, p) in w.added.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(style::icon_text(icons::ICON_FOLDER, 18.0).color(pal.text_secondary));
            ui.vertical(|ui| {
                let name = p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                ui.add(egui::Label::new(name).truncate());
                ui.add(
                    egui::Label::new(
                        RichText::new(p.display().to_string())
                            .small()
                            .color(pal.text_secondary),
                    )
                    .truncate(),
                );
            });
            if style::icon_button(ui, icons::ICON_CLOSE, &format!("Remove {}", p.display()))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        w.added.remove(i);
    }
}

impl App {
    /// True while the welcome screens replace the whole window.
    pub(super) fn welcome_active(&self) -> bool {
        self.welcome.is_some() && self.snapshot.folders.is_empty()
    }

    /// Draws the welcome and carries out what its buttons ask for.
    pub(super) fn welcome_screen(&mut self, ui: &mut egui::Ui) {
        let Some(mut w) = self.welcome.take() else {
            return;
        };
        let pal = Palette::of(ui.ctx());
        let mut action = None;
        let width = (ui.available_width() - 48.0).clamp(300.0, 560.0);
        egui::Panel::bottom("welcome-footer")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal.background)
                    .inner_margin(Margin::symmetric(24, 18)),
            )
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    progress_dots(ui, w.step);
                    ui.add_space(12.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 40.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| Self::welcome_buttons(ui, &mut w, &mut action),
                    );
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.background)
                    .inner_margin(Margin::symmetric(24, 16)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space((ui.available_height() * 0.06).min(48.0));
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| self.welcome_body(ui, &mut w),
                            );
                        });
                    });
            });
        self.apply_welcome_action(w, action);
    }

    fn apply_welcome_action(&mut self, mut w: Welcome, action: Option<WelcomeAction>) {
        match action {
            Some(WelcomeAction::SkipSetup) => {
                self.change_settings(|s| s.welcome_done = true);
                return;
            }
            Some(WelcomeAction::Finish) => {
                self.finish_welcome(w);
                return;
            }
            Some(WelcomeAction::Back) => {
                w.step = match w.step {
                    Step::Intro | Step::Folders => Step::Intro,
                    Step::Search => Step::Folders,
                    Step::Done => Step::Search,
                };
            }
            Some(action @ (WelcomeAction::Next | WelcomeAction::SkipSearch)) => {
                w.step = match w.step {
                    Step::Intro => Step::Folders,
                    Step::Folders => Step::Search,
                    Step::Search | Step::Done => Step::Done,
                };
                if w.step == Step::Done {
                    w.download = action == WelcomeAction::Next;
                }
            }
            None => {}
        }
        self.welcome = Some(w);
    }

    /// Back, the secondary choice and the primary button of the current screen.
    fn welcome_buttons(ui: &mut egui::Ui, w: &mut Welcome, out: &mut Option<WelcomeAction>) {
        if w.step == Step::Intro {
            if ui.button("I'll set it up myself").clicked() {
                *out = Some(WelcomeAction::SkipSetup);
            }
        } else if ui.button("Back").clicked() {
            *out = Some(WelcomeAction::Back);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (primary, action) = match w.step {
                Step::Intro => ("Get started", WelcomeAction::Next),
                Step::Folders if w.folders().is_empty() => {
                    ("Continue without folders", WelcomeAction::Next)
                }
                Step::Folders => ("Continue", WelcomeAction::Next),
                Step::Search => ("Download and turn on", WelcomeAction::Next),
                Step::Done => ("Start", WelcomeAction::Finish),
            };
            if style::primary_button(ui, primary).clicked() {
                *out = Some(action);
            }
            if w.step == Step::Search && ui.button("Not now").clicked() {
                *out = Some(WelcomeAction::SkipSearch);
            }
        });
    }

    fn welcome_body(&mut self, ui: &mut egui::Ui, w: &mut Welcome) {
        let pal = Palette::of(ui.ctx());
        let title = |ui: &mut egui::Ui, text: &str| {
            ui.label(style::semibold(text).size(28.0));
            ui.add_space(10.0);
        };
        match w.step {
            Step::Intro => {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(style::icon_text(icons::ICON_PHOTO_LIBRARY, 72.0).color(pal.accent));
                    ui.add_space(12.0);
                    ui.label(
                        style::semibold("Welcome to Recollect")
                            .size(15.0)
                            .color(pal.accent),
                    );
                    ui.add_space(4.0);
                    title(ui, "Find any photo by describing it");
                });
                ui.add_space(16.0);
                // The three lines sit as one block in the middle of the column.
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - 400.0) / 2.0).max(0.0));
                    ui.vertical(|ui| {
                        benefit(
                            ui,
                            icons::ICON_SEARCH,
                            "Search with words, like “kids on the beach”",
                        );
                        benefit(
                            ui,
                            icons::ICON_CALENDAR_MONTH,
                            "Browse everything by date, like any photo app",
                        );
                        benefit(ui, icons::ICON_LOCK, "Everything stays on this computer");
                    });
                });
            }
            Step::Folders => self.welcome_folders(ui, w),
            Step::Search => Self::welcome_search(ui, w),
            Step::Done => {
                ui.add_space(24.0);
                let text = done_text(!w.folders().is_empty(), w.download);
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = ui.available_width();
                job.halign = egui::Align::Center;
                job.append(
                    &text,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::new(
                            16.0,
                            egui::TextStyle::Body.resolve(ui.style()).family,
                        ),
                        color: pal.text,
                        ..Default::default()
                    },
                );
                ui.vertical_centered(|ui| {
                    ui.label(style::icon_text(icons::ICON_CHECK_CIRCLE, 72.0).color(pal.accent));
                    ui.add_space(12.0);
                    title(ui, "You're all set");
                    ui.add(egui::Label::new(job));
                });
            }
        }
    }

    fn welcome_folders(&mut self, ui: &mut egui::Ui, w: &mut Welcome) {
        let pal = Palette::of(ui.ctx());
        ui.label(style::semibold("Where are your photos?").size(28.0));
        ui.add_space(8.0);
        ui.label(
            "Choose the folders Recollect should look in. It only reads your photos and \
             videos; it never changes them.",
        );
        ui.add_space(12.0);
        for s in &mut w.suggestions {
            let label = s.name.clone();
            let r = ui
                .checkbox(&mut s.checked, label)
                .on_hover_text(s.path.display().to_string());
            r.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::CheckBox, true, s.checked, &s.name)
            });
            ui.add_space(4.0);
        }
        if !w
            .suggestions
            .iter()
            .any(|s| s.name == "Pictures" || s.name == "Videos")
        {
            ui.label("We didn't find a Pictures folder. Choose where your photos are.");
        }
        ui.add_space(8.0);
        if style::icon_label_button(ui, icons::ICON_CREATE_NEW_FOLDER, "Add another folder…")
            .clicked()
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose a folder with photos and videos")
                .pick_folder()
            && !w.added.contains(&dir)
        {
            w.added.push(dir);
        }
        welcome_added_folders(ui, w);
        ui.add_space(8.0);
        self.welcome_typed_path(ui, w);
        if w.folders().is_empty() {
            ui.add_space(10.0);
            ui.label(
                RichText::new("You can add folders later from the sidebar.")
                    .small()
                    .color(pal.text_secondary),
            );
        }
    }

    /// A text field for computers where the folder picker is missing or awkward.
    fn welcome_typed_path(&mut self, ui: &mut egui::Ui, w: &mut Welcome) {
        let pal = Palette::of(ui.ctx());
        egui::CollapsingHeader::new("More options")
            .id_salt("welcome-more-options")
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Or type a folder path")
                        .small()
                        .color(pal.text_secondary),
                );
                let mut add_typed = false;
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut w.typed)
                            .hint_text("Type or paste the folder's location")
                            .desired_width(ui.available_width() - 130.0),
                    );
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Folder path")
                    });
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let can = !w.typed.trim().is_empty();
                    add_typed = (ui
                        .add_enabled(can, egui::Button::new("Add this folder"))
                        .clicked()
                        || enter)
                        && can;
                });
                if add_typed {
                    let p = PathBuf::from(w.typed.trim());
                    if p.is_dir() {
                        if !w.added.contains(&p) {
                            w.added.push(p);
                        }
                        w.typed.clear();
                    } else {
                        self.notify_error(
                            "That folder couldn't be found. Check the spelling of the path, \
                             or use “Add another folder…” to pick it.",
                        );
                    }
                }
            });
    }

    fn welcome_search(ui: &mut egui::Ui, w: &mut Welcome) {
        let pal = Palette::of(ui.ctx());
        ui.label(style::semibold("Turn on smart search").size(28.0));
        ui.add_space(8.0);
        ui.label("Describe what you remember, like “kids on the beach”, and Recollect finds it.");
        ui.add_space(12.0);
        let recommended = w.recommended;
        for m in registry::MODELS {
            let note = if m.id == recommended {
                "Recommended for this computer"
            } else {
                ""
            };
            if choice_card(
                ui,
                m.id == w.choice,
                quality_name(m),
                &quality_blurb(m),
                note,
            ) {
                w.choice = m.id;
            }
            ui.add_space(6.0);
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(style::icon_text(icons::ICON_LOCK, 18.0).color(pal.text_secondary));
            ui.label(
                RichText::new(
                    "It downloads once. After that it works without the internet, and your \
                     photos never leave this computer.",
                )
                .color(pal.text_secondary),
            );
        });
    }

    /// Adds the chosen folders, starts the download if asked and lands on Photos. The folders
    /// are looked through before the download starts, so they can be browsed while it runs.
    fn finish_welcome(&mut self, w: Welcome) {
        // Switching first: the download that follows needs it. If it can't be done, nothing
        // else is, and the person can press Start again.
        if w.download
            && w.choice != self.lib.model().id
            && let Err(e) = self.lib.set_model(w.choice)
        {
            self.notify_problem(
                "Couldn't set up smart search. Please try again in a moment.",
                format!("{e:#}"),
                None,
            );
            self.welcome = Some(w);
            return;
        }
        let then = if w.download {
            crate::library::AfterAdd::DownloadModel
        } else {
            crate::library::AfterAdd::Nothing
        };
        self.lib.start_add_folders(w.folders(), then);
        self.snapshot_read.mark_again();
        self.change_settings(|s| s.welcome_done = true);
        self.set_view(View::Timeline);
        self.welcome = None;
    }
}
