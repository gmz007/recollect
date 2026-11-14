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
        let e = self.current.clone();
        let Shown {
            tex,
            loading,
            failed,
        } = self.pick_texture(previews, thumbs, fit);
        let painter = ui.painter_at(area);
        if let Some(tex) = tex {
            let native = tex.size_vec2();
            let size = if self.zoom && !loading {
                e.dims()
                    .map_or(native, |(w, h)| Vec2::new(w as f32, h as f32))
                    / ppp
            } else {
                native * (area.width() / native.x).min(area.height() / native.y)
            };
            if self.zoom && resp.dragged() {
                self.pan += resp.drag_delta();
            }
            let slack = ((size - area.size()) / 2.0).max(Vec2::ZERO);
            self.pan = self.pan.clamp(-slack, slack);
            let centre = if self.zoom {
                area.center() + self.pan
            } else {
                area.center()
            };
            painter.image(
                tex.id(),
                Rect::from_center_size(centre, size),
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
        if loading {
            ui.put(
                Rect::from_center_size(area.center(), Vec2::splat(28.0)),
                egui::Spinner::new(),
            );
        } else if failed {
            painter.text(
                area.center() + Vec2::new(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Preview unavailable",
                egui::FontId::proportional(15.0),
                Color32::from_gray(180),
            );
        }
        // Previous / next, drawn over the media.
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
