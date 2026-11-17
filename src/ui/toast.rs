//! The floating pill at the bottom of the window: the selection bar and the toasts (what just
//! happened, with "Undo" where there is a way back) share its look and position.

use super::notices::Undo;
use super::style::{self, Palette, icons};
use super::{App, NoticeAction, Notices};
use egui::{Align, Layout, RichText};

/// Distance between the pill and the bottom edge.
const PILL_MARGIN: f32 = 20.0;
/// Space between the selection bar and a toast above it.
const PILL_GAP: f32 = 8.0;
/// The widest a toast gets before its text wraps.
const TOAST_MAX_WIDTH: f32 = 560.0;

/// What was clicked on a toast.
#[derive(Default)]
struct ToastClicks {
    dismiss: bool,
    retry: Option<NoticeAction>,
    undo: bool,
}

/// Shows `add_contents` in a pill that floats at the bottom centre of `anchor` (the area of
/// the grid; the window's bottom centre when there is none).
///
/// The pill is centred using its size from the frame before, so `id` must change whenever the
/// contents change width (a new message): a new area is measured once, invisibly, before it
/// shows, instead of appearing off-centre for a frame.
pub(super) fn pill_area<R>(
    ctx: &egui::Context,
    id: egui::Id,
    anchor: Option<egui::Pos2>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let pos = anchor.unwrap_or_else(|| {
        let r = ctx.content_rect();
        egui::pos2(r.center().x, r.bottom() - PILL_MARGIN)
    });
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            style::pill_frame(ctx)
                .show(ui, |ui| {
                    ui.set_min_height(style::PILL_HEIGHT);
                    style::pill_buttons(ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), add_contents)
                        .inner
                })
                .inner
        })
        .inner
}

/// Where the pill floats: bottom centre of the grid area, [`PILL_MARGIN`] above its edge.
pub(super) fn pill_anchor_of(ui: &egui::Ui) -> egui::Pos2 {
    let r = ui.max_rect();
    egui::pos2(r.center().x, r.bottom() - PILL_MARGIN)
}

impl App {
    /// The current notice as a pill at the bottom of the window.
    pub(super) fn toasts(&mut self, ctx: &egui::Context) {
        // While the selection bar shows, a plain toast floats just above it.
        let anchor = self.pill_anchor.take().map(|a| {
            if self.confirmation_showing() {
                a
            } else {
                egui::pos2(a.x, a.y - style::PILL_HEIGHT - PILL_GAP)
            }
        });
        let Some(n) = self.notices.current_notice() else {
            return;
        };
        let id = egui::Id::unique(("toast", n.at));
        let (text, error, details, action, undo) = (
            n.text.clone(),
            n.error,
            n.details.clone(),
            n.action,
            n.undo.clone(),
        );
        let pal = Palette::of(ctx);
        let clicks = pill_area(ctx, id, anchor, |ui| {
            ui.set_max_width(TOAST_MAX_WIDTH);
            toast_contents(
                ui,
                &pal,
                &text,
                error,
                details.as_deref(),
                action,
                undo.as_ref(),
            )
        });
        if clicks.dismiss || clicks.retry.is_some() || clicks.undo {
            self.notices.dismiss_current();
        }
        if let Some(a) = clicks.retry {
            self.retry(a);
        }
        if clicks.undo
            && let Some(u) = undo
        {
            self.undo(u);
        }
    }
}

/// The message, "Undo" or "Try again" button, details and close button of a toast.
fn toast_contents(
    ui: &mut egui::Ui,
    pal: &Palette,
    text: &str,
    error: bool,
    details: Option<&str>,
    action: Option<NoticeAction>,
    undo: Option<&Undo>,
) -> ToastClicks {
    let mut clicks = ToastClicks::default();
    if error {
        ui.label(style::icon_text(icons::ICON_ERROR, 20.0).color(pal.danger));
    }
    let label = |ui: &mut egui::Ui| {
        let label = ui.label(text);
        if matches!(undo, Some(Undo::PutBackInAlbum { .. })) {
            label.on_hover_text("The photos stay in your library. Nothing is deleted.");
        }
    };
    if action.is_none() && details.is_none() {
        label(ui);
    } else {
        ui.vertical(|ui| {
            ui.add_space(8.0);
            label(ui);
            if let Some(a) = action
                && ui.button("Try again").clicked()
            {
                clicks.retry = Some(a);
            }
            if let Some(d) = details {
                egui::CollapsingHeader::new("Details")
                    .id_salt("toast-details")
                    .show(ui, |ui| {
                        ui.label(RichText::new(d).small());
                    });
            }
            ui.add_space(8.0);
        });
    }
    if undo.is_some() {
        ui.label(RichText::new("·").color(pal.pilltext.gamma_multiply(0.6)));
        let b = ui.add(egui::Button::new(style::semibold("Undo")));
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Undo"));
        clicks.undo = b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    } else if style::icon_button(ui, icons::ICON_CLOSE, "Dismiss").clicked() {
        clicks.dismiss = true;
    }
    clicks
}

impl Notices {
    /// Takes the message that [`Notices::current`] shows away (the toast's close button).
    pub(super) fn dismiss_current(&mut self) {
        if self.items.iter().any(|n| n.error) {
            self.items.retain(|n| !n.error);
        } else {
            self.items.pop();
        }
    }
}
