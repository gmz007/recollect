//! Settings › Features: switches for the extras Recollect shows.

use super::rows::{Rows, footnote, group, intro};
use crate::ui::App;

const BUTTON_WIDTH: f32 = 130.0;

/// The five switches, in order, and what was done in this frame.
#[derive(Default)]
struct Picks {
    switches: [Option<bool>; 5],
    clear_recent: bool,
    show_tips_again: bool,
}

impl App {
    pub(super) fn settings_features(&mut self, ui: &mut egui::Ui) {
        intro(
            ui,
            "Choose which extras Recollect shows. Turn any of them off for a simpler app.",
        );
        let mut picks = Picks::default();
        group(ui, |ui, rows| {
            self.feature_switches(ui, rows, &mut picks);
            self.feature_buttons(ui, rows, &mut picks);
        });
        footnote(
            ui,
            "Changes apply straight away. Nothing here affects your photos.",
        );
        self.apply_feature_picks(&picks);
    }

    fn feature_switches(&self, ui: &mut egui::Ui, rows: &mut Rows, picks: &mut Picks) {
        let s = &self.settings;
        let features = [
            (
                "On this day",
                "Show photos taken on today’s date in earlier years, at the top of Photos.",
                s.show_on_this_day,
            ),
            (
                "Tips",
                "Short hints that appear once, at the moment they’re useful.",
                s.show_tips,
            ),
            (
                "Remember recent searches",
                "Offer your last few searches when you click the search box. Kept only on \
                 this computer.",
                s.remember_searches,
            ),
            (
                "Example searches",
                "Suggest things to try, like “sunset over the sea”, in the search box.",
                s.show_example_searches,
            ),
            (
                "Progress in the sidebar",
                "Show the small ring while Recollect is finding photos or preparing search.",
                s.show_sidebar_progress,
            ),
        ];
        for (pick, (title, desc, on)) in picks.switches.iter_mut().zip(features) {
            *pick = rows.switch_row(ui, title, desc, on);
        }
    }

    /// The rows with a button: forget recent searches, bring the closed tips back.
    fn feature_buttons(&self, ui: &mut egui::Ui, rows: &mut Rows, picks: &mut Picks) {
        let s = &self.settings;
        let note = if s.recent_searches.is_empty() {
            "No searches are remembered."
        } else {
            "Forget the searches remembered so far."
        };
        rows.row(ui, "Recent searches", Some(note), BUTTON_WIDTH, |ui| {
            let enabled = !s.recent_searches.is_empty();
            picks.clear_recent = ui
                .add_enabled(enabled, egui::Button::new("Clear now"))
                .clicked();
        });
        let desc = "Bring back the tips you have closed.";
        rows.row(ui, "Tips you have closed", Some(desc), BUTTON_WIDTH, |ui| {
            let enabled = !s.tips_dismissed.is_empty();
            picks.show_tips_again = ui
                .add_enabled(enabled, egui::Button::new("Show again"))
                .clicked();
        });
    }

    /// Saves the switches that were turned and runs the buttons that were clicked.
    fn apply_feature_picks(&mut self, picks: &Picks) {
        if picks.switches.iter().any(Option::is_some) {
            let turned = picks.switches;
            self.change_settings(|s| {
                let fields = [
                    &mut s.show_on_this_day,
                    &mut s.show_tips,
                    &mut s.remember_searches,
                    &mut s.show_example_searches,
                    &mut s.show_sidebar_progress,
                ];
                for (field, turn) in fields.into_iter().zip(turned) {
                    if let Some(v) = turn {
                        *field = v;
                    }
                }
            });
        }
        if picks.clear_recent {
            self.change_settings(|s| s.recent_searches.clear());
        }
        if picks.show_tips_again {
            self.change_settings(|s| s.tips_dismissed.clear());
        }
    }
}
