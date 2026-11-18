//! Small pill-shaped chips that appear beside the content for a moment and fade away (the date
//! at the ruler, the photo size).

use super::Palette;
use egui::{Align2, CornerRadius, Id, Margin, Pos2};
use std::time::Duration;

/// How often a fading chip is redrawn while it fades.
const FADE_STEP: Duration = Duration::from_millis(30);

/// The opacity of a chip `age` after it last changed, when it stays for `stay` and spends the
/// last `fade` of that fading out. Asks for a repaint only when something is about to change:
/// the start of the fade, then every step of it.
pub fn fade_opacity(ctx: &egui::Context, age: Duration, stay: Duration, fade: Duration) -> f32 {
    let Some(left) = stay.checked_sub(age).filter(|l| !l.is_zero()) else {
        return 0.0;
    };
    if let Some(solid) = left.checked_sub(fade).filter(|s| !s.is_zero()) {
        ctx.request_repaint_after(solid);
        return 1.0;
    }
    ctx.request_repaint_after(FADE_STEP);
    left.as_secs_f32() / fade.as_secs_f32()
}

/// Shows a pill chip with `add_contents` at `pos` (`pivot` is the point of the chip that sits
/// there), above everything else, at `opacity`.
pub fn pill_chip(
    ctx: &egui::Context,
    id: Id,
    pos: Pos2,
    pivot: Align2,
    opacity: f32,
    add_contents: impl FnOnce(&mut egui::Ui, &Palette),
) {
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(pivot)
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            // A chip is one line. Pinned by its right edge, the area only has the width it had
            // last frame, so a longer text ("May" → "September") would otherwise wrap.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let pal = Palette::of(ui.ctx());
            egui::Frame::new()
                .fill(pal.pill)
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::symmetric(12, 5))
                .shadow(ui.ctx().global_style().visuals.popup_shadow)
                .show(ui, |ui| add_contents(ui, &pal));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAY: Duration = Duration::from_millis(1000);
    const FADE: Duration = Duration::from_millis(300);

    #[test]
    fn a_chip_is_solid_then_fades_then_is_gone() {
        let ctx = egui::Context::default();
        let at = |ms| fade_opacity(&ctx, Duration::from_millis(ms), STAY, FADE);
        assert!((at(0) - 1.0).abs() < f32::EPSILON);
        assert!((at(700) - 1.0).abs() < 1e-3);
        assert!((at(850) - 0.5).abs() < 1e-3);
        assert!(at(1000).abs() < f32::EPSILON);
        assert!(at(5000).abs() < f32::EPSILON);
    }

    /// The height of a chip pinned by its right edge near the window's right side after it showed
    /// `before` for a few frames and then `after`.
    fn chip_height(before: &str, after: &str) -> f32 {
        let ctx = egui::Context::default();
        let id = Id::unique("chip");
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        for text in [before, before, before, after, after, after] {
            let mut out = ctx.run_ui(input(), |ui| {
                pill_chip(
                    ui.ctx(),
                    id,
                    egui::pos2(772.0, 300.0),
                    Align2::RIGHT_CENTER,
                    1.0,
                    |ui, _| {
                        ui.label(egui::RichText::new(text).size(11.5));
                    },
                );
            });
            out.textures_delta.clear();
        }
        ctx.memory(|m| m.area_rect(id))
            .expect("the chip is shown")
            .height()
    }

    #[test]
    fn a_chip_keeps_its_words_on_one_line_when_they_grow() {
        let short = chip_height("May 2018", "May 2018");
        let grown = chip_height("May 2018", "September 2018");
        assert!(
            (grown - short).abs() < 1.0,
            "the longer date wrapped: {grown} high instead of {short}"
        );
    }
}
