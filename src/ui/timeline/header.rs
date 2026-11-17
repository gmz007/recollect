//! The day heading above a section of the grid: its name, and a circle (shown while the pointer
//! is on the heading) that selects or deselects the whole day.

use crate::ui::style::{self, Palette, icons};
use egui::{Rect, RichText, Sense, Stroke, Vec2};

/// Width of the clickable area of the circle.
const CIRCLE_ZONE: f32 = 26.0;
/// Diameter of the circle.
const CIRCLE_DIAMETER: f32 = 18.0;
/// Space before the heading's text.
const PAD: f32 = 4.0;
/// Size of the check mark in the circle.
const CHECK_SIZE: f32 = 12.0;

/// How much of a day is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DaySelection {
    NotAll,
    All,
}

/// Draws the heading `text` into `rect`; `opaque` paints the page colour behind it (when it is
/// pinned over photos). Returns true when the circle was clicked.
pub(super) fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    text: &str,
    selection: DaySelection,
    opaque: bool,
) -> bool {
    let pal = Palette::of(ui.ctx());
    if opaque {
        ui.painter().rect_filled(rect, 0.0, pal.bg);
    }
    let all = selection == DaySelection::All;
    let hovered = ui.rect_contains_pointer(rect);
    let zone = Rect::from_center_size(
        egui::pos2(rect.min.x + PAD + CIRCLE_ZONE / 2.0, rect.center().y),
        Vec2::splat(CIRCLE_ZONE),
    );
    let resp = ui.interact(zone, id, Sense::click());
    resp.widget_info(|| {
        let verb = if all {
            "Deselect all of"
        } else {
            "Select all of"
        };
        egui::WidgetInfo::selected(egui::Role::CheckBox, true, all, format!("{verb} {text}"))
    });
    let circle_shown = hovered || all || resp.has_focus();
    if circle_shown {
        paint_circle(ui, &pal, zone.center(), all);
    }
    let text_x = rect.min.x + PAD + if circle_shown { CIRCLE_ZONE } else { 0.0 };
    let text_rect = Rect::from_min_max(egui::pos2(text_x, rect.min.y), rect.max);
    let mut label = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    label.label(RichText::new(text).text_style(style::section()));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn paint_circle(ui: &egui::Ui, pal: &Palette, centre: egui::Pos2, all: bool) {
    let painter = ui.painter();
    let r = CIRCLE_DIAMETER / 2.0;
    if all {
        painter.circle_filled(centre, r, pal.accent);
        painter.text(
            centre,
            egui::Align2::CENTER_CENTER,
            icons::ICON_CHECK.codepoint,
            style::icon_font_id(icons::ICON_CHECK, CHECK_SIZE),
            pal.onaccent,
        );
    } else {
        painter.circle_stroke(centre, r - 0.75, Stroke::new(1.5, pal.muted));
    }
}
