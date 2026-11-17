//! The bar above the picture: back (naming where the viewer was opened from), the date and
//! position, and four icon buttons: Favorite, Add to album, Info and "⋯".

use super::{Action, Viewer};
use crate::media::MediaKind;
use crate::store::db::AlbumInfo;
use crate::ui::albums::{AlbumChoice, album_menu_items};
use crate::ui::preview::Previews;
use crate::ui::style::{self, Palette, ToggleLook, icons};
use crate::ui::timeline::{fmt_clock, fmt_day};
use egui::{Rect, RichText, Sense, Vec2};

/// Width of the "⋯" menu.
const MENU_WIDTH: f32 = 220.0;
/// Air between the bar's three groups.
const GROUP_GAP: f32 = 12.0;
/// Size of the back arrow.
const BACK_ICON: f32 = 20.0;

/// The width taken by the four buttons on the right of the bar.
const fn buttons_width() -> f32 {
    4.0 * style::ICON_BUTTON + 3.0 * style::GAP
}

/// The three parts of the bar: the back button's room, the centre and the buttons' room.
fn bar_parts(row: Rect) -> (Rect, Rect, Rect) {
    let side = buttons_width() + GROUP_GAP;
    let centre_w = (row.width() - 2.0 * side).max(0.0);
    let centre = Rect::from_center_size(row.center(), Vec2::new(centre_w, row.height()));
    let left = Rect::from_min_max(row.min, egui::pos2(centre.left(), row.max.y));
    let right = Rect::from_min_max(egui::pos2(row.max.x - buttons_width(), row.min.y), row.max);
    (left, centre, right)
}

impl Viewer {
    /// Draws the bar's contents into `ui` (its panel is exactly as high as the bar).
    pub(super) fn top_bar(
        &mut self,
        ui: &mut egui::Ui,
        previews: &mut Previews,
        env: &super::ViewerEnv<'_>,
        actions: &mut Vec<Action>,
    ) {
        let (left, centre, right) = bar_parts(ui.max_rect());
        self.back_button(ui, left, actions);
        self.date_and_position(ui, centre, previews);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = style::GAP;
                self.more_button(ui, env.can_find_similar, actions);
                self.info_button(ui);
                self.album_button(ui, env.albums, actions);
                self.favorite_button(ui, actions);
            },
        );
    }

    /// The arrow and the name of where the viewer was opened from, in the bar's left room.
    fn back_button(&self, ui: &mut egui::Ui, room: Rect, actions: &mut Vec<Action>) {
        let pal = Palette::of(ui.ctx());
        let font = egui::TextStyle::Button.resolve(ui.style());
        let max_text = (room.width() - BACK_ICON - 24.0).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(self.origin.clone(), font, pal.text);
        job.wrap = egui::text::TextWrapping {
            max_width: max_text,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = ui.painter().layout_job(job);
        let size = Vec2::new(
            (BACK_ICON + 20.0 + galley.size().x).min(room.width()),
            style::ICON_BUTTON,
        );
        let rect =
            Rect::from_min_size(egui::pos2(room.min.x, room.center().y - size.y / 2.0), size);
        let id = ui.make_persistent_id("viewer-back");
        let r = ui.interact(rect, id, Sense::click());
        if r.hovered() || r.has_focus() {
            ui.painter()
                .rect_filled(rect, style::CONTROL_RADIUS, pal.hover);
        }
        ui.painter().text(
            egui::pos2(rect.min.x + 8.0 + BACK_ICON / 2.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icons::ICON_ARROW_BACK.codepoint,
            style::icon_font_id(icons::ICON_ARROW_BACK, BACK_ICON),
            pal.text,
        );
        let text_pos = egui::pos2(
            rect.min.x + 8.0 + BACK_ICON + 4.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, pal.text);
        let name = format!("Back to {}", self.origin);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        if r.on_hover_text("Back (Esc)")
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            actions.push(Action::Close);
        }
    }

    /// The day (semibold) and under it the time and "2 of 4"; just the position when the
    /// item has no date.
    fn date_and_position(&self, ui: &mut egui::Ui, room: Rect, previews: &mut Previews) {
        let pal = Palette::of(ui.ctx());
        let e = self.current();
        let taken = previews
            .details(e.file_id)
            .map_or(e.taken_at, |d| d.taken_at);
        let position = format!("{} of {}", self.index + 1, self.len());
        let dated = taken > 0;
        let second = if dated {
            format!("{} · {position}", fmt_clock(taken))
        } else {
            position
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(room)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.set_max_width(room.width());
                ui.spacing_mut().item_spacing.y = 0.0;
                let used = if dated { 36.0 } else { 18.0 };
                ui.add_space(((room.height() - used) / 2.0).max(0.0));
                if dated {
                    let day = style::semibold(fmt_day(taken)).size(14.0).color(pal.text);
                    ui.add(egui::Label::new(day).truncate().selectable(false));
                }
                let line = RichText::new(second).size(12.0).color(pal.muted);
                ui.add(egui::Label::new(line).truncate().selectable(false));
            },
        );
    }

    fn favorite_button(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let e = self.current();
        let (icon, label, tip, on) = if e.favorite {
            (
                icons::ICON_FAVORITE,
                "Unfavorite",
                "Remove from favorites (F)",
                Some(ToggleLook::Strong),
            )
        } else {
            (
                icons::ICON_FAVORITE.outlined(),
                "Favorite",
                "Add to favorites (F)",
                None,
            )
        };
        if style::top_bar_toggle(ui, icon, label, tip, on).clicked() {
            actions.push(Action::Favorite(e.file_id, !e.favorite));
        }
    }

    fn album_button(&self, ui: &mut egui::Ui, albums: &[AlbumInfo], actions: &mut Vec<Action>) {
        let r = style::top_bar_button(
            ui,
            icons::ICON_PLAYLIST_ADD,
            "Add to album…",
            "Add to album",
        );
        let mut choice: Option<AlbumChoice> = None;
        egui::Popup::menu(&r)
            .align(egui::RectAlign::BOTTOM_END)
            .width(MENU_WIDTH)
            .show(|ui| album_menu_items(ui, albums, &mut choice));
        if let Some(c) = choice {
            actions.push(Action::AddToAlbum(self.current().file_id, c));
        }
    }

    fn info_button(&mut self, ui: &mut egui::Ui) {
        let on = self.info.then_some(ToggleLook::Pressed);
        if style::top_bar_toggle(ui, icons::ICON_INFO.outlined(), "Info", "Info (I)", on).clicked()
        {
            self.info = !self.info;
        }
    }

    fn more_button(
        &mut self,
        ui: &mut egui::Ui,
        can_find_similar: bool,
        actions: &mut Vec<Action>,
    ) {
        let r = style::top_bar_button(ui, icons::ICON_MORE_HORIZ, "More", "More");
        egui::Popup::menu(&r)
            .align(egui::RectAlign::BOTTOM_END)
            .width(MENU_WIDTH)
            .show(|ui| self.more_items(ui, can_find_similar, actions));
    }

    /// The rows of the "⋯" menu: zoom, the file, playing a video and Find similar.
    fn more_items(&mut self, ui: &mut egui::Ui, can_find_similar: bool, actions: &mut Vec<Action>) {
        style::menu_body(ui);
        let e = self.current().clone();
        if e.kind == MediaKind::Image {
            let label = if self.zoom {
                "Fit to window"
            } else {
                "Zoom to 100%"
            };
            if ui
                .add(egui::Button::new(label).shortcut_text("Z"))
                .clicked()
            {
                self.toggle_zoom();
                ui.close();
            }
            ui.separator();
        }
        if ui.button("Open with default app").clicked() {
            actions.push(Action::Open(e.path.clone()));
            ui.close();
        }
        if ui.button("Show in folder").clicked() {
            actions.push(Action::Reveal(e.path.clone()));
            ui.close();
        }
        if e.kind == MediaKind::Video && ui.button("Play video").clicked() {
            actions.push(Action::PlayAt(e.path.clone(), e.t.unwrap_or(0.0)));
            ui.close();
        }
        if can_find_similar
            && let Some(item_id) = e.similar_item()
            && ui.button("Find similar photos").clicked()
        {
            actions.push(Action::FindSimilar {
                item_id,
                label: e.name(),
            });
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_splits_into_room_for_back_a_centred_middle_and_the_buttons() {
        let row = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 52.0));
        let (left, centre, right) = bar_parts(row);
        assert_eq!(centre.center(), row.center());
        assert_eq!(left.max.x, centre.min.x);
        assert!(centre.max.x <= right.min.x);
        assert_eq!(right.max.x, row.max.x);
        assert_eq!(right.width(), buttons_width());
    }

    #[test]
    fn a_narrow_bar_leaves_no_middle() {
        let row = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 52.0));
        let (_, centre, _) = bar_parts(row);
        assert_eq!(centre.width(), 0.0);
    }
}
