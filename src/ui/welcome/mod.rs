//! The first-run welcome: a few full-window screens, one question each, shown the first time
//! Recollect starts with no folders. Nothing is changed until the last screen's button. A
//! photo mosaic fills one half of a wide window; the step's content is in the other.

pub(super) mod mosaic;
mod steps;

use super::App;
use super::folders::{standard_folders, without_nested};
use super::style::{self, Palette};
use super::timeline::View;
use egui::{Margin, Sense, Vec2};
use mosaic::{Mosaic, PhotoSet};
use std::path::PathBuf;

/// Windows narrower than this show only the content.
const MOSAIC_MIN_WIDTH: f32 = 900.0;
/// The content column's width.
const COLUMN_WIDTH: f32 = 420.0;
/// The step's own content has this much room; more scrolls. Fixed, so the buttons below it
/// never move from one step to the next.
const BODY_HEIGHT: f32 = 392.0;
const SECTION_GAP: f32 = 28.0;
const BUTTON_HEIGHT: f32 = 34.0;
const BUTTON_WIDTH: f32 = 96.0;
const MARK_SIZE: f32 = 20.0;
const HEADER_HEIGHT: f32 = MARK_SIZE;
const COLUMN_HEIGHT: f32 = HEADER_HEIGHT + BODY_HEIGHT + BUTTON_HEIGHT + 2.0 * SECTION_GAP;

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
    /// The photo mosaic (painted only while the window is wide enough).
    mosaic: Option<Mosaic>,
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
            mosaic: None,
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

    /// Keeps the mosaic in the photo set of the theme. It is kept (with its textures) while
    /// the welcome is open, whatever the window's width, so resizing never decodes the photos
    /// again; a narrow window just doesn't paint it.
    fn sync_mosaic(&mut self, ctx: &egui::Context, dark: bool) {
        let set = PhotoSet::for_theme(dark);
        if self.mosaic.as_ref().is_none_or(|m| m.set() != set) {
            self.mosaic = Some(Mosaic::new(ctx, set));
        }
    }

    #[cfg(test)]
    pub(super) fn mosaic_set_for_test(&self) -> Option<PhotoSet> {
        self.mosaic.as_ref().map(Mosaic::set)
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

/// The row of small dots that shows where the user is; the current one is a longer bar.
fn progress_dots(ui: &mut egui::Ui, step: Step) {
    const DOT: f32 = 4.0;
    const CURRENT: f32 = 18.0;
    const SPACING: f32 = 5.0;
    let pal = Palette::of(ui.ctx());
    let width = CURRENT + (Step::COUNT - 1) as f32 * (DOT + SPACING);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, DOT), Sense::hover());
    let mut x = r.min.x;
    for i in 0..Step::COUNT {
        let w = if i == step.number() { CURRENT } else { DOT };
        let bar = egui::Rect::from_min_size(egui::pos2(x, r.min.y), Vec2::new(w, DOT));
        let colour = if i <= step.number() {
            pal.accent
        } else {
            pal.ring
        };
        ui.painter().rect_filled(bar, DOT / 2.0, colour);
        x += w + SPACING;
    }
    let label = format!("Step {} of {}", step.number() + 1, Step::COUNT);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
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
        let wide = ui.available_width() >= MOSAIC_MIN_WIDTH;
        w.sync_mosaic(ui.ctx(), pal.dark);
        if let Some(mosaic) = w.mosaic.as_mut().filter(|_| wide) {
            egui::Panel::right("welcome-mosaic")
                .resizable(false)
                .exact_size(ui.available_width() / 2.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(pal.side))
                .show(ui, |ui| mosaic.show(ui));
        }
        let mut action = None;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(Margin::symmetric(32, 16)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("welcome-page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.welcome_column(ui, &mut w, &mut action));
            });
        self.apply_welcome_action(w, action);
    }

    /// The step's content, centred vertically: header, body, then the buttons.
    fn welcome_column(
        &mut self,
        ui: &mut egui::Ui,
        w: &mut Welcome,
        action: &mut Option<WelcomeAction>,
    ) {
        ui.add_space(((ui.available_height() - COLUMN_HEIGHT) / 2.0).max(0.0));
        let width = ui.available_width().min(COLUMN_WIDTH);
        ui.vertical_centered(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    steps::welcome_header(ui, w.step);
                    ui.add_space(SECTION_GAP);
                    ui.allocate_ui(Vec2::new(width, BODY_HEIGHT), |ui| {
                        ui.set_min_size(Vec2::new(width, BODY_HEIGHT));
                        egui::ScrollArea::vertical()
                            .id_salt("welcome-body")
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.welcome_body(ui, w));
                    });
                    ui.add_space(SECTION_GAP);
                    Self::welcome_buttons(ui, w, action);
                },
            );
        });
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

    /// The progress dots on the left; on the right Back and the screen's main button, whose
    /// edges never move. A quiet text button sits left of Back (in Back's place on the first
    /// screen, which has none): "I'll set it up myself" there, "Not now" on the smart-search
    /// screen.
    fn welcome_buttons(ui: &mut egui::Ui, w: &mut Welcome, out: &mut Option<WelcomeAction>) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), BUTTON_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                progress_dots(ui, w.step);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (primary, action, enabled) = match w.step {
                        Step::Intro => ("Get started", WelcomeAction::Next, true),
                        Step::Folders => ("Continue", WelcomeAction::Next, !w.folders().is_empty()),
                        Step::Search => ("Turn on", WelcomeAction::Next, true),
                        Step::Done => ("Start", WelcomeAction::Finish, true),
                    };
                    ui.add_enabled_ui(enabled, |ui| {
                        if style::primary_button_wide(ui, primary, BUTTON_WIDTH).clicked() {
                            *out = Some(action);
                        }
                    });
                    if w.step != Step::Intro {
                        let back =
                            egui::Button::new("Back").min_size(Vec2::new(BUTTON_WIDTH, 32.0));
                        if ui.add(back).clicked() {
                            *out = Some(WelcomeAction::Back);
                        }
                    }
                    Self::welcome_quiet_button(ui, w.step, out);
                });
            },
        );
    }

    fn welcome_quiet_button(ui: &mut egui::Ui, step: Step, out: &mut Option<WelcomeAction>) {
        let pal = Palette::of(ui.ctx());
        let (label, action) = match step {
            Step::Intro => ("I'll set it up myself", WelcomeAction::SkipSetup),
            Step::Search => ("Not now", WelcomeAction::SkipSearch),
            Step::Folders | Step::Done => return,
        };
        let quiet = egui::Button::new(egui::RichText::new(label).color(pal.text2)).frame(false);
        if ui.add(quiet).clicked() {
            *out = Some(action);
        }
    }

    fn welcome_body(&mut self, ui: &mut egui::Ui, w: &mut Welcome) {
        match w.step {
            Step::Intro => steps::welcome_intro(ui),
            Step::Folders => self.welcome_folders(ui, w),
            Step::Search => steps::welcome_search(ui, w),
            Step::Done => steps::welcome_done(ui, w),
        }
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
