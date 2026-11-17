//! The photo viewer.

use super::*;

#[test]
fn enter_opens_the_viewer_and_arrows_and_escape_navigate() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    // Any arrow picks the first item; Enter opens it.
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    // The viewer replaces the album: no grid, no sidebar.
    h.get_by_label_contains("1 of 240");
    h.get_by_label("Back to Photos");
    h.get_by_label("Next");
    h.get_by_label("Info");
    assert!(
        h.query_by_label("Details").is_none(),
        "the panel starts closed"
    );
    assert!(
        h.query_by_label("Previous").is_none(),
        "nothing before the first item"
    );
    assert!(h.query_by_label("Item img1.jpg").is_none());
    assert!(h.query_by_label("Recently added").is_none());
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label_contains("2 of 240");
    h.get_by_label("Previous");
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(3);
    h.get_by_label_contains("1 of 240");
    h.key_press(egui::Key::End);
    h.run_steps(3);
    h.get_by_label_contains("240 of 240");
    assert!(h.query_by_label("Next").is_none());
    h.key_press(egui::Key::Home);
    h.run_steps(3);
    h.get_by_label_contains("1 of 240");
    h.get_by_label("Next").click();
    h.run_steps(3);
    h.get_by_label_contains("2 of 240");
    // Esc returns to the grid with the item selected, even with the details open.
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Details");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().tl.grid.selected, Some(1));
    h.get_by_label("Item img2.jpg");
    h.get_by_label("Recently added");
}

#[test]
fn clicking_opens_the_viewer_and_the_info_panel_toggles() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    // The photo opens edge to edge; Info shows the details.
    assert!(h.query_by_label("Details").is_none());
    assert!(h.query_by_label("Find similar photos").is_none());
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Details");
    h.get_by_label("img1.jpg");
    assert_eq!(
        h.get_all_by_label_contains("Sunday 15 December 2019")
            .count(),
        2
    );
    h.get_by_label("Test Camera");
    h.get_by_label_contains("4000×3000");
    h.get_by_label("Show in folder");
    h.get_by_label("Find similar photos");
    h.get_by_label("Not in any album");
    // The bar shows the day, then the time and the position.
    h.get_by_label("Sunday 15 December 2019");
    h.get_by_label_contains("1 of 240");
    // `I` and the Info button hide and show the panel.
    h.key_press(egui::Key::I);
    h.run_steps(3);
    assert!(h.query_by_label("Find similar photos").is_none());
    assert!(h.query_by_label("Details").is_none());
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Find similar photos");
    // Back button.
    h.get_by_label("Back to Photos").click();
    h.run_steps(3);
    assert_eq!(app.borrow().tl.grid.selected, Some(0));
}

#[test]
fn the_heart_marks_a_favorite_and_the_favorites_view_lists_it() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    // The heart in the corner of a tile toggles its favorite mark without opening it.
    h.get_by_label("Add b.jpg to favorites").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    h.get_by_label("Remove b.jpg from favorites");
    assert_eq!(shown_favorites(&app), 1);
    // ...and the mark is stored.
    let fav = crate::store::db::TimelineFilter {
        favorites_only: true,
        ..Default::default()
    };
    settle(&mut h, || lib.timeline(&fav).unwrap().len() == 1);
    h.get_by_label("Favorites").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Favorites && a.tl.items.len() == 1
    });
    h.get_by_label("Item b.jpg");
    assert!(h.query_by_label("Item a.jpg").is_none());
    // The heart again removes the mark; the item leaves the view right away.
    h.get_by_label("Remove b.jpg from favorites").click();
    h.run_steps(3);
    assert!(h.query_by_label("Item b.jpg").is_none());
    // Once the write is in, the Favorites view is read again; wait for that reload too.
    settle(&mut h, || {
        let a = app.borrow();
        lib.timeline(&fav).unwrap().is_empty()
            && a.fav_version_seen == lib.favorites_version()
            && a.tl.awaiting.is_none()
    });
    h.run_steps(2);
    h.get_by_label("Click the heart on any photo to add it here.");
    h.get_by_label("Go to Photos");
    assert_eq!(app.borrow().tl.grid.selected, None);
    settle(&mut h, || lib.timeline(&fav).unwrap().is_empty());
    // ...and it stays gone after the view is reloaded.
    settle(&mut h, || app.borrow().fav_marks.is_empty());
    assert!(app.borrow().tl.items.is_empty());
}

/// How many items the grid shows as favorites: a mark not yet saved wins over the flag of the
/// loaded list, as on screen. (A list loaded while a write is on its way still has the old flag
/// until the write is confirmed.)
fn shown_favorites(app: &Rc<RefCell<App>>) -> usize {
    let a = app.borrow();
    a.tl.items
        .iter()
        .filter(|it| crate::ui::marked(&a.fav_marks, it.file_id, it.favorite))
        .count()
}

#[test]
fn f_marks_the_keyboard_selection_as_a_favorite() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.key_press(egui::Key::F);
    h.run_steps(3);
    assert_eq!(shown_favorites(&app), 1);
    h.key_press(egui::Key::F);
    h.run_steps(3);
    assert_eq!(shown_favorites(&app), 0);
    // Once both writes are in, the library and the list itself agree.
    let favorites = crate::store::db::TimelineFilter {
        favorites_only: true,
        ..Default::default()
    };
    settle(&mut h, || {
        lib.timeline(&favorites).unwrap().is_empty()
            && app.borrow().fav_marks.is_empty()
            && app.borrow().tl.items.iter().all(|it| !it.favorite)
    });
}

#[test]
fn opening_the_viewer_on_a_huge_album_copies_nothing() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut a = app.borrow_mut();
    let mut items = Vec::with_capacity(200_000);
    for i in 0..200_000 {
        let mut it = titem(i + 1, 2020, 1, 1);
        it.camera = None;
        items.push(it);
    }
    a.tl.set_items(items);
    assert_eq!(Arc::strong_count(&a.tl.items), 1);
    a.open_viewer_timeline(150_000);
    assert!(a.viewer.is_some());
    assert_eq!(
        Arc::strong_count(&a.tl.items),
        2,
        "the viewer shares the list"
    );
    // A favorite mark meanwhile does not copy it either.
    a.set_favorite(5, true);
    assert_eq!(Arc::strong_count(&a.tl.items), 2);
    a.close_viewer();
    assert_eq!(Arc::strong_count(&a.tl.items), 1);
}

#[test]
fn notices_show_inside_the_viewer() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    assert!(h.query_by_label("No video player found").is_none());
    app.borrow_mut().notify("No video player found");
    h.run_steps(3);
    h.get_by_label("No video player found");
}

#[test]
fn closing_the_viewer_frees_its_previews() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(5);
    assert!(
        app.borrow().previews.cached() > 0,
        "previews were requested"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().previews.cached(), 0);
}

#[test]
fn find_similar_from_the_viewer_closes_it_like_any_other_close() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(5);
    assert!(app.borrow().previews.cached() > 0);
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Find similar photos").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().previews.cached(), 0);
    assert!(app.borrow().searching());
}

#[test]
fn the_viewer_bar_has_back_and_exactly_four_icon_buttons() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    let mut names: Vec<String> = h
        .get_all_by_role(egui::accesskit::Role::Button)
        .filter(|n| n.rect().max.y <= 53.0)
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Add to album…",
            "Back to Photos",
            "Favorite",
            "Info",
            "More"
        ]
    );
    // The old labelled buttons are gone from the bar.
    for gone in ["Open with default app", "Zoom to 100%", "Play at 0:00"] {
        assert!(h.query_by_label(gone).is_none(), "{gone}");
    }
    // Favorite is filled once marked, and the button says what it would undo.
    h.get_by_label("Favorite").click();
    h.run_steps(3);
    h.get_by_label("Unfavorite");
    // F toggles it too.
    h.key_press(egui::Key::F);
    h.run_steps(3);
    h.get_by_label("Favorite");
    // Back closes the viewer and returns to the grid.
    h.get_by_label("Back to Photos").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    h.get_by_label("Item img1.jpg");
}

#[test]
fn the_more_menu_opens_files_and_toggles_the_zoom() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("More").click();
    h.run_steps(3);
    h.get_by_label("Open with default app");
    h.get_all_by_label("Show in folder").last().unwrap();
    h.get_by_label_contains("Zoom to 100%").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Open with default app").is_none(),
        "closed"
    );
    h.get_by_label("More").click();
    h.run_steps(3);
    h.get_by_label_contains("Fit to window").click();
    h.run_steps(3);
    h.get_by_label("More").click();
    h.run_steps(3);
    h.get_by_label_contains("Zoom to 100%");
    // Z does the same without the menu.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.key_press(egui::Key::Z);
    h.run_steps(3);
    h.get_by_label("More").click();
    h.run_steps(3);
    h.get_by_label_contains("Fit to window");
}

#[test]
fn clicking_a_filmstrip_thumbnail_goes_to_that_photo() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label_contains("1 of 240");
    h.get_by_label("Go to img3.jpg").click();
    h.run_steps(3);
    h.get_by_label_contains("3 of 240");
}

#[test]
fn the_filmstrip_fades_when_the_pointer_rests_and_returns_on_movement() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("Go to img2.jpg");
    // 2 s and a little more (frames are 50 ms) without the pointer moving.
    h.run_steps(60);
    assert!(h.query_by_label("Go to img2.jpg").is_none(), "faded away");
    h.event(egui::Event::PointerMoved(egui::pos2(300.0, 300.0)));
    h.run_steps(2);
    h.get_by_label("Go to img2.jpg");
}

#[test]
fn the_info_panel_names_the_folder_and_offers_to_show_it() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("Info").click();
    h.run_steps(3);
    // The folder's name, not its whole path.
    h.get_by_label("nowhere");
    assert!(h.query_by_label("/nowhere").is_none());
    h.get_by_label("Show in folder");
    h.get_by_label("Find similar photos");
}

#[test]
fn escape_closes_an_open_viewer_menu_but_not_the_viewer() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("New album…");
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    h.get_by_label_contains("1 of 240");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some(), "the viewer stayed open");
    assert!(h.query_by_label("New album…").is_none(), "the menu closed");
    // With the menu gone, Escape leaves the viewer as before.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
}

#[test]
fn a_narrow_window_keeps_the_four_buttons_clear_of_the_date() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness_wide(&app, 640.0);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    let day = h.get_by_label("Sunday 15 December 2019").rect();
    for label in ["Favorite", "Add to album…", "Info", "More"] {
        let r = h.get_by_label(label).rect();
        assert!(
            !r.intersects(day),
            "{label} {r:?} overlaps the date {day:?}"
        );
    }
    h.get_by_label("Back to Photos");
}

#[test]
fn the_details_panel_stays_as_it_was_left_when_the_viewer_opens_again() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    assert!(h.query_by_label("Details").is_none(), "closed at first");
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Details");
    h.get_by_label("Back to Photos").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    // Opened again, the panel is still open.
    open_tile(&mut h, "Item img2.jpg");
    h.run_steps(3);
    h.get_by_label("Details");
}

#[test]
fn escape_closes_the_viewer_at_once_whether_or_not_the_details_are_open() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    assert!(h.query_by_label("Details").is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    // With the details open, one Esc still goes back.
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Details");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
}

#[test]
fn the_filmstrip_and_arrows_stay_away_while_a_dialog_is_open() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    assert!(h.query_all_by_label_contains("Go to").count() > 0);
    h.get_by_label("Next");
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("New album…").click();
    h.run_steps(5);
    h.get_by_label("Album name");
    assert_eq!(
        h.query_all_by_label_contains("Go to").count(),
        0,
        "the filmstrip would draw over the dialog"
    );
    assert!(h.query_by_label("Next").is_none());
}
