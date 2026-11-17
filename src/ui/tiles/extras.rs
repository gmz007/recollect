//! The extra buttons of a result tile: "Find similar" at the top right while hovered, the pill
//! of a video that plays it at the matched moment (bottom left, right of the heart), and the
//! capture year (bottom right).

use super::{CIRCLE_INSET, Tile, TileAction, ZONE, paint_icon, picture_area};
use crate::ui::style;
use egui::{Color32, Pos2, Rect, Sense, Vec2};
use egui_material_icons::icons;

/// Diameter of the "Find similar" button and size of its icon.
const SIMILAR_DIAMETER: f32 = 26.0;
const SIMILAR_ICON: f32 = 16.0;
/// Size of the play icon in the pill.
const PILL_ICON: f32 = 14.0;
/// Height of the pill, and its distance from the bottom edge of the photo.
const PILL_HEIGHT: f32 = 22.0;
const PILL_MARGIN: f32 = 6.0;
/// Size of the year's text, its padding, and its distance from the corner.
const YEAR_SIZE: f32 = 11.0;
const YEAR_PAD: Vec2 = Vec2::new(5.0, 2.0);
const YEAR_MARGIN: f32 = 6.0;
/// The buttons only some tiles have: "Find similar" and the playable pill.
pub(super) struct ExtraZones {
    similar: Option<egui::Response>,
    play: Option<egui::Response>,
}

impl ExtraZones {
    pub(super) fn hovered(&self) -> bool {
        [&self.similar, &self.play]
            .into_iter()
            .flatten()
            .any(egui::Response::hovered)
    }

    pub(super) fn action(&self) -> Option<TileAction> {
        let clicked = |r: &Option<egui::Response>| r.as_ref().is_some_and(egui::Response::clicked);
        if clicked(&self.similar) {
            Some(TileAction::FindSimilar)
        } else if clicked(&self.play) {
            Some(TileAction::PlayAtBadge)
        } else {
            None
        }
    }
}

/// Where the "Find similar" button of a tile is: a square at the top right of the photo.
fn similar_zone(photo: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(photo.max.x - ZONE, photo.min.y),
        Vec2::splat(ZONE),
    )
}

/// Creates (and describes for assistive technology) the extra buttons of `tile`. Small tiles
/// have none: a click there goes to the tile itself.
pub(super) fn extra_zones(
    ui: &egui::Ui,
    id: egui::Id,
    rect: Rect,
    tile: &Tile<'_>,
    small: bool,
) -> ExtraZones {
    let photo = picture_area(rect, tile.checked());
    // While picking photos the buttons are off: a click anywhere selects the tile.
    let active = !small && !tile.extras.selecting;
    let similar = (tile.extras.similar && active).then(|| {
        let r = ui.interact(similar_zone(photo), id.with("similar"), Sense::click());
        let label = format!("Find similar to {}", (tile.name)());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        // The icon alone doesn't say what it does.
        r.on_hover_text("Find similar")
    });
    let play = tile
        .badge
        .as_ref()
        .filter(|_| tile.extras.plays && active)
        .and_then(|text| {
            let (pill, _) = pill_layout(&ui.painter_at(rect), photo, text, false, true)?;
            let r = ui.interact(pill, id.with("play"), Sense::click());
            let label = format!("Play at {text}");
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            Some(r.on_hover_text(label.as_str()))
        });
    ExtraZones { similar, play }
}

/// Where a pill whose text is `text_width` wide goes in `photo`: bottom right, or with
/// `left_side` bottom left, right of the heart's area so the two never overlap (at the corner on
/// small tiles, which have no heart), or `None` when the tile is too small for it.
fn pill_rect(photo: Rect, text_width: f32, small: bool, left_side: bool) -> Option<Rect> {
    let size = Vec2::new(text_width + PILL_ICON + 14.0, PILL_HEIGHT);
    let y = photo.max.y - PILL_MARGIN - size.y;
    let x = if !left_side {
        photo.max.x - PILL_MARGIN - size.x
    } else if small {
        photo.min.x + PILL_MARGIN
    } else {
        photo.min.x + ZONE
    };
    let pill = Rect::from_min_size(Pos2::new(x, y), size);
    let fits = if left_side {
        pill.max.x <= photo.max.x - PILL_MARGIN
    } else {
        pill.width() <= photo.width() - 8.0
    };
    fits.then_some(pill)
}

/// The pill's rectangle and text layout in `photo`.
fn pill_layout(
    painter: &egui::Painter,
    photo: Rect,
    text: &str,
    small: bool,
    left_side: bool,
) -> Option<(Rect, std::sync::Arc<egui::Galley>)> {
    let font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
    let galley = painter.layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let pill = pill_rect(photo, galley.size().x, small, left_side)?;
    Some((pill, galley))
}

/// A dark pill with a play icon and `text`. Returns where it ends (its right edge), or `None` when it did not fit.
pub(super) fn paint_pill(
    painter: &egui::Painter,
    photo: Rect,
    text: &str,
    small: bool,
    left_side: bool,
) -> Option<f32> {
    let (pill, galley) = pill_layout(painter, photo, text, small, left_side)?;
    painter.rect_filled(pill, 11.0, Color32::from_black_alpha(165));
    paint_icon(
        painter,
        Pos2::new(pill.min.x + 11.0, pill.center().y),
        icons::ICON_PLAY_ARROW,
        PILL_ICON + 2.0,
        Color32::WHITE,
    );
    painter.galley(
        Pos2::new(
            pill.min.x + 8.0 + PILL_ICON,
            pill.center().y - galley.size().y / 2.0,
        ),
        galley,
        Color32::WHITE,
    );
    Some(pill.max.x)
}

/// The capture year at the bottom right: small white text on a soft dark shade. Left out when
/// it would touch the pill, which ends at `pill_end`.
pub(super) fn paint_year(painter: &egui::Painter, photo: Rect, year: i32, pill_end: Option<f32>) {
    let font = egui::FontId::new(YEAR_SIZE, egui::FontFamily::Proportional);
    let galley = painter.layout_no_wrap(year.to_string(), font, style::OVERLAY_WHITE);
    let size = galley.size() + YEAR_PAD * 2.0;
    let chip = Rect::from_min_size(photo.max - size - Vec2::splat(YEAR_MARGIN), size);
    let clear_of_pill = pill_end.is_none_or(|x| chip.min.x >= x + 4.0);
    if !clear_of_pill || chip.min.x < photo.min.x + YEAR_MARGIN {
        return;
    }
    painter.rect_filled(chip, 4.0, style::OVERLAY_BLACK);
    painter.galley(chip.min + YEAR_PAD, galley, style::OVERLAY_WHITE);
}

/// The "Find similar" button at the top right of the photo: a dark round button.
pub(super) fn paint_similar(ui: &egui::Ui, painter: &egui::Painter, photo: Rect) {
    let zone = similar_zone(photo);
    let centre = Pos2::new(
        photo.max.x - CIRCLE_INSET - SIMILAR_DIAMETER / 2.0,
        photo.min.y + CIRCLE_INSET + SIMILAR_DIAMETER / 2.0,
    );
    let hot = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| zone.contains(p));
    let alpha = if hot { 190 } else { 120 };
    painter.circle_filled(
        centre,
        SIMILAR_DIAMETER / 2.0,
        Color32::from_black_alpha(alpha),
    );
    paint_icon(
        painter,
        centre,
        icons::ICON_IMAGE_SEARCH,
        SIMILAR_ICON,
        style::OVERLAY_WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heart's clickable area on `photo` (as `interact_zones` makes it).
    fn heart_zone(photo: Rect) -> Rect {
        Rect::from_min_size(
            Pos2::new(photo.min.x, photo.max.y - ZONE),
            Vec2::splat(ZONE),
        )
    }

    /// Whether two rectangles share area (touching edges do not count).
    fn overlap(a: Rect, b: Rect) -> bool {
        a.intersect(b).width() > 0.0 && a.intersect(b).height() > 0.0
    }

    #[test]
    fn the_pill_never_overlaps_the_heart_even_with_a_long_time() {
        // "1:02:03" in the pill's font is about 46 points wide, "0:00" about 26.
        for text_width in [26.0, 46.0] {
            for side in [72.0, 96.0, 120.0, 200.0, 320.0] {
                let photo = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::splat(side));
                if let Some(pill) = pill_rect(photo, text_width, false, true) {
                    assert!(
                        !overlap(pill, heart_zone(photo)),
                        "pill {pill:?} on a {side} point tile"
                    );
                    assert!(photo.contains_rect(pill));
                }
            }
        }
    }

    #[test]
    fn a_long_time_has_no_pill_on_a_tile_of_96_points() {
        let photo = Rect::from_min_size(Pos2::ZERO, Vec2::splat(96.0));
        assert!(pill_rect(photo, 46.0, false, true).is_none());
        assert!(pill_rect(photo, 26.0, false, true).is_some());
    }
}
