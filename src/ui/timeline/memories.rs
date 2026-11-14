//! "On this day": photos taken on today's date in earlier years.

use super::dates::{date_of, fmt_taken};
use super::{file_name, thumb_key};
use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::store::db::TimelineItem;
use crate::ui::style::{self, GAP};
use crate::ui::tiles::{self, Tile, TileEvent};
use crate::ui::{App, fmt_time, marked};
use chrono::{Datelike, NaiveDate};
use egui::{RichText, Sense, Vec2};

/// One "On this day" entry: an item of the album and how many years ago it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnThisDay {
    pub index: usize,
    pub years_ago: i32,
}

/// Most entries the "On this day" strip shows.
pub(super) const ON_THIS_DAY_MAX: usize = 30;

/// Items taken on the same month and day as `today` in earlier years, newest first (at most
/// [`ON_THIS_DAY_MAX`]). `index` points into `items`, whatever order they are in. Items whose
/// date is only the file's own time are left out: that is when the file was saved or copied,
/// not when it was taken. In years without a 29 February, its photos show up on 28 February.
pub fn on_this_day(items: &[TimelineItem], today: NaiveDate) -> Vec<OnThisDay> {
    let leap_day_too = today.month() == 2
        && today.day() == 28
        && NaiveDate::from_ymd_opt(today.year(), 2, 29).is_none();
    let mut hits: Vec<(i64, usize, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            if it.date_source == DateSource::File {
                return None;
            }
            let d = date_of(it.taken_at);
            let same_day = d.month() == today.month() && d.day() == today.day()
                || leap_day_too && d.month() == 2 && d.day() == 29;
            (same_day && d.year() < today.year()).then_some((
                it.taken_at,
                i,
                today.year() - d.year(),
            ))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(items[a.1].path.cmp(&items[b.1].path)));
    hits.truncate(ON_THIS_DAY_MAX);
    hits.into_iter()
        .map(|(_, index, years_ago)| OnThisDay { index, years_ago })
        .collect()
}

pub(super) fn years_ago_text(n: i32) -> String {
    if n == 1 {
        "1 year ago".to_string()
    } else {
        format!("{n} years ago")
    }
}

impl App {
    /// Today's date; tests can pin it.
    #[cfg_attr(
        not(test),
        expect(clippy::unused_self, reason = "only tests pin the date")
    )]
    pub(in crate::ui) fn today(&self) -> NaiveDate {
        #[cfg(test)]
        if let Some(d) = self.today_override {
            return d;
        }
        chrono::Local::now().date_naive()
    }

    /// A strip of photos taken on today's date in earlier years, above the timeline.
    pub(super) fn on_this_day_strip(&mut self, ui: &mut egui::Ui) {
        const SIZE: f32 = 96.0;
        let today = self.today();
        let entries: Vec<OnThisDay> = self.tl.on_this_day(today).to_vec();
        if entries.is_empty() {
            return;
        }
        ui.label(RichText::new("On this day").text_style(style::section()));
        ui.add_space(4.0);
        let mut open = None;
        let mut star = None;
        egui::ScrollArea::horizontal()
            .id_salt("on-this-day")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for (pos, e) in entries.iter().enumerate() {
                        let item = &self.tl.items[e.index];
                        ui.vertical(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(SIZE), Sense::hover());
                            let key = thumb_key(item);
                            let name = || file_name(item);
                            let tip = || format!("{}\n{}", file_name(item), fmt_taken(item));
                            let tile = Tile::new(&key, "On this day", &name)
                                .tip(&tip)
                                .badge(
                                    (item.kind == MediaKind::Video)
                                        .then(|| fmt_time(item.duration.unwrap_or(0.0))),
                                )
                                .favorite(marked(&self.fav_marks, item.file_id, item.favorite));
                            let id = ui.make_persistent_id(("otd", pos));
                            let o = tiles::show(ui, &mut self.thumbs, id, rect, &tile);
                            match o.event {
                                Some((TileEvent::Star, _)) => star = Some(e.index),
                                Some(_) => open = Some(pos),
                                None => {}
                            }
                            ui.label(RichText::new(years_ago_text(e.years_ago)).small());
                        });
                    }
                });
            });
        if let Some(i) = star {
            let it = &self.tl.items[i];
            let (id, fav) = (
                it.file_id,
                !marked(&self.fav_marks, it.file_id, it.favorite),
            );
            self.set_favorite(id, fav);
        }
        ui.add_space(6.0);
        if let Some(pos) = open {
            let idx: Vec<usize> = entries.iter().map(|e| e.index).collect();
            self.open_viewer_subset(&idx, pos);
        }
    }
}
