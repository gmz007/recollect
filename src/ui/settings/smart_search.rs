//! Settings › Smart search: the quality choice, its download, and switching between choices.

use super::hint;
use super::rows::{group, intro};
use super::widgets::segmented;
use crate::library::Activity;
use crate::model::registry::{self, ModelSpec, megabytes};
use crate::ui::App;
use crate::ui::style::{self, Palette};
use crate::ui::timeline::thousands;

/// "Best quality" or "Faster": how a smart-search choice is called in the interface.
pub(in crate::ui) fn quality_name(spec: &ModelSpec) -> &'static str {
    if spec.id == registry::SIGLIP_B16.id {
        "Best quality"
    } else {
        "Faster"
    }
}

/// One sentence about a smart-search choice, with its download size.
pub(in crate::ui) fn quality_blurb(spec: &ModelSpec) -> String {
    let mb = spec.size_mb();
    if spec.id == registry::SIGLIP_B16.id {
        format!(
            "Finds photos most reliably. Works best on computers with a graphics card. {mb} MB."
        )
    } else {
        format!("Quicker on older computers. {mb} MB.")
    }
}

/// The smart-search status in plain words.
pub(in crate::ui) fn smart_search_status(
    activity: &Activity,
    installed: bool,
    spec: &ModelSpec,
) -> String {
    match activity {
        Activity::Downloading { done, total, .. } => format!(
            "Downloading… {} of {} MB",
            thousands(usize::try_from(megabytes(*done)).unwrap_or(usize::MAX)),
            thousands(usize::try_from(megabytes(*total)).unwrap_or(usize::MAX))
        ),
        Activity::Idle if installed => format!("On — {}", quality_name(spec).to_lowercase()),
        Activity::Idle => "Off".to_string(),
        _ if installed => "Getting your photos ready…".to_string(),
        _ => "Off".to_string(),
    }
}

/// The collapsible "Details" section: name, license and source of the current choice.
fn smart_search_details(ui: &mut egui::Ui, current: &ModelSpec) {
    egui::CollapsingHeader::new("Details")
        .id_salt("settings-search-details")
        .show(ui, |ui| {
            ui.label(format!("Name: {}", current.name));
            ui.label(format!("License: {}", current.license));
            ui.label(format!("Source: huggingface.co/{}", current.repo));
        });
}

/// What the user did in the section this frame.
#[derive(Default)]
struct SmartSearchActions {
    download: bool,
    cancel: bool,
    style: Option<&'static str>,
    gpu: Option<bool>,
}

const CONTROL_WIDTH: f32 = 260.0;

impl App {
    pub(super) fn settings_smart_search(&mut self, ui: &mut egui::Ui) {
        intro(
            ui,
            "Smart search finds photos and videos when you describe them, like “birthday \
             cake”. It runs on this computer.",
        );
        if let Activity::Downloading { done, total, .. } = &self.status.activity {
            ui.add(
                egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                    .desired_height(6.0)
                    .corner_radius(3.0),
            );
            ui.add_space(8.0);
        }
        let mut act = SmartSearchActions::default();
        self.smart_search_rows(ui, &mut act);
        self.confirm_style_change(ui);
        hint_after_rows(ui, self.snapshot.model_installed);
        ui.add_space(10.0);
        smart_search_details(ui, self.lib.model());
        if act.download {
            self.turn_on_search();
        }
        if act.cancel {
            self.lib.cancel();
            self.set_search_prompt_dismissed(true);
        }
        if let Some(id) = act.style {
            self.dialogs.model_confirm = Some(id);
        }
        if let Some(on) = act.gpu {
            self.change_settings(|s| s.use_gpu = on);
        }
    }

    /// The group of rows: on or off with the download, the search style and the graphics card.
    fn smart_search_rows(&self, ui: &mut egui::Ui, act: &mut SmartSearchActions) {
        let installed = self.snapshot.model_installed;
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        let current = self.lib.model();
        let status = smart_search_status(&self.status.activity, installed, current);
        let download_label = format!("Download and turn on ({} MB)", current.size_mb());
        group(ui, |ui, rows| {
            rows.row(ui, "Smart search", Some(&status), CONTROL_WIDTH, |ui| {
                if downloading {
                    act.cancel = ui.button("Cancel").clicked();
                } else if !installed {
                    act.download = style::primary_button(ui, &download_label).clicked();
                }
            });
            rows.row(
                ui,
                "Search style",
                Some(&quality_blurb(current)),
                CONTROL_WIDTH,
                |ui| {
                    let items: Vec<_> = registry::MODELS
                        .iter()
                        .map(|m| (m.id, quality_name(m)))
                        .collect();
                    act.style = segmented(ui, &items, current.id);
                },
            );
            act.gpu = rows.switch_row(
                ui,
                "Use the graphics card when available",
                "Makes getting photos ready quicker, when your computer has one.",
                self.settings.use_gpu,
            );
        });
    }

    /// The question before changing the search style: it makes every photo get looked at again.
    fn confirm_style_change(&mut self, ui: &mut egui::Ui) {
        let Some(spec) = self.dialogs.model_confirm.and_then(registry::get) else {
            self.dialogs.model_confirm = None;
            return;
        };
        if spec.id == self.lib.model().id {
            self.dialogs.model_confirm = None;
            return;
        }
        let pal = Palette::of(ui.ctx());
        let here = self.snapshot.installed_models.contains(&spec.id);
        let mut answer = None;
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(pal.tint)
            .corner_radius(egui::CornerRadius::same(style::CARD_RADIUS))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(format!(
                    "Change to {}? Recollect will look through your photos again, which can \
                     take a while. You can keep using it meanwhile.",
                    quality_name(spec)
                ));
                if !here {
                    ui.label(format!(
                        "It needs a one-time download of {} MB.",
                        spec.size_mb()
                    ));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if style::primary_button(ui, "Change").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => {
                self.dialogs.model_confirm = None;
                self.switch_model(spec);
            }
            Some(false) => self.dialogs.model_confirm = None,
            None => {}
        }
    }

    /// Makes `spec` the smart-search choice (waits if a long task is running).
    pub(super) fn switch_model(&mut self, spec: &'static ModelSpec) {
        if self.lib.is_busy() {
            self.notify(
                "Recollect is busy right now. Wait for it to finish, or press Stop in the left \
                 panel, then try again.",
            );
            return;
        }
        if let Err(e) = self.lib.set_model(spec.id) {
            self.notify_problem(
                "Couldn't change smart search. Please try again in a moment.",
                format!("{e:#}"),
                None,
            );
        } else {
            // Old results and queries belong to the previous choice.
            self.set_search_prompt_dismissed(false);
            self.end_viewer();
            self.clear_search();
            self.refresh_snapshot(true);
        }
    }
}

/// The note under the rows: the download is a one-time thing.
fn hint_after_rows(ui: &mut egui::Ui, installed: bool) {
    if !installed {
        ui.add_space(8.0);
        hint(
            ui,
            "A one-time download. After that, search works without the internet.",
        );
    }
}
