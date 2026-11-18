//! Dates as the album shows them: section keys and headings,
//! and how a capture time is written out.

use crate::media::metadata::DateSource;
use crate::store::db::TimelineItem;
use chrono::{Datelike, NaiveDate};
use std::time::Duration;

pub(super) fn date_of(taken_at: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

/// Section key of a capture time (local wall-clock seconds stored as if UTC): the day. Keys of
/// neighbouring sections are not necessarily adjacent integers; they only identify a section.
pub fn day_key(taken_at: i64) -> i64 {
    i64::from(date_of(taken_at).num_days_from_ce())
}

/// The first and last year in which the items were taken. Items dated only by their file's own
/// time are left out (that is when the file was saved, not when it was taken); `None` when no
/// item has a known date.
pub fn year_range(items: &[TimelineItem]) -> Option<(i32, i32)> {
    items
        .iter()
        .filter(|it| it.date_source != DateSource::File)
        .map(|it| date_of(it.taken_at).year())
        .fold(None, |range, y| match range {
            None => Some((y, y)),
            Some((lo, hi)) => Some((lo.min(y), hi.max(y))),
        })
}

/// `1204` becomes `1,204`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Heading of the section of the day `key` (see [`day_key`]) as seen on `today`: "Today",
/// "Yesterday", or the weekday and date such as "Sunday 15 November", with the year added when it
/// is not this year ("Saturday 4 July 2019").
pub fn day_header_text(key: i64, today: NaiveDate) -> String {
    let Some(day) = i32::try_from(key)
        .ok()
        .and_then(NaiveDate::from_num_days_from_ce_opt)
    else {
        return String::new();
    };
    if day == today {
        "Today".to_string()
    } else if today.pred_opt() == Some(day) {
        "Yesterday".to_string()
    } else if day.year() == today.year() {
        day.format("%A %-d %B").to_string()
    } else {
        day.format("%A %-d %B %Y").to_string()
    }
}

pub(super) fn fmt_taken(item: &TimelineItem) -> String {
    fmt_taken_at(item.taken_at, Some(item.date_source))
}

/// "Saturday 4 July 2019, 12:30".
pub fn fmt_date(taken_at: i64) -> String {
    chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.naive_utc().format("%A %-d %B %Y, %H:%M").to_string())
        .unwrap_or_default()
}

/// "Saturday 4 July 2019": the day alone.
pub fn fmt_day(taken_at: i64) -> String {
    chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.naive_utc().format("%A %-d %B %Y").to_string())
        .unwrap_or_default()
}

/// "12:30": the time of day alone, as [`fmt_date`] shows it.
pub fn fmt_clock(taken_at: i64) -> String {
    chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.naive_utc().format("%-H:%M").to_string())
        .unwrap_or_default()
}

/// What to call a date that is only the file's own time rather than when the photo was taken.
pub const DATE_FROM_FILE: &str = "Date from the file";

/// [`fmt_date`] with "Date from the file" added when the time is only the file's.
pub fn fmt_taken_at(taken_at: i64, source: Option<DateSource>) -> String {
    let t = fmt_date(taken_at);
    if source == Some(DateSource::File) {
        format!("{t} · {DATE_FROM_FILE}")
    } else {
        t
    }
}

/// Time until the next local midnight after `now` (when "today" changes).
pub fn until_next_midnight(now: chrono::NaiveDateTime) -> Duration {
    now.date()
        .succ_opt()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|m| (m - now).to_std().ok())
        .unwrap_or(Duration::from_secs(3600))
}
