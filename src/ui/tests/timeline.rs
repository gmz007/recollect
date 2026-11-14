//! The timeline and album views.

use super::*;

#[test]
fn timeline_shows_month_headers_and_regroups() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("December 2019 · 1");
    h.get_by_label("240 photos");
    h.get_by_label("Thumbnail size");
    h.get_by_label("Sort: Newest first");
    // The "Months" menu regroups the timeline.
    h.get_by_label("Months").click();
    h.run();
    h.get_by_label("Years").click();
    h.run();
    h.get_by_label("2019 · 12");
    assert!(h.query_by_label("December 2019 · 1").is_none());
    h.get_by_label("Years").click();
    h.run();
    h.get_by_label("Days").click();
    h.run();
    h.get_by_label("Sunday 15 December 2019 · 1");
    // The sort menu says what it is set to and flips the order.
    h.get_by_label("Sort: Newest first").click();
    h.run_steps(8);
    h.get_by_label("Oldest first").click();
    h.run_steps(8);
    h.get_by_label("Sort: Oldest first");
    assert!(!app.borrow().tl.newest_first);
}

#[test]
fn navigator_jumps_to_a_year() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    assert!(h.query_by_label("December 2005 · 1").is_none());
    // The date list is folded away until asked for.
    assert!(h.query_by_label("2005 (12)").is_none());
    h.get_by_label("Jump to date").click();
    h.run();
    // The sidebar's date list is short in a window this size: scroll the year into view.
    h.get_by_label("2005 (12)").scroll_to_me();
    h.run();
    h.get_by_label("2005 (12)").click();
    h.run();
    h.run();
    h.get_by_label("December 2005 · 1");
    assert!(h.query_by_label("December 2019 · 1").is_none());
    // The year stays expanded and its months are listed.
    h.get_by_label("June (1)");
}

#[test]
fn clicking_a_folder_opens_its_view() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    assert_eq!(app.borrow().tl.view, View::Timeline);
    // Switching views loads items in the background and shows a spinner meanwhile, so step a
    // fixed number of frames instead of waiting for the UI to settle.
    h.get_by_label("nowhere").click();
    h.run_steps(3);
    assert_eq!(app.borrow().tl.view, View::Folder(1));
    h.get_by_label("Recently added").click();
    h.run_steps(3);
    assert_eq!(app.borrow().tl.view, View::RecentlyAdded);
    // No grouping choice for the recently added list.
    assert!(h.query_by_label("Months").is_none());
}

#[test]
fn banner_shows_while_the_timeline_stays_visible() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let h = album_harness(&app);
    h.get_by_label("Turn on smart search to find photos by describing them");
    h.get_by_label("Turn on");
    h.get_by_label("Not now");
    h.get_by_label("December 2019 · 1");
    assert_eq!(app.borrow().tl.group, Group::Month);
}

#[test]
fn clicking_a_tile_opens_the_viewer_with_the_details() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Item img1.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some());
    h.get_by_label("img1.jpg");
    h.get_by_label_contains("Sunday 15 December 2019");
    h.get_by_label("Test Camera");
    h.get_by_label("4000×3000");
    h.get_by_label("Find similar photos");
    h.get_by_label("Open with default app");
}

#[test]
fn only_the_circle_and_modifier_keys_select_instead_of_opening() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    h.get_by_label("Select img2.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().selected_file_ids(), [2]);
    h.get_by_label("1 selected");
    h.get_by_label("Cancel");
    // While picking, a plain click on another tile adds it instead of opening it.
    h.get_by_label("Item img4.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().selected_file_ids(), [2, 4]);
    h.get_by_label("2 selected");
    // Cancel leaves selection mode: the header comes back and a click opens again.
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(app.borrow().selected_file_ids().is_empty());
    assert!(h.query_by_label("2 selected").is_none());
    assert!(h.get_all_by_label("Photos").count() >= 1);
    h.get_by_label("Item img4.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some());
}

#[test]
fn on_this_day_strip_opens_the_viewer_on_its_own_list() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    // 15 March: one photo per year from 2000 to 2019 was taken on that day.
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let mut h = album_harness(&app);
    h.get_by_label("On this day");
    h.get_by_label("7 years ago");
    h.get_by_label("26 years ago");
    // Newest first: 2019 comes first. March 2019 is img10 in `spread_items` (Dec = 1).
    h.get_by_label("On this day img10.jpg").click();
    h.run_steps(3);
    h.get_by_label("img10.jpg");
    h.get_by_label("1 of 20");
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("2 of 20");
    h.get_by_label("img22.jpg"); // March 2018
    // Not in the other views.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.get_by_label("Recently added").click();
    h.run_steps(3);
    assert!(h.query_by_label("On this day").is_none());
}

#[test]
fn on_this_day_tiles_open_and_have_no_selection_circle() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let h = album_harness(&app);
    h.get_by_label("On this day img10.jpg");
    // img10 is only on screen in the strip (the grid starts at the newest photos).
    assert!(h.query_by_label("Select img10.jpg").is_none());
    assert!(h.query_by_label("Select On this day img10.jpg").is_none());
}
