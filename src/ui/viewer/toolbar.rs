//! The bar above the picture: back, position in the list, and the actions for the item.

use super::{Action, Viewer};
use crate::media::MediaKind;
use crate::store::db::AlbumInfo;
use crate::ui::fmt_time;
use crate::ui::style::{self, icons};
use egui::{Pos2, Rect, RichText, Sense, Vec2};

/// Height of the viewer's toolbar row.
const BAR_H: f32 = 34.0;

/// Whether the toolbar buttons carry a word next to their icon: only when the window is wide
/// enough (narrower windows show the icons alone, with tooltips).
pub(in crate::ui) fn toolbar_shows_text(window_width: f32) -> bool {
    window_width >= 1000.0
}

/// Where the "3 of 40" label starts (its left edge), or `None` when there is no room for it
/// between the buttons on the left (ending at `left_end`) and on the right (starting at
/// `right_start`). It prefers the middle of the bar, else sits right beside the buttons.
pub(super) fn position_slot(
    centre: f32,
    left_end: f32,
    right_start: f32,
    width: f32,
) -> Option<f32> {
    const AIR: f32 = 12.0;
    let centred = centre - width / 2.0;
    let beside = right_start - AIR - width;
    if centred >= left_end + AIR && centred + width <= right_start - AIR {
        Some(centred)
    } else if beside >= left_end + AIR {
        Some(beside)
    } else {
        None
    }
}

impl Viewer {
    pub(super) fn top_bar(
        &mut self,
        ui: &mut egui::Ui,
        albums: &[AlbumInfo],
        actions: &mut Vec<Action>,
    ) {
        let pal = style::Palette::of(ui.ctx());
        let text = toolbar_shows_text(ui.ctx().content_rect().width());
        let width = ui.available_width();
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, BAR_H), Sense::hover());
        // Keep clear of the window's edges.
        let row = Rect::from_min_max(row.min + Vec2::new(8.0, 0.0), row.max - Vec2::new(8.0, 0.0));
        // The right-hand buttons first, so the left side knows how much room is left.
        let right = ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = if text { 10.0 } else { 4.0 };
                self.right_buttons(ui, albums, actions, text);
            },
        );
        let right_start = right.response.rect.min.x;
        let pos = format!("{} of {}", self.index + 1, self.len());
        let pos_w = ui
            .painter()
            .layout_no_wrap(
                pos.clone(),
                egui::TextStyle::Body.resolve(ui.style()),
                pal.text2,
            )
            .size()
            .x;
        // Room for the name: what is left of the bar, minus the position and some air.
        let left_max = (right_start - 8.0 - row.min.x).max(0.0);
        let left_rect = Rect::from_min_size(row.min, Vec2::new(left_max, BAR_H));
        let left = ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                self.left_buttons(ui, row.width(), pos_w, text, actions);
            },
        );
        let left_end = left.response.rect.max.x;
        // The position sits in the middle of the bar when there is room there, else right
        // beside the buttons, else it is left out. It is plain text: never a click target.
        if let Some(x) = position_slot(row.center().x, left_end, right_start, pos_w) {
            let rect = Rect::from_min_size(
                Pos2::new(x, row.center().y - 11.0),
                Vec2::new(pos_w + 2.0, 22.0),
            );
            ui.put(
                rect,
                egui::Label::new(RichText::new(pos).color(pal.text2)).selectable(false),
            );
        }
    }

    /// The left of the toolbar: Back and, while the info panel is closed, the item's name.
    /// `row_width` is the bar's width and `pos_w` the position label's.
    fn left_buttons(
        &mut self,
        ui: &mut egui::Ui,
        row_width: f32,
        pos_w: f32,
        text: bool,
        actions: &mut Vec<Action>,
    ) {
        if style::toolbar_button(
            ui,
            icons::ICON_ARROW_BACK,
            text.then_some("Back"),
            "Back",
            "Back",
            None,
            false,
        )
        .clicked()
        {
            actions.push(Action::Close);
        }
        // The name is shown once: here while the info panel is closed, there while open.
        if self.info {
            return;
        }
        ui.add_space(8.0);
        let room = (ui.available_width() - pos_w - 24.0)
            .min(row_width * 0.3)
            .max(0.0);
        if room > 40.0 {
            ui.scope(|ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(style::semibold(self.current().name())).truncate());
            });
        }
    }

    /// The buttons on the right of the toolbar, laid out right to left. With `text`, each has a
    /// short word next to its icon.
    fn right_buttons(
        &mut self,
        ui: &mut egui::Ui,
        albums: &[AlbumInfo],
        actions: &mut Vec<Action>,
        text: bool,
    ) {
        let word = |w: &'static str| text.then_some(w);
        if self.current().kind == MediaKind::Image {
            self.zoom_button(ui, word);
        }
        self.file_buttons(ui, actions, text);
        if style::toolbar_button(
            ui,
            icons::ICON_INFO,
            word("Info"),
            "Info",
            "Photo details",
            None,
            self.info,
        )
        .clicked()
        {
            self.info = !self.info;
        }
        if let Some(c) = crate::ui::albums::album_menu_icon(ui, albums, word("Add to album")) {
            actions.push(Action::AddToAlbum(self.current().file_id, c));
        }
        self.favorite_button(ui, actions, text);
        if self.current().kind == MediaKind::Video {
            self.play_button(ui, actions);
        }
    }

    /// The zoom toggle (images only): "Zoom" to 100 % or "Fit" back to the window.
    fn zoom_button(
        &mut self,
        ui: &mut egui::Ui,
        word: impl Fn(&'static str) -> Option<&'static str>,
    ) {
        let (icon, label) = if self.zoom {
            (icons::ICON_ZOOM_OUT, "Fit to window")
        } else {
            (icons::ICON_ZOOM_IN, "Zoom to 100%")
        };
        let tip = if self.zoom {
            label.to_string()
        } else {
            "Zoom to 100% (one image pixel per screen pixel; photos larger than \
             4096 px on a side are scaled down to that first)"
                .to_string()
        };
        let short = if self.zoom { "Fit" } else { "Zoom" };
        if style::toolbar_button(ui, icon, word(short), label, &tip, None, self.zoom).clicked() {
            self.toggle_zoom();
        }
    }

    /// "Show in folder" and "Open", which act on the file on disk.
    fn file_buttons(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, text: bool) {
        let word = |w: &'static str| text.then_some(w);
        let path = self.current().path.clone();
        if style::toolbar_button(
            ui,
            icons::ICON_FOLDER_OPEN,
            word("Show in folder"),
            "Show in folder",
            "Show in folder",
            None,
            false,
        )
        .clicked()
        {
            actions.push(Action::Reveal(path.clone()));
        }
        if style::toolbar_button(
            ui,
            icons::ICON_OPEN_IN_NEW,
            word("Open"),
            "Open with default app",
            "Open with default app",
            None,
            false,
        )
        .clicked()
        {
            actions.push(Action::Open(path));
        }
    }

    /// The accent button that plays the current video from the chosen moment.
    fn play_button(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let pal = style::Palette::of(ui.ctx());
        let e = self.current().clone();
        ui.add_space(8.0);
        let t = e.t.unwrap_or(0.0);
        let label = format!("Play at {}", fmt_time(t));
        let job = style::icon_label_job(
            ui,
            icons::ICON_PLAY_ARROW,
            20.0,
            pal.onaccent,
            Some((&label, pal.onaccent)),
            4.0,
        );
        let b = egui::Button::new(job).fill(pal.accent);
        let r = ui.add(b);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if r.on_hover_text("Play this video").clicked() {
            actions.push(Action::PlayAt(e.path, t));
        }
    }

    fn favorite_button(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, text: bool) {
        let e = self.current();
        let pal = style::Palette::of(ui.ctx());
        let (icon, label, short, color) = if e.favorite {
            (icons::ICON_STAR, "Unfavorite", "Favorited", Some(pal.text))
        } else {
            (icons::ICON_STAR.outlined(), "Favorite", "Favorite", None)
        };
        let tip = if e.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        if style::toolbar_button(ui, icon, text.then_some(short), label, tip, color, false)
            .clicked()
        {
            actions.push(Action::Favorite(e.file_id, !e.favorite));
        }
    }
}
