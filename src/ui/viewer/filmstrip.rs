//! The strip that floats at the bottom of the picture: small thumbnails of the neighbouring
//! items (or, for a video, its matching moments and a Play button). It fades out when the
//! pointer rests and comes back when it moves.

use super::{Action, Viewer};
use crate::media::MediaKind;
use crate::ui::fmt_time;
use crate::ui::style::{self, ViewerColors, fade_opacity, icons};
use crate::ui::tiles::square_uv;
use crate::ui::workers::ThumbCache;
use egui::{Align2, Color32, CornerRadius, Margin, Rect, Response, Sense, Stroke, Vec2};
use std::time::Duration;

/// Side of a thumbnail.
const THUMB: f32 = 52.0;
/// Corner radius of a thumbnail.
const THUMB_RADIUS: u8 = 6;
/// Thumbnails shown either side of the current one.
const REACH: usize = 4;
/// Distance of the strip from the bottom of the picture area.
const BOTTOM_GAP: f32 = 12.0;
/// How long the strip stays after the pointer last moved, and how long it takes to fade.
const STAY: Duration = Duration::from_millis(2000);
const FADE: Duration = Duration::from_millis(400);
/// Most moments shown as chips in the strip.
const MAX_MOMENT_CHIPS: usize = 4;
/// Height of a time chip and of the Play button.
const CHIP_H: f32 = 28.0;

impl Viewer {
    /// Draws the strip over the bottom of `area` (unless it has faded) and applies a click.
    pub(super) fn filmstrip(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        thumbs: &mut ThumbCache,
        actions: &mut Vec<Action>,
    ) {
        let ctx = ui.ctx().clone();
        let strip = ui.make_persistent_id("viewer-strip");
        let opacity = self.controls_opacity(&ctx, strip);
        if opacity <= 0.0 {
            return;
        }
        let colors = style::Palette::of(&ctx).viewer();
        let pos = egui::pos2(area.center().x, area.bottom() - BOTTOM_GAP);
        let mut goto = None;
        let mut moment = None;
        let mut play = None;
        let video = self.current.kind == MediaKind::Video;
        egui::Area::new(strip)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .pivot(Align2::CENTER_BOTTOM)
            .show(&ctx, |ui| {
                strip_frame(colors, opacity).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                    ui.horizontal(|ui| {
                        if video {
                            self.moment_strip(ui, colors, opacity, &mut moment, &mut play);
                        } else {
                            goto = self.thumb_strip(ui, thumbs, colors, opacity);
                        }
                    });
                });
            });
        if let Some(i) = goto {
            self.go(i);
        }
        if let Some(t) = moment {
            self.pick_moment(t);
        }
        if let Some(t) = play {
            actions.push(Action::PlayAt(self.current.path.clone(), t));
        }
    }

    /// How visible the floating controls are: fully while the pointer moves or rests on them,
    /// then fading away. Moving the pointer or stepping to another item brings them back.
    fn controls_opacity(&mut self, ctx: &egui::Context, strip: egui::Id) -> f32 {
        let now = ctx.input(|i| i.time);
        // Where the strip was drawn last (nothing before it first appears).
        let strip = ctx.memory(|m| m.area_rect(strip));
        let (moving, over) = ctx.input(|i| {
            let over = i
                .pointer
                .hover_pos()
                .is_some_and(|p| strip.is_some_and(|r| r.contains(p)));
            (i.pointer.is_moving() || i.pointer.any_pressed(), over)
        });
        let last = match self.last_motion {
            Some(t) if !(moving || over) => t,
            _ => now,
        };
        self.last_motion = Some(last);
        let age = Duration::from_secs_f64((now - last).max(0.0));
        fade_opacity(ctx, age, STAY, FADE)
    }

    /// The thumbnails around the current item; returns the one that was clicked.
    fn thumb_strip(
        &self,
        ui: &mut egui::Ui,
        thumbs: &mut ThumbCache,
        colors: ViewerColors,
        opacity: f32,
    ) -> Option<usize> {
        let first = self.index.saturating_sub(REACH);
        let last = (self.index + REACH).min(self.len() - 1);
        let mut clicked = None;
        for i in first..=last {
            let (key, name) = self.source.thumb_at(i, self.moments.get(&i).copied());
            let (rect, r) = ui.allocate_exact_size(Vec2::splat(THUMB), Sense::click());
            let radius = CornerRadius::same(THUMB_RADIUS);
            if let Some(tex) = thumbs.get(&key) {
                let tint = Color32::WHITE.gamma_multiply(opacity);
                let shape = egui::epaint::RectShape::filled(rect, radius, tint)
                    .with_texture(tex.id(), square_uv(tex.size_vec2()));
                ui.painter().add(shape);
            } else {
                let skel = style::Palette::of(ui.ctx()).skel;
                ui.painter()
                    .rect_filled(rect, radius, skel.gamma_multiply(opacity));
            }
            if i == self.index {
                let stroke = Stroke::new(2.0, colors.controls_text.gamma_multiply(opacity));
                ui.painter()
                    .rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
            }
            let label = format!("Go to {name}");
            r.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, i == self.index, &label)
            });
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(i);
            }
        }
        clicked
    }

    /// The moments of a video as time chips, then Play.
    fn moment_strip(
        &self,
        ui: &mut egui::Ui,
        colors: ViewerColors,
        opacity: f32,
        moment: &mut Option<f64>,
        play: &mut Option<f64>,
    ) {
        let e = &self.current;
        let shown = e.moments.len() > 1 || (e.moments.len() == 1 && e.t.is_some());
        if shown {
            // The best few (the rest are in the details panel), and always the chosen one.
            let shown_moments = e
                .moments
                .iter()
                .enumerate()
                .filter(|(i, m)| *i < MAX_MOMENT_CHIPS || e.t == Some(m.t_start))
                .map(|(_, m)| m);
            for m in shown_moments {
                let text = m.label();
                let selected = e.t == Some(m.t_start);
                if chip(ui, &text, &text, selected, colors, opacity).clicked() {
                    *moment = Some(m.t_start);
                }
            }
        }
        let t = e.t.unwrap_or(0.0);
        if play_button(ui, t, opacity).clicked() {
            *play = Some(t);
        }
    }
}

/// The rounded container of the strip.
fn strip_frame(colors: ViewerColors, opacity: f32) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.controls.gamma_multiply(opacity))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(6))
}

/// A fully round time chip: inverted while `selected`. Named `accessible`.
fn chip(
    ui: &mut egui::Ui,
    text: &str,
    accessible: &str,
    selected: bool,
    colors: ViewerColors,
    opacity: f32,
) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let (fill, fg) = if selected {
        (colors.controls_text, colors.controls)
    } else {
        (
            colors.controls_text.gamma_multiply(0.1),
            colors.controls_text,
        )
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, fg.gamma_multiply(opacity));
    let (rect, r) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, CHIP_H), Sense::click());
    let fill = if r.hovered() && !selected {
        colors.controls_text.gamma_multiply(0.2)
    } else {
        fill
    };
    let round = CornerRadius::same((CHIP_H / 2.0) as u8);
    ui.painter()
        .rect_filled(rect, round, fill.gamma_multiply(opacity));
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, accessible));
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The accent button that opens the video at `t` seconds in the system player.
fn play_button(ui: &mut egui::Ui, t: f64, opacity: f32) -> Response {
    let pal = style::Palette::of(ui.ctx());
    let label = format!("Play at {}", fmt_time(t));
    let fg = pal.onaccent.gamma_multiply(opacity);
    let job = style::icon_label_job(
        ui,
        icons::ICON_PLAY_ARROW,
        18.0,
        fg,
        Some((&label, fg)),
        4.0,
    );
    let galley = ui.painter().layout_job(job);
    let (rect, r) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, CHIP_H), Sense::click());
    let fill = if r.hovered() {
        style::darken(pal.accent, 0.12)
    } else {
        pal.accent
    };
    ui.painter().rect_filled(
        rect,
        CornerRadius::same((CHIP_H / 2.0) as u8),
        fill.gamma_multiply(opacity),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    r.on_hover_text("Open this video in your video player")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
