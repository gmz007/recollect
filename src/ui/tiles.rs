//! Photo and video tiles for the grids: a square centre-cropped thumbnail with a selection
//! circle (top left) and a favorite star (bottom left) that appear on hover, and a duration pill
//! on videos.

use super::selection::ClickMods;
use super::style::{self, Palette, icons};
use super::workers::ThumbCache;
use crate::thumbs::ThumbKey;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

/// Size of the clickable areas of the circle and the star.
const ZONE: f32 = 34.0;

/// What a tile shows.
pub(super) struct Tile<'a> {
    pub key: &'a ThumbKey,
    /// File name (for assistive technology and the hover text).
    pub name: &'a str,
    /// Prefix of the accessible name of the tile ("Item", "Result").
    pub role: &'a str,
    /// Text of the pill on videos (a time).
    pub badge: Option<String>,
    pub favorite: bool,
    /// Part of the selection the user made on purpose (circle, Ctrl or Shift).
    pub checked: bool,
    /// The tile the keyboard is on.
    pub focused: bool,
    /// Selection mode: circles stay visible on every tile.
    pub picking: bool,
    /// Whether assistive technology should hear "selected".
    pub selected: bool,
    pub tip: String,
}

/// What the user did to a tile this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TileEvent {
    /// Click on the picture.
    Click,
    /// Click on the selection circle.
    Circle,
    /// Click on the star.
    Star,
}

pub(super) struct TileOut {
    pub resp: egui::Response,
    pub event: Option<(TileEvent, ClickMods)>,
}

/// What a tile event means for the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClickPlan {
    /// Open the item in the viewer.
    Open,
    /// Toggle the favorite mark.
    Favorite,
    /// Change the selection with `mods`. `fresh` starts a new selection (the keyboard's single
    /// selection does not count as part of one the user is picking).
    Select { mods: ClickMods, fresh: bool },
}

/// Decides what a click on a tile does. A plain click opens the item unless the user is already
/// picking items, in which case it toggles; the circle and Ctrl/Cmd toggle; Shift selects a
/// range.
pub(super) fn click_plan(event: TileEvent, mods: ClickMods, picking: bool) -> ClickPlan {
    match event {
        TileEvent::Star => ClickPlan::Favorite,
        TileEvent::Click if mods.is_plain() && !picking => ClickPlan::Open,
        _ if mods.range => ClickPlan::Select {
            mods: ClickMods {
                toggle: false,
                range: true,
            },
            fresh: false,
        },
        _ => ClickPlan::Select {
            mods: ClickMods {
                toggle: true,
                range: false,
            },
            fresh: !picking,
        },
    }
}

/// UV rectangle that crops a texture of `size` to its centred square.
pub(super) fn square_uv(size: Vec2) -> Rect {
    let (w, h) = (size.x.max(1.0), size.y.max(1.0));
    let (u, v) = if w > h {
        ((1.0 - h / w) / 2.0, 0.0)
    } else {
        (0.0, (1.0 - w / h) / 2.0)
    };
    Rect::from_min_max(Pos2::new(u, v), Pos2::new(1.0 - u, 1.0 - v))
}

fn paint_icon(
    painter: &egui::Painter,
    center: Pos2,
    icon: egui_material_icons::MaterialIcon,
    size: f32,
    color: Color32,
) {
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        icon.codepoint,
        style::icon_font_id(icon, size),
        color,
    );
}

/// Draws `tile` into `rect` and reports clicks. `id` must be unique per tile.
pub(super) fn show(
    ui: &mut egui::Ui,
    thumbs: &mut ThumbCache,
    id: egui::Id,
    rect: Rect,
    tile: &Tile<'_>,
) -> TileOut {
    let pal = Palette::of(ui.ctx());
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    let small = rect.width() < 72.0;
    let circle_zone = Rect::from_min_size(rect.min, Vec2::splat(ZONE));
    let star_zone =
        Rect::from_min_size(Pos2::new(rect.min.x, rect.max.y - ZONE), Vec2::splat(ZONE));
    let circle = ui.interact(circle_zone, id.with("select"), Sense::click());
    let star = ui.interact(star_zone, id.with("favorite"), Sense::click());
    let select_label = if tile.checked {
        format!("Deselect {}", tile.name)
    } else {
        format!("Select {}", tile.name)
    };
    circle.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, true, tile.checked, &select_label)
    });
    let star_label = if tile.favorite {
        format!("Remove {} from favorites", tile.name)
    } else {
        format!("Add {} to favorites", tile.name)
    };
    star.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, tile.favorite, &star_label)
    });
    let label = format!("{} {}", tile.role, tile.name);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, tile.selected, &label)
    });

    if ui.is_rect_visible(rect) {
        paint(ui, thumbs, rect, tile, hovered, small, &pal);
    }

    let mut event = None;
    if circle.clicked() {
        event = Some((TileEvent::Circle, ClickMods::of_click(ui.ctx())));
    } else if star.clicked() {
        event = Some((TileEvent::Star, ClickMods::default()));
    } else if resp.clicked() {
        event = Some((TileEvent::Click, ClickMods::of_click(ui.ctx())));
    }
    let hand = hovered && (circle.hovered() || star.hovered() || resp.hovered());
    if hand {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = if circle.hovered() || star.hovered() {
        resp
    } else {
        resp.on_hover_text(tile.tip.clone())
    };
    TileOut { resp, event }
}

fn paint(
    ui: &egui::Ui,
    thumbs: &mut ThumbCache,
    rect: Rect,
    tile: &Tile<'_>,
    hovered: bool,
    small: bool,
    pal: &Palette,
) {
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(style::TILE_RADIUS);
    let failed = thumbs.failed(tile.key);
    if let Some(tex) = thumbs.get(tile.key) {
        let shape = egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(tex.id(), square_uv(tex.size_vec2()));
        painter.add(shape);
    } else {
        painter.rect_filled(rect, radius, pal.surface);
        if failed && rect.width() > 110.0 {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Preview unavailable",
                egui::TextStyle::Small.resolve(ui.style()),
                pal.text_secondary,
            );
        }
    }

    let show_controls = (hovered || tile.picking || tile.checked) && !small;
    if hovered {
        // A soft shade in the top corner keeps the circle readable on bright photos.
        painter.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(rect.width(), 44.0)),
            CornerRadius {
                nw: style::TILE_RADIUS,
                ne: style::TILE_RADIUS,
                sw: 0,
                se: 0,
            },
            Color32::from_black_alpha(48),
        );
    }
    if show_controls {
        let c = circle_center(rect);
        if tile.checked {
            painter.circle_filled(c, 11.0, pal.accent);
            paint_icon(&painter, c, icons::ICON_CHECK, 16.0, pal.on_accent);
        } else {
            painter.circle_filled(c, 11.0, Color32::from_black_alpha(70));
            painter.circle_stroke(c, 10.0, Stroke::new(2.0, Color32::WHITE));
        }
    }
    if tile.favorite && !small {
        let c = star_center(rect);
        painter.circle_filled(c, 12.0, Color32::from_black_alpha(90));
        paint_icon(&painter, c, icons::ICON_STAR, 18.0, pal.favorite);
    } else if hovered && !small {
        let c = star_center(rect);
        painter.circle_filled(c, 12.0, Color32::from_black_alpha(90));
        paint_icon(
            &painter,
            c,
            icons::ICON_STAR.outlined(),
            18.0,
            Color32::WHITE,
        );
    }
    if let Some(text) = &tile.badge {
        paint_pill(&painter, rect, text);
    }
    if tile.checked {
        painter.rect_stroke(
            rect.shrink(1.5),
            radius,
            Stroke::new(3.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    } else if tile.focused {
        painter.rect_stroke(
            rect.shrink(1.0),
            radius,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
}

fn circle_center(rect: Rect) -> Pos2 {
    rect.min + Vec2::splat(ZONE / 2.0)
}

fn star_center(rect: Rect) -> Pos2 {
    Pos2::new(rect.min.x + ZONE / 2.0, rect.max.y - ZONE / 2.0)
}

/// A dark pill with a play icon and `text` in the bottom right corner.
fn paint_pill(painter: &egui::Painter, rect: Rect, text: &str) {
    let font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
    let galley = painter.layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let icon = 14.0;
    let size = Vec2::new(galley.size().x + icon + 14.0, 22.0);
    let pill = Rect::from_min_size(rect.max - size - Vec2::splat(6.0), size);
    if pill.width() > rect.width() - 8.0 {
        return;
    }
    painter.rect_filled(pill, 11.0, Color32::from_black_alpha(165));
    paint_icon(
        painter,
        Pos2::new(pill.min.x + 11.0, pill.center().y),
        icons::ICON_PLAY_ARROW,
        icon + 2.0,
        Color32::WHITE,
    );
    painter.galley(
        Pos2::new(
            pill.min.x + 8.0 + icon,
            pill.center().y - galley.size().y / 2.0,
        ),
        galley,
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_open_or_select_depending_on_the_mode() {
        let plain = ClickMods::default();
        let ctrl = ClickMods {
            toggle: true,
            range: false,
        };
        let shift = ClickMods {
            toggle: false,
            range: true,
        };
        assert_eq!(click_plan(TileEvent::Click, plain, false), ClickPlan::Open);
        assert_eq!(
            click_plan(TileEvent::Star, plain, false),
            ClickPlan::Favorite
        );
        // The circle starts a fresh selection, then adds to it.
        assert_eq!(
            click_plan(TileEvent::Circle, plain, false),
            ClickPlan::Select {
                mods: ctrl,
                fresh: true
            }
        );
        assert_eq!(
            click_plan(TileEvent::Circle, plain, true),
            ClickPlan::Select {
                mods: ctrl,
                fresh: false
            }
        );
        // While picking, a plain click toggles instead of opening.
        assert_eq!(
            click_plan(TileEvent::Click, plain, true),
            ClickPlan::Select {
                mods: ctrl,
                fresh: false
            }
        );
        assert_eq!(
            click_plan(TileEvent::Click, ctrl, false),
            ClickPlan::Select {
                mods: ctrl,
                fresh: true
            }
        );
        assert_eq!(
            click_plan(TileEvent::Click, shift, false),
            ClickPlan::Select {
                mods: shift,
                fresh: false
            }
        );
    }

    #[test]
    fn thumbnails_are_cropped_to_the_centre_square() {
        let wide = square_uv(Vec2::new(400.0, 200.0));
        assert!((wide.min.x - 0.25).abs() < 1e-6 && (wide.max.x - 0.75).abs() < 1e-6);
        assert_eq!((wide.min.y, wide.max.y), (0.0, 1.0));
        let tall = square_uv(Vec2::new(200.0, 400.0));
        assert!((tall.min.y - 0.25).abs() < 1e-6 && (tall.max.y - 0.75).abs() < 1e-6);
        assert_eq!((tall.min.x, tall.max.x), (0.0, 1.0));
        let square = square_uv(Vec2::new(100.0, 100.0));
        assert_eq!((square.min.x, square.max.x), (0.0, 1.0));
    }
}
