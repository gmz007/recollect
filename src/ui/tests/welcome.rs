//! The first-run welcome.

use super::*;

/// Chooses `dir` on the welcome's folder screen, as if it had been added with the picker.
fn choose_a_folder(app: &Rc<RefCell<App>>, dir: &std::path::Path) {
    app.borrow_mut()
        .welcome
        .as_mut()
        .unwrap()
        .add_folder_for_test(dir.to_path_buf());
}

#[test]
fn the_welcome_leads_to_photos_in_four_clicks() {
    let d = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    // First screen: the promise, the benefits, one button. No top bar or sidebar yet.
    h.get_by_label("Find any photo by describing it");
    h.get_by_label_contains("Everything stays on this computer");
    assert!(h.query_by_label("Settings").is_none());
    assert!(app.borrow().welcome_active());
    h.get_by_label("Step 1 of 4");
    let mut clicks = 0;
    h.get_by_label("Get started").click();
    clicks += 1;
    h.run_steps(3);
    // Second: with nothing chosen Continue is off and says why; typing a path (under More
    // options) adds one (not a "primary" click).
    h.get_by_label("Where are your photos?");
    h.get_by_label("Step 2 of 4");
    assert!(h.get_by_label("Continue").accesskit_node().is_disabled());
    h.get_by_label("Pick at least one folder to continue.");
    assert!(
        h.query_by_label_contains("/home/").is_none(),
        "no raw paths"
    );
    h.get_by_label("We didn't find a Pictures folder. Choose where your photos are.");
    assert!(h.query_by_label("Skip").is_none(), "no separate Skip");
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("Find any photo by describing it");
    h.get_by_label("Get started").click();
    h.run_steps(3);
    assert!(h.query_by_label("Folder path").is_none(), "folded away");
    h.get_by_label("More options").click();
    h.run_steps(3);
    h.get_by_label("Folder path").click();
    h.run_steps(2);
    h.get_by_label("Folder path")
        .type_text(&photos.path().display().to_string());
    h.run_steps(2);
    h.get_by_label("Add this folder").click();
    h.run_steps(3);
    assert!(!h.get_by_label("Continue").accesskit_node().is_disabled());
    assert_eq!(
        app.borrow().welcome.as_ref().unwrap().chosen_folders(),
        [photos.path().to_path_buf()]
    );
    h.get_by_label("Continue").click();
    clicks += 1;
    h.run_steps(3);
    // Third: the two choices with their sizes, privacy, and a way out.
    h.get_by_label("Turn on smart search");
    h.get_by_label("Best quality");
    h.get_by_label("Faster");
    h.get_by_label_contains("never leave this computer");
    h.get_by_label("Turn on");
    assert!(h.query_by_label("Skip for now").is_none() && h.query_by_label("Skip setup").is_none());
    h.get_by_label("Not now").click();
    clicks += 1;
    h.run_steps(3);
    // Fourth: what happens next.
    h.get_by_label("You're all set");
    h.get_by_label_contains("After you press Start, Recollect gets your photos ready.");
    h.get_by_label_contains("you can browse right away");
    assert!(
        app.borrow().welcome.is_some(),
        "nothing is added before Start"
    );
    assert!(lib.folders().unwrap().is_empty());
    h.get_by_label("Start").click();
    clicks += 1;
    assert!(clicks <= 4);
    settle(&mut h, || lib.folders().unwrap().len() == 1);
    h.run_steps(3);
    // Photos is shown, with the usual window around it.
    assert!(!app.borrow().welcome_active());
    assert_eq!(app.borrow().tl.view, View::Timeline);
    assert!(h.query_all_by_label("Photos").next().is_some());
    h.get_by_label("Settings");
    assert!(lib.settings().welcome_done);
    assert!(!lib.model_installed(), "skipping downloads nothing");
}

#[test]
fn the_welcome_can_be_skipped_and_existing_users_never_see_it() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    h.get_by_label("I'll set it up myself").click();
    h.run_steps(3);
    assert!(!app.borrow().welcome_active());
    // The empty library invites adding a folder instead.
    h.get_by_label("Add a folder with your photos and videos to get started.");
    let saved = crate::config::Settings::load(&lib.paths().settings_path()).unwrap();
    assert!(saved.welcome_done, "saved on disk");
    drop(h);
    drop(app);
    let again = App::new(&egui::Context::default(), Arc::clone(&lib));
    assert!(again.welcome.is_none(), "not shown again after a restart");

    // Someone who already has folders is never welcomed, and is marked as done.
    let d2 = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let lib2 = library(d2.path());
    lib2.add_folder(folder.path()).unwrap();
    assert!(!lib2.settings().welcome_done);
    let existing = App::new(&egui::Context::default(), Arc::clone(&lib2));
    assert!(existing.welcome.is_none());
    assert!(lib2.settings().welcome_done);
}

#[test]
fn continuing_without_ticks_adds_nothing_and_ticked_folders_are_added_exactly() {
    let d = tempfile::tempdir().unwrap();
    let pictures = tempfile::tempdir().unwrap();
    let desktop = tempfile::tempdir().unwrap();
    let offered = || {
        vec![
            ("Pictures".to_string(), pictures.path().to_path_buf()),
            ("Desktop".to_string(), desktop.path().to_path_buf()),
        ]
    };
    // Untick Pictures: nothing is chosen, the button says so, and nothing is added.
    let app = fresh_app_with(d.path(), offered());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    h.get_by_label("Get started").click();
    h.run_steps(3);
    // Friendly names only, never the path.
    h.get_by_label("Pictures");
    h.get_by_label("Desktop");
    assert!(
        h.query_by_label_contains(&pictures.path().display().to_string())
            .is_none()
    );
    h.get_by_label("Continue");
    h.get_by_label("Pictures").click();
    h.run_steps(3);
    assert!(
        app.borrow()
            .welcome
            .as_ref()
            .unwrap()
            .chosen_folders()
            .is_empty()
    );
    assert!(h.get_by_label("Continue").accesskit_node().is_disabled());
    h.get_by_label("Pick at least one folder to continue.");
    // Clicking the disabled button goes nowhere.
    h.get_by_label("Continue").click();
    h.run_steps(3);
    h.get_by_label("Where are your photos?");
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("I'll set it up myself").click();
    settle(&mut h, || lib.settings().welcome_done);
    lib.wait();
    assert!(lib.folders().unwrap().is_empty());
    drop(h);

    // Pictures and Desktop ticked: exactly those two.
    let d2 = tempfile::tempdir().unwrap();
    let app = fresh_app_with(d2.path(), offered());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    h.get_by_label("Get started").click();
    h.run_steps(3);
    h.get_by_label("Desktop").click();
    h.run_steps(3);
    h.get_by_label("Continue").click();
    h.run_steps(3);
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label("Start").click();
    settle(&mut h, || lib.folders().unwrap().len() == 2);
    lib.wait();
    let mut got: Vec<_> = lib.folders().unwrap().into_iter().map(|f| f.path).collect();
    got.sort();
    let mut want = vec![
        crate::library::canonical_path(pictures.path()).unwrap(),
        crate::library::canonical_path(desktop.path()).unwrap(),
    ];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn the_folders_screen_explains_a_missing_pictures_folder_and_names_added_folders() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    let mut h = album_harness(&app);
    h.get_by_label("Get started").click();
    h.run_steps(3);
    h.get_by_label("We didn't find a Pictures folder. Choose where your photos are.");
    h.get_by_label("Add a folder…");
    // With a Pictures folder the sentence is not shown.
    let d2 = tempfile::tempdir().unwrap();
    let pics = tempfile::tempdir().unwrap();
    let app = fresh_app_with(
        d2.path(),
        vec![("Pictures".into(), pics.path().to_path_buf())],
    );
    let mut h2 = album_harness(&app);
    h2.get_by_label("Get started").click();
    h2.run_steps(3);
    assert!(
        h2.query_by_label_contains("didn't find a Pictures")
            .is_none()
    );
    // A folder the person added shows its name, with the path as small text under it.
    let added = pics.path().join("Holiday 2024");
    std::fs::create_dir_all(&added).unwrap();
    app.borrow_mut()
        .welcome
        .as_mut()
        .unwrap()
        .add_folder_for_test(added.clone());
    h2.run_steps(3);
    h2.get_by_label("Holiday 2024");
    h2.get_by_label(&added.display().to_string());
    drop(h);
}

#[test]
fn nested_choices_keep_the_parent_only() {
    let d = tempfile::tempdir().unwrap();
    let parent = d.path().join("Photos");
    let child = parent.join("2024");
    let other = d.path().join("Videos");
    let w = crate::ui::welcome::Welcome::with_suggestions(
        vec![
            ("Pictures".into(), child),
            ("Videos".into(), other.clone()),
            ("Desktop".into(), parent.clone()),
        ],
        registry::CLIP_B32.id,
    );
    let mut w = w;
    w.add_folder_for_test(parent.clone());
    assert_eq!(w.chosen_folders(), [other, parent]);
    // The same rule, directly.
    assert_eq!(
        crate::ui::folders::without_nested(&["/a/b".into(), "/a".into(), "/c".into(), "/a".into()]),
        [std::path::PathBuf::from("/a"), "/c".into()]
    );
}

#[test]
fn the_computers_hardware_is_asked_once_not_every_frame() {
    use std::sync::atomic::Ordering;
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    choose_a_folder(&app, d.path());
    let mut h = album_harness(&app);
    h.get_by_label("Get started").click();
    h.run_steps(3);
    h.get_by_label_contains("Continue").click();
    h.run_steps(3);
    h.get_by_label("Turn on smart search");
    let asked = crate::library::HARDWARE_CHECKS.load(Ordering::Relaxed);
    h.run_steps(30);
    assert_eq!(
        crate::library::HARDWARE_CHECKS.load(Ordering::Relaxed),
        asked
    );
}

#[test]
fn every_step_after_the_first_has_back_and_the_first_can_be_skipped_in_one_way() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    choose_a_folder(&app, d.path());
    let mut h = album_harness(&app);
    assert!(h.query_by_label("Back").is_none());
    h.get_by_label("I'll set it up myself");
    assert!(h.query_by_label("Skip setup").is_none());
    h.get_by_label("Get started").click();
    h.run_steps(3);
    h.get_by_label("Back");
    assert!(h.query_by_label("I'll set it up myself").is_none());
    h.get_by_label_contains("Continue").click();
    h.run_steps(3);
    h.get_by_label("Back");
    h.get_by_label("Not now");
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label("Back");
    h.get_by_label_contains("After you press Start");
    assert!(h.query_by_label_contains("Search works once").is_none());
    // After choosing the download the same screen adds one sentence.
    h.get_by_label("Back").click();
    h.run_steps(3);
    h.get_by_label("Turn on").click();
    h.run_steps(3);
    h.get_by_label_contains("Search works once the download finishes.");
}

#[test]
fn a_failed_model_switch_at_start_keeps_the_welcome_and_says_so() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    choose_a_folder(&app, d.path());
    let lib = Arc::clone(&app.borrow().lib);
    // A choice the library does not know makes the switch fail at once.
    app.borrow_mut()
        .welcome
        .as_mut()
        .unwrap()
        .choose_for_test("no-such-model");
    let mut h = album_harness(&app);
    h.get_by_label("Get started").click();
    h.run_steps(3);
    h.get_by_label_contains("Continue").click();
    h.run_steps(3);
    h.get_by_label("Turn on").click();
    h.run_steps(3);
    h.get_by_label("Start").click();
    h.run_steps(3);
    h.get_by_label("Couldn't set up smart search. Please try again in a moment.");
    assert!(app.borrow().welcome_active(), "still on the last screen");
    assert!(!lib.settings().welcome_done);
}

#[test]
fn the_buttons_stay_where_they_are_on_every_step() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    choose_a_folder(&app, d.path());
    let mut h = album_harness(&app);
    let primary = ["Get started", "Continue", "Turn on", "Start"];
    let mut rects = Vec::new();
    for (i, label) in primary.iter().enumerate() {
        h.get_by_label(&format!("Step {} of 4", i + 1));
        let back = (i > 0).then(|| h.get_by_label("Back").rect());
        rects.push((back, h.get_by_label(label).rect()));
        h.get_by_label(label).click();
        h.run_steps(3);
    }
    // The first step has no Back; on the others it is always in one place.
    assert!(rects[0].0.is_none());
    assert!(rects[1..].windows(2).all(|p| p[0].0 == p[1].0));
    assert!(rects.windows(2).all(|p| p[0].1 == p[1].1), "{rects:?}");
}

#[test]
fn not_now_finishes_setup_without_smart_search() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    choose_a_folder(&app, d.path());
    let mut h = album_harness(&app);
    h.get_by_label("Get started").click();
    h.run_steps(3);
    h.get_by_label("Continue").click();
    h.run_steps(3);
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label("Off for now");
    h.get_by_label("Start").click();
    settle(&mut h, || lib.settings().welcome_done);
    assert!(!lib.model_installed());
}

#[test]
fn the_mosaic_shows_in_wide_windows_only_and_follows_the_theme() {
    use crate::config::Theme;
    use crate::ui::welcome::mosaic::PhotoSet;
    let set = |app: &Rc<RefCell<App>>| app.borrow().welcome.as_ref().unwrap().mosaic_set_for_test();
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
    app.borrow_mut().change_settings(|s| s.theme = Theme::Light);
    let mut h = album_harness(&app);
    h.run_steps(3);
    assert_eq!(set(&app), Some(PhotoSet::Light));
    app.borrow_mut().change_settings(|s| s.theme = Theme::Dark);
    h.run_steps(5);
    assert_eq!(set(&app), Some(PhotoSet::Dark));
    drop(h);

    // A narrow window shows the content alone, and the photos are not even decoded.
    let d2 = tempfile::tempdir().unwrap();
    let app = fresh_app(d2.path());
    let shown = Rc::clone(&app);
    let mut narrow = Harness::builder()
        .with_size(egui::vec2(700.0, 800.0))
        .with_max_steps(MAX_STEPS)
        .build_ui(move |ui| shown.borrow_mut().show(ui));
    narrow.run_steps(5);
    narrow.get_by_label("Find any photo by describing it");
    assert_eq!(set(&app), None);
}
