//! Settings, appearance and the smart-search switch.

use super::*;

#[test]
fn smart_search_is_offered_in_one_line_before_downloading() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    // One line: the offer, "Turn on", "Not now" and "Learn more".
    h.get_by_label("Turn on smart search to find photos by describing them");
    let on = h.get_by_label("Turn on").rect();
    let not_now = h.get_by_label("Not now").rect();
    let more = h.get_by_label("Learn more").rect();
    assert!(on.max.x <= not_now.min.x && not_now.max.x <= more.min.x);
    assert!((on.center().y - more.center().y).abs() < 4.0, "on one line");
    // The long explanation is gone (the size and privacy note is a tooltip).
    assert!(h.query_by_label_contains("one-time download of").is_none());
    assert!(h.query_by_label("Details").is_none());
    // "Learn more" opens Settings on Smart search.
    h.get_by_label("Learn more").click();
    h.run_steps(3);
    assert!(app.borrow().dialogs.settings_open);
    assert_eq!(
        app.borrow().dialogs.settings_section,
        crate::ui::settings::Section::SmartSearch
    );
    h.get_by_label("Download and turn on (608 MB)");
}

#[test]
fn declining_the_download_is_remembered_and_leaves_a_small_invitation() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    assert!(!lib.settings().search_prompt_dismissed);
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label("Smart search is off");
    assert!(h.query_by_label("Not now").is_none(), "the banner is gone");
    // Saved on disk, so it stays gone after a restart.
    let saved = crate::config::Settings::load(&lib.paths().settings_path()).unwrap();
    assert!(saved.search_prompt_dismissed);
    drop(h);
    let mut restarted = App::new(&egui::Context::default(), Arc::clone(&lib));
    restarted.frame_stats = None;
    restarted.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    restarted.inject_timeline(spread_items());
    let restarted = Rc::new(RefCell::new(restarted));
    let h2 = album_harness(&restarted);
    assert!(h2.query_by_label("Not now").is_none());
    h2.get_by_label("Smart search is off");
    drop(h2);
    // "Turn on" in the sidebar brings the offer back.
    let mut h = album_harness(&restarted);
    h.get_by_label("Turn on").click();
    h.run_steps(3);
    h.get_by_label("Not now");
    assert!(h.query_by_label("Smart search is off").is_none());
    assert!(!lib.settings().search_prompt_dismissed);
}

#[test]
fn clicking_the_disabled_search_field_opens_settings() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    h.get_by_label(crate::ui::chrome::SEARCH_OFF_LABEL).click();
    h.run();
    assert!(app.borrow().dialogs.settings_open);
    assert_eq!(
        app.borrow().dialogs.settings_section,
        crate::ui::settings::Section::SmartSearch
    );
    h.get_all_by_label("Smart search").next().unwrap();
    h.get_by_label("Download and turn on (608 MB)");
}

#[test]
fn empty_library_invites_adding_a_folder() {
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(lib);
    h.get_by_label("Add a folder with your photos and videos to get started.");
    h.get_by_label("No folders yet.");
    // The button in the middle and the + in the sidebar do the same thing.
    h.get_all_by_label("Add a folder").last().unwrap().click();
    h.run();
    h.get_by_label("or type a folder path");
    h.get_by_label("Choose a folder…");
}

#[test]
fn settings_sheet_lists_its_sections() {
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(lib);
    h.get_by_label("Settings").click();
    h.run();
    // Appearance is first; every section can be reached from the list on the left.
    h.get_by_label("Appearance");
    for section in [
        "Appearance",
        "Features",
        "Smart search",
        "Library",
        "Storage",
        "About",
    ] {
        h.get_by_label(&format!("{section} settings"));
    }
    // The list switches the section on the right.
    h.get_by_label("Features settings").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Progress in the sidebar");
    h.get_by_label("Library settings").click();
    h.run();
    h.get_by_label("Add a folder…");
    h.get_by_label("Close settings").click();
    h.run();
    assert!(h.query_by_label("Close settings").is_none());
}

#[test]
fn the_settings_shortcut_opens_settings() {
    let d = tempfile::tempdir().unwrap();
    let mut h = harness(library(d.path()));
    assert!(h.query_by_label("Close settings").is_none());
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::Comma]);
    h.run_steps(3);
    h.get_by_label("Close settings");
}

#[test]
fn appearance_changes_apply_at_once() {
    use crate::config::{TextSize, Theme};
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    let mut h = harness(Arc::clone(&lib));
    open_section(&mut h, "Appearance");
    h.get_by_label("System");
    h.get_by_label("Dark").click();
    h.run();
    assert_eq!(lib.settings().theme, Theme::Dark);
    assert!(h.ctx.global_style().visuals.dark_mode);
    h.get_by_label("Light").click();
    h.run();
    assert_eq!(lib.settings().theme, Theme::Light);
    // Photo size has Small, Medium and Large too; the text size row comes first.
    h.get_all_by_label("Large").next().unwrap().click();
    h.run();
    assert_eq!(lib.settings().text_size, TextSize::Large);
    assert!((h.ctx.zoom_factor() - 15.0 / 13.0).abs() < 1e-6);
    h.get_all_by_label("Medium").next().unwrap().click();
    h.run();
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-6);
    // The choices are saved to disk, not only kept in memory.
    let saved = crate::config::Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(
        (saved.theme, saved.text_size),
        (Theme::Light, TextSize::Medium)
    );
}

#[test]
fn smart_search_settings_show_status_and_confirm_a_switch() {
    use crate::library::Activity;
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(Arc::clone(&lib));
    open_section(&mut h, "Smart search");
    h.get_by_label("On — faster");
    h.get_by_label("Best quality");
    h.get_by_label("Faster");
    h.get_by_label_contains("Quicker on older computers. 608 MB.");
    h.get_by_role_and_label(
        egui::accesskit::Role::CheckBox,
        "Use the graphics card when available",
    );
    h.get_by_label("Details");
    assert!(h.query_by_label_contains("huggingface").is_none());
    // The graphics card option is saved at once.
    h.get_by_role_and_label(
        egui::accesskit::Role::CheckBox,
        "Use the graphics card when available",
    )
    .click();
    h.run();
    assert!(!lib.settings().use_gpu);
    // Status in words while things happen.
    lib.update_status(|s| {
        s.activity = Activity::Downloading {
            model: "m".into(),
            done: 120_000_000,
            total: 485_000_000,
        }
    });
    h.run_steps(2);
    h.get_by_label("Downloading… 120 of 485 MB");
    lib.update_status(|s| s.activity = Activity::LoadingModel);
    h.run_steps(2);
    h.get_by_label("Getting your photos ready…");
    lib.update_status(|s| s.activity = Activity::Idle);
    h.run_steps(2);
    // Choosing the other option asks first and explains what happens.
    h.get_by_label("Best quality").click();
    h.run_steps(3);
    h.get_by_label_contains("Change to Best quality?");
    h.get_by_label_contains("look through your photos again");
    h.get_by_label_contains("one-time download of 486 MB");
    assert_eq!(lib.model().id, registry::CLIP_B32.id, "asks first");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Change to Best quality?")
            .is_none()
    );
    assert_eq!(lib.model().id, registry::CLIP_B32.id, "nothing changed yet");
    h.get_by_label("Best quality").click();
    h.run_steps(3);
    h.get_by_label("Change").click();
    settle(&mut h, || lib.model().id == registry::SIGLIP_B16.id);
    assert_eq!(
        lib.settings().model.as_deref(),
        Some(registry::SIGLIP_B16.id)
    );
}

#[test]
fn smart_search_can_be_turned_on_from_settings() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    open_section(&mut h, "Smart search");
    h.get_by_label("Off");
    h.get_by_label("Download and turn on (608 MB)");
}

#[test]
fn library_settings_show_counts_and_retry_unreadable_files() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 3);
    let mut h = quick_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    lib.wait();
    let ids: Vec<i64> = ["p0.jpg", "p1.jpg", "p2.jpg"]
        .iter()
        .map(|n| file_id_of(&lib, n))
        .collect();
    {
        let db = lib.db.lock();
        for id in &ids {
            db.set_error(*id, "broken").unwrap();
        }
    }
    app.borrow_mut().refresh_snapshot(true);
    settle(&mut h, || app.borrow().snapshot.stats.errors == 3);
    open_section(&mut h, "Library");
    assert!(
        h.get_all_by_label("nowhere").count() >= 2,
        "sidebar and settings"
    );
    h.get_by_label("3 photos · 3 couldn't be opened");
    h.get_by_label("Stop showing “nowhere”…");
    h.get_by_label("Add a folder…");
    assert!(
        h.query_by_label("Try again").is_none(),
        "hidden until asked for"
    );
    h.get_by_label("Show").click();
    h.run_steps(3);
    h.get_by_label("p0.jpg");
    h.get_by_label("Hide");
    let before = lib.jobs_finished();
    h.get_by_label("Try again").click();
    settle(&mut h, || lib.jobs_finished() > before);
    // The folder of these files does not exist; the finished run says so, and nothing else.
    assert_eq!(
        app.borrow().notices.current(),
        Some("Some folders couldn't be reached, so they were left as they were.")
    );
    // Check for new photos is offered here too (the sidebar has a button of the same name).
    assert!(
        !h.get_all_by_label("Check for new photos")
            .collect::<Vec<_>>()
            .is_empty()
    );
    // Stopping a folder asks the same question as in the sidebar.
    h.get_by_label("Stop showing “nowhere”…").click();
    h.run_steps(3);
    h.get_by_label("Stop showing “nowhere”?");
    h.get_by_label("Your files stay where they are.");
    assert_eq!(lib.folders().unwrap().len(), 1);
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Your files stay where they are.")
            .is_none()
    );
    assert_eq!(lib.folders().unwrap().len(), 1);
    assert!(app.borrow().dialogs.settings_open, "the sheet stays open");
}

#[test]
fn storage_settings_set_the_preview_space_and_show_where_data_lives() {
    use egui_kittest::kittest::Queryable as _;
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    let mut h = harness(Arc::clone(&lib));
    open_section(&mut h, "Storage");
    assert!(h.get_all_by_label("Space for previews").count() >= 1);
    h.get_by_label("Where Recollect keeps its data");
    h.get_by_label("Open folder for library");
    h.get_by_label("Open folder for previews");
    h.get_by_label("Open folder for settings");
    h.get_by_label(&lib.paths().data.display().to_string());
    assert_eq!(lib.settings().thumb_cache_mb, 1024);
    h.get_by_role(egui::accesskit::Role::Slider)
        .set_value_accesskit(2048.0);
    h.run_steps(3);
    assert_eq!(lib.settings().thumb_cache_mb, 2048);
    let saved = crate::config::Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(saved.thumb_cache_mb, 2048);
}

#[test]
fn about_states_the_privacy_promise_and_the_version() {
    let d = tempfile::tempdir().unwrap();
    let mut h = harness(library(d.path()));
    open_section(&mut h, "About");
    h.get_by_label(&format!("Version {}", env!("CARGO_PKG_VERSION")));
    h.get_by_label("Your photos stay private");
    h.get_by_label_contains("Your photos never leave this computer.");
    h.get_by_label_contains("never changes or deletes your files");
    h.get_by_label_contains("one-time download for smart search");
    h.get_by_label("Open-source licenses").click();
    h.run_steps(3);
    h.get_by_label("Inter typeface: SIL Open Font License 1.1");
    h.get_by_label("Material Symbols icons: Apache License 2.0");
    h.get_by_label("Photos on the first screen: Unsplash, public domain (CC0)");
    h.get_by_label("ONNX Runtime: MIT License");
    h.get_by_label("FFmpeg: GNU LGPL");
}

#[test]
fn escape_closes_the_sheet_and_the_topmost_question_first() {
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(lib);
    open_section(&mut h, "Smart search");
    h.get_by_label("Best quality").click();
    h.run_steps(3);
    h.get_by_label_contains("Change to Best quality?");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Change to Best quality?")
            .is_none()
    );
    h.get_by_label("Close settings");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Close settings").is_none());
}

#[test]
fn the_theme_and_text_size_settings_apply() {
    use crate::config::{TextSize, Theme};
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    let mut h = harness(Arc::clone(&lib));
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-6);
    let set = |theme, text_size| {
        lib.update_settings(|s| {
            s.theme = theme;
            s.text_size = text_size;
        })
        .unwrap();
    };
    set(Theme::Light, TextSize::Large);
    h.run();
    assert!(!h.ctx.global_style().visuals.dark_mode);
    assert_eq!(
        h.ctx.global_style().visuals.panel_fill,
        egui::Color32::WHITE
    );
    assert!((h.ctx.zoom_factor() - 15.0 / 13.0).abs() < 1e-6);
    set(Theme::Dark, TextSize::Medium);
    h.run();
    assert!(h.ctx.global_style().visuals.dark_mode);
    assert_eq!(
        h.ctx.global_style().visuals.panel_fill,
        egui::Color32::from_rgb(0x16, 0x16, 0x16)
    );
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-6);
    // The text styles follow the spec.
    let style = h.ctx.global_style();
    assert_eq!(style.text_styles[&egui::TextStyle::Body].size, 13.0);
    assert_eq!(style.text_styles[&egui::TextStyle::Heading].size, 20.0);
    assert_eq!(style.text_styles[&egui::TextStyle::Small].size, 12.0);
    assert_eq!(style.text_styles[&crate::ui::style::section()].size, 14.0);
}
