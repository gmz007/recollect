//! The picture itself: choosing the sharpest image that is ready, zoom and pan, and the
//! previous / next buttons.

use super::Viewer;
use crate::media::MediaKind;
use crate::ui::preview::{self, Preview, Previews};
use crate::ui::workers::ThumbCache;
use egui::{Color32, Pos2, Rect, Sense, Vec2};

/// What the canvas draws for the current item.
pub(super) struct Shown {
    pub(super) tex: Option<egui::TextureHandle>,
    /// Only the thumbnail is available so far, and the preview is on its way.
    pub(super) loading: bool,
    pub(super) failed: bool,
}

impl Viewer {
    /// Asks for what the canvas needs (and forgets what it no longer wants) and picks the
    /// texture to draw.
    pub(super) fn pick_texture(
        &self,
        previews: &mut Previews,
        thumbs: &mut ThumbCache,
        fit: u32,
    ) -> Shown {
        let e = &self.current;
        // The neighbours are prefetched; the item on screen is asked for urgently every frame,
        // so it is decoded before them even if they were queued first (the fitted preview
        // last of all, hence first in line). Whatever is no longer wanted is forgotten.
        let mut wanted = Vec::new();
        for i in [self.index + 1, self.index.wrapping_sub(1)] {
            if i < self.len() {
                let k = self.entry_at(i).preview_key(fit, false);
                previews.get(&k);
                wanted.push(k);
            }
        }
        let fit_key = e.preview_key(fit, false);
        let zoom_key = (self.zoom && e.kind == MediaKind::Image)
            .then(|| {
                let longest = e.dims().map_or(preview::MAX_ZOOM_SIDE, |(w, h)| w.max(h));
                longest.min(preview::MAX_ZOOM_SIDE)
            })
            .filter(|&s| s > fit)
            .map(|s| e.preview_key(s, true));
        let mut tex = None;
        let mut failed = false;
        if let Some(k) = &zoom_key {
            wanted.push(k.clone());
        }
        wanted.push(fit_key.clone());
        if let Some(k) = &zoom_key {
            previews.get_urgent(k);
        }
        match previews.get_urgent(&fit_key) {
            Preview::Ready(t) => tex = Some(t.clone()),
            Preview::Failed => failed = true,
            Preview::Loading => {}
        }
        if let Some(k) = &zoom_key
            && let Preview::Ready(t) = previews.get(k)
        {
            tex = Some(t.clone());
        }
        previews.retain_queued(&wanted);
        let mut loading = false;
        if tex.is_none() {
            // Until the preview arrives, the (already cached) thumbnail stands in for it.
            tex = thumbs.get(&e.thumb_key()).cloned();
            loading = !failed;
        }
        Shown {
            tex,
            loading,
            failed,
        }
    }

    /// The media area: preview (or the thumbnail until it is ready), zoom and previous / next.
    pub(super) fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        previews: &mut Previews,
        thumbs: &mut ThumbCache,
    ) {
        let ctx = ui.ctx().clone();
        let area = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(area, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Viewer canvas"));
        if resp.double_clicked() {
            self.toggle_zoom();
        }
        let ppp = ctx.pixels_per_point();
        let fit = preview::fit_side(ctx.content_rect().size(), ppp);
        let Shown {
            tex,
            loading,
            failed,
        } = self.pick_texture(previews, thumbs, fit);
        if let Some(tex) = tex {
            self.draw_picture(ui, area, &resp, &tex, (loading, ppp));
        }
        paint_status(ui, area, loading, failed);
        self.nav_buttons(ui, area);
    }

    /// Draws `tex` fitted to `area`, or at full size and draggable when zoomed in. `state` is
    /// whether `tex` is only the stand-in thumbnail, and the screen's pixels per point.
    fn draw_picture(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        resp: &egui::Response,
        tex: &egui::TextureHandle,
        (loading, ppp): (bool, f32),
    ) {
        let native = tex.size_vec2();
        let full = (self.zoom && !loading).then(|| {
            self.current
                .dims()
                .map_or(native, |(w, h)| Vec2::new(w as f32, h as f32))
                / ppp
        });
        if self.zoom && resp.dragged() {
            self.pan += resp.drag_delta();
        }
        let (rect, pan) = picture_rect(area, native, full, self.pan, self.zoom);
        self.pan = pan;
        ui.painter_at(area).image(
            tex.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if self.zoom && !loading {
            ui.output_mut(|o| {
                o.cursor_icon = if resp.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                }
            });
        }
    }

    /// Previous / next, drawn over the media.
    fn nav_buttons(&mut self, ui: &mut egui::Ui, area: Rect) {
        let mid = area.center().y;
        let prev = Rect::from_center_size(Pos2::new(area.left() + 34.0, mid), Vec2::splat(44.0));
        let next = Rect::from_center_size(Pos2::new(area.right() - 34.0, mid), Vec2::splat(44.0));
        if self.index > 0 && nav_button(ui, prev, "Previous", -1.0) {
            self.go(self.index - 1);
        }
        if self.index + 1 < self.len() && nav_button(ui, next, "Next", 1.0) {
            self.go(self.index + 1);
        }
    }
}

/// Where a picture of `native` size (in points) is drawn inside `area`, and the pan to keep.
///
/// With `full` (its size at one image pixel per screen pixel) the picture is drawn that large,
/// moved by `pan`, which is clamped so no more than the overflow can be dragged past the
/// centre. Without it the picture is fitted to the area (never larger than the area) and
/// `pan` is clamped too, so only `zoomed` decides whether it moves the picture.
pub(super) fn picture_rect(
    area: Rect,
    native: Vec2,
    full: Option<Vec2>,
    pan: Vec2,
    zoomed: bool,
) -> (Rect, Vec2) {
    if native.x <= 0.0 || native.y <= 0.0 {
        return (
            Rect::from_center_size(area.center(), Vec2::ZERO),
            Vec2::ZERO,
        );
    }
    let size =
        full.unwrap_or_else(|| native * (area.width() / native.x).min(area.height() / native.y));
    let slack = ((size - area.size()) / 2.0).max(Vec2::ZERO);
    let pan = pan.clamp(-slack, slack);
    let centre = if zoomed {
        area.center() + pan
    } else {
        area.center()
    };
    (Rect::from_center_size(centre, size), pan)
}

/// The spinner while a preview is on its way, or a note when it cannot be made.
fn paint_status(ui: &mut egui::Ui, area: Rect, loading: bool, failed: bool) {
    if loading {
        ui.put(
            Rect::from_center_size(area.center(), Vec2::splat(28.0)),
            egui::Spinner::new(),
        );
    } else if failed {
        ui.painter_at(area).text(
            area.center() + Vec2::new(0.0, 40.0),
            egui::Align2::CENTER_CENTER,
            "Preview unavailable",
            egui::FontId::proportional(15.0),
            Color32::from_gray(180),
        );
    }
}

/// A round button with a chevron; `dir` is -1 for "previous" and 1 for "next".
pub(super) fn nav_button(ui: &mut egui::Ui, rect: Rect, label: &str, dir: f32) -> bool {
    let resp = ui.interact(
        rect,
        ui.make_persistent_id(("viewer-nav", label)),
        Sense::click(),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let alpha = if resp.hovered() { 200 } else { 110 };
    let p = ui.painter();
    p.circle_filled(
        rect.center(),
        rect.width() / 2.0,
        Color32::from_black_alpha(alpha),
    );
    let c = rect.center();
    let stroke = egui::Stroke::new(3.0, Color32::WHITE);
    let (tip, tail) = (
        c + Vec2::new(5.0 * dir, 0.0),
        c + Vec2::new(-4.0 * dir, 0.0),
    );
    p.line_segment([tip, tail + Vec2::new(0.0, -11.0)], stroke);
    p.line_segment([tip, tail + Vec2::new(0.0, 11.0)], stroke);
    resp.on_hover_text(label).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 200.0));

    #[test]
    fn a_picture_is_fitted_inside_the_area_and_centred() {
        let (rect, pan) = picture_rect(AREA, Vec2::new(800.0, 800.0), None, Vec2::ZERO, false);
        assert_eq!(rect.size(), Vec2::new(200.0, 200.0));
        assert_eq!(rect.center(), AREA.center());
        assert_eq!(pan, Vec2::ZERO);
    }

    #[test]
    fn a_small_picture_is_scaled_up_to_fit() {
        let (rect, _) = picture_rect(AREA, Vec2::new(100.0, 50.0), None, Vec2::ZERO, false);
        assert_eq!(rect.size(), Vec2::new(400.0, 200.0));
    }

    #[test]
    fn zoomed_in_the_picture_keeps_its_size_and_follows_the_pan() {
        let full = Some(Vec2::new(1000.0, 600.0));
        let (rect, pan) = picture_rect(
            AREA,
            Vec2::new(10.0, 10.0),
            full,
            Vec2::new(30.0, -20.0),
            true,
        );
        assert_eq!(rect.size(), Vec2::new(1000.0, 600.0));
        assert_eq!(pan, Vec2::new(30.0, -20.0));
        assert_eq!(rect.center(), AREA.center() + pan);
    }

    #[test]
    fn the_pan_stops_at_the_edge_of_the_picture() {
        let full = Some(Vec2::new(1000.0, 600.0));
        let (rect, pan) = picture_rect(
            AREA,
            Vec2::new(10.0, 10.0),
            full,
            Vec2::new(9999.0, -9999.0),
            true,
        );
        assert_eq!(pan, Vec2::new(300.0, -200.0));
        assert_eq!(rect.left(), AREA.left());
        assert_eq!(rect.bottom(), AREA.bottom());
    }

    #[test]
    fn a_zoomed_picture_smaller_than_the_area_cannot_be_dragged_off_centre() {
        let full = Some(Vec2::new(100.0, 50.0));
        let (rect, pan) = picture_rect(
            AREA,
            Vec2::new(10.0, 10.0),
            full,
            Vec2::new(40.0, 40.0),
            true,
        );
        assert_eq!(pan, Vec2::ZERO);
        assert_eq!(rect.center(), AREA.center());
        assert_eq!(rect.size(), Vec2::new(100.0, 50.0));
    }

    #[test]
    fn nothing_is_drawn_for_an_empty_picture_and_the_pan_resets() {
        let (rect, pan) = picture_rect(AREA, Vec2::ZERO, None, Vec2::new(5.0, 5.0), true);
        assert_eq!(rect.size(), Vec2::ZERO);
        assert_eq!(pan, Vec2::ZERO);
    }

    #[test]
    fn an_empty_area_gives_an_empty_fit() {
        let area = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::ZERO);
        let (rect, pan) = picture_rect(area, Vec2::new(40.0, 30.0), None, Vec2::ZERO, false);
        assert_eq!(rect.size(), Vec2::ZERO);
        assert_eq!(pan, Vec2::ZERO);
    }
}
