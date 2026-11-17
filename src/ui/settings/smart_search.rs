//! Settings › Smart search: the quality choice, its download, and switching between choices.

use super::widgets::choice_card;
use super::{heading, hint};
use crate::library::Activity;
use crate::model::registry::{self, ModelSpec, megabytes};
use crate::ui::App;
use crate::ui::dialogs::{Confirmation, confirm_dialog};
use crate::ui::style::{self, Palette, icons};
use crate::ui::timeline::thousands;
use egui::RichText;

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

impl App {
    pub(super) fn settings_smart_search(&mut self, ui: &mut egui::Ui) {
        let installed = self.snapshot.model_installed;
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        self.smart_search_status_card(ui);
        ui.add_space(8.0);
        ui.label(
            "Smart search finds photos and videos when you describe them, like “birthday \
             cake”. It runs on this computer.",
        );
        heading(ui, "Choose one");
        self.smart_search_choices(ui);
        hint(
            ui,
            "Switching makes Recollect look at your photos again, which can take a while.",
        );
        if !installed && !downloading {
            self.download_button(ui);
        }
        ui.add_space(10.0);
        let mut gpu = self.settings.use_gpu;
        if style::switch(ui, &mut gpu, "Use the graphics card when available").changed() {
            self.change_settings(|s| s.use_gpu = gpu);
        }
        ui.add_space(6.0);
        smart_search_details(ui, self.lib.model());
    }

    /// The card at the top: whether smart search is on, with the download progress.
    fn smart_search_status_card(&self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let installed = self.snapshot.model_installed;
        let status = smart_search_status(&self.status.activity, installed, self.lib.model());
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        style::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let on = installed && !downloading;
                let (icon, color) = if on {
                    (icons::ICON_CHECK_CIRCLE, pal.accent)
                } else {
                    (icons::ICON_AUTO_AWESOME, pal.text2)
                };
                ui.label(style::icon_text(icon, 22.0).color(color));
                ui.label(style::semibold(status).size(15.0));
            });
            if let Activity::Downloading { done, total, .. } = &self.status.activity {
                ui.add_space(4.0);
                ui.add(
                    egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                        .desired_height(6.0)
                        .corner_radius(3.0),
                );
            }
        });
    }

    /// One card per smart-search choice; picking another asks for confirmation.
    fn smart_search_choices(&mut self, ui: &mut egui::Ui) {
        let current = self.lib.model();
        for m in registry::MODELS {
            let here = self.snapshot.installed_models.contains(&m.id);
            let note = if here { "Already on this computer" } else { "" };
            if choice_card(
                ui,
                m.id == current.id,
                quality_name(m),
                &quality_blurb(m),
                note,
            ) && m.id != current.id
            {
                self.dialogs.model_confirm = Some(m.id);
            }
            ui.add_space(4.0);
        }
    }

    /// The accent button that downloads the current choice and turns smart search on.
    fn download_button(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        ui.add_space(8.0);
        let label = format!("Download and turn on ({} MB)", self.lib.model().size_mb());
        let b = egui::Button::new(RichText::new(&label).color(pal.onaccent)).fill(pal.accent);
        let r = ui.add(b);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if r.clicked() {
            self.turn_on_search();
        }
        hint(
            ui,
            "A one-time download. After that, search works without the internet.",
        );
    }

    /// The question before switching smart search: it makes every photo get looked at again.
    pub(in crate::ui) fn model_confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.dialogs.model_confirm else {
            return;
        };
        let Some(spec) = registry::get(id) else {
            self.dialogs.model_confirm = None;
            return;
        };
        let here = self.snapshot.installed_models.contains(&spec.id);
        let title = format!("Switch to {}?", quality_name(spec));
        let choice = confirm_dialog(ctx, &title, "Switch", "Cancel", true, |ui| {
            ui.set_max_width(420.0);
            ui.label(
                "Recollect will look at all your photos again so search works with this \
                 choice. That can take a while, and you can keep using Recollect meanwhile.",
            );
            if !here {
                ui.add_space(4.0);
                ui.label(format!(
                    "It needs a one-time download of {} MB.",
                    spec.size_mb()
                ));
            }
            ui.add_space(8.0);
        });
        match choice {
            Some(Confirmation::Confirmed) => {
                self.dialogs.model_confirm = None;
                self.switch_model(spec);
            }
            Some(Confirmation::Cancelled) => self.dialogs.model_confirm = None,
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
