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
    h.get_by_label("img1.jpg");
    h.get_by_label("1 of 240");
    h.get_by_label("Back");
    h.get_by_label("Next");
    h.get_by_label("Info");
    assert!(
        h.query_by_label("Previous").is_none(),
        "nothing before the first item"
    );
    assert!(h.query_by_label("Item img1.jpg").is_none());
    assert!(h.query_by_label("Recently added").is_none());
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("img2.jpg");
    h.get_by_label("Previous");
    assert!(h.query_by_label("img1.jpg").is_none());
    h.key_press(egui::Key::End);
    h.run_steps(3);
    h.get_by_label("240 of 240");
    assert!(h.query_by_label("Next").is_none());
    h.key_press(egui::Key::Home);
    h.run_steps(3);
    h.get_by_label("1 of 240");
    h.get_by_label("Next").click();
    h.run_steps(3);
    h.get_by_label("img2.jpg");
    // Esc returns to the grid with the item selected.
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
    h.get_by_label("img1.jpg");
    h.get_by_label_contains("Sunday 15 December 2019");
    h.get_by_label("Test Camera");
    h.get_by_label("4000×3000");
    h.get_by_label("Open with default app");
    h.get_by_label("Show in folder");
    h.get_by_label("Find similar photos");
    h.get_by_label("Favorite");
    // `I` hides the panel (the name moves to the top bar) and shows it again; the buttons stay.
    h.key_press(egui::Key::I);
    h.run_steps(3);
    h.get_by_label("img1.jpg");
    assert!(h.query_by_label("Find similar photos").is_none());
    h.get_by_label("Open with default app");
    h.get_by_label("Favorite");
    h.key_press(egui::Key::I);
    h.run_steps(3);
    h.get_by_label("Find similar photos");
    // Zoom toggles with Z and with the button.
    h.get_by_label("Zoom to 100%");
    h.key_press(egui::Key::Z);
    h.run_steps(3);
    h.get_by_label("Fit to window").click();
    h.run_steps(3);
    h.get_by_label("Zoom to 100%");
    // Back button.
    h.get_by_label("Back").click();
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
    h.get_by_label("1 photo");
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
    h.get_by_label("Find similar photos").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().previews.cached(), 0);
    assert!(app.borrow().searching());
}

#[test]
fn the_viewer_toolbar_has_labelled_icon_buttons() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    for label in [
        "Back",
        "Favorite",
        "Add to album…",
        "Info",
        "Open with default app",
        "Show in folder",
        "Zoom to 100%",
        "1 of 240",
    ] {
        h.get_by_label(label);
    }
    // Info shows and hides the details panel.
    h.get_by_label("Find similar photos");
    h.get_by_label("Info").click();
    h.run_steps(3);
    assert!(h.query_by_label("Find similar photos").is_none());
    assert!(!app.borrow().viewer.as_ref().unwrap().info);
    h.get_by_label("Info").click();
    h.run_steps(3);
    h.get_by_label("Find similar photos");
    // Favorite is filled once marked, and the button says what it would undo.
    h.get_by_label("Favorite").click();
    h.run_steps(3);
    h.get_by_label("Unfavorite");
    // Back closes the viewer and returns to the grid.
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    h.get_by_label("Item img1.jpg");
}

#[test]
fn the_viewer_position_is_plain_text_beside_labelled_buttons_in_a_wide_window() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness_wide(&app, 1200.0);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    let pos = h.get_by_label("1 of 240");
    assert_eq!(pos.accesskit_node().role(), egui::accesskit::Role::Label);
    let pos_rect = pos.rect();
    let buttons = [
        "Back",
        "Favorite",
        "Add to album…",
        "Info",
        "Open with default app",
        "Show in folder",
        "Zoom to 100%",
    ];
    for label in buttons {
        let r = h.get_by_label(label).rect();
        assert!(
            !r.intersects(pos_rect),
            "{label} {r:?} overlaps the position {pos_rect:?}"
        );
    }
    // Wide window: icon and word ("Info" is wider than a bare icon button).
    assert!(h.get_by_label("Info").rect().width() > 70.0);
    assert!(h.get_by_label("Show in folder").rect().width() > 100.0);
    // Clicking where the position is does nothing.
    let info_before = app.borrow().viewer.as_ref().unwrap().info;
    h.event(egui::Event::PointerMoved(pos_rect.center()));
    h.event(egui::Event::PointerButton {
        pos: pos_rect.center(),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    h.event(egui::Event::PointerButton {
        pos: pos_rect.center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(3);
    let a = app.borrow();
    let v = a.viewer.as_ref().expect("still open");
    assert_eq!(v.info, info_before);
}

#[test]
fn the_viewer_toolbar_shows_only_icons_in_a_narrow_window() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness_wide(&app, 800.0);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    for label in [
        "Info",
        "Open with default app",
        "Show in folder",
        "Favorite",
    ] {
        assert!(
            h.get_by_label(label).rect().width() < 55.0,
            "{label} is an icon only"
        );
    }
    let pos = h.get_by_label("1 of 240").rect();
    assert!(!pos.intersects(h.get_by_label("Info").rect()));
}

#[test]
fn the_info_panel_names_the_folder_and_the_date_source() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    // The folder's name, not its whole path.
    h.get_by_label("nowhere");
    assert!(h.query_by_label("/nowhere").is_none());
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
    h.get_by_label("1 of 240");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some(), "the viewer stayed open");
    assert!(h.query_by_label("New album…").is_none(), "the menu closed");
    // With the menu gone, Escape leaves the viewer as before.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
}
