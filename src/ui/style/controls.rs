//! Chips, switches, the floating pill and the look of menu contents.

use super::{CHIP_HEIGHT, CONTROL_RADIUS, Palette};
use egui::{Color32, CornerRadius, Frame, Margin, Response, Sense, Stroke, TextStyle, Vec2};

/// Height of the floating pill (selection bar and confirmations).
pub const PILL_HEIGHT: f32 = 44.0;
/// Height of a row in a menu or popover.
const MENU_ROW: f32 = 32.0;
/// Width and height of a switch, and the diameter of its knob.
const SWITCH_SIZE: Vec2 = Vec2::new(40.0, 24.0);
const SWITCH_KNOB: f32 = 18.0;

fn chip_radius() -> CornerRadius {
    CornerRadius::same((CHIP_HEIGHT / 2.0) as u8)
}

/// Sets up the contents of a menu or popover: 32 point rows with a rounded hover fill.
pub fn menu_body(ui: &mut egui::Ui) {
    let pal = Palette::of(ui.ctx());
    let radius = CornerRadius::same(CONTROL_RADIUS);
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = MENU_ROW;
    spacing.item_spacing.y = 2.0;
    spacing.button_padding = Vec2::new(10.0, 0.0);
    ui.visuals_mut().override_text_color = None;
    let w = &mut ui.visuals_mut().widgets;
    for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = radius;
        state.fg_stroke.color = pal.text;
        state.bg_stroke = Stroke::NONE;
    }
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.hovered.weak_bg_fill = pal.hover;
    w.active.weak_bg_fill = pal.sel;
    w.open.weak_bg_fill = pal.hover;
}

/// A drop-down chip: `text` with a caret that opens a menu; named `accessible` for assistive
/// technology. An `on` chip (something other than the default is chosen) uses the pill
/// colours, an off one the field colours.
pub fn chip<R>(
    ui: &mut egui::Ui,
    accessible: &str,
    text: &str,
    on: bool,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Response {
    let pal = Palette::of(ui.ctx());
    let (fill, fg) = if on {
        (pal.pill, pal.pilltext)
    } else {
        (pal.field, pal.text)
    };
    let r = ui
        .scope(|ui| {
            let spacing = ui.spacing_mut();
            spacing.interact_size.y = CHIP_HEIGHT;
            spacing.button_padding = Vec2::new(12.0, 0.0);
            let hover = if on { fill } else { pal.hover };
            let w = &mut ui.visuals_mut().widgets;
            for (state, bg) in [
                (&mut w.inactive, fill),
                (&mut w.hovered, hover),
                (&mut w.active, hover),
                (&mut w.open, hover),
            ] {
                state.weak_bg_fill = bg;
                state.bg_stroke = Stroke::NONE;
                state.corner_radius = chip_radius();
                state.fg_stroke.color = fg;
            }
            let job = super::dropdown_job(ui, text);
            ui.menu_button(job, |ui| {
                menu_body(ui);
                add_contents(ui)
            })
            .response
        })
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), accessible));
    r
}

/// A filter chip: fully rounded, the pill colours while `active`, the field colour otherwise.
/// Named `accessible` for assistive technology; true when clicked.
pub fn filter_chip(ui: &mut egui::Ui, text: &str, accessible: &str, active: bool) -> bool {
    let pal = Palette::of(ui.ctx());
    let font = TextStyle::Body.resolve(ui.style());
    let fg = if active { pal.pilltext } else { pal.text };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let size = Vec2::new(galley.size().x + 24.0, CHIP_HEIGHT);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if active {
            pal.pill
        } else if r.hovered() {
            pal.hover
        } else {
            pal.field
        };
        ui.painter().rect_filled(rect, chip_radius(), fill);
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, fg);
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, active, accessible));
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A switch (40 by 24 points, knob 18) with its `label` beside it. Behaves like a checkbox for
/// assistive technology; the label is part of the click area.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> Response {
    const GAP: f32 = 10.0;
    let pal = Palette::of(ui.ctx());
    let font = TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, pal.text);
    let size = Vec2::new(SWITCH_SIZE.x + GAP + galley.size().x, SWITCH_SIZE.y);
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let track_rect = egui::Rect::from_min_size(rect.min, SWITCH_SIZE);
        let track = if *on { super::SWITCH_ON } else { pal.sel };
        ui.painter()
            .rect_filled(track_rect, CornerRadius::same(12), track);
        let inset = (SWITCH_SIZE.y - SWITCH_KNOB) / 2.0;
        let x = if *on {
            track_rect.right() - inset - SWITCH_KNOB / 2.0
        } else {
            track_rect.left() + inset + SWITCH_KNOB / 2.0
        };
        ui.painter().circle_filled(
            egui::pos2(x, track_rect.center().y),
            SWITCH_KNOB / 2.0,
            Color32::WHITE,
        );
        let text_pos = egui::pos2(
            track_rect.right() + GAP,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, pal.text);
    }
    let value = *on;
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), value, label)
    });
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The frame of the floating pill: the pill colours, fully rounded, 44 points high.
pub fn pill_frame(ctx: &egui::Context) -> Frame {
    let pal = Palette::of(ctx);
    Frame::new()
        .fill(pal.pill)
        .stroke(Stroke::new(1.0, pal.line))
        .corner_radius(CornerRadius::same((PILL_HEIGHT / 2.0) as u8))
        .inner_margin(Margin::symmetric(18, 0))
        .shadow(ctx.global_style().visuals.popup_shadow)
}

/// Buttons on the pill: pill text on a faint veil that grows on hover.
pub fn pill_buttons(ui: &mut egui::Ui) {
    let pal = Palette::of(ui.ctx());
    let v = ui.visuals_mut();
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.fg_stroke.color = pal.pilltext;
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = CornerRadius::same((CHIP_HEIGHT / 2.0) as u8);
        w.weak_bg_fill = Color32::TRANSPARENT;
    }
    v.widgets.hovered.weak_bg_fill = pal.pilltext.gamma_multiply(0.16);
    v.widgets.open.weak_bg_fill = pal.pilltext.gamma_multiply(0.16);
    v.widgets.active.weak_bg_fill = pal.pilltext.gamma_multiply(0.26);
    v.override_text_color = Some(pal.pilltext);
}
