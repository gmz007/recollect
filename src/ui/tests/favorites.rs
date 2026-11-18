//! Favorite marks.

use super::*;

#[test]
fn favorites_toggle_in_the_viewer() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    open_tile(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("Favorite").click();
    h.run_steps(3);
    h.get_by_label("Unfavorite");
    // The viewer holds the list, so the list itself is left alone and the mark is drawn over it.
    let shown = |app: &Rc<RefCell<App>>| {
        let a = app.borrow();
        crate::ui::marked(&a.fav_marks, a.tl.items[0].file_id, a.tl.items[0].favorite)
    };
    assert!(shown(&app));
    h.key_press(egui::Key::F);
    h.run_steps(3);
    h.get_by_label("Favorite");
    assert!(!shown(&app));
}

#[test]
fn a_pending_mark_wins_over_a_reload_with_stale_values() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    let b = file_id_of(&lib, "b.jpg");
    let mut a = app.borrow_mut();
    a.set_favorite(b, true);
    // A list that was read before the write finished arrives (the writer is not polled here,
    // so the mark is still pending).
    let stale: Vec<_> =
        a.tl.items
            .iter()
            .cloned()
            .map(|mut it| {
                it.favorite = false;
                it
            })
            .collect();
    let filter = a.tl.filter();
    a.tl.awaiting = Some((u64::MAX, filter.clone()));
    a.tl.worker.inject(crate::ui::timeline::Loaded {
        id: u64::MAX,
        filter,
        items: Ok(stale),
    });
    a.update_timeline();
    let it = a.tl.items.iter().find(|it| it.file_id == b).unwrap();
    assert!(!it.favorite, "the list itself is stale");
    assert!(
        crate::ui::marked(&a.fav_marks, b, it.favorite),
        "the mark wins"
    );
    drop(a);
    // Once the write is confirmed and nothing stale can arrive, the list carries the value.
    settle(&mut h, || app.borrow().fav_marks.is_empty());
    assert!(app.borrow().tl.items.iter().any(|it| it.favorite));
}

#[test]
fn a_stale_favorites_list_does_not_bring_back_an_unfavorited_item() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let b = file_id_of(&lib, "b.jpg");
    lib.set_favorite(&[b], true).unwrap();
    app.borrow_mut().set_view(View::Favorites);
    let mut h = album_harness(&app);
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Favorites && a.tl.items.len() == 1
    });
    let mut a = app.borrow_mut();
    let stale = a.tl.items.to_vec();
    a.set_favorite(b, false);
    assert!(a.tl.items.is_empty());
    let filter = a.tl.filter();
    a.tl.awaiting = Some((u64::MAX, filter.clone()));
    a.tl.worker.inject(crate::ui::timeline::Loaded {
        id: u64::MAX,
        filter,
        items: Ok(stale),
    });
    a.update_timeline();
    assert!(a.tl.items.is_empty());
}

#[test]
fn a_failed_favorite_write_takes_the_mark_back() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    let b = file_id_of(&lib, "b.jpg");
    lib.fail_favorites
        .store(true, std::sync::atomic::Ordering::Release);
    app.borrow_mut().set_favorite(b, true);
    let shown = || {
        let a = app.borrow();
        let it = a.tl.items.iter().find(|it| it.file_id == b).unwrap();
        crate::ui::marked(&a.fav_marks, b, it.favorite)
    };
    assert!(shown(), "marked right away");
    settle(&mut h, || {
        app.borrow()
            .notices
            .current()
            .is_some_and(|n| n.contains("Couldn't save your favorite"))
    });
    assert!(!shown(), "the flag is back to what the library has");
    settle(&mut h, || app.borrow().fav_marks.is_empty());
    assert!(app.borrow().tl.items.iter().all(|it| !it.favorite));
    assert!(
        lib.timeline(&crate::store::db::TimelineFilter {
            favorites_only: true,
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    );
}

#[test]
fn a_bulk_favorite_failure_is_one_notice() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    lib.fail_favorites
        .store(true, std::sync::atomic::Ordering::Release);
    click_with(&mut h, "Item a.jpg", egui::Modifiers::COMMAND);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(3);
    h.get_by_label("Favorite").click();
    settle(&mut h, || {
        app.borrow()
            .notices
            .current()
            .is_some_and(|n| n.contains("Couldn't save favorites for 3 items"))
    });
    settle(&mut h, || app.borrow().fav_marks.is_empty());
    assert!(app.borrow().tl.items.iter().all(|it| !it.favorite));
}

#[test]
fn keys_do_not_reach_the_grid_behind_an_open_tile_menu() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 4);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 4);
    click_with(&mut h, "Item p1.jpg", egui::Modifiers::COMMAND);
    let before = lib.favorites_version();
    h.get_by_label("Item p1.jpg").click_secondary();
    h.run_steps(3);
    assert_eq!(tl_selection(&app).len(), 1, "the tile is selected");
    assert!(
        h.get_all_by_label("Add to album…").count() > 0,
        "the menu is open"
    );
    h.key_press(egui::Key::F);
    h.run_steps(3);
    lib.wait_writes();
    assert_eq!(lib.favorites_version(), before, "nothing was favorited");
    assert!(app.borrow().fav_marks.is_empty());
}
