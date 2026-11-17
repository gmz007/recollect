//! The extra buttons of a result tile: "Find similar" at the top right while hovered, and the
//! pill of a video that plays it at the matched moment.

use super::{CIRCLE_INSET, Tile, TileAction, ZONE, paint_icon, picture_area};
use crate::ui::style;
use egui::{Color32, Pos2, Rect, Sense, Vec2};
use egui_material_icons::icons;

/// Diameter of the "Find similar" button and size of its icon.
const SIMILAR_DIAMETER: f32 = 26.0;
const SIMILAR_ICON: f32 = 16.0;
/// Size of the play icon in the pill.
const PILL_ICON: f32 = 14.0;
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
    let similar = (tile.extras.similar && !small).then(|| {
        let r = ui.interact(similar_zone(photo), id.with("similar"), Sense::click());
        let label = format!("Find similar to {}", (tile.name)());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        r
    });
    let play = tile
        .badge
        .as_ref()
        .filter(|_| tile.extras.plays && !small)
        .and_then(|text| {
            let (pill, _) = pill_layout(&ui.painter_at(rect), photo, text)?;
            let r = ui.interact(pill, id.with("play"), Sense::click());
            let label = format!("Play at {text}");
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            Some(r.on_hover_text(label.as_str()))
        });
    ExtraZones { similar, play }
}

/// Where the pill with the time goes in `rect` (bottom right) and its text, or `None` when
/// the tile is too small for it.
fn pill_layout(
    painter: &egui::Painter,
    rect: Rect,
    text: &str,
) -> Option<(Rect, std::sync::Arc<egui::Galley>)> {
    let font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
    let galley = painter.layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let size = Vec2::new(galley.size().x + PILL_ICON + 14.0, 22.0);
    let pill = Rect::from_min_size(rect.max - size - Vec2::splat(6.0), size);
    (pill.width() <= rect.width() - 8.0).then_some((pill, galley))
}

/// A dark pill with a play icon and `text` in the bottom right corner.
pub(super) fn paint_pill(painter: &egui::Painter, rect: Rect, text: &str) {
    let Some((pill, galley)) = pill_layout(painter, rect, text) else {
        return;
    };
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
