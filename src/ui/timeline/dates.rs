//! Dates as the album shows them: section keys and headings, the year and month index,
//! and how a capture time is written out.

use super::Group;
use crate::media::metadata::DateSource;
use crate::store::db::TimelineItem;
use chrono::{Datelike, NaiveDate};
use std::collections::BTreeMap;
use std::time::Duration;

pub(super) fn date_of(taken_at: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

/// Section key of a capture time (local wall-clock seconds stored as if UTC). Keys of
/// neighbouring sections are not necessarily adjacent integers; they only identify a section.
pub fn group_key(taken_at: i64, group: Group) -> i64 {
    let d = date_of(taken_at);
    match group {
        Group::Day => i64::from(d.num_days_from_ce()),
        Group::Month => i64::from(d.year()) * 12 + i64::from(d.month0()),
        Group::Year => i64::from(d.year()),
    }
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

/// Section title such as "Saturday 4 July 2019 · 12", "July 2019 · 132" or "2019 · 1,204".
pub fn header_text(group: Group, key: i64, count: usize) -> String {
    let name = match group {
        Group::Day => i32::try_from(key)
            .ok()
            .and_then(NaiveDate::from_num_days_from_ce_opt)
            .map(|d| d.format("%A %-d %B %Y").to_string()),
        Group::Month => i32::try_from(key.div_euclid(12))
            .ok()
            .zip(u32::try_from(key.rem_euclid(12) + 1).ok())
            .and_then(|(year, month)| NaiveDate::from_ymd_opt(year, month, 1))
            .map(|d| d.format("%B %Y").to_string()),
        Group::Year => Some(key.to_string()),
    }
    .unwrap_or_default();
    format!("{name} · {}", thousands(count))
}

/// Years (newest first) with their months, for the date navigator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearEntry {
    pub year: i32,
    pub count: usize,
    /// Index of the year's first item in display order.
    pub first: usize,
    pub months: Vec<MonthEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthEntry {
    pub month: u32,
    pub count: usize,
    pub first: usize,
}

pub fn date_index(items: &[TimelineItem]) -> Vec<YearEntry> {
    let mut by_month: BTreeMap<(i32, u32), (usize, usize)> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        let d = date_of(it.taken_at);
        let e = by_month.entry((d.year(), d.month())).or_insert((0, i));
        e.0 += 1;
        e.1 = e.1.min(i);
    }
    let mut years: Vec<YearEntry> = Vec::new();
    for (&(year, month), &(count, first)) in by_month.iter().rev() {
        let m = MonthEntry {
            month,
            count,
            first,
        };
        match years.last_mut() {
            Some(y) if y.year == year => {
                y.count += count;
                y.first = y.first.min(first);
                y.months.push(m);
            }
            _ => years.push(YearEntry {
                year,
                count,
                first,
                months: vec![m],
            }),
        }
    }
    years
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
