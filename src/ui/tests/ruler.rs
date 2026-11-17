//! The timeline ruler: the scroll bar of the grids.

use super::*;
use crate::ui::ruler::Month;

/// A point on the ruler column of a 1200x800 window, `y` points from the top.
fn on_ruler(y: f32) -> egui::Pos2 {
    egui::pos2(1200.0 - 20.0, y)
}

/// Where the ruler's first and last dash are (the ruler runs under the top bar).
const NEAR_TOP: f32 = 62.0;
const NEAR_BOTTOM: f32 = 790.0;

fn tl_offset(app: &Rc<RefCell<App>>) -> f32 {
    app.borrow().tl.grid.ruler.offset()
}

fn press(h: &mut Harness<'_>, y: f32) {
    h.hover_at(on_ruler(y));
    h.run_steps(2);
    h.drag_at(on_ruler(y));
    h.run_steps(3);
}

#[test]
fn pressing_the_ruler_scrolls_the_photos_to_that_time() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    assert_eq!(tl_offset(&app), 0.0);
    press(&mut h, 440.0);
    settle(&mut h, || tl_offset(&app) > 1000.0);
    let middle = tl_offset(&app);
    // The top item is somewhere in the middle of the library, not at either end.
    let top = app.borrow().tl.top_item.expect("a photo is at the top");
    assert!((60..180).contains(&top), "{top}");
    // Dragging further down scrubs further.
    h.hover_at(on_ruler(700.0));
    settle(&mut h, || tl_offset(&app) > middle + 1000.0);
    // Letting go leaves the grid where it is.
    h.drop_at(on_ruler(700.0));
    h.run_steps(3);
    let stopped = tl_offset(&app);
    h.run_steps(3);
    assert_eq!(tl_offset(&app), stopped);
}

#[test]
fn the_top_of_the_ruler_is_the_top_of_the_page_and_the_bottom_the_end() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    press(&mut h, NEAR_BOTTOM);
    h.run_steps(5);
    settle(&mut h, || app.borrow().tl.top_item.is_some_and(|t| t > 230));
    h.drop_at(on_ruler(NEAR_BOTTOM));
    press(&mut h, NEAR_TOP);
    h.run_steps(5);
    settle(&mut h, || tl_offset(&app) < 100.0);
    assert!(app.borrow().tl.top_item.is_none_or(|t| t < 3));
}

#[test]
fn a_year_dash_can_be_reached_with_the_keyboard() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Jump to 2019");
    h.get_by_label("Jump to 2005").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    // December 2005 is item 168 of the 240 (one photo a month, newest first) and starts there.
    settle(&mut h, || app.borrow().tl.top_item == Some(168));
}

#[test]
fn with_few_years_every_month_is_a_stop() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut items = spread_items();
    items.truncate(30);
    app.borrow_mut().tl.tile_size = 320.0;
    app.borrow_mut().inject_timeline(items);
    let h = album_harness(&app);
    h.get_by_label("Jump to December 2019");
    h.get_by_label("Jump to July 2017");
    assert!(h.query_by_label("Jump to 2019").is_none());
}

#[test]
fn the_pointer_on_the_ruler_names_the_date() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.hover_at(on_ruler(NEAR_TOP));
    h.run_steps(3);
    h.get_by_label("December 2019");
    h.hover_at(on_ruler(NEAR_BOTTOM + 5.0));
    h.run_steps(3);
    h.get_by_label("January 2000");
}

#[test]
fn everything_fitting_in_the_window_needs_no_ruler() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut items = spread_items();
    items.truncate(1);
    app.borrow_mut().inject_timeline(items);
    let mut h = album_harness(&app);
    h.run_steps(3);
    assert!(h.query_by_label_contains("Jump to ").is_none());
    assert!(app.borrow().tl.grid.ruler.fits());
}

#[test]
fn albums_are_a_flat_grid_that_scrolls_by_position() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    {
        let mut a = app.borrow_mut();
        a.tl.view = View::Album(1);
        a.inject_timeline(spread_items());
    }
    let mut h = album_harness(&app);
    // No day headings, but the photos are there.
    assert!(h.query_by_label("Sunday 15 December 2019").is_none());
    h.get_by_label("Item img1.jpg");
    // The ruler follows the scroll position: its dashes are labelled by percent.
    h.get_by_label("Scroll to 0%");
    press(&mut h, NEAR_BOTTOM + 5.0);
    settle(&mut h, || tl_offset(&app) > 100.0);
    let at_end = tl_offset(&app);
    assert!(at_end > 1000.0);
    h.drop_at(on_ruler(NEAR_BOTTOM));
    h.run_steps(3);
    assert_eq!(tl_offset(&app), at_end);
    // The photo at a position gives the date: the end of the list is January 2000.
    let date = app.borrow_mut().tl.with_ruler(|_, date_at| date_at(1.0));
    assert_eq!(date.map(Month::name).as_deref(), Some("January 2000"));
}

#[test]
fn the_other_views_with_days_keep_their_headings() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().tl.view = View::Favorites;
    app.borrow_mut().inject_timeline(spread_items());
    let h = album_harness(&app);
    h.get_by_label("Sunday 15 December 2019");
    h.get_by_label("Jump to 2019");
}

#[test]
fn recently_added_scrolls_by_position() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().tl.view = View::RecentlyAdded;
    app.borrow_mut().inject_timeline(spread_items());
    let h = album_harness(&app);
    h.get_by_label("Scroll to 0%");
    assert!(h.query_by_label("Jump to 2019").is_none());
}

/// A search with `n` results, one per day of 2020.
fn many_results(d: &std::path::Path, n: usize) -> Harness<'static> {
    let mut app = grid_app(d);
    app.res.hits = Arc::new(
        (0..n)
            .map(|i| {
                let mut h = hit(&format!("/nowhere/{i}.jpg"), MediaKind::Image, None);
                h.file_id = i as i64 + 1;
                h.taken_at = titem(0, 2020, 1, 1).taken_at + i as i64 * 86_400;
                h
            })
            .collect(),
    );
    grid_harness(app)
}

#[test]
fn search_results_have_a_narrow_ruler_that_follows_the_scroll_position() {
    let d = tempfile::tempdir().unwrap();
    let mut h = many_results(d.path(), 300);
    h.get_by_label("Scroll to 0%");
    // 40 points wide, so a press in the last 40 points reaches it.
    let x = 1200.0 - 15.0;
    h.hover_at(egui::pos2(x, NEAR_BOTTOM + 5.0));
    h.run_steps(2);
    h.drag_at(egui::pos2(x, NEAR_BOTTOM + 5.0));
    h.run_steps(5);
    // The last result is in view.
    h.get_by_label("Result 299.jpg");
}

#[test]
fn a_short_list_of_results_has_no_ruler() {
    let d = tempfile::tempdir().unwrap();
    let h = many_results(d.path(), 3);
    assert!(h.query_by_label("Scroll to 0%").is_none());
}

#[test]
fn the_banner_asks_for_a_large_preview() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let h = album_harness(&app);
    h.get_by_label("On this day");
    let sides = app.borrow().previews.requested_sides();
    assert!(
        sides.iter().any(|&s| s >= 1024),
        "no large preview was asked for: {sides:?}"
    );
}
