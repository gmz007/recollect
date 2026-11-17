//! The photo grid: day headings, selecting by day, photo size and what a finished action does
//! to the selection.

use super::*;
use egui::{Key, Modifiers};

const FIRST_DAY: &str = "Sunday 15 December 2019";

/// Presses `key` with Ctrl (Cmd on a Mac) held.
fn command_key(h: &mut Harness<'_>, key: Key) {
    h.key_combination_modifiers(Modifiers::COMMAND, &[key]);
    h.run_steps(3);
}

/// The labels of the radio buttons that are selected (Settings choices).
fn selected_choices(h: &Harness<'_>) -> Vec<String> {
    h.root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::RadioButton)
        .filter(|n| {
            let node = n.accesskit_node();
            node.is_selected() == Some(true)
                || node.toggled() == Some(egui::accesskit::Toggled::True)
        })
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

#[test]
fn the_circle_of_a_day_heading_selects_and_deselects_the_whole_day() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    // 24 photos were taken on the first day; the rest are on other days.
    h.get_by_label(FIRST_DAY);
    h.get_by_label(&format!("Select all of {FIRST_DAY}"))
        .click();
    h.run_steps(3);
    assert_eq!(tl_selection(&app).len(), 24);
    h.get_by_label("24 selected");
    assert!(app.borrow().viewer.is_none());
    h.get_by_label(&format!("Deselect all of {FIRST_DAY}"))
        .click();
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
    // A day completes a selection that was begun by hand.
    click_with(&mut h, "Select img2.jpg", Modifiers::NONE);
    h.get_by_label(&format!("Select all of {FIRST_DAY}"))
        .click();
    h.run_steps(3);
    assert_eq!(tl_selection(&app).len(), 24);
}

#[test]
fn while_photos_are_selected_a_click_anywhere_on_a_photo_toggles_it() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    click_with(&mut h, "Select img2.jpg", Modifiers::NONE);
    assert_eq!(tl_selection(&app), [2]);
    click_with(&mut h, "Item img3.jpg", Modifiers::NONE);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(tl_selection(&app), [2, 3]);
    click_with(&mut h, "Item img2.jpg", Modifiers::NONE);
    assert_eq!(tl_selection(&app), [3]);
    // Shift selects the range from the last clicked photo.
    click_with(&mut h, "Item img7.jpg", Modifiers::SHIFT);
    assert_eq!(tl_selection(&app), [3, 4, 5, 6, 7]);
    // Ctrl+A selects everything in the view, Esc clears.
    command_key(&mut h, Key::A);
    assert_eq!(tl_selection(&app).len(), 240);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
}

#[test]
fn ctrl_equals_minus_and_zero_change_the_photo_size() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    // egui's own keyboard zoom would change the text size behind its setting's back.
    assert!(!h.ctx.options(|o| o.zoom_with_keyboard));
    assert!((app.borrow().tl.tile_size - 168.0).abs() < 1e-3);
    assert!(h.query_by_label("Photo size").is_none());
    command_key(&mut h, Key::Equals);
    assert!((app.borrow().tl.tile_size - 204.0).abs() < 1e-3);
    // The chip shows the level while the size changes.
    h.get_by_label("Photo size");
    command_key(&mut h, Key::Minus);
    command_key(&mut h, Key::Minus);
    assert!((app.borrow().tl.tile_size - 144.0).abs() < 1e-3);
    command_key(&mut h, Key::Num0);
    assert!((app.borrow().tl.tile_size - 168.0).abs() < 1e-3);
    command_key(&mut h, Key::Equals);
    command_key(&mut h, Key::Equals);
    // The size is saved once it stops changing.
    settle(&mut h, || (lib.settings().photo_size - 240.0).abs() < 1e-3);
    // The chip fades after about a second.
    settle(&mut h, || h_has_no_chip(&app));
}

fn h_has_no_chip(app: &Rc<RefCell<App>>) -> bool {
    app.borrow().size_change_idle()
}

#[test]
fn pinching_changes_the_photo_size_smoothly_within_its_range() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.event(egui::Event::Zoom(1.25));
    h.run_steps(3);
    assert!((app.borrow().tl.tile_size - 210.0).abs() < 1.0);
    h.event(egui::Event::Zoom(10.0));
    h.run_steps(3);
    assert!((app.borrow().tl.tile_size - 320.0).abs() < 1e-3);
    h.event(egui::Event::Zoom(0.001));
    h.run_steps(3);
    assert!((app.borrow().tl.tile_size - 96.0).abs() < 1e-3);
}

#[test]
fn settings_has_a_photo_size_row_that_sets_the_same_size() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    open_section(&mut h, "Appearance");
    h.get_by_label("Photo size");
    // The default is Medium (and so is the text size).
    assert_eq!(
        selected_choices(&h),
        ["Match my computer", "Medium", "Medium"]
    );
    // The photo size row comes after the text size row.
    h.get_all_by_label("Large").nth(1).unwrap().click();
    h.run();
    assert!((app.borrow().tl.tile_size - 240.0).abs() < 1e-3);
    assert!((lib.settings().photo_size - 240.0).abs() < 1e-3);
    h.get_all_by_label("Small").nth(1).unwrap().click();
    h.run();
    assert!((app.borrow().tl.tile_size - 120.0).abs() < 1e-3);
    // A size in between marks none of the three.
    app.borrow_mut().tl.tile_size = 150.0;
    h.run();
    assert_eq!(selected_choices(&h), ["Match my computer", "Medium"]);
}

#[test]
fn the_selection_is_cleared_after_a_favorite_and_undo_still_works() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 4);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 4);
    click_with(&mut h, "Item p3.jpg", Modifiers::COMMAND);
    click_with(&mut h, "Item p2.jpg", Modifiers::COMMAND);
    h.get_by_label("2 selected");
    h.get_by_label("Favorite").click();
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
    assert!(h.query_by_label("2 selected").is_none());
    h.get_by_label_contains("Added 2 to Favorites");
    h.get_by_label("Undo").click();
    settle(&mut h, || {
        lib.timeline(&crate::store::db::TimelineFilter {
            favorites_only: true,
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    });
}

#[test]
fn old_settings_files_without_a_photo_size_load() {
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    std::fs::create_dir_all(lib.paths().config.clone()).unwrap();
    std::fs::write(
        lib.paths().settings_path(),
        "group = \"Day\"\nnewest_first = false\n",
    )
    .unwrap();
    let s = crate::config::Settings::load(&lib.paths().settings_path()).unwrap();
    assert!((s.photo_size - 168.0).abs() < 1e-3);
}

#[test]
fn unreadable_folders_are_found_by_a_background_check() {
    let d = tempfile::tempdir().unwrap();
    // The library's only folder, "/nowhere", does not exist on this computer.
    let (_lib, app) = files_app(d.path(), 2);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().missing_folders == [1]);
    // The facts the screens read do not carry the answer: asking the file system is not part
    // of reading them.
    assert!(!app.borrow().folder_check.is_pending());
}
