//! The maths of the timeline ruler (pure functions): which scroll offset a point of the ruler
//! stands for and back, and where the dashes go.
//!
//! A ruler either follows time ([`TimeScale`]: every calendar month between the newest and the
//! oldest photo gets the same share of the ruler, whatever it holds) or the plain scroll
//! position ([`Mapping::Flat`]).

use chrono::{Datelike, NaiveDate};

/// Smallest distance between two dashes, in points.
pub(super) const MIN_SPACING: f32 = 3.5;
/// Smallest distance between two year labels, in points.
const LABEL_SPACING: f32 = 14.0;
/// Dashes of a ruler that follows the scroll position.
const FLAT_DASHES: usize = 96;
/// Keyboard stops of a ruler that follows the scroll position.
const FLAT_STOPS: usize = 8;
/// With at most this many years, every month is a keyboard stop, else every year.
const FEW_YEARS: usize = 3;

/// A calendar month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
}

impl Month {
    /// The month of a capture time (seconds since the epoch), if it is a real date.
    pub fn of_timestamp(taken_at: i64) -> Option<Self> {
        if taken_at <= 0 {
            return None;
        }
        let d = chrono::DateTime::from_timestamp(taken_at, 0)?.date_naive();
        Some(Self {
            year: d.year(),
            month: d.month(),
        })
    }

    /// The month of a day number counted from 0001-01-01 (see `day_key`).
    pub fn of_day_key(key: i64) -> Option<Self> {
        let d = NaiveDate::from_num_days_from_ce_opt(i32::try_from(key).ok()?)?;
        Some(Self {
            year: d.year(),
            month: d.month(),
        })
    }

    /// Months since year 0, so neighbouring months differ by one.
    fn index(self) -> i32 {
        self.year * 12 + self.month.cast_signed() - 1
    }

    fn from_index(i: i32) -> Self {
        Self {
            year: i.div_euclid(12),
            month: i.rem_euclid(12) as u32 + 1,
        }
    }

    /// "March 2024".
    pub fn name(self) -> String {
        NaiveDate::from_ymd_opt(self.year, self.month, 1)
            .map_or_else(String::new, |d| d.format("%B %Y").to_string())
    }
}

/// How a point of the ruler maps to a scroll offset.
#[derive(Debug, Clone, PartialEq)]
pub enum Mapping {
    /// The ruler is the scroll range, evenly.
    Flat,
    /// The ruler is the range of months of the library.
    Time(TimeScale),
}

impl Mapping {
    /// The scroll offset for the point `frac` (0 top, 1 bottom) of the ruler; the content can
    /// scroll to at most `max`.
    pub fn offset_at(&self, frac: f32, max: f32) -> f32 {
        match self {
            Self::Flat => frac.clamp(0.0, 1.0) * max.max(0.0),
            Self::Time(t) => t.offset_at(frac, max),
        }
    }

    /// The point of the ruler for a scroll offset.
    pub fn fraction_at(&self, offset: f32, max: f32) -> f32 {
        match self {
            Self::Flat if max > 0.0 => (offset / max).clamp(0.0, 1.0),
            Self::Flat => 0.0,
            Self::Time(t) => t.fraction_at(offset, max),
        }
    }
}

/// Months of a library, newest first, with the scroll offset where each starts.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeScale {
    newest: i32,
    /// `months + 1` ascending offsets: where each month starts, and the end of the content. An
    /// empty month starts where the next older one does.
    tops: Vec<f32>,
}

impl TimeScale {
    /// `sections` lists, newest first, each section's month and where it starts; `total` is the
    /// end of the content. The first month starts at offset 0 (the top of the page).
    pub fn build(sections: impl IntoIterator<Item = (Month, f32)>, total: f32) -> Option<Self> {
        let sections: Vec<(Month, f32)> = sections.into_iter().collect();
        let (first, last) = (sections.first()?.0.index(), sections.last()?.0.index());
        let count = usize::try_from((first - last).max(0)).ok()? + 1;
        let mut starts: Vec<Option<f32>> = vec![None; count];
        for (month, top) in sections {
            let m = usize::try_from((first - month.index()).max(0)).unwrap_or(0);
            let slot = &mut starts[m.min(count - 1)];
            slot.get_or_insert(top);
        }
        let mut tops = vec![total; count + 1];
        let mut next = total;
        for m in (0..count).rev() {
            next = starts[m].unwrap_or(next).min(next);
            tops[m] = next;
        }
        tops[0] = 0.0;
        Some(Self {
            newest: first,
            tops,
        })
    }

    /// Number of calendar months covered.
    pub fn months(&self) -> usize {
        self.tops.len() - 1
    }

    /// The month at the point `frac` of the ruler.
    pub fn month_at(&self, frac: f32) -> Month {
        self.month_of_slot(self.slot(frac))
    }

    /// The month of slot `m` (0 is the newest).
    fn month_of_slot(&self, m: usize) -> Month {
        Month::from_index(self.newest - i32::try_from(m).unwrap_or(i32::MAX))
    }

    fn slot(&self, frac: f32) -> usize {
        let n = self.months();
        ((frac.clamp(0.0, 1.0) * n as f32) as usize).min(n - 1)
    }

    fn offset_at(&self, frac: f32, max: f32) -> f32 {
        let x = frac.clamp(0.0, 1.0) * self.months() as f32;
        let m = self.slot(frac);
        let (a, b) = (self.tops[m], self.tops[m + 1]);
        (a + (x - m as f32) * (b - a)).min(max.max(0.0))
    }

    fn fraction_at(&self, offset: f32, max: f32) -> f32 {
        if max > 0.0 && offset >= max - 0.5 {
            return 1.0;
        }
        let n = self.months();
        let m = self.tops[..n]
            .partition_point(|&t| t <= offset)
            .saturating_sub(1);
        let span = self.tops[m + 1] - self.tops[m];
        let within = if span > 0.0 {
            ((offset - self.tops[m]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((m as f32 + within) / n as f32).clamp(0.0, 1.0)
    }
}

/// Length class of a dash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashSize {
    Year,
    Month,
    Minor,
}

impl DashSize {
    /// Length of the dash at rest, in points.
    pub fn length(self) -> f32 {
        match self {
            Self::Year => 13.0,
            Self::Month => 8.0,
            Self::Minor => 4.0,
        }
    }
}

/// A dash the keyboard can reach.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    /// What assistive technology says, like "Jump to March 2024".
    pub label: String,
    /// The point of the ruler it jumps to.
    pub target: f32,
}

/// One dash of the ruler.
#[derive(Debug, Clone, PartialEq)]
pub struct Dash {
    /// Where it sits: 0 at the top of the ruler, 1 at the bottom.
    pub frac: f32,
    pub size: DashSize,
    /// The year written beside it, if it starts a year and there is room.
    pub year: Option<i32>,
    pub stop: Option<Stop>,
}

/// The dashes of a ruler `height` points tall, top to bottom.
pub fn dashes(mapping: &Mapping, height: f32) -> Vec<Dash> {
    match mapping {
        Mapping::Flat => flat_dashes(height),
        Mapping::Time(t) => time_dashes(t, height),
    }
}

fn flat_dashes(height: f32) -> Vec<Dash> {
    let n = ((height / MIN_SPACING) as usize).clamp(1, FLAT_DASHES);
    let stop_every = (n / FLAT_STOPS).max(1);
    (0..n)
        .map(|i| {
            let target = i as f32 / n as f32;
            Dash {
                frac: (i as f32 + 0.5) / n as f32,
                size: if i % 4 == 0 {
                    DashSize::Month
                } else {
                    DashSize::Minor
                },
                year: None,
                stop: (i % stop_every == 0).then(|| Stop {
                    label: format!("Scroll to {}%", (target * 100.0).round()),
                    target,
                }),
            }
        })
        .collect()
}

/// How many dashes each month gets, and how many months share one dash when even one per month
/// would be closer than [`MIN_SPACING`].
fn density(months: usize, height: f32) -> (usize, usize) {
    let per_month = [4_usize, 2]
        .into_iter()
        .find(|&s| height / (months * s) as f32 >= MIN_SPACING)
        .unwrap_or(1);
    let thin = if per_month == 1 {
        ((MIN_SPACING * months as f32 / height).ceil() as usize).max(1)
    } else {
        1
    };
    (per_month, thin)
}

fn time_dashes(t: &TimeScale, height: f32) -> Vec<Dash> {
    let n = t.months();
    let (per_month, thin) = density(n, height);
    let year_starts = |m: usize| m == 0 || t.month_of_slot(m).year != t.month_of_slot(m - 1).year;
    let years = (0..n).filter(|&m| year_starts(m)).count();
    let mut out = Vec::new();
    let mut last_label = f32::NEG_INFINITY;
    for m in (0..n).filter(|&m| year_starts(m) || m % thin == 0) {
        let month = t.month_of_slot(m);
        let target = m as f32 / n as f32;
        let at = |sub: usize| (m as f32 + (sub as f32 + 0.5) / per_month as f32) / n as f32;
        let starts_year = year_starts(m);
        let mut year = None;
        if starts_year && at(0) * height - last_label >= LABEL_SPACING {
            year = Some(month.year);
            last_label = at(0) * height;
        }
        let stop = if years > FEW_YEARS {
            starts_year.then(|| format!("Jump to {}", month.year))
        } else {
            Some(format!("Jump to {}", month.name()))
        };
        out.push(Dash {
            frac: at(0),
            size: if starts_year {
                DashSize::Year
            } else {
                DashSize::Month
            },
            year,
            stop: stop.map(|label| Stop { label, target }),
        });
        out.extend((1..per_month).map(|sub| Dash {
            frac: at(sub),
            size: DashSize::Minor,
            year: None,
            stop: None,
        }));
    }
    out
}

/// Index of the dash nearest to the point `frac` (`dashes` is not empty).
pub fn nearest(dashes: &[Dash], frac: f32) -> usize {
    let after = dashes.partition_point(|d| d.frac < frac);
    match after {
        0 => 0,
        i if i == dashes.len() => i - 1,
        i => {
            if frac - dashes[i - 1].frac <= dashes[i].frac - frac {
                i - 1
            } else {
                i
            }
        }
    }
}

#[cfg(test)]
mod tests;
