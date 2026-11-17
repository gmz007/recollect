//! Sections and rows with known vertical offsets, so only what is in view is drawn and
//! the arrow keys can follow the rows (pure functions).

use super::dates::day_key;
use crate::store::db::TimelineItem;
use crate::ui::Nav;
use crate::ui::style::GAP;
use std::ops::Range;

pub(super) const HEADER_H: f32 = 36.0;
pub(super) const SECTION_GAP: f32 = 12.0;

/// A run of consecutive items that share a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSpan {
    pub key: i64,
    pub start: usize,
    pub end: usize,
}

/// Splits `items` (already in display order) into sections, one per day. Without headers
/// everything is one section.
pub fn compute_groups(items: &[TimelineItem], headers: bool) -> Vec<GroupSpan> {
    let mut out: Vec<GroupSpan> = Vec::new();
    if items.is_empty() {
        return out;
    }
    if !headers {
        out.push(GroupSpan {
            key: 0,
            start: 0,
            end: items.len(),
        });
        return out;
    }
    for (i, it) in items.iter().enumerate() {
        let key = day_key(it.taken_at);
        match out.last_mut() {
            Some(g) if g.key == key => g.end = i + 1,
            _ => out.push(GroupSpan {
                key,
                start: i,
                end: i + 1,
            }),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header { group: usize, first: usize },
    Tiles { start: usize, end: usize },
}

impl Row {
    pub(super) fn first_item(self) -> usize {
        match self {
            Row::Header { first, .. } => first,
            Row::Tiles { start, .. } => start,
        }
    }
}

/// Rows with vertical offsets for one width and tile size.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub rows: Vec<Row>,
    /// Top of each row, ascending.
    pub tops: Vec<f32>,
    pub total: f32,
    pub cols: usize,
    /// Edge length of a tile after fitting whole columns into the width.
    pub tile: f32,
    /// Index of the first row of each group (its header, when there are headers).
    pub group_row: Vec<usize>,
}

impl Layout {
    pub fn build(groups: &[GroupSpan], width: f32, tile_size: f32, headers: bool) -> Self {
        let cols = (((width + GAP) / (tile_size + GAP)).floor() as usize).max(1);
        let tile = ((width - GAP * (cols - 1) as f32) / cols as f32).max(1.0);
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        let mut group_row = Vec::with_capacity(groups.len());
        let mut y = 0.0;
        for (gi, g) in groups.iter().enumerate() {
            group_row.push(rows.len());
            if headers {
                tops.push(y);
                rows.push(Row::Header {
                    group: gi,
                    first: g.start,
                });
                y += HEADER_H;
            }
            let mut s = g.start;
            while s < g.end {
                let e = (s + cols).min(g.end);
                tops.push(y);
                rows.push(Row::Tiles { start: s, end: e });
                y += tile + GAP;
                s = e;
            }
            if headers {
                y += SECTION_GAP;
            }
        }
        Self {
            rows,
            tops,
            total: y,
            cols,
            tile,
            group_row,
        }
    }

    /// Rows that intersect the vertical span `y0..y1`.
    pub fn visible(&self, y0: f32, y1: f32) -> Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }
        let start = self.tops.partition_point(|&t| t <= y0).saturating_sub(1);
        let end = self.tops.partition_point(|&t| t < y1);
        start..end.max(start)
    }

    /// The section that row `row` belongs to (with headers).
    pub fn group_of_row(&self, row: usize) -> usize {
        self.group_row
            .partition_point(|&r| r <= row)
            .saturating_sub(1)
    }

    /// The section whose heading is pinned at the top of a view that starts `view_top` into the
    /// rows, and where the heading sits (row coordinates): at the top of the view, pushed up by
    /// the next section's heading as it arrives. Nothing is pinned at the very top or in a
    /// layout without headings.
    pub fn pinned_header(&self, view_top: f32) -> Option<(usize, f32)> {
        if view_top <= 0.0 || !matches!(self.rows.first(), Some(Row::Header { .. })) {
            return None;
        }
        let row = self.visible(view_top, view_top + 1.0).start;
        let group = self.group_of_row(row);
        let natural = self.tops[*self.group_row.get(group)?];
        let mut y = view_top.max(natural);
        if let Some(&next) = self.group_row.get(group + 1) {
            y = y.min(self.tops[next] - HEADER_H);
        }
        Some((group, y))
    }

    /// The row that draws `item`.
    pub fn row_of_item(&self, item: usize) -> usize {
        self.rows
            .partition_point(|r| r.first_item() <= item)
            .saturating_sub(1)
    }

    /// Vertical span `(top, height)` to reveal for `item`; includes the section header when the
    /// item is in the first tile row of its section.
    pub fn reveal_span(&self, item: usize) -> (f32, f32) {
        let r = self.row_of_item(item);
        let mut top = self.tops[r];
        if r > 0 && matches!(self.rows[r - 1], Row::Header { .. }) {
            top = self.tops[r - 1];
        }
        (top, self.tops[r] + self.tile - top)
    }
}

impl Layout {
    /// Scroll offset for jumping to `item`: the section header when the item starts a section,
    /// else the row that contains it (e.g. a month inside a year section).
    pub fn jump_top(&self, groups: &[GroupSpan], item: usize) -> f32 {
        let g = groups.partition_point(|g| g.end <= item);
        if groups.get(g).is_some_and(|g| g.start == item)
            && let Some(&top) = self.group_row.get(g).and_then(|&r| self.tops.get(r))
        {
            return top;
        }
        self.tops
            .get(self.row_of_item(item))
            .copied()
            .unwrap_or(0.0)
    }

    /// Item reached by an arrow key from `cur` (`cur < len`). Up and down go to the same
    /// column of the previous / next row of tiles in the layout, across section boundaries,
    /// clamped to the last tile of a shorter row; left and right go to the previous / next item.
    pub fn step(&self, cur: usize, len: usize, key: Nav) -> usize {
        // A layout built for a longer list must not point past the end of this one.
        self.step_unclamped(cur, len, key)
            .min(len.saturating_sub(1))
    }

    fn step_unclamped(&self, cur: usize, len: usize, key: Nav) -> usize {
        match key {
            Nav::Left => cur.saturating_sub(1),
            Nav::Right => (cur + 1).min(len.saturating_sub(1)),
            Nav::Up | Nav::Down => {
                let r = self.row_of_item(cur);
                let Some(Row::Tiles { start, .. }) = self.rows.get(r).copied() else {
                    return cur;
                };
                let col = cur.saturating_sub(start);
                let target = if key == Nav::Up {
                    self.rows[..r].iter().rev().find_map(tiles_of)
                } else {
                    self.rows[r + 1..].iter().find_map(tiles_of)
                };
                match target {
                    Some((s, e)) => (s + col).min(e.saturating_sub(1)),
                    None => cur,
                }
            }
        }
    }
}

fn tiles_of(r: &Row) -> Option<(usize, usize)> {
    match *r {
        Row::Tiles { start, end } => Some((start, end)),
        Row::Header { .. } => None,
    }
}

/// Scroll offset that makes `top..top + height` visible, or `None` if it already is.
pub(super) fn reveal(top: f32, height: f32, view: f32, offset: f32) -> Option<f32> {
    if top < offset {
        Some(top)
    } else if top + height > offset + view {
        Some((top + height - view).max(0.0))
    } else {
        None
    }
}
