//! The "On this day" banner at the top of Photos: a photo taken on today's date in an earlier
//! year, what day it is, and a button that opens all the photos of that day.

use super::memories::years_ago_text;
use super::thumb_key;
use crate::store::db::TimelineItem;
use crate::ui::style::{self, Palette, icons};
use crate::ui::workers::ThumbCache;
use chrono::NaiveDate;
use egui::{Color32, CornerRadius, Rect, RichText, Sense, Vec2};

/// Height of the banner.
pub(super) const BANNER_HEIGHT: f32 = 220.0;
/// Space under the banner.
pub(super) const BANNER_GAP: f32 = 14.0;
/// Corner radius of the banner.
const RADIUS: u8 = 12;
/// Height of the text area at the bottom.
const TEXT_HEIGHT: f32 = 90.0;
/// Space between the text and the edges of the banner.
const TEXT_MARGIN: f32 = 20.0;
/// Size of the hide button and of the "See" button.
const HIDE_SIZE: f32 = 28.0;
const SEE_HEIGHT: f32 = 32.0;

/// What the banner shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Banner {
    /// Indexes of the day's photos in the timeline, newest first.
    pub entries: Vec<usize>,
    pub years_ago: i32,
    pub date: NaiveDate,
}

impl Banner {
    /// "17 November, 3 years ago".
    fn title(&self) -> String {
        format!(
            "{}, {}",
            self.date.format("%-d %B"),
            years_ago_text(self.years_ago)
        )
    }

    /// "See 12 photos".
    fn see_text(&self) -> String {
        let n = self.entries.len();
        format!("See {n} {}", if n == 1 { "photo" } else { "photos" })
    }
}

/// What the user did to the banner this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BannerEvent {
    See,
    Hide,
}

/// Draws the banner across `width` and reports a click on one of its buttons.
pub(super) fn show(
    ui: &mut egui::Ui,
    thumbs: &mut ThumbCache,
    items: &[TimelineItem],
    banner: &Banner,
    width: f32,
) -> Option<BannerEvent> {
    let pal = Palette::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BANNER_HEIGHT), Sense::hover());
    let text_top = rect.max.y - TEXT_HEIGHT;
    let photo = Rect::from_min_max(rect.min, egui::pos2(rect.max.x, text_top));
    paint_photo(ui, thumbs, items.get(banner.entries[0]), photo, &pal);
    ui.painter().rect_filled(
        Rect::from_min_max(egui::pos2(rect.min.x, text_top), rect.max),
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: RADIUS,
            se: RADIUS,
        },
        pal.pill,
    );
    let text_area = Rect::from_min_max(egui::pos2(rect.min.x, text_top), rect.max)
        .shrink2(Vec2::new(TEXT_MARGIN, 0.0));
    let mut event = None;
    if paint_text(ui, text_area, banner, &pal) {
        event = Some(BannerEvent::See);
    }
    if hide_button(ui, rect) {
        event = Some(BannerEvent::Hide);
    }
    event
}

/// The photo, cropped to fill `area`, with rounded top corners.
fn paint_photo(
    ui: &egui::Ui,
    thumbs: &mut ThumbCache,
    item: Option<&TimelineItem>,
    area: Rect,
    pal: &Palette,
) {
    let radius = CornerRadius {
        nw: RADIUS,
        ne: RADIUS,
        sw: 0,
        se: 0,
    };
    let tex = item.and_then(|it| thumbs.get(&thumb_key(it)));
    match tex {
        Some(tex) => {
            let uv = cover_uv(tex.size_vec2(), area.size());
            ui.painter().add(
                egui::epaint::RectShape::filled(area, radius, Color32::WHITE)
                    .with_texture(tex.id(), uv),
            );
        }
        None => {
            ui.painter().rect_filled(area, radius, pal.side);
        }
    }
}

/// UV rectangle that crops a texture of `tex` size so it covers `target` without stretching,
/// keeping the centre.
fn cover_uv(tex: Vec2, target: Vec2) -> Rect {
    let tex_aspect = tex.x.max(1.0) / tex.y.max(1.0);
    let target_aspect = target.x.max(1.0) / target.y.max(1.0);
    let (u, v) = if tex_aspect > target_aspect {
        ((1.0 - target_aspect / tex_aspect) / 2.0, 0.0)
    } else {
        (0.0, (1.0 - tex_aspect / target_aspect) / 2.0)
    };
    Rect::from_min_max(egui::pos2(u, v), egui::pos2(1.0 - u, 1.0 - v))
}

/// The two lines of text and the "See" button; true when the button was clicked.
fn paint_text(ui: &mut egui::Ui, area: Rect, banner: &Banner, pal: &Palette) -> bool {
    let left = Rect::from_min_max(
        area.min + Vec2::new(0.0, 14.0),
        egui::pos2(area.max.x - 140.0, area.max.y - 10.0),
    );
    // Child uis do not move the parent's cursor, which must stay at the banner's bottom edge.
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left)
            .layout(egui::Layout::bottom_up(egui::Align::Min)),
    );
    text.spacing_mut().item_spacing.y = 2.0;
    text.label(
        style::semibold(banner.title())
            .size(20.0)
            .color(pal.pilltext),
    );
    text.label(RichText::new("On this day").size(12.0).color(pal.pilltext));
    let right = Rect::from_min_max(
        egui::pos2(area.max.x - 240.0, area.max.y - 18.0 - SEE_HEIGHT),
        egui::pos2(area.max.x, area.max.y - 18.0),
    );
    let mut action = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let button = egui::Button::new(RichText::new(banner.see_text()).color(pal.pill))
        .fill(pal.pilltext)
        .corner_radius(CornerRadius::same(16))
        .min_size(Vec2::new(0.0, SEE_HEIGHT));
    action.add(button).clicked()
}

/// The round button at the top right that hides the banner until tomorrow.
fn hide_button(ui: &mut egui::Ui, banner: Rect) -> bool {
    let rect = Rect::from_min_size(
        egui::pos2(banner.max.x - 10.0 - HIDE_SIZE, banner.min.y + 10.0),
        Vec2::splat(HIDE_SIZE),
    );
    let resp = ui.interact(rect, ui.scope_id().with("hide-on-this-day"), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Hide On this day"));
    let fill = Color32::from_black_alpha(if resp.hovered() { 120 } else { 77 });
    ui.painter()
        .circle_filled(rect.center(), HIDE_SIZE / 2.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::ICON_CLOSE.codepoint,
        style::icon_font_id(icons::ICON_CLOSE, 16.0),
        Color32::WHITE,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text("Hide until tomorrow").clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_are_cropped_to_cover_the_banner() {
        // A square photo in a wide banner loses the top and bottom.
        let wide = cover_uv(Vec2::new(100.0, 100.0), Vec2::new(400.0, 100.0));
        assert_eq!((wide.min.x, wide.max.x), (0.0, 1.0));
        assert!((wide.min.y - 0.375).abs() < 1e-6 && (wide.max.y - 0.625).abs() < 1e-6);
        // A wider photo than the banner loses its sides.
        let tall = cover_uv(Vec2::new(400.0, 100.0), Vec2::new(100.0, 100.0));
        assert!((tall.min.x - 0.375).abs() < 1e-6 && (tall.max.x - 0.625).abs() < 1e-6);
        assert_eq!((tall.min.y, tall.max.y), (0.0, 1.0));
        // Same shape: nothing is lost.
        let same = cover_uv(Vec2::new(200.0, 100.0), Vec2::new(400.0, 200.0));
        assert_eq!(
            same,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
        );
    }

    #[test]
    fn the_banner_reads_naturally() {
        let b = Banner {
            entries: vec![3, 9],
            years_ago: 3,
            date: NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(),
        };
        assert_eq!(b.title(), "17 November, 3 years ago");
        assert_eq!(b.see_text(), "See 2 photos");
        let one = Banner {
            entries: vec![1],
            years_ago: 1,
            ..b
        };
        assert_eq!(one.title(), "17 November, 1 year ago");
        assert_eq!(one.see_text(), "See 1 photo");
    }
}
