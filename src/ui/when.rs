//! The "When" and "Sort" choices that narrow and order search results.

use super::style;
use crate::search::SearchSort;
use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Any,
    ThisYear,
    LastYear,
    Year(i32),
    /// Every year from `DateFilter::from` through `DateFilter::to`.
    Custom,
}

/// The date restriction of a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateFilter {
    pub when: When,
    pub from: i32,
    pub to: i32,
}

impl Default for DateFilter {
    fn default() -> Self {
        Self {
            when: When::Any,
            from: 0,
            to: 0,
        }
    }
}

/// Wall-clock seconds (see `media::metadata`) of the first and last second of the years
/// `from..=to` (swapped if given the wrong way round).
fn years_range(from: i32, to: i32) -> Option<(i64, i64)> {
    let (a, b) = (from.min(to), from.max(to));
    let start = NaiveDate::from_ymd_opt(a, 1, 1)?.and_hms_opt(0, 0, 0)?;
    let end = NaiveDate::from_ymd_opt(b, 12, 31)?.and_hms_opt(23, 59, 59)?;
    Some((start.and_utc().timestamp(), end.and_utc().timestamp()))
}

impl DateFilter {
    /// The range to search in, given the current year; `None` means any time.
    pub fn range(&self, this_year: i32) -> Option<(i64, i64)> {
        match self.when {
            When::Any => None,
            When::ThisYear => years_range(this_year, this_year),
            When::LastYear => years_range(this_year - 1, this_year - 1),
            When::Year(y) => years_range(y, y),
            When::Custom => years_range(self.from, self.to),
        }
    }

    fn label(&self) -> String {
        match self.when {
            When::Any => "Any time".into(),
            When::ThisYear => "This year".into(),
            When::LastYear => "Last year".into(),
            When::Year(y) => y.to_string(),
            When::Custom => "Custom…".into(),
        }
    }

    /// Draws the "When" chip (and, for a custom range, the "From" and "To" chips); `years` are
    /// the years that have items, newest first. Returns true when something changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, years: &[i32], this_year: i32) -> bool {
        let before = *self;
        let accessible = format!("When: {}", self.label());
        style::chip(
            ui,
            &accessible,
            &self.label(),
            self.when != When::Any,
            |ui| {
                ui.selectable_value(&mut self.when, When::Any, "Any time");
                ui.selectable_value(&mut self.when, When::ThisYear, "This year");
                ui.selectable_value(&mut self.when, When::LastYear, "Last year");
                for &y in years {
                    ui.selectable_value(&mut self.when, When::Year(y), y.to_string());
                }
                ui.selectable_value(&mut self.when, When::Custom, "Custom…");
            },
        );
        if self.when == When::Custom {
            if before.when != When::Custom {
                // Start from the most recent year with items.
                let y = years.first().copied().unwrap_or(this_year);
                (self.from, self.to) = (y, y);
            }
            // Offer every year in between, not only years with items.
            let (lo, hi) = (
                years
                    .iter()
                    .copied()
                    .min()
                    .unwrap_or(self.from)
                    .min(self.from),
                years.iter().copied().max().unwrap_or(self.to).max(self.to),
            );
            // Right-to-left layouts add the last chip first.
            let rtl = ui.layout().prefer_right_to_left();
            let mut chips = [("To", &mut self.to), ("From", &mut self.from)];
            if !rtl {
                chips.reverse();
            }
            for (label, value) in chips {
                let text = format!("{label}: {value}");
                style::chip(ui, &text, &text, true, |ui| {
                    for y in (lo..=hi).rev() {
                        ui.selectable_value(value, y, y.to_string());
                    }
                });
            }
        }
        *self != before
    }
}

/// Draws the "Sort" chip; returns true when it changed.
pub fn sort_ui(ui: &mut egui::Ui, sort: &mut SearchSort) -> bool {
    let before = *sort;
    let name = |s: SearchSort| match s {
        SearchSort::Relevance => "Best match",
        SearchSort::Newest => "Newest",
        SearchSort::Oldest => "Oldest",
    };
    let accessible = format!("Sort: {}", name(*sort));
    style::chip(
        ui,
        &accessible,
        name(*sort),
        *sort != SearchSort::Relevance,
        |ui| {
            for s in [
                SearchSort::Relevance,
                SearchSort::Newest,
                SearchSort::Oldest,
            ] {
                ui.selectable_value(sort, s, name(s));
            }
        },
    );
    *sort != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
            .and_utc()
            .timestamp()
    }

    #[test]
    fn choices_become_inclusive_year_ranges() {
        let f = |when, from, to| DateFilter { when, from, to };
        assert_eq!(f(When::Any, 0, 0).range(2026), None);
        let this = (ts(2026, 1, 1, 0, 0, 0), ts(2026, 12, 31, 23, 59, 59));
        assert_eq!(f(When::ThisYear, 0, 0).range(2026), Some(this));
        let last = (ts(2025, 1, 1, 0, 0, 0), ts(2025, 12, 31, 23, 59, 59));
        assert_eq!(f(When::LastYear, 0, 0).range(2026), Some(last));
        assert_eq!(f(When::Year(2025), 0, 0).range(2030), Some(last));
        let span = (ts(2019, 1, 1, 0, 0, 0), ts(2021, 12, 31, 23, 59, 59));
        assert_eq!(f(When::Custom, 2019, 2021).range(2026), Some(span));
        // A reversed pair means the same span.
        assert_eq!(f(When::Custom, 2021, 2019).range(2026), Some(span));
    }
}
