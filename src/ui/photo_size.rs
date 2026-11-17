//! Photo size: how big the tiles of the grids are. A pinch on a trackpad, Ctrl/Cmd and the mouse
//! wheel, and Ctrl/Cmd with `=`, `-` and `0` change it; a small "Photo size" chip shows the
//! level while it changes. Settings has the same choice as Small, Medium and Large.

use super::style::{Palette, fade_opacity, pill_chip};
use super::{App, TOP_BAR_HEIGHT};
use crate::config::{PHOTO_SIZE_DEFAULT, PHOTO_SIZE_MAX, PHOTO_SIZE_MIN, clamp_photo_size};
use egui::{CornerRadius, Vec2};
use std::time::{Duration, Instant};

/// The sizes the keys step through.
const STOPS: [f32; 8] = [96.0, 120.0, 144.0, 168.0, 204.0, 240.0, 280.0, 320.0];
/// How long the chip stays after the last change, and how much of that it spends fading.
const CHIP_FOR: Duration = Duration::from_millis(1000);
const CHIP_FADE: Duration = Duration::from_millis(300);
/// How long after the last change the size is saved to the settings.
const SAVE_AFTER: Duration = Duration::from_millis(600);
/// Length and thickness of the level bar in the chip.
const BAR: Vec2 = Vec2::new(64.0, 4.0);
/// Distance of the chip below the top bar.
const CHIP_TOP: f32 = 10.0;
/// Differences below this many points do not count as a change.
const EPSILON: f32 = 0.5;

/// The named sizes of the Settings choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PhotoSizeLevel {
    Small,
    Medium,
    Large,
}

impl PhotoSizeLevel {
    pub(super) const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    /// The tile side of this level, in points.
    pub(super) fn points(self) -> f32 {
        match self {
            Self::Small => 120.0,
            Self::Medium => PHOTO_SIZE_DEFAULT,
            Self::Large => 240.0,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    /// The level that `size` is exactly, or `None` for a size in between.
    pub(super) fn of(size: f32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| (l.points() - size).abs() < EPSILON)
    }
}

/// The next stop above `size`.
fn bigger(size: f32) -> f32 {
    STOPS
        .into_iter()
        .find(|&s| s > size + EPSILON)
        .unwrap_or(PHOTO_SIZE_MAX)
}

/// The next stop below `size`.
fn smaller(size: f32) -> f32 {
    STOPS
        .into_iter()
        .rev()
        .find(|&s| s < size - EPSILON)
        .unwrap_or(PHOTO_SIZE_MIN)
}

/// What the user asked for this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SizeInput {
    /// Pinch or Ctrl/Cmd and scroll: a factor.
    Factor(f32),
    Bigger,
    Smaller,
    Reset,
}

impl SizeInput {
    /// The size that results from applying this to `size`.
    fn apply(self, size: f32) -> f32 {
        clamp_photo_size(match self {
            Self::Factor(f) => size * f,
            Self::Bigger => bigger(size),
            Self::Smaller => smaller(size),
            Self::Reset => PHOTO_SIZE_DEFAULT,
        })
    }
}

/// When the size last changed (for the chip) and whether the new size is still to be saved.
#[derive(Default)]
pub(super) struct SizeChange {
    at: Option<Instant>,
    unsaved: bool,
}

impl SizeChange {
    /// Whether the latest size is not in the settings yet.
    pub(super) fn is_unsaved(&self) -> bool {
        self.unsaved
    }
}

/// Takes the Ctrl/Cmd key presses that change the size out of this frame's events.
fn take_size_keys(ctx: &egui::Context) -> Vec<SizeInput> {
    use egui::Key;
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = e
            else {
                return true;
            };
            if !modifiers.command || modifiers.alt {
                return true;
            }
            let input = match key {
                Key::Equals | Key::Plus => SizeInput::Bigger,
                Key::Minus => SizeInput::Smaller,
                Key::Num0 => SizeInput::Reset,
                _ => return true,
            };
            out.push(input);
            false
        });
    });
    out
}

impl App {
    /// Reads this frame's pinch, Ctrl/Cmd and scroll, and size keys while a grid is shown.
    pub(super) fn handle_photo_size(&mut self, ctx: &egui::Context, grid_shown: bool) {
        if !grid_shown || self.dialogs.settings_open || self.dialog_open() {
            return;
        }
        let mut inputs = Vec::new();
        let factor = ctx.input(egui::InputState::zoom_delta);
        if (factor - 1.0).abs() > f32::EPSILON {
            inputs.push(SizeInput::Factor(factor));
        }
        if !self.keys_blocked(ctx) {
            inputs.extend(take_size_keys(ctx));
        }
        let size = inputs
            .into_iter()
            .fold(self.tl.tile_size, |size, input| input.apply(size));
        self.set_photo_size(size);
    }

    /// Makes tiles `size` points wide; shows the chip if that is a change. The new size is
    /// saved once it stops changing.
    pub(super) fn set_photo_size(&mut self, size: f32) {
        let size = clamp_photo_size(size);
        if (size - self.tl.tile_size).abs() < f32::EPSILON {
            return;
        }
        self.tl.tile_size = size;
        self.size_change.at = Some(Instant::now());
        self.size_change.unsaved = true;
    }

    /// Settings: one of the named sizes, saved right away.
    pub(super) fn choose_photo_size(&mut self, level: PhotoSizeLevel) {
        self.tl.tile_size = level.points();
        self.size_change.at = None;
        self.size_change.unsaved = false;
        let size = self.tl.tile_size;
        self.change_settings(|s| s.photo_size = size);
    }

    /// Saves the size once it has stopped changing, and draws the chip while it changes.
    pub(super) fn photo_size_chip(&mut self, ctx: &egui::Context) {
        let Some(at) = self.size_change.at else {
            return;
        };
        let age = at.elapsed();
        if self.size_change.unsaved && age >= SAVE_AFTER {
            self.save_photo_size();
        }
        if age >= CHIP_FOR {
            if !self.size_change.unsaved {
                self.size_change.at = None;
            }
            return;
        }
        let alpha = fade_opacity(ctx, age, CHIP_FOR, CHIP_FADE);
        let x = self
            .pill_anchor
            .map_or_else(|| ctx.content_rect().center().x, |a| a.x);
        let pos = egui::pos2(x, TOP_BAR_HEIGHT + CHIP_TOP);
        let level = (self.tl.tile_size - PHOTO_SIZE_MIN) / (PHOTO_SIZE_MAX - PHOTO_SIZE_MIN);
        pill_chip(
            ctx,
            egui::Id::NULL.with("photo-size-chip"),
            pos,
            egui::Align2::CENTER_TOP,
            alpha,
            |ui, pal| draw_chip(ui, pal, level),
        );
    }

    /// Whether the chip is gone (tests wait for it to fade).
    #[cfg(test)]
    pub(super) fn size_change_idle(&self) -> bool {
        self.size_change.at.is_none()
    }

    /// Stores the size in the settings (if it differs from what is stored).
    pub(super) fn save_photo_size(&mut self) {
        self.size_change.unsaved = false;
        let size = self.tl.tile_size;
        if (self.settings.photo_size - size).abs() >= f32::EPSILON {
            self.change_settings(|s| s.photo_size = size);
        }
    }
}

/// The contents of the chip: "Photo size" and a bar that shows where the size is between
/// smallest and largest.
fn draw_chip(ui: &mut egui::Ui, pal: &Palette, level: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(
            egui::RichText::new("Photo size")
                .size(12.0)
                .color(pal.pilltext),
        );
        let (rect, _) = ui.allocate_exact_size(BAR, egui::Sense::hover());
        let round = CornerRadius::same(2);
        ui.painter()
            .rect_filled(rect, round, pal.pilltext.gamma_multiply(0.3));
        let mut filled = rect;
        filled.max.x = rect.min.x + rect.width() * level.clamp(0.0, 1.0);
        ui.painter().rect_filled(filled, round, pal.pilltext);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_step_through_the_stops() {
        assert!((bigger(168.0) - 204.0).abs() < f32::EPSILON);
        assert!((smaller(168.0) - 144.0).abs() < f32::EPSILON);
        // A size between stops goes to the next one in the direction asked.
        assert!((bigger(180.0) - 204.0).abs() < f32::EPSILON);
        assert!((smaller(180.0) - 168.0).abs() < f32::EPSILON);
        // The ends hold.
        assert!((bigger(320.0) - 320.0).abs() < f32::EPSILON);
        assert!((smaller(96.0) - 96.0).abs() < f32::EPSILON);
    }

    #[test]
    fn gestures_scale_and_stay_in_range() {
        assert!((SizeInput::Factor(1.1).apply(200.0) - 220.0).abs() < 1e-3);
        assert!((SizeInput::Factor(10.0).apply(200.0) - PHOTO_SIZE_MAX).abs() < f32::EPSILON);
        assert!((SizeInput::Factor(0.01).apply(200.0) - PHOTO_SIZE_MIN).abs() < f32::EPSILON);
        assert!((SizeInput::Reset.apply(300.0) - PHOTO_SIZE_DEFAULT).abs() < f32::EPSILON);
    }

    #[test]
    fn named_sizes_are_recognised_only_when_exact() {
        assert_eq!(PhotoSizeLevel::of(120.0), Some(PhotoSizeLevel::Small));
        assert_eq!(PhotoSizeLevel::of(168.0), Some(PhotoSizeLevel::Medium));
        assert_eq!(PhotoSizeLevel::of(240.0), Some(PhotoSizeLevel::Large));
        assert_eq!(PhotoSizeLevel::of(150.0), None);
    }
}
