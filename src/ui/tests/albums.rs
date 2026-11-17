//! Albums and multi-select.

use super::*;

/// What the Delete dialog says about album `name`.
fn delete_text(name: &str) -> String {
    format!(
        "Delete the album “{name}”? The photos stay in your library — only the album is removed."
    )
}

/// Opens the album's "⋯" menu and chooses `item`.
fn open_album_menu(h: &mut Harness<'_>, item: &str) {
    h.get_by_label("More actions").click();
    h.run_steps(3);
    h.get_by_label(item).click();
    h.run_steps(2);
}

#[test]
fn ctrl_and_shift_click_arrows_select_all_and_escape() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    let (ctrl, shift) = (egui::Modifiers::COMMAND, egui::Modifiers::SHIFT);
    click_with(&mut h, "Item img2.jpg", ctrl);
    assert_eq!(tl_selection(&app), [2]);
    h.get_by_label("1 selected");
    click_with(&mut h, "Item img4.jpg", ctrl);
    assert_eq!(tl_selection(&app), [2, 4]);
    h.get_by_label("2 selected");
    // Shift+click selects from the anchor (img4) to the clicked item.
    click_with(&mut h, "Item img7.jpg", shift);
    assert_eq!(tl_selection(&app), [4, 5, 6, 7]);
    h.get_by_label("4 selected");
    // Ctrl+click toggles one item off again.
    click_with(&mut h, "Item img5.jpg", ctrl);
    assert_eq!(tl_selection(&app), [4, 6, 7]);
    // Ctrl+A selects the whole list.
    h.key_press_modifiers(ctrl, egui::Key::A);
    h.run_steps(3);
    assert_eq!(tl_selection(&app).len(), 240);
    h.get_by_label("240 selected");
    // A plain arrow collapses to one selection, moved.
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [8]);
    // Shift+arrows extend it.
    h.key_press_modifiers(shift, egui::Key::ArrowRight);
    h.run_steps(3);
    h.key_press_modifiers(shift, egui::Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [8, 9, 10]);
    h.get_by_label("3 selected");
    // Escape clears everything.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
    assert_eq!(app.borrow().tl.grid.selected, None);
    assert!(h.query_by_label("3 selected").is_none());
}

#[test]
fn the_selection_survives_a_reload() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    click_with(&mut h, "Item img1.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Item img2.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Item img3.jpg", egui::Modifiers::COMMAND);
    assert_eq!(tl_selection(&app), [1, 2, 3]);
    // A reload in which img2 is gone and a new photo came in.
    let mut items = spread_items();
    items.retain(|it| it.file_id != 2);
    items.insert(0, titem(999, 2020, 6, 1));
    app.borrow_mut().inject_timeline(items);
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [1, 3]);
    h.get_by_label("2 selected");
}

#[test]
fn albums_can_be_created_filled_trimmed_renamed_and_deleted() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 5);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 5);
    let (ctrl, shift) = (egui::Modifiers::COMMAND, egui::Modifiers::SHIFT);

    // Create an album from the sidebar.
    h.get_by_label("No albums yet.");
    h.get_by_label("New album…").click();
    type_name(&mut h, "Trip");
    h.get_by_label("Create").click();
    settle(&mut h, || app.borrow().albums.len() == 1);
    h.get_by_label("Trip (0)");

    // Add a multi-selection through the selection bar.
    click_with(&mut h, "Item p4.jpg", ctrl);
    click_with(&mut h, "Item p2.jpg", ctrl);
    click_with(&mut h, "Item p0.jpg", ctrl);
    h.get_by_label("3 selected");
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Trip").click();
    settle(&mut h, || {
        lib.albums().unwrap()[0].count == 3 && app.borrow().albums[0].count == 3
    });
    h.get_by_label_contains("Added 3 to Trip");
    // Adding them again changes nothing. (The confirmation took the bar's place.)
    // The selection was cleared once they were added: pick them again.
    assert!(tl_selection(&app).is_empty());
    app.borrow_mut().notices.items.clear();
    click_with(&mut h, "Item p4.jpg", ctrl);
    click_with(&mut h, "Item p2.jpg", ctrl);
    click_with(&mut h, "Item p0.jpg", ctrl);
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Trip").click();
    settle(&mut h, || {
        app.borrow()
            .notices
            .current()
            .is_some_and(|n| n.contains("Already in"))
    });
    assert_eq!(lib.albums().unwrap()[0].count, 3);

    // The album view lists exactly those items.
    h.get_by_label("Trip (3)").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Album(a.albums[0].id) && a.tl.items.len() == 3 && a.tl.loaded
    });
    h.run_steps(3);
    h.get_by_label("Item p4.jpg");
    h.get_by_label("Item p2.jpg");
    h.get_by_label("Item p0.jpg");
    assert!(h.query_by_label("Item p3.jpg").is_none());
    h.get_by_label("Add photos");
    h.get_by_label("More actions");

    // Remove one from the album with the selection bar's button.
    click_with(&mut h, "Item p2.jpg", ctrl);
    h.get_by_label("Remove from album").click();
    h.run_steps(3);
    // The grid drops the item once the library has confirmed the removal.
    settle(&mut h, || {
        lib.albums().unwrap()[0].count == 2 && app.borrow().tl.items.len() == 2
    });
    h.run_steps(2);
    assert!(h.query_by_label("Item p2.jpg").is_none());
    // ...and two with the selection bar (once the confirmation has given the bar back).
    app.borrow_mut().notices.items.clear();
    click_with(&mut h, "Item p4.jpg", ctrl);
    click_with(&mut h, "Item p0.jpg", shift);
    h.get_by_label("Remove from album").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 0);
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.items.is_empty() && a.tl.awaiting.is_none() && a.tl.loaded
    });
    h.run_steps(3);
    h.get_by_label("This album is empty");
    assert_eq!(
        lib.timeline(&crate::store::db::TimelineFilter::default())
            .unwrap()
            .len(),
        5
    );

    // Put one back, then rename.
    let p1 = file_id_of(&lib, "p1.jpg");
    lib.add_to_album(lib.albums().unwrap()[0].id, &[p1])
        .unwrap();
    open_album_menu(&mut h, "Rename…");
    h.run_steps(2);
    // The field starts with the old name: select all of it and replace it.
    h.key_press_modifiers(ctrl, egui::Key::A);
    h.run_steps(1);
    h.get_by_label("Album name").type_text("  Holiday ");
    h.run_steps(2);
    h.get_by_label("Rename").click();
    settle(&mut h, || {
        lib.albums().unwrap()[0].name == "Holiday" && app.borrow().albums[0].name == "Holiday"
    });
    h.get_by_label("Holiday");
    h.get_by_label_contains("Holiday (");

    // Delete needs a confirmation that says the photos stay.
    open_album_menu(&mut h, "Delete album…");
    h.run_steps(3);
    h.get_by_label(&delete_text("Holiday"));
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(lib.albums().unwrap().len(), 1);
    open_album_menu(&mut h, "Delete album…");
    h.run_steps(3);
    h.get_by_label("Delete").click();
    // The sidebar's album list is read again in the background; wait for it as well.
    settle(&mut h, || {
        let a = app.borrow();
        lib.albums().unwrap().is_empty()
            && a.albums.is_empty()
            && a.tl.view == View::Timeline
            && a.tl.items.len() == 5
    });
    h.run_steps(2);
    h.get_by_label("No albums yet.");
    h.get_by_label("Item p4.jpg");
}

#[test]
fn a_new_album_from_the_add_menu_gets_the_selection_and_names_must_be_unique() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 4);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 4);
    lib.create_album("Existing", &[]).unwrap();
    app.borrow_mut().refresh_albums();
    settle(&mut h, || app.borrow().albums.len() == 1);
    click_with(&mut h, "Item p3.jpg", egui::Modifiers::COMMAND);
    // One item picked: the bar's menu; "New album…" creates an album holding the item.
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Existing");
    h.get_all_by_label("New album…").last().unwrap().click();
    h.run_steps(3);
    h.get_by_label("1 item will be added to it.");
    // A duplicate name (any case) is refused and the dialog stays open.
    type_name(&mut h, "EXISTING");
    h.get_by_label("Create").click();
    h.run_steps(3);
    h.get_by_label_contains("already exists");
    assert_eq!(lib.albums().unwrap().len(), 1);
    // Fix the name (clicking the button took the focus away).
    h.get_by_label("Album name").click();
    h.run_steps(2);
    h.get_by_label("Album name").type_text("2");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    // Wait until the app has taken in the result too (its notice and the album list), so
    // nothing arriving later changes the screen under the next clicks.
    settle(&mut h, || {
        let a = app.borrow();
        lib.albums().unwrap().len() == 2
            && a.albums.len() == 2
            && a.notices
                .items
                .iter()
                .any(|n| n.text.starts_with("Created"))
    });
    h.run_steps(2);
    let created = lib
        .albums()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "EXISTING2")
        .unwrap();
    assert_eq!(created.count, 1);
    // The selection was cleared once the album was created. Escape closes a dialog without
    // touching a selection.
    assert!(tl_selection(&app).is_empty());
    click_with(&mut h, "Item p3.jpg", egui::Modifiers::COMMAND);
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_all_by_label("New album…").last().unwrap().click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().dialogs.new_album.is_none());
    assert_eq!(tl_selection(&app).len(), 1);
}

#[test]
fn the_viewer_offers_add_to_album() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 3);
    lib.create_album("Trip", &[]).unwrap();
    let mut h = quick_harness(&app);
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.items.len() == 3 && a.albums.len() == 1
    });
    settle(&mut h, || !app.borrow().albums.is_empty());
    open_tile(&mut h, "Item p1.jpg");
    h.run_steps(3);
    let target = file_id_of(&lib, "p1.jpg");
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Trip").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 1);
    let id = lib.albums().unwrap()[0].id;
    let items = lib
        .timeline(&crate::store::db::TimelineFilter {
            album: Some(id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        items.iter().map(|i| i.file_id).collect::<Vec<_>>(),
        [target]
    );
}

#[test]
fn typing_in_the_viewers_new_album_field_changes_only_the_text() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = files_app(d.path(), 3);
    let mut h = quick_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    open_tile(&mut h, "Item p1.jpg");
    h.run_steps(3);
    let shown = |app: &Rc<RefCell<App>>| app.borrow().viewer.as_ref().unwrap().current().file_id;
    let before = shown(&app);
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_all_by_label("New album…").last().unwrap().click();
    h.run_steps(3);
    type_name(&mut h, "fiz");
    for key in [
        egui::Key::F,
        egui::Key::I,
        egui::Key::Z,
        egui::Key::ArrowRight,
        egui::Key::ArrowLeft,
        egui::Key::Home,
        egui::Key::End,
    ] {
        h.key_press(key);
        h.run_steps(2);
    }
    let a = app.borrow();
    assert!(a.viewer.is_some(), "still open");
    assert_eq!(a.viewer.as_ref().unwrap().current().file_id, before);
    assert!(a.fav_marks.is_empty(), "no favorite change");
    assert_eq!(a.dialogs.new_album.as_ref().unwrap().name, "fiz");
}

#[test]
fn keys_do_not_reach_the_grid_behind_a_dialog_and_enter_never_deletes() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app, mut h) = album_view(d.path(), 3, album_harness);
    open_album_menu(&mut h, "Delete album…");
    h.run_steps(3);
    // Clicks do not reach the grid behind the dialog either: select straight in the state.
    {
        let mut a = app.borrow_mut();
        let id = a.tl.items[1].file_id;
        a.tl.grid.sel.only(id);
        a.tl.grid.picking = true;
        a.tl.grid.selected = Some(1);
    }
    h.run_steps(2);
    h.get_by_label(&delete_text("A"));
    // Enter, F and arrows behind the dialog do nothing.
    for key in [egui::Key::Enter, egui::Key::F, egui::Key::ArrowRight] {
        h.key_press(key);
        h.run_steps(3);
    }
    {
        let a = app.borrow();
        assert!(a.viewer.is_none());
        assert!(a.fav_marks.is_empty());
        assert!(a.dialogs.delete_album.is_some(), "still asking");
    }
    assert_eq!(lib.albums().unwrap().len(), 1, "nothing deleted");
    // Escape cancels and leaves the selection alone.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().dialogs.delete_album.is_none());
    assert_eq!(tl_selection(&app).len(), 1);
}

#[test]
fn a_failing_remove_or_delete_changes_nothing_and_shows_an_error() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app, mut h) = album_view(d.path(), 3, album_harness);
    lib.fail_albums
        .store(true, std::sync::atomic::Ordering::Release);
    click_with(&mut h, "Item p1.jpg", egui::Modifiers::COMMAND);
    h.get_by_label("Remove from album").click();
    settle(&mut h, || {
        app.borrow()
            .notices
            .current_notice()
            .and_then(|n| n.details.as_deref())
            .is_some_and(|d| d.contains("simulated failure"))
    });
    assert_eq!(app.borrow().tl.items.len(), 3, "the item is still listed");
    h.get_by_label("Item p1.jpg");
    assert_eq!(lib.albums().unwrap()[0].count, 3);
    // Deleting: the dialog stays, the view stays.
    open_album_menu(&mut h, "Delete album…");
    h.run_steps(3);
    h.get_by_label("Delete").click();
    // A click counts in the next frame: only then is the deletion queued for the writer.
    h.run_steps(1);
    // The writer has answered; the next frames show the error and re-enable the buttons.
    lib.wait_writes();
    h.run_steps(3);
    assert!(
        app.borrow()
            .dialogs
            .delete_album
            .as_ref()
            .is_some_and(|d| !d.pending())
    );
    assert!(matches!(app.borrow().tl.view, View::Album(_)));
    assert_eq!(app.borrow().albums.len(), 1);
    assert_eq!(lib.albums().unwrap().len(), 1);
    // Once the library works again the same dialog deletes it, and only then the view moves.
    lib.fail_albums
        .store(false, std::sync::atomic::Ordering::Release);
    h.get_by_label("Delete").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Timeline && a.dialogs.delete_album.is_none() && a.albums.is_empty()
    });
    assert!(lib.albums().unwrap().is_empty());
}

#[test]
fn a_failed_create_reopens_the_dialog_with_the_name_and_the_items() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 3);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    lib.fail_albums
        .store(true, std::sync::atomic::Ordering::Release);
    click_with(&mut h, "Item p2.jpg", egui::Modifiers::COMMAND);
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_all_by_label("New album…").last().unwrap().click();
    h.run_steps(3);
    // While the window is open the sidebar cannot start another one.
    assert!(h.get_by_label("New album…").accesskit_node().is_disabled());
    type_name(&mut h, "Fresh");
    h.get_by_label("Create").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.dialogs
            .new_album
            .as_ref()
            .is_some_and(|n| n.name == "Fresh" && n.add.len() == 1)
    });
    h.get_by_label("1 item will be added to it.");
    assert!(lib.albums().unwrap().is_empty());
}

#[test]
fn long_album_names_are_refused_and_shortened() {
    assert_eq!(crate::ui::albums::short_name("Trip", 28), "Trip");
    let long = "Summer holidays with the whole family in the mountains";
    let short = crate::ui::albums::short_name(long, 28);
    assert_eq!(short.chars().count(), 28);
    assert!(short.ends_with('…'));
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 2);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 2);
    h.get_by_label("New album…").click();
    type_name(&mut h, &"x".repeat(101));
    h.get_by_label("Create").click();
    h.run_steps(3);
    h.get_by_label_contains("at most 100");
    assert!(lib.albums().unwrap().is_empty());
    // A long but valid name shows shortened in the sidebar.
    app.borrow_mut().dialogs.new_album = None;
    h.run_steps(2);
    lib.create_album(long, &[]).unwrap();
    settle(&mut h, || app.borrow().albums.len() == 1);
    h.get_by_label(&format!("{short} (0)"));
}

#[test]
fn right_clicking_inside_a_selection_acts_on_all_of_it() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 4);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 4);
    let album = lib.create_album("A", &[]).unwrap();
    settle(&mut h, || app.borrow().albums.len() == 1);
    click_with(&mut h, "Item p3.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Item p1.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Item p0.jpg", egui::Modifiers::COMMAND);
    h.get_by_label("Item p1.jpg").click_secondary();
    h.run_steps(3);
    assert_eq!(tl_selection(&app).len(), 3, "the selection is kept");
    h.get_all_by_label("Add to album…").last().unwrap().click();
    h.run_steps(3);
    h.get_all_by_label("A").last().unwrap().click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 3);
    assert_eq!(lib.albums().unwrap()[0].id, album);
    // A right-click outside the selection selects that item alone.
    h.get_by_label("Item p2.jpg").click_secondary();
    h.run_steps(3);
    let id = file_id_of(&lib, "p2.jpg");
    assert_eq!(tl_selection(&app), [id]);
}

#[test]
fn tiles_tell_assistive_tech_whether_they_are_selected() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    click_with(&mut h, "Item img1.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Item img3.jpg", egui::Modifiers::COMMAND);
    let on = |h: &Harness<'_>, l: &str| {
        h.get_by_label(l)
            .accesskit_node()
            .toggled()
            .map(|t| format!("{t:?}"))
    };
    assert_eq!(on(&h, "Item img1.jpg").as_deref(), Some("True"));
    assert_eq!(on(&h, "Item img3.jpg").as_deref(), Some("True"));
    assert_eq!(on(&h, "Item img2.jpg").as_deref(), Some("False"));
}

#[test]
fn a_refused_album_name_leaves_the_field_ready_for_typing() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = files_app(d.path(), 2);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 2);
    h.get_by_label("New album…").click();
    h.run_steps(3);
    type_name(&mut h, &"x".repeat(101));
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    h.get_by_label_contains("at most 100");
    assert!(
        h.ctx.text_edit_focused(),
        "the name field takes focus again"
    );
}

#[test]
fn small_tiles_expose_no_select_or_favorite_buttons() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = small_tiles_harness(&app);
    h.get_by_label("Item img1.jpg");
    assert!(h.query_by_label("Select Item img1.jpg").is_none());
    assert!(h.query_by_label("Add Item img1.jpg to favorites").is_none());
}

/// Dates for `album_view`'s photos: 2017 for all of them (see `files_app`).
#[test]
fn the_album_header_counts_photos_and_gives_the_years() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app, h) = album_view(d.path(), 3, album_harness);
    h.get_by_label("A");
    h.get_by_label("3 photos · 2017");
    assert_eq!(app.borrow().tl.years, Some((2017, 2017)));
    // Two years: a range with the en dash.
    {
        let mut a = app.borrow_mut();
        let mut items = (*a.tl.items).clone();
        items[0].taken_at = titem(0, 2019, 5, 1).taken_at;
        a.inject_timeline(items);
    }
    let mut h = h;
    h.run_steps(3);
    h.get_by_label("3 photos · 2017 – 2019");
}

#[test]
fn the_more_menu_has_rename_and_delete() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app, mut h) = album_view(d.path(), 3, album_harness);
    assert!(h.query_by_label("Rename…").is_none(), "hidden until opened");
    h.get_by_label("More actions").click();
    h.run_steps(3);
    h.get_by_label("Rename…");
    h.get_by_label("Delete album…");
    h.get_by_label("Rename…").click();
    h.run_steps(3);
    assert!(app.borrow().dialogs.rename_album.is_some());
}

#[test]
fn add_photos_chooses_photos_and_adds_them_with_undo() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 5);
    let first = file_id_of(&lib, "p0.jpg");
    lib.create_album("Family", &[first]).unwrap();
    let mut h = album_harness(&app);
    settle(&mut h, || {
        app.borrow().albums.len() == 1 && app.borrow().tl.items.len() == 5
    });
    h.get_by_label_contains("Family (1)").click();
    settle(&mut h, || {
        let a = app.borrow();
        matches!(a.tl.view, View::Album(_)) && a.tl.items.len() == 1 && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    h.get_by_label("Add photos").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Timeline && a.tl.items.len() == 5 && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    h.get_by_label("Choose photos to add to Family");
    // Nothing chosen yet: the main button waits.
    assert!(
        h.get_by_label("Add to Family")
            .accesskit_node()
            .is_disabled()
    );
    // A plain click chooses a photo instead of opening it.
    h.get_by_label("Item p3.jpg").click();
    h.run_steps(3);
    h.get_by_label("Item p4.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    h.get_by_label("2 selected");
    h.get_by_label("Add to Family").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 3);
    settle(&mut h, || {
        let a = app.borrow();
        matches!(a.tl.view, View::Album(_)) && a.tl.items.len() == 3 && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    assert!(app.borrow().adding_to().is_none());
    h.get_by_label("Added 2 to Family");
    h.get_by_label_contains("3 photos");
    // Undo takes them out again.
    h.get_by_label("Undo").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 1);
}

/// Family with one photo in it, opened, then "Add photos" clicked and Photos shown.
fn choosing_photos(d: &std::path::Path) -> (Arc<Library>, Rc<RefCell<App>>, Harness<'static>) {
    let (lib, app) = files_app(d, 5);
    let first = file_id_of(&lib, "p0.jpg");
    lib.create_album("Family", &[first]).unwrap();
    let mut h = album_harness(&app);
    settle(&mut h, || {
        app.borrow().albums.len() == 1 && app.borrow().tl.items.len() == 5
    });
    h.get_by_label_contains("Family (1)").click();
    settle(&mut h, || {
        let a = app.borrow();
        matches!(a.tl.view, View::Album(_)) && a.tl.items.len() == 1 && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    h.get_by_label("Add photos").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Timeline && a.tl.items.len() == 5 && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    (lib, app, h)
}

#[test]
fn cancel_and_escape_leave_add_photos_without_changes() {
    for use_escape in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let (lib, app, mut h) = choosing_photos(d.path());
        h.get_by_label("Item p3.jpg").click();
        h.run_steps(3);
        h.get_by_label("1 selected");
        if use_escape {
            h.key_press(egui::Key::Escape);
        } else {
            h.get_by_label("Cancel").click();
        }
        settle(&mut h, || {
            let a = app.borrow();
            matches!(a.tl.view, View::Album(_)) && a.tl.items.len() == 1 && a.tl.awaiting.is_none()
        });
        assert!(app.borrow().adding_to().is_none());
        assert!(tl_selection(&app).is_empty());
        assert_eq!(lib.albums().unwrap()[0].count, 1);
        h.get_by_label("Add photos");
    }
}

#[test]
fn going_elsewhere_ends_add_photos() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app, mut h) = choosing_photos(d.path());
    h.get_by_label("Recently added").click();
    h.run_steps(4);
    assert!(app.borrow().adding_to().is_none());
    assert!(!app.borrow().tl.grid.picking);
    assert!(h.query_by_label_contains("Choose photos to add").is_none());
}

#[test]
fn an_empty_album_says_so_and_add_photos_starts_choosing() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = files_app(d.path(), 3);
    let lib = Arc::clone(&app.borrow().lib);
    lib.create_album("Quiet", &[]).unwrap();
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().albums.len() == 1);
    h.get_by_label("Quiet (0)").click();
    settle(&mut h, || {
        let a = app.borrow();
        // The timeline had photos: no items means the album's own (empty) list has arrived.
        matches!(a.tl.view, View::Album(_))
            && a.tl.items.is_empty()
            && a.tl.loaded
            && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    h.get_by_label("This album is empty");
    h.get_by_label("Add photos from Photos, or select photos anywhere and choose Add to album.");
    h.get_by_label("Albums never change or delete your files.");
    // Two buttons: the header's and the empty state's.
    h.get_all_by_label("Add photos").last().unwrap().click();
    settle(&mut h, || {
        let a = app.borrow();
        a.adding_to().is_some() && a.tl.items.len() == 3 && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    h.get_by_label("Choose photos to add to Quiet");
    assert_eq!(app.borrow().tl.view, View::Timeline);
}

#[test]
fn help_opens_with_f1_on_an_album_and_on_search_results() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app, mut h) = album_view(d.path(), 3, album_harness);
    h.key_press(egui::Key::F1);
    settle(&mut h, || app.borrow().dialogs.help_open);
    h.get_by_label("Close help").click();
    h.run_steps(3);
    assert!(!app.borrow().dialogs.help_open);
    // Search results (the same setup as the results tests).
    let d = tempfile::tempdir().unwrap();
    let app = Rc::new(RefCell::new(grid_app_with(d.path(), false)));
    let mut h = album_harness(&app);
    assert!(app.borrow().searching());
    h.key_press(egui::Key::F1);
    settle(&mut h, || app.borrow().dialogs.help_open);
    h.get_by_label("Keyboard shortcuts help").click();
    h.run_steps(3);
    h.get_by_label("Tab, then Enter");
}
