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
/// Radius of the selection circle and of the dark disc behind the star.
const CIRCLE_RADIUS: f32 = 11.0;
const STAR_DISC_RADIUS: f32 = 12.0;
/// Size of the check mark and of the star.
const CHECK_SIZE: f32 = 16.0;
const STAR_SIZE: f32 = 18.0;
/// Opacity (0-255) of the dark discs behind the circle and the star.
const CIRCLE_DISC_ALPHA: u8 = 70;
const STAR_DISC_ALPHA: u8 = 90;
/// The shades at the top and bottom edge while hovered: height and opacity at the edge.
const SHADE_HEIGHT: f32 = 64.0;
const SHADE_ALPHA: u8 = 105;

/// How a tile takes part in the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum TileMark {
    #[default]
    None,
    /// The tile the keyboard is on.
    Focused,
    /// Part of the selection the user made on purpose (circle, Ctrl or Shift).
    Checked,
}

/// What a tile shows.
pub(super) struct Tile<'a> {
    pub key: &'a ThumbKey,
    /// Prefix of the accessible name of the tile ("Item", "Result").
    role: &'a str,
    /// File name (for assistive technology and the hover text); built only when needed.
    name: &'a dyn Fn() -> String,
    /// The hover text if it is not just the name; built only when the tile is hovered.
    tip: Option<&'a dyn Fn() -> String>,
    /// Text of the pill on videos (a time).
    badge: Option<String>,
    favorite: bool,
    mark: TileMark,
    /// Selection mode: circles stay visible on every tile.
    picking: bool,
    /// Whether the tile has a selection circle (a tile that only opens has none).
    selectable: bool,
}

impl<'a> Tile<'a> {
    /// A tile that only opens (no selection circle).
    pub(super) fn new(key: &'a ThumbKey, role: &'a str, name: &'a dyn Fn() -> String) -> Self {
        Self {
            key,
            role,
            name,
            tip: None,
            badge: None,
            favorite: false,
            mark: TileMark::None,
            picking: false,
            selectable: false,
        }
    }

    pub(super) fn tip(mut self, tip: &'a dyn Fn() -> String) -> Self {
        self.tip = Some(tip);
        self
    }

    pub(super) fn badge(mut self, badge: Option<String>) -> Self {
        self.badge = badge;
        self
    }

    pub(super) fn favorite(mut self, favorite: bool) -> Self {
        self.favorite = favorite;
        self
    }

    /// Gives the tile a selection circle, in the given selection state.
    pub(super) fn selectable(mut self, mark: TileMark, picking: bool) -> Self {
        self.selectable = true;
        self.mark = mark;
        self.picking = picking;
        self
    }

    fn checked(&self) -> bool {
        self.mark == TileMark::Checked
    }
}

impl TileMark {
    /// The mark of an item that is `in_sel` (selected) and/or the keyboard's `primary`, given
    /// whether the user is deliberately picking items.
    pub(super) fn of(in_sel: bool, primary: bool, picking_or_multi: bool) -> Self {
        if in_sel && picking_or_multi {
            Self::Checked
        } else if in_sel || primary {
            Self::Focused
        } else {
            Self::None
        }
    }
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
    let (circle, star) = interact_zones(ui, id, rect, tile, small);
    describe_zones(tile, small, &resp, &circle, &star);

    if ui.is_rect_visible(rect) {
        paint(ui, thumbs, rect, tile, hovered, small, &pal);
    }

    let event = click_event(ui, &resp, &circle, &star);
    // The zones of small tiles are not drawn and take no clicks, so they are no controls.
    let over_control = !small && ((tile.selectable && circle.hovered()) || star.hovered());
    if hovered && (over_control || resp.hovered()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = if over_control {
        resp
    } else {
        resp.on_hover_ui(|ui| {
            ui.label(tile.tip.map_or_else(|| (tile.name)(), |tip| tip()));
        })
    };
    TileOut { resp, event }
}

/// The areas of the selection circle and of the star.
fn interact_zones(
    ui: &egui::Ui,
    id: egui::Id,
    rect: Rect,
    tile: &Tile<'_>,
    small: bool,
) -> (egui::Response, egui::Response) {
    let circle_zone = Rect::from_min_size(rect.min, Vec2::splat(ZONE));
    let star_zone =
        Rect::from_min_size(Pos2::new(rect.min.x, rect.max.y - ZONE), Vec2::splat(ZONE));
    // The circle and the star are not drawn on small tiles, so their zones must not take
    // clicks there either: the click then goes to the tile itself.
    let circle_sense = if tile.selectable && !small {
        Sense::click()
    } else {
        Sense::hover()
    };
    let circle = ui.interact(circle_zone, id.with("select"), circle_sense);
    let star_sense = if small {
        Sense::hover()
    } else {
        Sense::click()
    };
    let star = ui.interact(star_zone, id.with("favorite"), star_sense);
    (circle, star)
}

/// What assistive technology reads for the circle, the star and the tile itself. Small tiles
/// draw neither the circle nor the star, so only the tile is described there.
fn describe_zones(
    tile: &Tile<'_>,
    small: bool,
    resp: &egui::Response,
    circle: &egui::Response,
    star: &egui::Response,
) {
    if tile.selectable && !small {
        circle.widget_info(|| {
            let verb = if tile.checked() { "Deselect" } else { "Select" };
            let label = format!("{verb} {}", (tile.name)());
            egui::WidgetInfo::selected(egui::Role::CheckBox, true, tile.checked(), &label)
        });
    }
    if !small {
        star.widget_info(|| {
            let name = (tile.name)();
            let label = if tile.favorite {
                format!("Remove {name} from favorites")
            } else {
                format!("Add {name} to favorites")
            };
            egui::WidgetInfo::selected(egui::Role::Button, true, tile.favorite, &label)
        });
    }
    resp.widget_info(|| {
        let label = format!("{} {}", tile.role, (tile.name)());
        egui::WidgetInfo::selected(
            egui::Role::Button,
            true,
            tile.mark != TileMark::None,
            &label,
        )
    });
}

/// The click on the circle, the star or the picture, if there was one this frame.
fn click_event(
    ui: &egui::Ui,
    resp: &egui::Response,
    circle: &egui::Response,
    star: &egui::Response,
) -> Option<(TileEvent, ClickMods)> {
    if circle.clicked() {
        Some((TileEvent::Circle, ClickMods::of_click(ui.ctx())))
    } else if star.clicked() {
        Some((TileEvent::Star, ClickMods::default()))
    } else if resp.clicked() {
        Some((TileEvent::Click, ClickMods::of_click(ui.ctx())))
    } else {
        None
    }
}

/// What shows where the picture is not (yet) available.
fn paint_placeholder(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    failed: bool,
    pal: &Palette,
) {
    let radius = CornerRadius::same(style::TILE_RADIUS);
    painter.rect_filled(rect, radius, pal.surface);
    if failed {
        // A picture that is not there: the icon alone on small tiles, with a line of
        // text on larger ones.
        let wide = rect.width() > 110.0;
        let size = (rect.width() * 0.3).clamp(14.0, 32.0);
        let centre = if wide {
            rect.center() - Vec2::new(0.0, 10.0)
        } else {
            rect.center()
        };
        paint_icon(
            painter,
            centre,
            icons::ICON_BROKEN_IMAGE,
            size,
            pal.text_secondary,
        );
        if wide {
            painter.text(
                rect.center() + Vec2::new(0.0, size / 2.0 + 2.0),
                egui::Align2::CENTER_CENTER,
                "Preview unavailable",
                egui::TextStyle::Small.resolve(ui.style()),
                pal.text_secondary,
            );
        }
    }
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
    paint_picture(ui, &painter, thumbs, rect, tile, pal);
    if hovered {
        paint_shades(&painter, rect);
    }
    if tile.selectable && (hovered || tile.picking || tile.checked()) && !small {
        paint_select_circle(&painter, rect, tile.checked(), pal);
    }
    if !small && (tile.favorite || hovered) {
        paint_star(&painter, rect, tile.favorite, pal);
    }
    if let Some(text) = &tile.badge {
        paint_pill(&painter, rect, text);
    }
    paint_mark_ring(&painter, rect, tile.mark, pal);
}

/// The thumbnail, or the placeholder while there is none.
fn paint_picture(
    ui: &egui::Ui,
    painter: &egui::Painter,
    thumbs: &mut ThumbCache,
    rect: Rect,
    tile: &Tile<'_>,
    pal: &Palette,
) {
    let failed = thumbs.failed(tile.key);
    if let Some(tex) = thumbs.get(tile.key) {
        let radius = CornerRadius::same(style::TILE_RADIUS);
        let shape = egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(tex.id(), square_uv(tex.size_vec2()));
        painter.add(shape);
    } else {
        paint_placeholder(ui, painter, rect, failed, pal);
    }
}

/// Soft shades at the top and bottom keep the circle, the star and the time readable on
/// bright photos, without a visible edge.
fn paint_shades(painter: &egui::Painter, rect: Rect) {
    for top in [true, false] {
        painter.add(fade(
            rect,
            top,
            SHADE_HEIGHT,
            SHADE_ALPHA,
            f32::from(style::TILE_RADIUS),
        ));
    }
}

/// The selection circle at the top left: empty, or filled with a check mark.
fn paint_select_circle(painter: &egui::Painter, rect: Rect, checked: bool, pal: &Palette) {
    let c = circle_center(rect);
    if checked {
        painter.circle_filled(c, CIRCLE_RADIUS, pal.accent);
        paint_icon(painter, c, icons::ICON_CHECK, CHECK_SIZE, pal.on_accent);
    } else {
        painter.circle_filled(
            c,
            CIRCLE_RADIUS,
            Color32::from_black_alpha(CIRCLE_DISC_ALPHA),
        );
        painter.circle_stroke(c, CIRCLE_RADIUS - 1.0, Stroke::new(2.0, Color32::WHITE));
    }
}

/// The star at the bottom left: filled for a favorite, an outline while hovered otherwise.
fn paint_star(painter: &egui::Painter, rect: Rect, favorite: bool, pal: &Palette) {
    let c = star_center(rect);
    painter.circle_filled(
        c,
        STAR_DISC_RADIUS,
        Color32::from_black_alpha(STAR_DISC_ALPHA),
    );
    if favorite {
        paint_icon(painter, c, icons::ICON_STAR, STAR_SIZE, pal.favorite);
    } else {
        paint_icon(
            painter,
            c,
            icons::ICON_STAR.outlined(),
            STAR_SIZE,
            Color32::WHITE,
        );
    }
}

/// The ring around a checked tile (thick) or the tile the keyboard is on (thin).
fn paint_mark_ring(painter: &egui::Painter, rect: Rect, mark: TileMark, pal: &Palette) {
    let radius = CornerRadius::same(style::TILE_RADIUS);
    if mark == TileMark::Checked {
        painter.rect_stroke(
            rect.shrink(1.5),
            radius,
            Stroke::new(3.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    } else if mark == TileMark::Focused {
        painter.rect_stroke(
            rect.shrink(1.0),
            radius,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
}

/// A black fade over the top or bottom edge of `rect`: `alpha` at the edge, falling smoothly to
/// nothing `height` points inside. The outer corners follow the tile's rounded corners.
fn fade(rect: Rect, top: bool, height: f32, alpha: u8, radius: f32) -> egui::Shape {
    let height = height.min(rect.height());
    let mut mesh = egui::Mesh::default();
    // Rows from the edge inwards: a few close together where the corner curves, then the rest.
    let steps = [0.0, 0.1, 0.25, 0.45, 0.7, 1.0];
    let mut rows: Vec<f32> = (0..=4).map(|i| radius * i as f32 / 4.0).collect();
    rows.extend(steps.iter().map(|t| t * height).filter(|&y| y > radius));
    rows.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    for (i, &d) in rows.iter().enumerate() {
        let inset = if d < radius {
            radius - (radius * radius - (radius - d) * (radius - d)).sqrt()
        } else {
            0.0
        };
        let t = (d / height).clamp(0.0, 1.0);
        let a = (f32::from(alpha) * (1.0 - t) * (1.0 - t)).round() as u8;
        let color = Color32::from_black_alpha(a);
        let y = if top { rect.min.y + d } else { rect.max.y - d };
        mesh.colored_vertex(Pos2::new(rect.min.x + inset, y), color);
        mesh.colored_vertex(Pos2::new(rect.max.x - inset, y), color);
        if i > 0 {
            let k = u32::try_from(i * 2).unwrap_or(u32::MAX);
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    egui::Shape::mesh(mesh)
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

    /// Clicks the bottom-left corner of a tile of `side` points and returns what it reported.
    fn corner_click(side: f32) -> Option<TileEvent> {
        use egui_kittest::Harness;
        use std::cell::RefCell;
        use std::rc::Rc;
        let dir = tempfile::tempdir().unwrap();
        let mut thumbs = ThumbCache::new(&egui::Context::default(), dir.path(), 1);
        let key = ThumbKey {
            path: "/nowhere/a.jpg".into(),
            mtime: 1,
            t_ms: None,
        };
        let seen = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(side));
        let mut h = Harness::builder()
            .with_size(Vec2::new(300.0, 300.0))
            .build_ui(move |ui| {
                let name = || "a.jpg".to_string();
                let tile = Tile::new(&key, "Item", &name).selectable(TileMark::None, false);
                let out = show(ui, &mut thumbs, ui.scope_id().with("tile"), rect, &tile);
                if let Some((event, _)) = out.event {
                    *sink.borrow_mut() = Some(event);
                }
            });
        style::apply(
            &h.ctx,
            crate::config::Theme::default(),
            crate::config::TextSize::default(),
        );
        h.run();
        let pos = Pos2::new(rect.min.x + 6.0, rect.max.y - 6.0);
        h.hover_at(pos);
        h.run_steps(2);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(2);
        }
        *seen.borrow()
    }

    #[test]
    fn the_corner_of_a_small_tile_clicks_the_tile_not_the_hidden_star() {
        assert_eq!(corner_click(60.0), Some(TileEvent::Click));
    }

    #[test]
    fn the_corner_of_a_large_tile_still_clicks_the_star() {
        assert_eq!(corner_click(120.0), Some(TileEvent::Star));
    }
}
