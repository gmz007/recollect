//! The search field's click area, the floating selection bar and confirmations with Undo.

use super::*;
use crate::store::db::TimelineFilter;
use std::time::Duration;

const CTRL: egui::Modifiers = egui::Modifiers::COMMAND;

fn favorite_flags(lib: &Library) -> Vec<bool> {
    lib.wait_writes();
    lib.timeline(&TimelineFilter::default())
        .unwrap()
        .iter()
        .map(|it| it.favorite)
        .collect()
}

/// An app with a library of five photos and one album, "Trip", holding `members` of them.
fn trip_app(d: &std::path::Path, members: &[&str]) -> (Arc<Library>, Rc<RefCell<App>>, i64) {
    let (lib, app) = files_app(d, 5);
    let picked: Vec<i64> = members.iter().map(|n| file_id_of(&lib, n)).collect();
    let album = lib.create_album("Trip", &picked).unwrap();
    (lib, app, album)
}

#[test]
fn a_click_on_the_search_fields_icon_or_padding_focuses_the_text_box() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    assert!(!app.borrow().search_help.focused);
    let field = app.borrow().search_help.field.expect("the field is drawn");
    // The left padding, left of the magnifier icon.
    let pos = egui::pos2(field.left() + 4.0, field.center().y);
    h.drag_at(pos);
    h.run_steps(2);
    h.drop_at(pos);
    settle(&mut h, || app.borrow().search_help.focused);
}

#[test]
fn adding_to_an_album_confirms_with_undo_in_place_of_the_bar() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app, album) = trip_app(d.path(), &[]);
    let mut h = album_harness(&app);
    settle(&mut h, || {
        app.borrow().tl.items.len() == 5 && app.borrow().albums.len() == 1
    });
    for name in ["p4.jpg", "p3.jpg", "p2.jpg"] {
        click_with(&mut h, &format!("Item {name}"), CTRL);
    }
    h.get_by_label("3 selected");
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Trip").click();
    settle_on_label(&mut h, "Added 3 to Trip");
    assert!(app.borrow().confirmation_showing());
    assert!(
        h.query_by_label("3 selected").is_none(),
        "the toast replaces the bar"
    );
    assert_eq!(lib.albums().unwrap()[0].count, 3);

    h.get_by_label("Undo").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 0);
    settle_on_label(&mut h, "Undone");
    assert!(
        h.query_by_label("Undo").is_none(),
        "Undo is not offered for Undo"
    );
    assert_eq!(lib.albums().unwrap()[0].id, album);
}

#[test]
fn removing_from_an_album_acts_at_once_and_undo_puts_them_back() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app, album) = trip_app(d.path(), &["p0.jpg", "p1.jpg", "p2.jpg"]);
    app.borrow_mut().set_view(View::Album(album));
    let mut h = album_harness(&app);
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.items.len() == 3 && a.tl.loaded
    });
    click_with(&mut h, "Item p1.jpg", CTRL);
    click_with(&mut h, "Item p2.jpg", CTRL);
    h.get_by_label("2 selected");
    h.get_by_label("Remove from album").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 1);
    settle_on_label(&mut h, "Removed 2 from Trip");
    assert!(app.borrow().confirmation_showing());
    assert_eq!(
        lib.albums().unwrap().len(),
        1,
        "no file or album is deleted"
    );

    h.get_by_label("Undo").click();
    settle(&mut h, || {
        lib.albums().unwrap()[0].count == 3 && app.borrow().tl.items.len() == 3
    });
}

#[test]
fn favoriting_a_selection_can_be_undone_per_file() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 5);
    let first = file_id_of(&lib, "p0.jpg");
    // One of the three is a favorite already; Undo must leave it one.
    lib.set_favorite(&[first], true).unwrap();
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 5);
    for name in ["p0.jpg", "p1.jpg", "p2.jpg"] {
        click_with(&mut h, &format!("Item {name}"), CTRL);
    }
    h.get_by_label("Favorite").click();
    settle_on_label(&mut h, "Added 2 to Favorites");
    assert!(app.borrow().confirmation_showing());
    assert_eq!(favorite_flags(&lib).iter().filter(|f| **f).count(), 3);

    h.get_by_label("Undo").click();
    settle(&mut h, || {
        favorite_flags(&lib).iter().filter(|f| **f).count() == 1
    });
    let timeline = lib.timeline(&TimelineFilter::default()).unwrap();
    assert!(timeline.iter().any(|it| it.file_id == first && it.favorite));
}

#[test]
fn a_confirmation_is_gone_after_five_seconds() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app, _album) = trip_app(d.path(), &[]);
    let mut h = album_harness(&app);
    app.borrow_mut().notify_undoable(
        "Added 1 to Trip",
        super::super::notices::Undo::Favorites(Vec::new()),
    );
    h.run_steps(2);
    h.get_by_label("Added 1 to Trip");
    for n in &mut app.borrow_mut().notices.items {
        n.at -= Duration::from_secs(5);
    }
    settle(&mut h, || h_none(&app));
    assert!(h.query_by_label("Added 1 to Trip").is_none());
}

fn h_none(app: &Rc<RefCell<App>>) -> bool {
    app.borrow().notices.current().is_none()
}

/// Runs frames until `label` is on screen. A new pill is measured for one frame before it
/// shows, so the app can hold a notice a frame before the notice can be seen or clicked.
fn settle_on_label(h: &mut Harness<'_>, label: &str) {
    for _ in 0..500 {
        h.run_steps(1);
        if h.query_by_label(label).is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("{label:?} not shown in time");
}

#[test]
fn the_photo_and_video_choice_of_a_search_does_not_filter_browsing() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = files_app(d.path(), 5);
    // Videos was chosen in an earlier search; the timeline still lists the photos.
    app.borrow_mut().kind = Some(crate::media::MediaKind::Video);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 5);
    app.borrow_mut().kind = Some(crate::media::MediaKind::Image);
    h.run_steps(3);
    assert_eq!(app.borrow().tl.items.len(), 5);
}

#[test]
fn f1_and_the_settings_shortcut_wait_while_a_dialog_is_open() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    app.borrow_mut().dialogs.new_album = Some(crate::ui::albums::NewAlbum::with_items(Vec::new()));
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    h.key_press_modifiers(CTRL, egui::Key::Comma);
    h.run_steps(3);
    assert!(!app.borrow().dialogs.help_open);
    assert!(!app.borrow().dialogs.settings_open);
    app.borrow_mut().dialogs.new_album = None;
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    settle(&mut h, || app.borrow().dialogs.help_open);
}

#[test]
fn a_selection_change_keeps_the_bar_in_the_same_area() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = files_app(d.path(), 5);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 5);
    click_with(&mut h, "Item p0.jpg", CTRL);
    h.get_by_label("1 selected");
    let areas = |h: &Harness<'_>| h.ctx.memory(|m| m.areas().visible_layer_ids().len());
    let before = areas(&h);
    click_with(&mut h, "Item p1.jpg", CTRL);
    h.get_by_label("2 selected");
    assert_eq!(areas(&h), before, "no new area per count");
}
