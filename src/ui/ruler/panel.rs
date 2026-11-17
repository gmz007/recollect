//! The ruler column beside whichever grid is shown.

use super::{WIDTH, WIDTH_SEARCH, clear_stops, show};
use crate::ui::App;
use crate::ui::style::Palette;

impl App {
    /// Width of the ruler beside the shown grid, or `None` when there is nothing to scroll.
    fn ruler_width(&self) -> Option<f32> {
        if self.snapshot.folders.is_empty() {
            return None;
        }
        if self.searching() {
            return (!self.res.hits.is_empty() && !self.res.grid.ruler.fits())
                .then_some(WIDTH_SEARCH);
        }
        self.tl.ruler_shown().then_some(WIDTH)
    }

    /// The ruler column at the right edge, under the top bar. When everything fits in the
    /// window there is no ruler at all.
    pub(in crate::ui) fn ruler_panel(&mut self, ui: &mut egui::Ui) {
        let Some(width) = self.ruler_width() else {
            clear_stops(ui.ctx());
            return;
        };
        let pal = Palette::of(ui.ctx());
        let searching = self.searching();
        let target = egui::Panel::right("ruler")
            .exact_size(width)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(pal.bg))
            .show(ui, |ui| {
                if searching {
                    self.res.with_ruler(|st, date_at| show(ui, st, date_at))
                } else {
                    self.tl.with_ruler(|st, date_at| show(ui, st, date_at))
                }
            })
            .inner;
        // Holding the pointer still on the ruler asks for the same offset every frame: only an
        // offset that differs from where the grid is needs another frame.
        if let Some(y) = target.filter(|y| (y - self.grid().ruler.offset()).abs() > 0.5) {
            self.active_grid().scroll_to = Some(y);
            ui.ctx().request_repaint();
        }
    }
}
