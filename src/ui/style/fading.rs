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
}
