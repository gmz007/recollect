//! Controls shared by the settings sections and the welcome screens.

use crate::ui::style::{self, Palette};
use egui::{CornerRadius, Galley, Margin, Pos2, Sense, Stroke, Vec2};
use std::sync::Arc;

/// "512 MB", "1 GB", "1.5 GB".
pub(super) fn fmt_space(mb: f64) -> String {
    if mb >= 1024.0 {
        let gb = mb / 1024.0;
        if (gb - gb.round()).abs() < 0.05 {
            format!("{} GB", gb.round() as u64)
        } else {
            format!("{gb:.1} GB")
        }
    } else {
        format!("{} MB", mb.round() as u64)
    }
}

/// Reads "2 GB", "512 MB" or a bare number of megabytes.
pub(super) fn parse_space(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase();
    if let Some(n) = t.strip_suffix("gb") {
        n.trim().parse::<f64>().ok().map(|g| g * 1024.0)
    } else {
        t.strip_suffix("mb").unwrap_or(&t).trim().parse().ok()
    }
}

/// A row of mutually exclusive choices. Returns the one clicked, if it is not the current.
pub(super) fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    items: &[(T, &str)],
    current: T,
) -> Option<T> {
    let pal = Palette::of(ui.ctx());
    let mut picked = None;
    egui::Frame::new()
        .fill(pal.field)
        .corner_radius(CornerRadius::same(style::CONTROL_RADIUS))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                // A right-to-left row lays the options out backwards: undo that.
                let mut order: Vec<&(T, &str)> = items.iter().collect();
                if ui.layout().prefer_right_to_left() {
                    order.reverse();
                }
                for (value, label) in order {
                    let selected = *value == current;
                    if segment_button(ui, pal, label, selected).clicked() && !selected {
                        picked = Some(*value);
                    }
                }
            });
        });
    picked
}

/// One option of a [`segmented`] row: its label centred on a filled or hovered background.
fn segment_button(ui: &mut egui::Ui, pal: Palette, label: &str, selected: bool) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let g = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, pal.text);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(g.size().x + 28.0, 30.0), Sense::click());
    let fg = if selected { pal.onaccent } else { pal.text };
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(7), pal.accent);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(7), pal.line);
    }
    let g = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::TextStyle::Body.resolve(ui.style()),
        fg,
    );
    ui.painter().galley(
        Pos2::new(
            rect.center().x - g.size().x / 2.0,
            rect.center().y - g.size().y / 2.0,
        ),
        g,
        fg,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The laid-out texts of a [`choice_card`].
struct CardText {
    title: Arc<Galley>,
    blurb: Arc<Galley>,
    note: Option<Arc<Galley>>,
}

impl CardText {
    /// Lays the texts out `text_w` wide.
    fn measure(
        ui: &egui::Ui,
        pal: Palette,
        text_w: f32,
        title: &str,
        blurb: &str,
        note: &str,
    ) -> Self {
        let painter = ui.painter();
        Self {
            title: painter.layout(
                title.to_string(),
                egui::FontId::new(15.0, style::semibold_family()),
                pal.text,
                text_w,
            ),
            blurb: painter.layout(
                blurb.to_string(),
                egui::TextStyle::Body.resolve(ui.style()),
                pal.text2,
                text_w,
            ),
            note: (!note.is_empty()).then(|| {
                painter.layout(
                    note.to_string(),
                    egui::TextStyle::Small.resolve(ui.style()),
                    pal.accent,
                    text_w,
                )
            }),
        }
    }

    /// The card's height: padding around the stacked texts.
    fn height(&self) -> f32 {
        28.0 + self.title.size().y
            + 4.0
            + self.blurb.size().y
            + self.note.as_ref().map_or(0.0, |g| 4.0 + g.size().y)
    }
}

/// A large choice with a title and a description. Returns true when clicked.
pub(in crate::ui) fn choice_card(
    ui: &mut egui::Ui,
    selected: bool,
    title: &str,
    blurb: &str,
    note: &str,
) -> bool {
    let pal = Palette::of(ui.ctx());
    let width = ui.available_width();
    let text = CardText::measure(ui, pal, width - 56.0, title, blurb, note);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, text.height()), Sense::click());
    paint_choice_card_background(ui, pal, rect, resp.hovered(), selected);
    paint_choice_card_text(ui, pal, rect, selected, text);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, title));
    // The text drawn under the title is what assistive technology reads as its description.
    ui.ctx().accesskit_node_builder(resp.id, |b| {
        b.set_description(blurb);
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The card's fill (accent tint when selected, lighter when hovered) and its outline.
fn paint_choice_card_background(
    ui: &egui::Ui,
    pal: Palette,
    rect: egui::Rect,
    hovered: bool,
    selected: bool,
) {
    let painter = ui.painter();
    let radius = CornerRadius::same(style::CARD_RADIUS);
    painter.rect_filled(
        rect,
        radius,
        if selected {
            pal.accent.gamma_multiply(0.14)
        } else if hovered {
            pal.line
        } else {
            pal.field
        },
    );
    if selected {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
}

/// The radio dot and the stacked title, description and note.
fn paint_choice_card_text(
    ui: &egui::Ui,
    pal: Palette,
    rect: egui::Rect,
    selected: bool,
    text: CardText,
) {
    let painter = ui.painter();
    let dot = Pos2::new(
        rect.min.x + 26.0,
        rect.min.y + 14.0 + text.title.size().y / 2.0,
    );
    painter.circle_stroke(
        dot,
        9.0,
        Stroke::new(1.5, if selected { pal.accent } else { pal.text2 }),
    );
    if selected {
        painter.circle_filled(dot, 5.0, pal.accent);
    }
    let x = rect.min.x + 48.0;
    let mut y = rect.min.y + 14.0;
    let h = text.title.size().y + 4.0;
    painter.galley(Pos2::new(x, y), text.title, pal.text);
    y += h;
    let h = text.blurb.size().y + 4.0;
    painter.galley(Pos2::new(x, y), text.blurb, pal.text2);
    y += h;
    if let Some(g) = text.note {
        painter.galley(Pos2::new(x, y), g, pal.accent);
    }
}

/// Whether a slider draft written in frame `written` still belongs to a drag that is going on
/// in frame `now`. A drag writes its draft every frame, so an older one is left over from a
/// window that was closed while the button was held, and must not be applied later.
fn draft_is_live(written: u64, now: u64) -> bool {
    now.saturating_sub(written) <= 1
}

/// A slider whose value is applied when the user lets go (or types a value or uses the
/// keyboard), not on every step of a drag. `current` is the saved value.
pub(super) fn committed_slider(
    ui: &mut egui::Ui,
    name: &str,
    current: u64,
    range: std::ops::RangeInclusive<u64>,
    step: f64,
    format: fn(f64) -> String,
) -> Option<u64> {
    let key = egui::Id::unique(("settings-slider", name));
    let now = ui.ctx().cumulative_frame_nr();
    let kept = ui
        .data(|d| d.get_temp::<(u64, u64)>(key))
        .filter(|&(_, written)| draft_is_live(written, now))
        .map(|(value, _)| value);
    if kept.is_none() {
        ui.data_mut(|d| d.remove_temp::<(u64, u64)>(key));
    }
    let mut v = kept.unwrap_or(current) as f64;
    let slider = egui::Slider::new(&mut v, *range.start() as f64..=*range.end() as f64)
        .step_by(step)
        .custom_formatter(move |n, _| format(n))
        .custom_parser(parse_space);
    ui.spacing_mut().slider_width = 260.0;
    let r = ui.add(slider);
    r.widget_info(|| egui::WidgetInfo::slider(true, v, name));
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, (v as u64, now)));
    } else if let Some(value) = kept.filter(|_| r.dragged()) {
        // Held still: the draft stays current.
        ui.data_mut(|d| d.insert_temp(key, (value, now)));
    }
    let draft = ui.data(|d| d.get_temp::<(u64, u64)>(key));
    match draft {
        Some((value, _)) if !r.dragged() => {
            ui.data_mut(|data| data.remove_temp::<(u64, u64)>(key));
            Some(value)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::draft_is_live;

    #[test]
    fn a_draft_from_the_previous_frame_is_live_but_an_older_one_is_not() {
        assert!(draft_is_live(10, 10));
        assert!(draft_is_live(10, 11));
        assert!(!draft_is_live(10, 12));
        assert!(!draft_is_live(10, 500));
    }
}
