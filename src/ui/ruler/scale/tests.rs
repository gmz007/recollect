use super::*;

fn m(year: i32, month: u32) -> Month {
    Month { year, month }
}

/// Four months, Nov 2024 down to Aug 2024, with September empty. Sections start at 0, 100 and
/// 300, the content ends at 400.
fn scale() -> TimeScale {
    TimeScale::build(
        [
            (m(2024, 11), 20.0),
            (m(2024, 10), 100.0),
            (m(2024, 8), 300.0),
        ],
        400.0,
    )
    .unwrap()
}

#[test]
fn every_month_in_the_range_gets_a_slot() {
    let s = scale();
    assert_eq!(s.months(), 4);
    assert_eq!(s.month_at(0.0), m(2024, 11));
    assert_eq!(s.month_at(0.3), m(2024, 10));
    assert_eq!(s.month_at(0.6), m(2024, 9));
    assert_eq!(s.month_at(1.0), m(2024, 8));
}

#[test]
fn the_top_is_the_top_of_the_page_and_the_bottom_the_end() {
    let map = Mapping::Time(scale());
    assert_eq!(map.offset_at(0.0, 250.0), 0.0);
    assert_eq!(map.offset_at(1.0, 250.0), 250.0);
    assert_eq!(map.fraction_at(0.0, 250.0), 0.0);
    assert_eq!(map.fraction_at(250.0, 250.0), 1.0);
}

#[test]
fn the_position_in_a_month_maps_to_the_position_in_its_photos() {
    let map = Mapping::Time(scale());
    // Halfway through October (slot 1 of 4: 0.25..0.5) is halfway through its 100..300.
    assert!((map.offset_at(0.375, 1000.0) - 200.0).abs() < 1e-3);
    assert!((map.fraction_at(200.0, 1000.0) - 0.375).abs() < 1e-3);
    // The first month spans the top of the page to where October starts.
    assert!((map.offset_at(0.125, 1000.0) - 50.0).abs() < 1e-3);
}

#[test]
fn an_empty_month_has_no_height_to_scroll() {
    let map = Mapping::Time(scale());
    // September is empty: its whole slot stays at the offset where August starts.
    assert!((map.offset_at(0.5, 1000.0) - 300.0).abs() < 1e-3);
    assert!((map.offset_at(0.7, 1000.0) - 300.0).abs() < 1e-3);
    // Scrolled into August the marker is in August's slot.
    assert!(map.fraction_at(310.0, 1000.0) >= 0.75);
}

#[test]
fn offsets_never_pass_what_the_content_can_scroll() {
    let map = Mapping::Time(scale());
    for i in 0..=20 {
        assert!(map.offset_at(i as f32 / 20.0, 150.0) <= 150.0);
    }
    // Nothing to scroll: everything maps to the top.
    assert_eq!(map.offset_at(0.9, 0.0), 0.0);
}

#[test]
fn the_mapping_is_monotonic_both_ways() {
    let map = Mapping::Time(scale());
    let mut last = -1.0;
    for i in 0..=100 {
        let o = map.offset_at(i as f32 / 100.0, 1000.0);
        assert!(o >= last);
        last = o;
    }
    let mut last = -1.0;
    for o in 0..=400 {
        let f = map.fraction_at(o as f32, 1000.0);
        assert!(f >= last);
        last = f;
    }
}

#[test]
fn a_library_of_one_month_is_one_slot() {
    let s = TimeScale::build([(m(2020, 2), 0.0)], 900.0).unwrap();
    assert_eq!(s.months(), 1);
    let map = Mapping::Time(s);
    let Mapping::Time(t) = &map else {
        unreachable!()
    };
    assert_eq!(t.month_at(0.8), m(2020, 2));
    assert!((map.offset_at(0.5, 600.0) - 450.0).abs() < 1e-3);
    assert!((map.fraction_at(300.0, 600.0) - 0.333).abs() < 0.01);
    assert!(TimeScale::build([], 10.0).is_none());
}

#[test]
fn a_flat_ruler_follows_the_scroll_fraction() {
    let map = Mapping::Flat;
    assert_eq!(map.offset_at(0.5, 800.0), 400.0);
    assert_eq!(map.offset_at(1.5, 800.0), 800.0);
    assert_eq!(map.fraction_at(200.0, 800.0), 0.25);
    assert_eq!(map.fraction_at(5.0, 0.0), 0.0);
}

#[test]
fn months_roll_over_year_ends() {
    let s = TimeScale::build([(m(2025, 1), 0.0), (m(2024, 11), 50.0)], 100.0).unwrap();
    assert_eq!(s.months(), 3);
    assert_eq!(s.month_at(0.4), m(2024, 12));
    assert_eq!(m(2024, 3).name(), "March 2024");
}

#[test]
fn dates_come_from_timestamps_and_day_keys() {
    assert_eq!(Month::of_timestamp(0), None);
    assert_eq!(Month::of_timestamp(1_710_000_000), Some(m(2024, 3)));
    let key = i64::from(
        NaiveDate::from_ymd_opt(2019, 12, 15)
            .unwrap()
            .num_days_from_ce(),
    );
    assert_eq!(Month::of_day_key(key), Some(m(2019, 12)));
}

fn years(first: i32, last: i32) -> TimeScale {
    let sections: Vec<(Month, f32)> = (last..=first)
        .rev()
        .flat_map(|y| (1..=12).rev().map(move |mo| (m(y, mo), 0.0)))
        .enumerate()
        .map(|(i, (mo, _))| (mo, i as f32 * 10.0))
        .collect();
    let total = sections.len() as f32 * 10.0;
    TimeScale::build(sections, total).unwrap()
}

#[test]
fn years_get_a_long_dash_and_a_label_when_there_is_room() {
    let ds = dashes(&Mapping::Time(years(2024, 2015)), 700.0);
    let labelled: Vec<i32> = ds.iter().filter_map(|d| d.year).collect();
    assert_eq!(labelled.first(), Some(&2024));
    assert_eq!(labelled.len(), 10);
    assert!(ds.iter().filter(|d| d.size == DashSize::Year).count() >= 10);
    assert!(ds.windows(2).all(|w| w[0].frac < w[1].frac));
    // Many years: only the years are keyboard stops.
    assert!(
        ds.iter()
            .filter_map(|d| d.stop.as_ref())
            .all(|s| s.label.starts_with("Jump to 20") && s.label.len() == "Jump to 2024".len())
    );
}

#[test]
fn few_years_make_every_month_a_stop() {
    let ds = dashes(&Mapping::Time(years(2024, 2023)), 700.0);
    let stops: Vec<&str> = ds
        .iter()
        .filter_map(|d| d.stop.as_ref().map(|s| s.label.as_str()))
        .collect();
    assert_eq!(stops.len(), 24);
    assert_eq!(stops[0], "Jump to December 2024");
}

#[test]
fn a_huge_range_thins_out_instead_of_crowding() {
    let ds = dashes(&Mapping::Time(years(2024, 1900)), 700.0);
    let spacing = |w: &[Dash]| (w[1].frac - w[0].frac) * 700.0;
    // Month dashes are spaced out; only the (rare) year dashes may sit closer.
    assert!(ds.len() < 700);
    assert!(ds.windows(2).filter(|w| spacing(w) < 1.0).count() < ds.len() / 10);
    let labels: Vec<f32> = ds
        .iter()
        .filter(|d| d.year.is_some())
        .map(|d| d.frac * 700.0)
        .collect();
    assert!(labels.windows(2).all(|w| w[1] - w[0] >= 14.0 - 1e-3));
}

#[test]
fn a_flat_ruler_has_evenly_spaced_dashes() {
    let ds = dashes(&Mapping::Flat, 700.0);
    assert_eq!(ds.len(), 96);
    assert!(ds.iter().all(|d| d.year.is_none()));
    assert_eq!(ds[0].size, DashSize::Month);
    assert_eq!(ds[1].size, DashSize::Minor);
    let stops = ds.iter().filter(|d| d.stop.is_some()).count();
    assert!((1..=12).contains(&stops));
    // A short ruler gets fewer dashes.
    assert_eq!(dashes(&Mapping::Flat, 40.0).len(), 11);
    assert_eq!(dashes(&Mapping::Flat, 1.0).len(), 1);
}

#[test]
fn the_nearest_dash_is_found_by_position() {
    let ds = dashes(&Mapping::Flat, 700.0);
    assert_eq!(nearest(&ds, 0.0), 0);
    assert_eq!(nearest(&ds, 1.0), 95);
    assert_eq!(nearest(&ds, 0.49), 47);
    assert_eq!(nearest(&ds[..1], 0.9), 0);
}
