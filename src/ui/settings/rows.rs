//! The building blocks of a settings section: a bordered group of rows, each with a title and
//! an optional description on the left and its control on the right.

use crate::ui::style::{self, Palette};
use egui::{CornerRadius, Margin, Rect, RichText, Stroke, Ui, UiBuilder, Vec2};

/// Space kept between a row's text and its control.
const ROW_GAP: f32 = 16.0;
/// Width of the column a switch sits in.
const SWITCH_COLUMN: f32 = 40.0;

/// Where the next row of a group goes: the first has no line above it.
pub(super) struct Rows {
    first: bool,
}

/// A bordered group; `rows` fills it row by row.
pub(super) fn group(ui: &mut Ui, rows: impl FnOnce(&mut Ui, &mut Rows)) {
    let pal = Palette::of(ui.ctx());
    egui::Frame::new()
        .stroke(Stroke::new(1.0, pal.line))
        .corner_radius(CornerRadius::same(style::CARD_RADIUS))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            rows(ui, &mut Rows { first: true });
        });
}

impl Rows {
    /// One row: `title` and `desc` on the left; `control` is laid out right-aligned in a
    /// column `control_w` wide.
    pub(super) fn row(
        &mut self,
        ui: &mut Ui,
        title: &str,
        desc: Option<&str>,
        control_w: f32,
        control: impl FnOnce(&mut Ui),
    ) {
        let pal = Palette::of(ui.ctx());
        if !self.first {
            let y = ui.cursor().top();
            let x = ui.available_rect_before_wrap().x_range();
            ui.painter().hline(x, y, Stroke::new(1.0, pal.line));
        }
        self.first = false;
        egui::Frame::new()
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(32.0);
                let total = ui.available_width();
                let top = ui.cursor().min;
                let text_w = (total - control_w - ROW_GAP).max(120.0);
                let left = ui.allocate_ui(Vec2::new(text_w, 0.0), |ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(style::medium(title));
                    if let Some(d) = desc {
                        ui.label(RichText::new(d).size(12.0).color(pal.muted));
                    }
                });
                let height = left.response.rect.height().max(32.0);
                let at = Rect::from_min_size(
                    egui::pos2(top.x + total - control_w, top.y),
                    Vec2::new(control_w, height),
                );
                let layout = egui::Layout::right_to_left(egui::Align::Center);
                ui.scope_builder(UiBuilder::new().max_rect(at).layout(layout), control);
            });
    }

    /// A row with a switch; true when it was turned.
    pub(super) fn switch_row(
        &mut self,
        ui: &mut Ui,
        title: &str,
        desc: &str,
        on: bool,
    ) -> Option<bool> {
        let mut value = on;
        let mut changed = false;
        self.row(ui, title, Some(desc), SWITCH_COLUMN, |ui| {
            changed = style::switch_bare(ui, &mut value, title).changed();
        });
        changed.then_some(value)
    }
}

/// An explanation above or below a group, in body colour.
pub(super) fn intro(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.label(RichText::new(text).color(pal.text2));
    ui.add_space(8.0);
}

/// A quiet line below a group.
pub(super) fn footnote(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(12.0).color(pal.muted));
}

/// A small heading above a group.
pub(super) fn group_title(ui: &mut Ui, text: &str) {
    ui.add_space(16.0);
    ui.label(style::semibold(text));
    ui.add_space(6.0);
}
