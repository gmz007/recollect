//! The app mark: a large blue circle with a small amber one, drawn with the painter so it is
//! crisp at every size and takes the colour behind it for the gap.

use egui::{Color32, Rect, Sense, Ui, Vec2};

/// The side of the grid the mark is designed on.
const GRID: f32 = 128.0;
const BLUE: Color32 = Color32::from_rgb(0x33, 0x66, 0xE6);
const AMBER: Color32 = Color32::from_rgb(0xFF, 0xB5, 0x47);
/// Width of the gap between the circles, in grid units.
const GAP: f32 = 6.0;

/// Paints the mark into `rect`; `background` is the colour behind it (it cuts the gap).
pub fn paint_app_mark(painter: &egui::Painter, rect: Rect, background: Color32) {
    let k = rect.width() / GRID;
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * k;
    painter.circle_filled(at(54.0, 74.0), 34.0 * k, BLUE);
    // The outline is centred on the circle's edge and drawn over its fill, so the amber that
    // shows is half the outline smaller than the circle.
    painter.circle_filled(at(88.0, 40.0), (18.0 + GAP / 2.0) * k, background);
    painter.circle_filled(at(88.0, 40.0), (18.0 - GAP / 2.0) * k, AMBER);
}

/// Allocates a `size` square and paints the mark on `background`.
pub fn app_mark(ui: &mut Ui, size: f32, background: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_app_mark(ui.painter(), rect, background);
}
