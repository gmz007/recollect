//! What the timeline tells its ruler: the months and where they start in the layout.

use super::View;
use super::{Layout, TimelineState};
use crate::ui::ruler::{Mapping, Month, RulerState, TimeScale, key_of, month_at_fraction};

impl TimelineState {
    /// Whether the ruler has something to do: items are shown and they do not all fit.
    pub(in crate::ui) fn ruler_shown(&self) -> bool {
        self.loaded
            && self.matches(&self.filter())
            && !self.items.is_empty()
            && !self.grid.ruler.fits()
    }

    /// Brings the ruler's mapping up to date with the layout (done when the layout changes).
    /// Views grouped by day follow time; the others follow the scroll position.
    pub(super) fn update_ruler(&mut self) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let headers = self.headers();
        let lead = self.lead;
        let key = key_of(&(self.layout_key, headers, lead.to_bits()));
        if !self.grid.ruler.needs_mapping(key) {
            return;
        }
        let mapping = if headers {
            let sections = self.groups.iter().enumerate().filter_map(|(gi, g)| {
                let row = *layout.group_row.get(gi)?;
                Some((Month::of_day_key(g.key)?, layout.tops.get(row)? + lead))
            });
            TimeScale::build(sections, layout.total + lead).map_or(Mapping::Flat, Mapping::Time)
        } else {
            self.dated_mapping(layout)
        };
        self.grid.ruler.set_mapping(key, mapping);
    }

    /// The ruler of a list that follows the scroll position. Albums are in date order (newest
    /// first), so their ruler writes each year where it starts; the other lists are in the
    /// order files were added, which says nothing about dates, so they get no labels.
    fn dated_mapping(&self, layout: &Layout) -> Mapping {
        if !matches!(self.view, View::Album(_)) {
            return Mapping::Flat;
        }
        let mut years: Vec<(i32, f32)> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            let Some(month) = Month::of_timestamp(item.taken_at) else {
                continue;
            };
            if years.last().is_none_or(|&(y, _)| y != month.year) {
                years.push((month.year, layout.reveal_span(i).0 + self.lead));
            }
        }
        if years.is_empty() {
            Mapping::Flat
        } else {
            Mapping::Dated(years)
        }
    }

    /// Runs `f` with the ruler's state and a way to find the date of the photo at a point of
    /// the ruler (for rulers that follow the scroll position): the first photo at the top, the
    /// last at the bottom.
    pub(in crate::ui) fn with_ruler<R>(
        &mut self,
        f: impl FnOnce(&mut RulerState, &dyn Fn(f32) -> Option<Month>) -> R,
    ) -> R {
        let items = &self.items;
        let date_at = |frac: f32| {
            month_at_fraction(items.len(), frac, |i| items.get(i).map(|it| it.taken_at))
        };
        f(&mut self.grid.ruler, &date_at)
    }
}
