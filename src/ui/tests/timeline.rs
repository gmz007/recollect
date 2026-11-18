//! The timeline and album views.

use super::*;

#[test]
fn the_timeline_has_day_headers_and_no_view_header() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = album_harness(&app);
    h.get_by_label("Sunday 15 December 2019");
    // The title row with its count and the size, sort and grouping controls are gone.
    assert!(h.query_by_label("240 photos").is_none());
    assert!(h.query_by_label("Thumbnail size").is_none());
    assert!(h.query_by_label("Sort: Newest first").is_none());
    assert!(h.query_by_label("Months").is_none());
}

#[test]
fn the_top_bar_holds_only_help_and_settings_beside_the_search_field() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = album_harness(&app);
    h.get_by_label("Help");
    h.get_by_label("Settings");
    // The type filter lives with the search results, not in the bar.
    assert!(h.query_by_label("Everything").is_none());
    assert!(h.query_by_label("Photos only").is_none());
}

#[test]
fn the_sidebar_has_no_date_list() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = album_harness(&app);
    assert!(h.query_by_label("Jump to date").is_none());
    h.get_by_label("Recently added");
    h.get_by_label("Favorites");
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
    h.get_by_label("Sunday 15 December 2019");
}

#[test]
fn clicking_a_tile_opens_the_viewer_with_the_details() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Item img1.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some());
    assert!(h.query_by_label("Details").is_none(), "closed at first");
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("img1.jpg");
    assert_eq!(
        h.get_all_by_label_contains("Sunday 15 December 2019")
            .count(),
        2
    );
    h.get_by_label("Test Camera");
    h.get_by_label_contains("4000×3000");
    h.get_by_label("Find similar photos");
    h.get_by_label("More");
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
fn the_on_this_day_banner_opens_the_viewer_on_its_own_list() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    // 15 March: one photo per year from 2000 to 2019 was taken on that day.
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let mut h = album_harness(&app);
    h.get_by_label("On this day");
    // The newest of them is from 2019.
    h.get_by_label("15 March over the years");
    h.get_by_label("See 20 photos").click();
    h.run_steps(3);
    // March 2019 is img10 in `spread_items` (Dec = 1).
    h.get_by_label_contains("1 of 20");
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("img10.jpg");
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label_contains("2 of 20");
    h.get_by_label("img22.jpg"); // March 2018
    // Not in the other views. One Esc leaves the viewer, details open or not.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.get_by_label("Recently added").click();
    h.run_steps(3);
    assert!(h.query_by_label("On this day").is_none());
}

#[test]
fn the_on_this_day_banner_can_be_hidden_until_tomorrow() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let mut h = album_harness(&app);
    h.get_by_label("On this day");
    h.get_by_label("Hide On this day").click();
    h.run_steps(3);
    assert!(h.query_by_label("On this day").is_none());
    let lib = Arc::clone(&app.borrow().lib);
    assert_eq!(
        lib.settings().on_this_day_hidden_on.as_deref(),
        Some("2026-03-15")
    );
    // The next day, it is back (there are photos from the 16th too: none, so pick another day).
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 4, 15);
    h.run_steps(3);
    h.get_by_label("On this day");
}

#[test]
fn there_is_no_on_this_day_banner_without_photos_from_that_day() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    // Photos were taken on the 15th of each month only.
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 16);
    let h = album_harness(&app);
    assert!(h.query_by_label("On this day").is_none());
    assert!(h.query_by_label_contains("See ").is_none());
}

#[test]
fn the_on_this_day_banner_names_one_year_when_all_photos_are_from_it() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut items = spread_items();
    // Only 2019 has a photo on 15 March; the rest are moved off that day.
    for it in &mut items {
        if it.taken_at < titem(0, 2019, 1, 1).taken_at {
            it.taken_at += 86_400;
        }
    }
    app.borrow_mut().inject_timeline(items);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let mut h = album_harness(&app);
    h.get_by_label("15 March, 7 years ago");
    h.get_by_label("See 1 photo").click();
    h.run_steps(3);
    h.get_by_label_contains("1 of 1");
}

#[test]
fn the_on_this_day_count_is_what_the_viewer_opens() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    // More photos of one day than the old limit of 30, all in one year.
    let items: Vec<_> = (0..45)
        .map(|n| {
            let mut it = titem(n + 1, 2019, 3, 15);
            it.taken_at += n;
            it
        })
        .collect();
    app.borrow_mut().inject_timeline(items);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let mut h = album_harness(&app);
    h.get_by_label("15 March, 7 years ago");
    h.get_by_label("See 45 photos").click();
    h.run_steps(3);
    h.get_by_label_contains("1 of 45");
}

#[test]
fn photos_shows_that_it_is_loading_while_a_load_is_pending() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    {
        let mut a = app.borrow_mut();
        a.tl.view = View::Favorites;
        a.inject_timeline(Vec::new());
        // A load is on its way (the answer never comes in this test).
        let filter = a.tl.filter();
        a.tl.awaiting = Some((u64::MAX, filter));
    }
    // The spinner never rests, so the frames are stepped by hand.
    let shown = Rc::clone(&app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| shown.borrow_mut().show(ui));
    h.run_steps(3);
    h.get_by_label("Loading…");
    // Once the load is over, it gives way to the empty state.
    app.borrow_mut().tl.awaiting = None;
    h.run_steps(3);
    assert!(h.query_by_label("Loading…").is_none());
}
