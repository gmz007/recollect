//! Settings › Features: each switch turns its feature off on the main screens.

use super::*;
use crate::ui::guide::Tip;
use egui::accesskit::Role;

/// Turns the feature `title` off in Settings and closes Settings again.
fn turn_off(h: &mut Harness<'_>, title: &str) {
    open_section(h, "Features");
    h.get_by_role_and_label(Role::CheckBox, title).click();
    h.run_steps(3);
    h.get_by_label("Close settings").click();
    h.run_steps(3);
}

fn open_search_help(h: &mut Harness<'_>) {
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
}

#[test]
fn every_feature_is_on_by_default_and_the_switches_show_it() {
    let d = tempfile::tempdir().unwrap();
    let mut h = harness(library(d.path()));
    open_section(&mut h, "Features");
    for title in [
        "On this day",
        "Tips",
        "Remember recent searches",
        "Example searches",
        "Progress in the sidebar",
    ] {
        assert!(is_selected_role(&h, title), "{title} starts on");
    }
}

fn is_selected_role(h: &Harness<'_>, title: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, title)
        .accesskit_node()
        .toggled()
        .is_some_and(|t| format!("{t:?}") == "True")
}

#[test]
fn turning_off_on_this_day_hides_the_banner() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    h.get_by_label("On this day");
    turn_off(&mut h, "On this day");
    assert!(!lib.settings().show_on_this_day);
    assert!(h.query_by_label("On this day").is_none());
}

#[test]
fn turning_off_tips_hides_them() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label(Tip::Search.text());
    turn_off(&mut h, "Tips");
    assert!(h.query_by_label(Tip::Search.text()).is_none());
    assert!(h.query_by_label("Got it").is_none());
}

#[test]
fn turning_off_recent_searches_stops_offering_and_recording_them() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let lib = Arc::clone(&app.borrow().lib);
    app.borrow_mut().remember_search("snow");
    let mut h = album_harness(&app);
    open_search_help(&mut h);
    h.get_by_label("Recent search: snow");
    turn_off(&mut h, "Remember recent searches");
    app.borrow_mut().remember_search("dog");
    assert_eq!(
        lib.settings().recent_searches,
        ["snow"],
        "kept, not added to"
    );
    h.run_steps(2);
    open_search_help(&mut h);
    h.get_by_label("kids on the beach");
    assert!(h.query_by_label("Recent search: snow").is_none());
    // "Clear now" forgets what was kept.
    open_section(&mut h, "Features");
    h.get_by_label("Clear now").click();
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
}

#[test]
fn turning_off_example_searches_stops_offering_them() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().remember_search("snow");
    let mut h = album_harness(&app);
    open_search_help(&mut h);
    h.get_by_label("kids on the beach");
    turn_off(&mut h, "Example searches");
    h.run_steps(2);
    open_search_help(&mut h);
    h.get_by_label("Recent search: snow");
    assert!(h.query_by_label("kids on the beach").is_none());
}

#[test]
fn turning_off_sidebar_progress_hides_the_ring() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    let scanning = || crate::library::Activity::Scanning {
        folder: "/nowhere".into(),
    };
    lib.update_status(|s| s.activity = scanning());
    h.run_steps(3);
    h.get_by_label_contains("Progress:");
    // (A turning ring never lets the harness settle, so Settings is used while idle.)
    lib.update_status(|s| s.activity = crate::library::Activity::Idle);
    h.run_steps(3);
    turn_off(&mut h, "Progress in the sidebar");
    lib.update_status(|s| s.activity = scanning());
    h.run_steps(3);
    assert!(h.query_by_label_contains("Progress:").is_none());
}
