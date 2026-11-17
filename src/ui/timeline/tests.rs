#![expect(clippy::too_many_lines, reason = "a scenario test reads top to bottom")]
#![expect(clippy::cast_possible_wrap, reason = "small test fixture indices")]
use super::dates::{day_header_text, day_key, until_next_midnight};
use super::layout::{HEADER_H, SECTION_GAP, reveal};
use super::memories::{ON_THIS_DAY_MAX, years_ago_text};
use super::*;
use crate::media::metadata::DateSource;
use crate::ui::style::GAP;
use crate::ui::{Nav, selection};
use std::time::Duration;

fn ts(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, mi, s)
        .unwrap()
        .and_utc()
        .timestamp()
}

#[test]
fn stepping_never_leaves_a_shorter_list() {
    // A layout built for 12 items, used while only 6 remain.
    let groups = [GroupSpan {
        start: 0,
        end: 12,
        key: 0,
    }];
    let l = Layout::build(&groups, 400.0, 100.0, false);
    assert!(l.cols >= 2);
    for cur in 0..6 {
        for key in [Nav::Left, Nav::Right, Nav::Up, Nav::Down] {
            assert!(l.step(cur, 6, key) < 6, "{cur} {key:?}");
        }
    }
    assert_eq!(l.step(0, 0, Nav::Right), 0);
    // An out-of-range current item is also tolerated.
    assert!(l.step(40, 6, Nav::Up) < 6);
}

fn item(taken_at: i64) -> TimelineItem {
    TimelineItem {
        file_id: taken_at,
        item_id: None,
        path: format!("/p/{taken_at}.jpg").into(),
        kind: MediaKind::Image,
        mtime: 0,
        taken_at,
        date_source: DateSource::Exif,
        duration: None,
        width: None,
        height: None,
        favorite: false,
        camera: None,
    }
}

#[test]
fn keys_split_at_midnight_and_year_end() {
    let last = ts(2019, 12, 31, 23, 59, 59);
    let first = ts(2020, 1, 1, 0, 0, 0);
    assert_ne!(day_key(last), day_key(first));
    assert_eq!(
        day_key(ts(2019, 7, 4, 0, 0, 0)),
        day_key(ts(2019, 7, 4, 23, 59, 59))
    );
    // Leap day is its own day.
    let leap = ts(2020, 2, 29, 12, 0, 0);
    assert_ne!(day_key(leap), day_key(ts(2020, 3, 1, 0, 0, 0)));
    assert_eq!(day_key(ts(2020, 3, 1, 0, 0, 0)) - day_key(leap), 1);
}

#[test]
fn headers_read_naturally() {
    let today = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
    let head = |y, m, d| day_header_text(day_key(ts(y, m, d, 10, 0, 0)), today);
    assert_eq!(head(2026, 11, 17), "Today");
    assert_eq!(head(2026, 11, 16), "Yesterday");
    // This year: no year.
    assert_eq!(head(2026, 11, 15), "Sunday 15 November");
    // Another year: with the year.
    assert_eq!(head(2019, 7, 6), "Saturday 6 July 2019");
    // Yesterday across a year boundary.
    let new_year = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    assert_eq!(
        day_header_text(day_key(ts(2026, 12, 31, 9, 0, 0)), new_year),
        "Yesterday"
    );
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(999), "999");
    assert_eq!(thousands(1_000_000), "1,000,000");
}

#[test]
fn groups_follow_display_order() {
    let items = vec![
        item(ts(2020, 1, 1, 9, 0, 0)),
        item(ts(2020, 1, 1, 8, 0, 0)),
        item(ts(2019, 12, 31, 23, 59, 59)),
        item(ts(2019, 12, 1, 8, 0, 0)),
    ];
    let g = compute_groups(&items, true);
    let spans: Vec<_> = g.iter().map(|g| (g.start, g.end)).collect();
    assert_eq!(spans, [(0, 2), (2, 3), (3, 4)]);
    let flat = compute_groups(&items, false);
    assert_eq!(flat.len(), 1);
    assert_eq!((flat[0].start, flat[0].end), (0, 4));
    assert!(compute_groups(&[], true).is_empty());
}

#[test]
fn layout_offsets_and_visibility() {
    // Two sections of 5 and 2 items; width fits 3 tiles of 100 plus two gaps.
    let groups = [
        GroupSpan {
            key: 1,
            start: 0,
            end: 5,
        },
        GroupSpan {
            key: 2,
            start: 5,
            end: 7,
        },
    ];
    let l = Layout::build(&groups, 300.0 + 2.0 * GAP, 100.0, true);
    assert_eq!(l.cols, 3);
    assert!((l.tile - 100.0).abs() < 1e-3);
    assert_eq!(l.rows.len(), 5);
    assert_eq!(l.group_row, [0, 3]);
    let row = 100.0 + GAP;
    let expect = [
        0.0,
        HEADER_H,
        HEADER_H + row,
        HEADER_H + 2.0 * row + SECTION_GAP,
        2.0 * HEADER_H + 2.0 * row + SECTION_GAP,
    ];
    for (a, b) in l.tops.iter().zip(expect) {
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }
    assert!((l.total - (expect[4] + row + SECTION_GAP)).abs() < 1e-3);
    // Visible rows: any row intersecting the span.
    assert_eq!(l.visible(0.0, 10.0), 0..1);
    assert_eq!(l.visible(0.0, HEADER_H + 1.0), 0..2);
    assert_eq!(l.visible(HEADER_H + 5.0, HEADER_H + 6.0), 1..2);
    assert_eq!(l.visible(expect[2] + 1.0, expect[4] - 1.0), 2..4);
    assert_eq!(l.visible(1000.0, 1100.0).start, 4);
    // Item to row.
    assert_eq!(l.row_of_item(0), 1);
    assert_eq!(l.row_of_item(2), 1);
    assert_eq!(l.row_of_item(3), 2);
    assert_eq!(l.row_of_item(5), 4);
    // Revealing the first tile row of a section includes its header.
    assert_eq!(l.reveal_span(5).0, expect[3]);
    assert_eq!(l.reveal_span(3).0, expect[2]);
    // No headers: tiles only.
    let flat = Layout::build(
        &[GroupSpan {
            key: 0,
            start: 0,
            end: 7,
        }],
        300.0 + 2.0 * GAP,
        100.0,
        false,
    );
    assert_eq!(flat.rows.len(), 3);
    assert_eq!(flat.tops[0], 0.0);
    assert!(
        Layout::build(&[], 300.0, 100.0, true)
            .visible(0.0, 50.0)
            .is_empty()
    );
}

#[test]
fn visible_rows_stay_cheap_for_huge_libraries() {
    let n = 200_000;
    let items: Vec<_> = (0..n)
        .map(|i| item(1_500_000_000 - i as i64 * 3000))
        .collect();
    let groups = compute_groups(&items, true);
    let l = Layout::build(&groups, 1000.0, 180.0, true);
    let y = l.total * 0.5;
    let r = l.visible(y, y + 800.0);
    assert!(!r.is_empty() && r.len() < 20, "{r:?}");
    let first = l.rows[r.start].first_item();
    assert!(first > 0 && first < n);
}

/// Sections of 5 and 2 items with 3 columns: rows are
/// header, [0 1 2], [3 4], header, [5 6].
fn two_sections() -> (Vec<GroupSpan>, Layout) {
    let groups = vec![
        GroupSpan {
            key: 1,
            start: 0,
            end: 5,
        },
        GroupSpan {
            key: 2,
            start: 5,
            end: 7,
        },
    ];
    let l = Layout::build(&groups, 300.0 + 2.0 * GAP, 100.0, true);
    (groups, l)
}

#[test]
fn arrows_follow_the_tile_rows_across_sections() {
    let (_, l) = two_sections();
    let n = 7;
    // Down across the boundary keeps the column.
    assert_eq!(l.step(3, n, Nav::Down), 5);
    assert_eq!(l.step(4, n, Nav::Down), 6);
    // Down into a shorter row clamps to its last tile (within a section and across).
    assert_eq!(l.step(2, n, Nav::Down), 4);
    assert_eq!(l.step(4, n, Nav::Down), 6);
    // Up across the boundary, from the short last row into the full row above... and from
    // a column that the short row above does not have.
    assert_eq!(l.step(5, n, Nav::Up), 3);
    assert_eq!(l.step(6, n, Nav::Up), 4);
    assert_eq!(l.step(3, n, Nav::Up), 0);
    // Nothing above the first row / below the last one.
    assert_eq!(l.step(1, n, Nav::Up), 1);
    assert_eq!(l.step(6, n, Nav::Down), 6);
    // Left and right walk the items in order, also across sections.
    assert_eq!(l.step(5, n, Nav::Left), 4);
    assert_eq!(l.step(4, n, Nav::Right), 5);
    assert_eq!(l.step(6, n, Nav::Right), 6);
    assert_eq!(l.step(0, n, Nav::Left), 0);
}

#[test]
fn arrows_clamp_to_a_short_row_in_the_next_section() {
    // Sections of 4 and 1 item with 3 columns: [0 1 2], [3], header, [4].
    let groups = [
        GroupSpan {
            key: 1,
            start: 0,
            end: 4,
        },
        GroupSpan {
            key: 2,
            start: 4,
            end: 5,
        },
    ];
    let l = Layout::build(&groups, 300.0 + 2.0 * GAP, 100.0, true);
    assert_eq!(l.step(2, 5, Nav::Down), 3);
    assert_eq!(l.step(3, 5, Nav::Down), 4);
    assert_eq!(l.step(4, 5, Nav::Up), 3);
}

#[test]
fn jumps_go_to_the_header_only_for_section_starts() {
    let (groups, l) = two_sections();
    // Item 5 starts the second section: its header.
    assert_eq!(l.jump_top(&groups, 5), l.tops[3]);
    assert_eq!(l.jump_top(&groups, 0), l.tops[0]);
    // Item 4 is in the middle of a section (a month inside a year): its own row.
    assert_eq!(l.jump_top(&groups, 4), l.tops[2]);
    assert_eq!(l.jump_top(&groups, 1), l.tops[1]);
}

#[test]
fn reload_keeps_the_photo_at_the_top_in_view() {
    let d = tempfile::tempdir().unwrap();
    let lib = crate::Library::open(
        crate::config::Paths::under(d.path()),
        Some(crate::model::registry::CLIP_B32.id),
    )
    .unwrap();
    let mut st = TimelineState::new(lib, &egui::Context::default());
    let items: Vec<_> = (0..20).map(|i| item(1_600_000_000 - i * 86_400)).collect();
    st.set_items(items.clone());
    let gen0 = st.generation;
    st.set_items(items.clone());
    assert_eq!(st.generation, gen0, "identical items keep the layout");
    // Scrolled to item 10; three newer photos arrive at the front.
    st.top_item = Some(10);
    let mut newer: Vec<_> = (1..=3).map(|i| item(1_600_000_000 + i * 86_400)).collect();
    newer.extend(items.clone());
    st.anchor_scroll(&newer);
    assert_eq!(st.pending, Some((13, false)));
    // The photo disappeared: the nearest position (clamped to the new end).
    st.pending = None;
    st.top_item = Some(19);
    st.anchor_scroll(&items[..5]);
    assert_eq!(st.pending, Some((4, false)));
    // At the very top nothing is anchored.
    st.pending = None;
    st.top_item = Some(0);
    st.anchor_scroll(&newer);
    assert_eq!(st.pending, None);
}

#[test]
fn reveal_scrolls_minimally() {
    assert_eq!(reveal(100.0, 50.0, 400.0, 0.0), None);
    assert_eq!(reveal(500.0, 50.0, 400.0, 0.0), Some(150.0));
    assert_eq!(reveal(100.0, 50.0, 400.0, 300.0), Some(100.0));
}

#[test]
fn on_this_day_lists_earlier_years_newest_first() {
    let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
    let items = vec![
        item(ts(2026, 7, 4, 9, 0, 0)),    // today, this year: not "earlier"
        item(ts(2019, 7, 4, 8, 0, 0)),    // 7 years ago
        item(ts(2019, 7, 5, 8, 0, 0)),    // next day
        item(ts(2020, 6, 4, 8, 0, 0)),    // other month
        item(ts(2024, 7, 4, 23, 59, 59)), // 2 years ago, late in the day
        item(ts(2025, 7, 4, 0, 0, 0)),    // 1 year ago, first second of the day
        item(ts(2019, 7, 4, 20, 0, 0)),   // same day as another 2019 photo
    ];
    let got = on_this_day(&items, today);
    assert_eq!(
        got,
        vec![
            OnThisDay {
                index: 5,
                years_ago: 1
            },
            OnThisDay {
                index: 4,
                years_ago: 2
            },
            OnThisDay {
                index: 6,
                years_ago: 7
            },
            OnThisDay {
                index: 1,
                years_ago: 7
            },
        ]
    );
    assert_eq!(years_ago_text(1), "1 year ago");
    assert_eq!(years_ago_text(7), "7 years ago");
    // A different day finds different photos; nothing matching gives an empty strip.
    let other = NaiveDate::from_ymd_opt(2026, 7, 5).unwrap();
    assert_eq!(on_this_day(&items, other).len(), 1);
    assert!(on_this_day(&items, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).is_empty());
    assert!(on_this_day(&[], today).is_empty());
}

#[test]
fn on_this_day_is_capped_and_ignores_the_list_order() {
    let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
    // 40 earlier years on the same day, oldest first.
    let items: Vec<_> = (1980..2020).map(|y| item(ts(y, 7, 4, 12, 0, 0))).collect();
    let got = on_this_day(&items, today);
    assert_eq!(got.len(), ON_THIS_DAY_MAX);
    assert_eq!(items[got[0].index].taken_at, ts(2019, 7, 4, 12, 0, 0));
    assert_eq!(got[0].years_ago, 7);
    assert_eq!(got.last().unwrap().years_ago, 7 + 29);
}

#[test]
fn leap_day_photos_show_on_28_february_in_years_without_one() {
    let leap = vec![
        item(ts(2020, 2, 29, 12, 0, 0)),
        item(ts(2021, 2, 28, 9, 0, 0)),
    ];
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    // 2026 has no 29 February: both photos belong to 28 February.
    let got = on_this_day(&leap, day(2026, 2, 28));
    assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [1, 0]);
    // 2028 does: 28 February is its own day, and 29 February matches only 29 February.
    let got = on_this_day(&leap, day(2028, 2, 28));
    assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [1]);
    let got = on_this_day(&leap, day(2028, 2, 29));
    assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [0]);
    assert!(on_this_day(&leap, day(2026, 3, 1)).is_empty());
}

#[test]
fn dates_taken_from_the_file_time_are_not_memories() {
    let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
    let mut copied = item(ts(2019, 7, 4, 12, 0, 0));
    copied.date_source = DateSource::File;
    let mut video = item(ts(2018, 7, 4, 12, 0, 0));
    video.date_source = DateSource::Video;
    let items = vec![copied, video, item(ts(2017, 7, 4, 12, 0, 0))];
    let got = on_this_day(&items, today);
    assert_eq!(got.iter().map(|e| e.index).collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn the_day_rolls_over_at_local_midnight() {
    let at = |h, m, s| {
        NaiveDate::from_ymd_opt(2026, 12, 31)
            .unwrap()
            .and_hms_opt(h, m, s)
            .unwrap()
    };
    assert_eq!(until_next_midnight(at(23, 59, 0)), Duration::from_secs(60));
    assert_eq!(
        until_next_midnight(at(0, 0, 0)),
        Duration::from_secs(86_400)
    );
}

#[test]
fn removing_a_file_keeps_the_selection_sane() {
    let mut tl_items: Vec<TimelineItem> = (0..5).map(|i| item(1_000_000 + i)).collect();
    let mut st = state_for_test();
    st.set_items(std::mem::take(&mut tl_items));
    st.grid.selected = Some(3);
    st.remove_file(1_000_001);
    assert_eq!(st.items.len(), 4);
    assert_eq!(st.grid.selected, Some(2), "still the same file");
    assert_eq!(st.items[2].file_id, 1_000_003);
    // Removing the selected one selects the next; at the end, the new last one.
    st.remove_file(1_000_003);
    assert_eq!(st.grid.selected, Some(2));
    assert_eq!(st.items[2].file_id, 1_000_004);
    st.remove_file(1_000_004);
    assert_eq!(st.grid.selected, Some(1));
    st.remove_file(1_000_000);
    st.remove_file(1_000_002);
    assert_eq!(st.grid.selected, None);
    assert!(st.items.is_empty());
    st.remove_file(42); // unknown: nothing happens
}

#[test]
fn removing_many_files_fixes_selection_and_scroll_once() {
    let mut st = state_for_test();
    let items: Vec<_> = (0..50_000)
        .map(|i| {
            let mut it = item(1_000_000 + i);
            it.file_id = i;
            it
        })
        .collect();
    st.set_items(items);
    // Every even file goes; the selection is file 11.
    let gone: HashSet<i64> = (0..50_000).filter(|i| i % 2 == 0).collect();
    st.grid.selected = Some(11);
    st.grid.sel.only(11);
    st.top_item = Some(1_001);
    let generation = st.generation;
    st.remove_files(&gone);
    assert_eq!(st.items.len(), 25_000);
    assert_eq!(
        st.generation,
        generation + 1,
        "one rebuild, not one per file"
    );
    assert!(st.items.iter().all(|it| it.file_id % 2 == 1));
    // File 11 is now at index 5; the file at the top (1001, odd) is at 500.
    assert_eq!(st.grid.selected, Some(5));
    assert!(st.grid.sel.contains(11));
    assert_eq!(st.pending, Some((500, false)));
    // Removing the primary with others selected hands it to the first remaining one.
    let mut st = state_for_test();
    st.set_items(
        (0..10)
            .map(|i| {
                let mut it = item(1_000_000 + i);
                it.file_id = i;
                it
            })
            .collect(),
    );
    st.grid.selected = Some(2);
    st.grid.sel.only(2);
    let ids = |v: &[i64]| v.iter().copied().collect::<HashSet<i64>>();
    st.grid.sel.click(
        Some(2),
        10,
        &|k| k as i64,
        6,
        selection::ClickMods {
            toggle: true,
            range: false,
        },
    );
    st.grid.sel.click(
        Some(6),
        10,
        &|k| k as i64,
        8,
        selection::ClickMods {
            toggle: true,
            range: false,
        },
    );
    st.grid.selected = Some(8);
    st.remove_files(&ids(&[8, 2, 0]));
    // Items 1,3,4,5,6,7,9 remain; 6 is the first selected one left, at index 4.
    assert_eq!(st.grid.selected, Some(4));
    assert_eq!(st.items[4].file_id, 6);
    assert_eq!(st.grid.sel.len(), 1);
}

/// A timeline state without a real library behind it.
fn state_for_test() -> TimelineState {
    crate::gpu::force_cpu(true);
    let d = tempfile::tempdir().unwrap();
    let lib = Library::open(
        crate::config::Paths::under(d.path()),
        Some(crate::model::registry::CLIP_B32.id),
    )
    .unwrap();
    TimelineState::new(lib, &egui::Context::default())
}

#[test]
fn the_heading_at_the_top_of_the_view_stays_pinned_until_the_next_one_pushes_it_up() {
    // Two days of 5 and 2 photos in rows of 3 (tiles of 100).
    let groups = [
        GroupSpan {
            key: 1,
            start: 0,
            end: 5,
        },
        GroupSpan {
            key: 2,
            start: 5,
            end: 7,
        },
    ];
    let l = Layout::build(&groups, 300.0 + 2.0 * GAP, 100.0, true);
    let second = l.tops[l.group_row[1]];
    // Nothing is pinned at the very top.
    assert_eq!(l.pinned_header(0.0), None);
    assert_eq!(l.pinned_header(-20.0), None);
    // Inside the first section: its heading sits at the top of the view.
    assert_eq!(l.pinned_header(HEADER_H + 30.0), Some((0, HEADER_H + 30.0)));
    // Just before the next heading arrives: still at the top.
    let before = second - HEADER_H - 1.0;
    assert_eq!(l.pinned_header(before), Some((0, before)));
    // As the next heading arrives, it pushes the pinned one up.
    let pushing = second - 10.0;
    assert_eq!(l.pinned_header(pushing), Some((0, second - HEADER_H)));
    // At the next heading, that section is pinned.
    assert_eq!(l.pinned_header(second), Some((1, second)));
    assert_eq!(l.pinned_header(second + 20.0), Some((1, second + 20.0)));
    // No headings, nothing pinned.
    let flat = Layout::build(&compute_groups(&[item(1)], false), 300.0, 100.0, false);
    assert_eq!(flat.pinned_header(50.0), None);
}

#[test]
fn the_ruler_of_a_huge_library_is_built_once_and_answers_by_binary_search() {
    use crate::ui::ruler::{Mapping, Month, TimeScale};
    // 200 000 photos over 20 years, newest first.
    let newest = ts(2024, 12, 31, 12, 0, 0);
    let items: Vec<TimelineItem> = (0..200_000_i64)
        .map(|i| item(newest - i * (20 * 365 * 86_400) / 200_000))
        .collect();
    let groups = compute_groups(&items, true);
    let layout = Layout::build(&groups, 900.0, 168.0, true);
    let sections = groups
        .iter()
        .enumerate()
        .filter_map(|(gi, g)| Some((Month::of_day_key(g.key)?, layout.tops[layout.group_row[gi]])));
    let scale = TimeScale::build(sections, layout.total).unwrap();
    assert!((235..=242).contains(&scale.months()), "{}", scale.months());
    let map = Mapping::Time(scale);
    let max = layout.total - 700.0;
    // Every point of the ruler maps into the scroll range, in order, and back again.
    let mut last = 0.0;
    for i in 0..=1000 {
        let f = i as f32 / 1000.0;
        let o = map.offset_at(f, max);
        assert!((last..=max).contains(&o), "{f}: {o}");
        last = o;
        if i < 1000 {
            let back = map.fraction_at(o, max);
            assert!((back - f).abs() < 0.01, "{f} -> {o} -> {back}");
        }
    }
}
