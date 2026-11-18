//! "On this day": photos taken on today's date in earlier years.

use super::banner::{Banner, BannerEvent};
use super::dates::date_of;
use crate::media::metadata::DateSource;
use crate::store::db::TimelineItem;
use crate::ui::App;
use chrono::{Datelike, NaiveDate};

/// One "On this day" entry: an item of the album and how many years ago it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnThisDay {
    pub index: usize,
    pub years_ago: i32,
}

/// Most entries the "On this day" banner shows.
pub(super) const ON_THIS_DAY_MAX: usize = 200;

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

    /// What the "On this day" banner shows, if it is to be shown: photos were taken on today's
    /// date in earlier years and the user has not hidden it today.
    pub(super) fn on_this_day_banner(&mut self) -> Option<Banner> {
        let today = self.today();
        if !self.settings.show_on_this_day {
            return None;
        }
        if self.settings.on_this_day_hidden_on.as_deref() == Some(today.to_string().as_str()) {
            return None;
        }
        let entries = self.tl.on_this_day(today);
        let first = entries.first()?;
        let one_year = entries.iter().all(|e| e.years_ago == first.years_ago);
        Some(Banner {
            years_ago: one_year.then_some(first.years_ago),
            entries: entries.iter().map(|e| e.index).collect(),
            date: today,
        })
    }

    /// Acts on a click on the banner: opens its photos, or hides it until tomorrow.
    pub(super) fn banner_event(&mut self, banner: &Banner, event: BannerEvent) {
        match event {
            BannerEvent::See => self.open_viewer_subset(&banner.entries, 0),
            BannerEvent::Hide => {
                let today = banner.date.to_string();
                self.change_settings(|s| s.on_this_day_hidden_on = Some(today));
            }
        }
    }
}
