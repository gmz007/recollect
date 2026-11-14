//! Headless UI tests (`egui_kittest`). No model or GPU needed.
#![expect(clippy::cast_possible_wrap, reason = "small test fixture indices")]

use super::App;
use crate::Library;
use crate::config::Paths;
use crate::model::{download, registry};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::sync::Arc;

fn library(dir: &std::path::Path) -> Arc<Library> {
    crate::gpu::force_cpu(true);
    Library::open(Paths::under(dir), Some(registry::CLIP_B32.id)).unwrap()
}

/// Creates sparse placeholder model files so the app believes the model is installed.
fn fake_install(lib: &Library) {
    let dir = download::model_dir(&lib.paths().models_dir(), lib.model());
    std::fs::create_dir_all(&dir).unwrap();
    for f in lib.model().files {
        std::fs::File::create(dir.join(f.local))
            .unwrap()
            .set_len(f.size)
            .unwrap();
    }
    assert!(lib.model_installed());
}

fn harness(lib: Arc<Library>) -> Harness<'static> {
    // Most tests are about the app itself, not the first-run welcome.
    lib.update_settings(|s| s.welcome_done = true).unwrap();
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.show(ui));
    h.run();
    h
}

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
    assert!(app.borrow().settings_open);
    assert_eq!(
        app.borrow().settings_section,
        super::settings::Section::SmartSearch
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
    h.get_by_label(super::chrome::SEARCH_OFF_LABEL).click();
    h.run();
    assert!(app.borrow().settings_open);
    assert_eq!(
        app.borrow().settings_section,
        super::settings::Section::SmartSearch
    );
    h.get_by_label("Smart search");
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
    for section in ["Appearance", "Smart search", "Library", "Storage", "About"] {
        h.get_by_label(&format!("{section} settings"));
    }
    h.get_by_label("Close settings").click();
    h.run();
    assert!(h.query_by_label("Close settings").is_none());
}

/// Opens Settings on `section` by its list entry.
fn open_section(h: &mut Harness<'_>, section: &str) {
    if h.query_by_label("Close settings").is_none() {
        h.get_by_label("Settings").click();
        h.run();
    }
    h.get_by_label(&format!("{section} settings")).click();
    h.run();
}

#[test]
fn appearance_changes_apply_at_once() {
    use crate::config::{TextSize, Theme};
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    let mut h = harness(Arc::clone(&lib));
    open_section(&mut h, "Appearance");
    h.get_by_label("Match my computer");
    h.get_by_label("Dark").click();
    h.run();
    assert_eq!(lib.settings().theme, Theme::Dark);
    assert!(h.ctx.global_style().visuals.dark_mode);
    h.get_by_label("Light").click();
    h.run();
    assert_eq!(lib.settings().theme, Theme::Light);
    h.get_by_label("Large").click();
    h.run();
    assert_eq!(lib.settings().text_size, TextSize::Large);
    assert!((h.ctx.zoom_factor() - 1.15).abs() < 1e-6);
    h.get_by_label("Normal").click();
    h.run();
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-6);
    // The choices are saved to disk, not only kept in memory.
    let saved = crate::config::Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(
        (saved.theme, saved.text_size),
        (Theme::Light, TextSize::Normal)
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
    let described = |h: &Harness<'_>, title: &str| {
        h.get_by_label(title)
            .accesskit_node()
            .description()
            .unwrap_or_default()
    };
    assert_eq!(
        described(&h, "Best quality"),
        "Finds photos most reliably. Works best on computers with a graphics card. 486 MB."
    );
    assert_eq!(
        described(&h, "Faster"),
        "Quicker on older computers. 608 MB."
    );
    h.get_by_label("Use the graphics card when available");
    h.get_by_label("Details");
    assert!(h.query_by_label_contains("huggingface").is_none());
    // The graphics card option is saved at once.
    h.get_by_label("Use the graphics card when available")
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
    h.get_by_label("Switch to Best quality?");
    h.get_by_label_contains("look at all your photos again");
    h.get_by_label_contains("one-time download of 486 MB");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label("Switch to Best quality?").is_none());
    assert_eq!(lib.model().id, registry::CLIP_B32.id, "nothing changed yet");
    h.get_by_label("Best quality").click();
    h.run_steps(3);
    h.get_by_label("Switch").click();
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
    assert!(app.borrow().notices.current().is_none());
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
    assert!(app.borrow().settings_open, "the sheet stays open");
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
    h.get_by_label_contains("never leave it");
    h.get_by_label_contains("one-time download for smart search");
    h.get_by_label("Open-source licenses").click();
    h.run_steps(3);
    h.get_by_label("Inter typeface: SIL Open Font License 1.1");
    h.get_by_label("Material Symbols icons: Apache License 2.0");
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
    h.get_by_label("Switch to Best quality?");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Switch to Best quality?").is_none());
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
    assert!((h.ctx.zoom_factor() - 1.15).abs() < 1e-6);
    set(Theme::Dark, TextSize::Normal);
    h.run();
    assert!(h.ctx.global_style().visuals.dark_mode);
    assert_eq!(
        h.ctx.global_style().visuals.panel_fill,
        egui::Color32::from_rgb(0x12, 0x12, 0x14)
    );
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-6);
    // The text styles follow the spec.
    let style = h.ctx.global_style();
    assert_eq!(style.text_styles[&egui::TextStyle::Body].size, 14.5);
    assert_eq!(style.text_styles[&egui::TextStyle::Heading].size, 22.0);
    assert_eq!(style.text_styles[&egui::TextStyle::Small].size, 12.5);
    assert_eq!(style.text_styles[&super::style::section()].size, 17.0);
}

fn hit(path: &str, kind: crate::media::MediaKind, t: Option<f64>) -> crate::search::SearchHit {
    crate::search::SearchHit {
        item_id: 1,
        file_id: 1,
        path: path.into(),
        kind,
        mtime: 0,
        taken_at: 0,
        favorite: false,
        score: 0.31,
        t_start: t,
        t_end: t.map(|t| t + 2.0),
        width: Some(1280),
        height: Some(720),
        duration: t.map(|_| 600.0),
        moments: t
            .map(|_| {
                vec![crate::search::Moment {
                    item_id: 2,
                    t_start: 100.0,
                    t_end: 104.0,
                    score: 0.2,
                }]
            })
            .unwrap_or_default(),
    }
}

#[test]
fn results_show_a_plain_header_and_square_tiles() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    // No real folder: adding one would start a background scan that races the test.
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.results = Arc::new(vec![
        hit("/nowhere/movie.mp4", MediaKind::Video, Some(42.0)),
        hit("/nowhere/photo.jpg", MediaKind::Image, None),
    ]);
    app.current = Some(super::Query::Text("a dragon".into()));
    app.result_label = "a dragon".into();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.show(ui));
    h.run();
    h.get_by_label("Results for “a dragon”");
    h.get_by_label("1 photo and 1 video");
    h.get_by_label("Result movie.mp4");
    h.get_by_label("Result photo.jpg");
    h.get_by_label("Clear search");
    assert!(h.query_by_label_contains("score").is_none());
    assert!(h.query_by_label_contains(" ms").is_none());
}

/// Whether the tile labelled `label` is selected, as assistive technology sees it.
fn is_selected(h: &Harness<'_>, label: &str) -> bool {
    h.get_by_label(label)
        .accesskit_node()
        .toggled()
        .is_some_and(|t| format!("{t:?}") == "True")
}

#[test]
fn arrow_keys_move_the_selection() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    assert!(!is_selected(&h, "Result c.jpg"));
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result a.jpg"));
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result c.jpg"));
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    assert!(is_selected(&h, "Result b.jpg"));
    assert!(!is_selected(&h, "Result c.jpg"));
}

/// Three image results, nothing selected, ready for keyboard tests.
fn grid_app(d: &std::path::Path) -> App {
    grid_app_with(d, true)
}

/// `installed` false leaves the model missing, so a re-run search fails cleanly instead of
/// loading placeholder files into ONNX Runtime.
fn grid_app_with(d: &std::path::Path, installed: bool) -> App {
    use crate::media::MediaKind;
    let lib = library(d);
    if installed {
        fake_install(&lib);
    }
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.results = Arc::new(
        ["a.jpg", "b.jpg", "c.jpg"]
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut h = hit(&format!("/nowhere/{n}"), MediaKind::Image, None);
                h.file_id = i as i64 + 1;
                h
            })
            .collect(),
    );
    app.current = Some(super::Query::Text("a dragon".into()));
    app.result_label = "a dragon".into();
    app
}

fn grid_harness(app: App) -> Harness<'static> {
    let mut app = app;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.show(ui));
    h.run();
    h
}

#[test]
fn typing_in_search_blocks_arrows_until_focus_leaves() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap()
        .click();
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(!is_selected(&h, "Result a.jpg"));
    h.key_press(egui::Key::Escape);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result a.jpg"));
}

#[test]
fn modified_arrows_do_not_move_the_selection() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    h.run();
    assert!(!is_selected(&h, "Result a.jpg"));
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result a.jpg"));
}

#[test]
fn arrows_are_ignored_while_settings_is_open() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Settings").click();
    h.run();
    h.get_by_label("Appearance");
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(!is_selected(&h, "Result a.jpg"));
}

#[test]
fn clicking_a_tile_keeps_arrows_working() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Result a.jpg")
        .click_modifiers(egui::Modifiers::COMMAND);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result b.jpg"));
    assert!(!is_selected(&h, "Result a.jpg"));
}

// ---- Album views -------------------------------------------------------------------------

use super::timeline::{Group, View};
use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::store::db::TimelineItem;
use std::cell::RefCell;
use std::rc::Rc;

fn titem(id: i64, y: i32, m: u32, d: u32) -> TimelineItem {
    let taken_at = chrono::NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    TimelineItem {
        file_id: id,
        item_id: Some(id),
        path: format!("/nowhere/img{id}.jpg").into(),
        kind: MediaKind::Image,
        mtime: 0,
        taken_at,
        date_source: DateSource::Exif,
        duration: None,
        width: Some(4000),
        height: Some(3000),
        favorite: false,
        camera: Some("Test Camera".into()),
    }
}

/// One photo per month from 2000 through 2019, newest first.
fn spread_items() -> Vec<TimelineItem> {
    let mut v = Vec::new();
    for y in (2000..2020).rev() {
        for m in (1..=12).rev() {
            v.push(titem(v.len() as i64 + 1, y, m, 15));
        }
    }
    v
}

fn album_app(d: &std::path::Path, installed: bool) -> Rc<RefCell<App>> {
    let lib = library(d);
    if installed {
        fake_install(&lib);
    }
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.inject_timeline(spread_items());
    Rc::new(RefCell::new(app))
}

fn album_harness(app: &Rc<RefCell<App>>) -> Harness<'static> {
    let app = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.borrow_mut().show(ui));
    h.run();
    h
}

#[test]
fn timeline_shows_month_headers_and_regroups() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("December 2019 · 1");
    h.get_by_label("240 photos");
    h.get_by_label("Thumbnail size");
    h.get_by_label("Sort: Newest first");
    // The "Months" menu regroups the timeline.
    h.get_by_label("Months").click();
    h.run();
    h.get_by_label("Years").click();
    h.run();
    h.get_by_label("2019 · 12");
    assert!(h.query_by_label("December 2019 · 1").is_none());
    h.get_by_label("Years").click();
    h.run();
    h.get_by_label("Days").click();
    h.run();
    h.get_by_label("Sunday 15 December 2019 · 1");
    // The sort menu says what it is set to and flips the order.
    h.get_by_label("Sort: Newest first").click();
    h.run_steps(8);
    h.get_by_label("Oldest first").click();
    h.run_steps(8);
    h.get_by_label("Sort: Oldest first");
    assert!(!app.borrow().tl.newest_first);
}

#[test]
fn navigator_jumps_to_a_year() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    assert!(h.query_by_label("December 2005 · 1").is_none());
    // The date list is folded away until asked for.
    assert!(h.query_by_label("2005 (12)").is_none());
    h.get_by_label("Jump to date").click();
    h.run();
    // The sidebar's date list is short in a window this size: scroll the year into view.
    h.get_by_label("2005 (12)").scroll_to_me();
    h.run();
    h.get_by_label("2005 (12)").click();
    h.run();
    h.run();
    h.get_by_label("December 2005 · 1");
    assert!(h.query_by_label("December 2019 · 1").is_none());
    // The year stays expanded and its months are listed.
    h.get_by_label("June (1)");
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
    h.get_by_label("December 2019 · 1");
    assert_eq!(app.borrow().tl.group, Group::Month);
}

#[test]
fn clicking_a_tile_opens_the_viewer_with_the_details() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Item img1.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some());
    h.get_by_label("img1.jpg");
    h.get_by_label_contains("Sunday 15 December 2019");
    h.get_by_label("Test Camera");
    h.get_by_label("4000×3000");
    h.get_by_label("Find similar photos");
    h.get_by_label("Open with default app");
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

// ---- Viewer, favorites, search options, On this day ----------------------------------------

/// Runs frames until `cond` holds (background threads need a moment); panics after ~5 s.
fn settle(h: &mut Harness<'_>, mut cond: impl FnMut() -> bool) {
    for _ in 0..500 {
        h.run_steps(2);
        if cond() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("condition not reached in time");
}

/// Like `album_harness` with short frames.
fn quick_harness(app: &Rc<RefCell<App>>) -> Harness<'static> {
    let app = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_step_dt(0.05)
        .build_ui(move |ui| app.borrow_mut().show(ui));
    h.run();
    h
}

/// Clicks a tile and checks that the viewer opened (so the tile is gone).
fn open_tile(h: &mut Harness<'_>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
    assert!(
        h.query_by_label(label).is_none(),
        "the viewer should be open"
    );
}

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
    assert_eq!(app.borrow().tl.selected, Some(1));
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
    assert_eq!(app.borrow().tl.selected, Some(0));
}

#[test]
fn viewer_from_a_search_hit_offers_the_moments_of_a_video() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    let results = Arc::make_mut(&mut app.results);
    results[0] = hit("/nowhere/clip.mp4", MediaKind::Video, Some(42.0));
    for (i, h) in results.iter_mut().enumerate() {
        h.file_id = i as i64 + 1;
    }
    app.selected = Some(0);
    app.selected_t = Some(42.0);
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.get_by_label("clip.mp4");
    h.get_by_label("Play at 0:42");
    h.get_by_label("Matching moments");
    h.get_by_label("0:42 – 0:44");
    h.get_by_label("1:40 – 1:44").click();
    h.run_steps(3);
    h.get_by_label("Play at 1:40");
    // Next goes to the following hit (an image).
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("b.jpg");
    assert!(h.query_by_label("Matching moments").is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(app.borrow().selected, Some(1));
}

/// A library with three dated files in the database, so favorites can round-trip through it.
fn favorites_app(d: &std::path::Path) -> (Arc<Library>, Rc<RefCell<App>>) {
    use crate::media::ScannedFile;
    use crate::media::metadata::{Capture, DateSource};
    let lib = library(d);
    {
        let mut db = lib.db.lock();
        let f = db.add_folder(std::path::Path::new("/nowhere")).unwrap();
        let files: Vec<ScannedFile> = ["a", "b", "c"]
            .iter()
            .map(|n| ScannedFile {
                path: format!("/nowhere/{n}.jpg").into(),
                kind: MediaKind::Image,
                size: 1,
                mtime: 1,
            })
            .collect();
        db.sync_folder(f, &files, &[]).unwrap();
        let caps: Vec<_> = db
            .pending("none")
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    r.id,
                    Capture {
                        taken_at: 1_500_000_000 + i as i64 * 86_400,
                        source: DateSource::Exif,
                        camera: None,
                        duration: None,
                    },
                )
            })
            .collect();
        db.set_captures(&caps).unwrap();
    }
    let mut app = App::new(&egui::Context::default(), Arc::clone(&lib));
    app.frame_stats = None;
    (lib, Rc::new(RefCell::new(app)))
}

#[test]
fn the_star_marks_a_favorite_and_the_favorites_view_lists_it() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    // The star in the corner of a tile toggles its favorite mark without opening it.
    h.get_by_label("Add b.jpg to favorites").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    h.get_by_label("Remove b.jpg from favorites");
    assert!(app.borrow().tl.items.iter().any(|it| it.favorite));
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
    // The star again removes the mark; the item leaves the view right away.
    h.get_by_label("Remove b.jpg from favorites").click();
    h.run_steps(3);
    assert!(h.query_by_label("Item b.jpg").is_none());
    settle(&mut h, || app.borrow().tl.awaiting.is_none());
    h.run_steps(2);
    h.get_by_label("Click the star on any photo to add it here.");
    h.get_by_label("Go to Photos");
    assert_eq!(app.borrow().tl.selected, None);
    settle(&mut h, || lib.timeline(&fav).unwrap().is_empty());
    // ...and it stays gone after the view is reloaded.
    settle(&mut h, || app.borrow().fav_marks.is_empty());
    assert!(app.borrow().tl.items.is_empty());
}

#[test]
fn f_marks_the_keyboard_selection_as_a_favorite() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.key_press(egui::Key::F);
    h.run_steps(3);
    assert_eq!(
        app.borrow()
            .tl
            .items
            .iter()
            .filter(|it| it.favorite)
            .count(),
        1
    );
    h.key_press(egui::Key::F);
    h.run_steps(3);
    assert!(app.borrow().tl.items.iter().all(|it| !it.favorite));
}

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
        super::marked(&a.fav_marks, a.tl.items[0].file_id, a.tl.items[0].favorite)
    };
    assert!(shown(&app));
    h.key_press(egui::Key::F);
    h.run_steps(3);
    h.get_by_label("Favorite");
    assert!(!shown(&app));
}

#[test]
fn search_shows_the_when_and_sort_chips_and_the_top_bar_filters_the_type() {
    let d = tempfile::tempdir().unwrap();
    let app = Rc::new(RefCell::new(grid_app_with(d.path(), false)));
    let mut h = album_harness(&app);
    h.get_by_label("When: Any time");
    h.get_by_label("Sort: Best match");
    // The type filter exists once: the top bar's All / Photos / Videos.
    assert!(h.query_by_label_contains("Type:").is_none());
    h.get_by_label("Everything");
    h.get_by_label("Photos only");
    assert_eq!(app.borrow().date.when, super::when::When::Any);
    h.get_by_label("When: Any time").click();
    h.run_steps(3);
    h.get_by_label("Last year").click();
    h.run_steps(3);
    assert_eq!(app.borrow().date.when, super::when::When::LastYear);
    h.get_by_label("When: Last year");
    h.get_by_label("Videos only").click();
    h.run_steps(3);
    assert_eq!(app.borrow().kind, Some(MediaKind::Video));
    assert!(h.query_by_label_contains("Type:").is_none());
    h.get_by_label("Sort: Best match").click();
    h.run_steps(3);
    h.get_by_label("Oldest").click();
    h.run_steps(3);
    assert_eq!(app.borrow().sort, crate::search::SearchSort::Oldest);
    h.get_by_label("Sort: Oldest");
}

#[test]
fn browsing_has_no_when_and_sort_selectors() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = album_harness(&app);
    assert!(h.query_by_label_contains("When:").is_none());
}

#[test]
fn on_this_day_strip_opens_the_viewer_on_its_own_list() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    // 15 March: one photo per year from 2000 to 2019 was taken on that day.
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let mut h = album_harness(&app);
    h.get_by_label("On this day");
    h.get_by_label("7 years ago");
    h.get_by_label("26 years ago");
    // Newest first: 2019 comes first. March 2019 is img10 in `spread_items` (Dec = 1).
    h.get_by_label("On this day img10.jpg").click();
    h.run_steps(3);
    h.get_by_label("img10.jpg");
    h.get_by_label("1 of 20");
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("2 of 20");
    h.get_by_label("img22.jpg"); // March 2018
    // Not in the other views.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.get_by_label("Recently added").click();
    h.run_steps(3);
    assert!(h.query_by_label("On this day").is_none());
}

// ---- Favorites: pending marks, write failures ---------------------------------------------

fn file_id_of(lib: &Library, name: &str) -> i64 {
    lib.timeline(&crate::store::db::TimelineFilter::default())
        .unwrap()
        .into_iter()
        .find(|t| t.path.ends_with(name))
        .unwrap()
        .file_id
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
    let filter = a.tl.filter(a.kind);
    a.tl.awaiting = Some((u64::MAX, filter.clone()));
    a.tl.worker.inject(super::timeline::Loaded {
        id: u64::MAX,
        filter,
        items: Ok(stale),
    });
    a.update_timeline();
    let it = a.tl.items.iter().find(|it| it.file_id == b).unwrap();
    assert!(!it.favorite, "the list itself is stale");
    assert!(super::marked(&a.fav_marks, b, it.favorite), "the mark wins");
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
    let filter = a.tl.filter(a.kind);
    a.tl.awaiting = Some((u64::MAX, filter.clone()));
    a.tl.worker.inject(super::timeline::Loaded {
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
        super::marked(&a.fav_marks, b, it.favorite)
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
fn years_for_the_when_selector_come_from_the_whole_library() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    // 1_500_000_000 is in 2017; the list is read in the background.
    settle(&mut h, || app.borrow().known_years == [2017]);
}

// ---- Viewer: shared list, notices, cleanup --------------------------------------------------

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

// ---- Albums and multi-select ----------------------------------------------------------------

/// A library with `n` dated files `p0.jpg`..: `p{n-1}` is the newest (first in the timeline).
fn files_app(d: &std::path::Path, n: usize) -> (Arc<Library>, Rc<RefCell<App>>) {
    use crate::media::ScannedFile;
    use crate::media::metadata::{Capture, DateSource};
    let lib = library(d);
    {
        let mut db = lib.db.lock();
        let f = db.add_folder(std::path::Path::new("/nowhere")).unwrap();
        let files: Vec<ScannedFile> = (0..n)
            .map(|i| ScannedFile {
                path: format!("/nowhere/p{i}.jpg").into(),
                kind: MediaKind::Image,
                size: 1,
                mtime: 1,
            })
            .collect();
        db.sync_folder(f, &files, &[]).unwrap();
        let caps: Vec<_> = db
            .pending("none")
            .unwrap()
            .iter()
            .map(|r| {
                let i: i64 = r.path.file_stem().unwrap().to_str().unwrap()[1..]
                    .parse()
                    .unwrap();
                (
                    r.id,
                    Capture {
                        taken_at: 1_500_000_000 + i * 86_400,
                        source: DateSource::Exif,
                        camera: None,
                        duration: None,
                    },
                )
            })
            .collect();
        db.set_captures(&caps).unwrap();
    }
    let mut app = App::new(&egui::Context::default(), Arc::clone(&lib));
    app.frame_stats = None;
    (lib, Rc::new(RefCell::new(app)))
}

fn click_with(h: &mut Harness<'_>, label: &str, mods: egui::Modifiers) {
    h.get_by_label(label).click_modifiers(mods);
    h.run_steps(3);
}

fn type_name(h: &mut Harness<'_>, name: &str) {
    h.run_steps(2);
    h.get_by_label("Album name").type_text(name);
    h.run_steps(2);
}

/// The album grouped by year with small tiles, so the first twelve photos are all on screen.
fn small_tiles_harness(app: &Rc<RefCell<App>>) -> Harness<'static> {
    {
        let mut a = app.borrow_mut();
        a.tl.group = super::timeline::Group::Year;
        a.tl.tile_size = 96.0;
    }
    album_harness(app)
}

fn tl_selection(app: &Rc<RefCell<App>>) -> Vec<i64> {
    app.borrow().selected_file_ids()
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
    assert_eq!(app.borrow().tl.selected, None);
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
fn cancel_button_and_search_results_multi_select() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    app.results = Arc::new(
        (1..=4)
            .map(|i| {
                let mut h = hit(&format!("/nowhere/r{i}.jpg"), MediaKind::Image, None);
                h.file_id = i;
                h
            })
            .collect(),
    );
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    click_with(&mut h, "Result r1.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::SHIFT);
    assert_eq!(tl_selection(&app), [1, 2, 3]);
    h.get_by_label("3 selected");
    h.get_by_label("Add to album…");
    h.get_by_label("Favorite");
    h.get_by_label("Unfavorite");
    assert!(h.query_by_label("Remove from album").is_none());
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
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
    h.get_by_label_contains("Added 3 items to “Trip”");
    // Adding them again changes nothing.
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
    h.get_by_label("Rename…");
    h.get_by_label("Delete album…");

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
    // ...and two with the selection bar.
    click_with(&mut h, "Item p4.jpg", ctrl);
    click_with(&mut h, "Item p0.jpg", shift);
    h.get_by_label("Remove from album").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 0);
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.items.is_empty() && a.tl.awaiting.is_none() && a.tl.loaded
    });
    h.run_steps(3);
    h.get_by_label("This album is empty. Select photos and choose Add to album.");
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
    h.get_by_label("Rename…").click();
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
    h.get_by_label("Delete album…").click();
    h.run_steps(3);
    h.get_by_label("The photos and videos in it are not deleted.");
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(lib.albums().unwrap().len(), 1);
    h.get_by_label("Delete album…").click();
    h.run_steps(3);
    h.get_by_label("Delete").click();
    settle(&mut h, || {
        let a = app.borrow();
        lib.albums().unwrap().is_empty() && a.tl.view == View::Timeline && a.tl.items.len() == 5
    });
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
    settle(&mut h, || lib.albums().unwrap().len() == 2);
    let created = lib
        .albums()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "EXISTING2")
        .unwrap();
    assert_eq!(created.count, 1);
    // Escape closes a dialog without touching the selection.
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_all_by_label("New album…").last().unwrap().click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().new_album.is_none());
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

// ---- Album dialogs, keys, failures, per-file selection ----------------------------------------

fn all_file_ids(lib: &Library) -> Vec<i64> {
    lib.timeline(&crate::store::db::TimelineFilter::default())
        .unwrap()
        .iter()
        .map(|t| t.file_id)
        .collect()
}

/// `files_app` with one album "A" holding all files, opened as the current view.
fn album_view(
    d: &std::path::Path,
    n: usize,
    h: impl FnOnce(&Rc<RefCell<App>>) -> Harness<'static>,
) -> (Arc<Library>, Rc<RefCell<App>>, Harness<'static>) {
    let (lib, app) = files_app(d, n);
    let ids = all_file_ids(&lib);
    lib.create_album("A", &ids).unwrap();
    let mut h = h(&app);
    settle(&mut h, || {
        app.borrow().albums.len() == 1 && app.borrow().tl.items.len() == n
    });
    h.get_by_label(&format!("A ({n})")).click();
    settle(&mut h, || {
        let a = app.borrow();
        matches!(a.tl.view, View::Album(_)) && a.tl.items.len() == n && a.tl.loaded
    });
    h.run_steps(3);
    (lib, app, h)
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
    assert_eq!(a.new_album.as_ref().unwrap().name, "fiz");
}

#[test]
fn keys_do_not_reach_the_grid_behind_a_dialog_and_enter_never_deletes() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app, mut h) = album_view(d.path(), 3, album_harness);
    h.get_by_label("Delete album…").click();
    h.run_steps(3);
    // Clicks do not reach the grid behind the dialog either: select straight in the state.
    {
        let mut a = app.borrow_mut();
        let id = a.tl.items[1].file_id;
        a.tl.sel.only(id);
        a.picking = true;
        a.tl.selected = Some(1);
    }
    h.run_steps(2);
    h.get_by_label("The photos and videos in it are not deleted.");
    // Enter, F and arrows behind the dialog do nothing.
    for key in [egui::Key::Enter, egui::Key::F, egui::Key::ArrowRight] {
        h.key_press(key);
        h.run_steps(3);
    }
    {
        let a = app.borrow();
        assert!(a.viewer.is_none());
        assert!(a.fav_marks.is_empty());
        assert!(a.delete_album.is_some(), "still asking");
    }
    assert_eq!(lib.albums().unwrap().len(), 1, "nothing deleted");
    // Escape cancels and leaves the selection alone.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().delete_album.is_none());
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
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    h.get_by_label("Delete album…").click();
    h.run_steps(3);
    h.get_by_label("Delete").click();
    // The writer has answered; the next frames show the error and re-enable the buttons.
    lib.wait_writes();
    h.run_steps(3);
    assert!(
        app.borrow()
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
        a.tl.view == View::Timeline && a.delete_album.is_none() && a.albums.is_empty()
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
        a.new_album
            .as_ref()
            .is_some_and(|n| n.name == "Fresh" && n.add.len() == 1)
    });
    h.get_by_label("1 item will be added to it.");
    assert!(lib.albums().unwrap().is_empty());
}

#[test]
fn long_album_names_are_refused_and_shortened() {
    assert_eq!(super::albums::short_name("Trip", 28), "Trip");
    let long = "Summer holidays with the whole family in the mountains";
    let short = super::albums::short_name(long, 28);
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
    app.borrow_mut().new_album = None;
    h.run_steps(2);
    lib.create_album(long, &[]).unwrap();
    settle(&mut h, || app.borrow().albums.len() == 1);
    h.get_by_label(&format!("{short} (0)"));
}

#[test]
fn search_selection_counts_files_and_toggles_per_file() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    // r1 and r3 are two hits of file 1.
    app.results = Arc::new(
        [1, 2, 1, 3]
            .iter()
            .enumerate()
            .map(|(k, &f)| {
                let mut h = hit(&format!("/nowhere/r{}.jpg", k + 1), MediaKind::Image, None);
                h.file_id = f;
                h
            })
            .collect(),
    );
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    click_with(&mut h, "Result r1.jpg", egui::Modifiers::COMMAND);
    // A range over the visible hits r1..r3 covers two files.
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::SHIFT);
    assert_eq!(tl_selection(&app), [1, 2]);
    h.get_by_label("2 selected");
    click_with(&mut h, "Result r4.jpg", egui::Modifiers::COMMAND);
    assert_eq!(tl_selection(&app), [1, 2, 3]);
    h.get_by_label("3 selected");
    // Ctrl+click on the other hit of selected file 1 deselects that file, not everything.
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::COMMAND);
    assert_eq!(tl_selection(&app), [2, 3]);
    h.get_by_label("2 selected");
    assert!(app.borrow().result_sel.contains(3));
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

// ---- Folders, activity, toasts, wording ---------------------------------------------------------

#[test]
fn adding_a_folder_from_the_dialog_by_path() {
    let d = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(Arc::clone(&lib));
    h.get_all_by_label("Add a folder").next().unwrap().click();
    h.run_steps(3);
    h.get_by_label("Choose a folder…");
    h.get_by_label("or type a folder path");
    // A path that does not exist is refused with a plain message and the dialog stays.
    h.get_by_label("Folder path").click();
    h.run_steps(2);
    h.get_by_label("Folder path")
        .type_text("/definitely/not/here");
    h.run_steps(2);
    h.get_by_label("Add").click();
    h.run_steps(3);
    h.get_by_label_contains("couldn't be found");
    h.get_by_label("or type a folder path");
    // A real folder is added and the dialog closes.
    h.get_by_label("Folder path").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_label("Folder path")
        .type_text(photos.path().to_str().unwrap());
    h.run_steps(2);
    h.get_by_label("Add").click();
    settle(&mut h, || lib.folders().unwrap().len() == 1);
    h.run_steps(3);
    assert!(h.query_by_label("or type a folder path").is_none());
}

#[test]
fn stopping_a_folder_asks_first_and_keeps_the_files() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 2);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 2);
    h.get_by_label("Stop showing nowhere").click();
    h.run_steps(3);
    h.get_by_label("Stop showing “nowhere”?");
    h.get_by_label("Your files stay where they are.");
    assert_eq!(lib.folders().unwrap().len(), 1, "nothing happens yet");
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Your files stay where they are.")
            .is_none()
    );
    assert_eq!(lib.folders().unwrap().len(), 1);
    h.get_by_label("Stop showing nowhere").click();
    h.run_steps(3);
    h.get_by_label("Stop showing").click();
    settle(&mut h, || lib.folders().unwrap().is_empty());
}

#[test]
fn the_activity_card_describes_what_is_happening() {
    use crate::library::{Activity, IndexProgress};
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    assert!(h.query_by_label_contains("Looking for").is_none());
    let show = |h: &mut Harness<'_>, a: Activity| {
        lib.update_status(|s| s.activity = a);
        h.run_steps(2);
    };
    show(
        &mut h,
        Activity::Scanning {
            folder: "/x".into(),
        },
    );
    h.get_by_label("Looking for new photos…");
    h.get_by_label("Pause");
    h.get_by_label("Stop");
    show(
        &mut h,
        Activity::ReadingDates {
            done: 1234,
            total: 5000,
        },
    );
    h.get_by_label("Reading photo dates… 1,234 of 5,000");
    show(
        &mut h,
        Activity::Indexing(IndexProgress {
            done: 45,
            total: 100,
            ..Default::default()
        }),
    );
    h.get_by_label("Getting your photos ready for search… 45 %");
    show(
        &mut h,
        Activity::Downloading {
            model: "m".into(),
            done: 120_000_000,
            total: 485_000_000,
        },
    );
    h.get_by_label("Downloading smart search… 120 of 485 MB");
    assert!(
        h.query_by_label("Pause").is_none(),
        "a download is only cancelled"
    );
    show(&mut h, Activity::Idle);
    assert!(h.query_by_label_contains("Downloading").is_none());
}

#[test]
fn notices_appear_as_toasts_and_errors_can_be_closed() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    app.borrow_mut().notify("Added 2 items to “Trip”");
    h.run_steps(2);
    h.get_by_label("Added 2 items to “Trip”");
    // Information fades by itself, and can also be closed.
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert!(h.query_by_label("Added 2 items to “Trip”").is_none());
    app.borrow_mut().notify("Added 3 items to “Trip”");
    h.run_steps(2);
    h.get_by_label("Added 3 items to “Trip”");
    app.borrow_mut()
        .notify_error("That didn't work. Please try again.");
    h.run_steps(2);
    h.get_by_label("That didn't work. Please try again.");
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("That didn't work. Please try again.")
            .is_none()
    );
}

#[test]
fn help_has_a_section_for_each_question() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Help").click();
    h.run_steps(3);
    // Getting started comes first.
    h.get_by_label("Add your photo folders");
    h.get_by_label("Search or browse");
    for (topic, heading, text) in [
        (
            "Searching",
            "Describe what you remember",
            "kids on the beach",
        ),
        ("Searching", "Narrow the results", "When"),
        ("Searching", "Moments in videos", "at 1:23"),
        ("Organizing", "Favorites", "star"),
        ("Organizing", "Albums", "Add to album"),
        ("Organizing", "Selecting several photos", "circle"),
        ("Keyboard shortcuts", "Arrow keys", "Move between photos"),
        (
            "Privacy",
            "Everything stays on this computer",
            "never sent anywhere",
        ),
    ] {
        h.get_by_label(&format!("{topic} help")).click();
        h.run_steps(3);
        // (Favorites and Albums are also names in the sidebar behind the sheet.)
        assert!(h.query_all_by_label(heading).next().is_some(), "{heading}");
        assert!(
            h.query_all_by_label_contains(text).next().is_some(),
            "{text}"
        );
    }
    h.get_by_label("Close help").click();
    h.run_steps(3);
    assert!(!app.borrow().help_open);
    // Esc closes it too.
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!app.borrow().help_open);
}

/// Words that must never appear in the everyday screens.
const BANNED: &[&str] = &[
    "model",
    "index",
    "indexed",
    "indexing",
    "vector",
    "embedding",
    "siglip",
    "clip",
    "onnx",
    "rescan",
    "threads",
    "cache",
    "ms",
    "score",
    "apache",
    "huggingface",
];

/// Every label and value the accessibility tree exposes (what a person can read or hear).
fn visible_text(h: &Harness<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for node in h.root().children_recursive() {
        let n = node.accesskit_node();
        out.extend(n.label());
        out.extend(n.value());
        out.extend(n.description());
    }
    out
}

fn banned_words_in(texts: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for t in texts {
        // File system paths are shown as they are (Settings > Storage).
        if t.starts_with('/') {
            continue;
        }
        for word in t
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            if BANNED.contains(&word.to_lowercase().as_str()) {
                found.push(format!("{word:?} in {t:?}"));
            }
        }
    }
    found
}

#[test]
fn the_banned_word_check_works() {
    let t = vec!["Download the Model".to_string(), "500 ms".to_string()];
    assert_eq!(banned_words_in(&t).len(), 2);
    let ok = vec!["Months".to_string(), "Smart search is off".to_string()];
    assert!(banned_words_in(&ok).is_empty());
}

#[test]
fn the_main_screens_use_no_technical_words() {
    use crate::library::Activity;
    let d = tempfile::tempdir().unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut check = |name: &str, h: &Harness<'_>| {
        let texts = visible_text(h);
        assert!(!texts.is_empty(), "{name}: nothing to read");
        let bad = banned_words_in(&texts);
        assert!(bad.is_empty(), "{name}: {bad:?}");
        seen.extend(texts);
    };

    // Empty library.
    {
        let d = tempfile::tempdir().unwrap();
        let lib = library(d.path());
        fake_install(&lib);
        let h = harness(lib);
        check("empty library", &h);
    }
    // The welcome, every screen.
    {
        let d = tempfile::tempdir().unwrap();
        let app = fresh_app(d.path());
        let mut h = album_harness(&app);
        check("welcome: first screen", &h);
        h.get_by_label("Get started").click();
        h.run_steps(3);
        check("welcome: folders", &h);
        h.get_by_label("Continue without folders").click();
        h.run_steps(3);
        check("welcome: smart search", &h);
        h.get_by_label("Not now").click();
        h.run_steps(3);
        check("welcome: all set", &h);
    }
    // Every empty state.
    for (name, which) in EMPTY_VIEWS {
        let d = tempfile::tempdir().unwrap();
        let (_app, mut h) = empty_view(d.path(), which);
        h.run_steps(2);
        check(name, &h);
    }
    // Timeline with items, smart search off (banner, invitation), then on.
    {
        let app = album_app(d.path(), false);
        let h = album_harness(&app);
        check("timeline, smart search off", &h);
    }
    {
        let d1 = tempfile::tempdir().unwrap();
        let app = album_app(d1.path(), false);
        let mut h = album_harness(&app);
        open_section(&mut h, "Smart search");
        h.get_by_label("Download and turn on (608 MB)");
        check("settings: smart search off", &h);
    }
    {
        let d2 = tempfile::tempdir().unwrap();
        let app = album_app(d2.path(), true);
        let mut h = album_harness(&app);
        check("timeline", &h);
        // Activity card in each state.
        let lib = Arc::clone(&app.borrow().lib);
        for a in [
            Activity::Scanning {
                folder: "/x".into(),
            },
            Activity::ReadingDates { done: 1, total: 2 },
            Activity::LoadingModel,
            Activity::Indexing(crate::library::IndexProgress {
                done: 1,
                total: 2,
                ..Default::default()
            }),
            Activity::Downloading {
                model: "m".into(),
                done: 1,
                total: 2,
            },
        ] {
            lib.update_status(|s| s.activity = a);
            h.run_steps(2);
            check("activity", &h);
        }
        lib.update_status(|s| s.activity = Activity::Idle);
        h.run_steps(2);
        // Selection mode.
        h.get_by_label("Select img2.jpg").click();
        h.run_steps(3);
        h.get_by_label("1 selected");
        check("selection mode", &h);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        // Notices.
        app.borrow_mut()
            .notify_error("Couldn't save your favorite. Try again in a moment.");
        h.run_steps(2);
        check("error toast", &h);
        // Settings, every section. The Advanced and Details parts stay folded away.
        {
            let mut a = app.borrow_mut();
            a.snapshot.stats.images = 240;
            a.snapshot.stats.indexed = 238;
            a.snapshot.stats.errors = 2;
            a.snapshot.errors = vec![("/nowhere/x.jpg".into(), "unreadable".into())];
            a.failed_open = true;
        }
        for section in ["Appearance", "Smart search", "Library", "Storage", "About"] {
            open_section(&mut h, section);
            check(&format!("settings: {section}"), &h);
        }
        open_section(&mut h, "Smart search");
        h.get_by_label("Best quality").click();
        h.run_steps(3);
        h.get_by_label("Switch to Best quality?");
        check("settings: switching", &h);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        h.get_by_label("Close settings").click();
        h.run_steps(3);
        // Help.
        h.get_by_label("Help").click();
        h.run_steps(3);
        for topic in [
            "Getting started",
            "Searching",
            "Organizing",
            "Keyboard shortcuts",
            "Privacy",
        ] {
            h.get_by_label(&format!("{topic} help")).click();
            h.run_steps(3);
            check(&format!("help: {topic}"), &h);
        }
        h.get_by_label("Close help").click();
        h.run_steps(3);
        // The search help, with a recent search in it, and the tips.
        app.borrow_mut().remember_search("snow");
        app.borrow_mut().snapshot.model_installed = true;
        h.get_by_label("Search your photos").click();
        h.run_steps(3);
        h.get_by_label("kids on the beach");
        check("search help", &h);
        app.borrow_mut().search_help.focused = false;
        app.borrow_mut().search_help.field = None;
        for tip in [super::guide::Tip::Search, super::guide::Tip::Select] {
            h.run_steps(3);
            h.get_by_label(tip.text());
            check("tip", &h);
            h.get_by_label("Got it").click();
            h.run_steps(3);
        }
    }
    // Search results and an empty result.
    {
        let d3 = tempfile::tempdir().unwrap();
        let mut app = grid_app(d3.path());
        let results = Arc::make_mut(&mut app.results);
        results[0] = hit("/nowhere/movie.mp4", MediaKind::Video, Some(42.0));
        let app = Rc::new(RefCell::new(app));
        let mut h = album_harness(&app);
        check("search results", &h);
        let mut a = app.borrow_mut();
        a.results = Arc::default();
        drop(a);
        h.run_steps(2);
        check("no results", &h);
    }
    // The viewer, with its labelled toolbar and the details panel.
    {
        let d5 = tempfile::tempdir().unwrap();
        let app = album_app(d5.path(), true);
        let mut h = quick_harness(&app);
        open_tile(&mut h, "Item img1.jpg");
        h.run_steps(3);
        check("viewer", &h);
    }
    // An album view.
    {
        let d4 = tempfile::tempdir().unwrap();
        let (_lib, _app, h) = album_view(d4.path(), 3, album_harness);
        check("album", &h);
    }
    // Something was actually read.
    assert!(seen.iter().any(|t| t == "Recently added"));
    assert!(
        seen.iter()
            .any(|t| t.starts_with("Getting your photos ready for search"))
    );
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
fn results_header_titles_searches_and_similar_photos() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    app.current = Some(super::Query::Similar {
        item_id: 1,
        label: "beach.jpg".into(),
    });
    app.result_label = "similar to beach.jpg".into();
    let app = Rc::new(RefCell::new(app));
    let h = album_harness(&app);
    h.get_by_label("Photos similar to beach.jpg");
    h.get_by_label("3 photos");
    h.get_by_label("Clear search");
    assert!(h.query_by_label_contains("score").is_none());
}

/// Like `quick_harness` for a window `width` points wide.
fn quick_harness_wide(app: &Rc<RefCell<App>>, width: f32) -> Harness<'static> {
    let app = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(width, 800.0))
        .with_step_dt(0.05)
        .build_ui(move |ui| app.borrow_mut().show(ui));
    h.run();
    h
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
    assert!(h.get_by_label("Show in folder").rect().width() > 120.0);
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
fn the_skipped_folders_notice_shows_when_a_run_ends() {
    use crate::library::RunSummary;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    lib.update_status(|s| {
        s.last_run = Some(RunSummary {
            skipped_folders: vec!["/gone".into()],
            ..Default::default()
        });
    });
    // The app saw the library busy on the previous frame; now it is idle.
    app.borrow_mut().was_busy = true;
    h.run_steps(2);
    h.get_by_label("Some folders couldn't be reached, so they were left as they were.");
    // Without a run that just ended it stays quiet.
    app.borrow_mut().notices.dismiss_current();
    h.run_steps(3);
    assert!(app.borrow().notices.current().is_none());
}

const SOMETHING_WRONG: &str =
    "Something went wrong. Please try again, and restart Recollect if it keeps happening.";

#[test]
fn an_identical_error_toasts_again() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    lib.update_status(|s| s.set_error("disk full"));
    h.run_steps(2);
    h.get_by_label(SOMETHING_WRONG);
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("Something went wrong").is_none());
    // Nothing new: no new toast.
    h.run_steps(3);
    assert!(h.query_by_label_contains("Something went wrong").is_none());
    // The same text again is a new failure.
    lib.update_status(|s| s.set_error("disk full"));
    h.run_steps(2);
    h.get_by_label(SOMETHING_WRONG);
}

#[test]
fn starting_a_search_leaves_the_timelines_selection_mode() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    h.get_by_label("Select img2.jpg").click();
    h.run_steps(3);
    h.get_by_label("1 selected");
    assert!(app.borrow().picking);
    app.borrow_mut()
        .submit(super::Query::Text("a dragon".into()));
    h.run_steps(3);
    let a = app.borrow();
    assert!(!a.picking);
    assert!(a.tl.sel.is_empty() && a.tl.selected.is_none());
}

#[test]
fn each_grid_has_its_own_selection_mode() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app_with(d.path(), false);
    // The timeline was in selection mode before the search started.
    app.picking = true;
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    // A plain click in the results opens the photo.
    h.get_by_label("Result a.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some(), "a plain click opens");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    // In the results' own selection mode a plain click selects.
    h.get_by_label("Select b.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().result_picking);
    h.get_by_label("Result c.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none(), "now clicks select");
    assert_eq!(app.borrow().result_sel.len(), 2);
}

#[test]
fn the_results_title_follows_the_current_query() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    app.current = Some(super::Query::Text("a cat".into()));
    app.result_label = "a dog".into();
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    h.get_by_label("Results for “a cat”");
    assert!(h.query_by_label("Results for “a dog”").is_none());
    app.borrow_mut().current = Some(super::Query::Similar {
        item_id: 1,
        label: "beach.jpg".into(),
    });
    h.run_steps(2);
    h.get_by_label("Photos similar to beach.jpg");
}

#[test]
fn folder_suggestions_are_worked_out_once_when_the_dialog_opens() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let candidates = vec![
        (
            "Pictures".to_string(),
            std::path::PathBuf::from("/home/a/Pictures"),
        ),
        (
            "Videos".to_string(),
            std::path::PathBuf::from("/home/a/Videos"),
        ),
    ];
    app.borrow_mut().open_add_folder_with(candidates);
    h.run_steps(3);
    h.get_by_label("Add Pictures");
    h.get_by_label("Add Videos");
    // The library changes while the dialog is open: the list is not recomputed.
    app.borrow_mut()
        .snapshot
        .folders
        .push(crate::store::db::Folder {
            id: 2,
            path: "/home/a/Pictures".into(),
        });
    h.run_steps(3);
    h.get_by_label("Add Pictures");
    assert_eq!(
        app.borrow()
            .add_folder_dialog
            .as_ref()
            .unwrap()
            .suggestion_count(),
        2
    );
}

#[test]
fn on_this_day_tiles_open_and_have_no_selection_circle() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
    let h = album_harness(&app);
    h.get_by_label("On this day img10.jpg");
    // img10 is only on screen in the strip (the grid starts at the newest photos).
    assert!(h.query_by_label("Select img10.jpg").is_none());
    assert!(h.query_by_label("Select On this day img10.jpg").is_none());
}

// --- Guidance: welcome, empty states, search help, tips, help, friendly errors -------------

/// A first launch: no folders, welcome not done, no standard folders on offer (so the test does
/// not depend on this computer's home directory).
fn fresh_app(d: &std::path::Path) -> Rc<RefCell<App>> {
    fresh_app_with(d, Vec::new())
}

/// Like [`fresh_app`], offering these standard folders.
fn fresh_app_with(
    d: &std::path::Path,
    folders: Vec<(String, std::path::PathBuf)>,
) -> Rc<RefCell<App>> {
    let lib = library(d);
    let mut app = App::new(&egui::Context::default(), Arc::clone(&lib));
    app.frame_stats = None;
    assert!(
        app.welcome.is_some(),
        "a new library starts with the welcome"
    );
    app.welcome = Some(super::welcome::Welcome::with_suggestions(
        folders,
        lib.model().id,
    ));
    Rc::new(RefCell::new(app))
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
    h.get_by_label("Everything stays on this computer");
    assert!(h.query_by_label("Settings").is_none());
    assert!(app.borrow().welcome_active());
    h.get_by_label("Step 1 of 4");
    let mut clicks = 0;
    h.get_by_label("Get started").click();
    clicks += 1;
    h.run_steps(3);
    // Second: with nothing chosen the button says so; typing a path (under More options)
    // adds one (not a "primary" click).
    h.get_by_label("Where are your photos?");
    h.get_by_label("Step 2 of 4");
    assert!(
        !h.get_by_label("Continue without folders")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("You can add folders later from the sidebar.");
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
    assert!(h.query_by_label("Continue without folders").is_none());
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
    h.get_by_label("Download and turn on");
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

#[derive(Clone, Copy)]
enum EmptyView {
    NoFolders,
    Photos,
    Recent,
    Favorites,
    Folder,
    Album,
    NoResults,
    NoResultsDated,
    SearchOff,
}

const EMPTY_VIEWS: [(&str, EmptyView); 9] = [
    ("empty: no folders", EmptyView::NoFolders),
    ("empty: photos", EmptyView::Photos),
    ("empty: recently added", EmptyView::Recent),
    ("empty: favorites", EmptyView::Favorites),
    ("empty: folder", EmptyView::Folder),
    ("empty: album", EmptyView::Album),
    ("empty: no results", EmptyView::NoResults),
    ("empty: no results, dated", EmptyView::NoResultsDated),
    ("empty: search off", EmptyView::SearchOff),
];

/// An app showing the empty version of `which`, and its harness.
fn empty_view(d: &std::path::Path, which: EmptyView) -> (Rc<RefCell<App>>, Harness<'static>) {
    use super::when::{DateFilter, When};
    let lib = library(d);
    lib.update_settings(|s| s.welcome_done = true).unwrap();
    let mut app = App::new(&egui::Context::default(), Arc::clone(&lib));
    app.frame_stats = None;
    if !matches!(which, EmptyView::NoFolders) {
        app.snapshot.folders = vec![crate::store::db::Folder {
            id: 1,
            path: "/nowhere".into(),
        }];
    }
    match which {
        EmptyView::NoResults | EmptyView::NoResultsDated | EmptyView::SearchOff => {
            app.current = Some(super::Query::Text("snow".into()));
            app.result_label = "snow".into();
            if matches!(which, EmptyView::NoResultsDated) {
                app.date = DateFilter {
                    when: When::ThisYear,
                    from: 0,
                    to: 0,
                };
            }
            if matches!(which, EmptyView::SearchOff) {
                app.result_error = Some(super::guide::search_problem(
                    crate::library::SearchError::ModelMissing,
                ));
            }
        }
        _ => {}
    }
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    let view = match which {
        EmptyView::Recent => Some(View::RecentlyAdded),
        EmptyView::Favorites => Some(View::Favorites),
        EmptyView::Folder => Some(View::Folder(1)),
        EmptyView::Album => {
            lib.create_album("Empty", &[]).unwrap();
            settle(&mut h, || app.borrow().albums.len() == 1);
            Some(View::Album(lib.albums().unwrap()[0].id))
        }
        _ => None,
    };
    if let Some(v) = view {
        app.borrow_mut().set_view(v);
    }
    if matches!(
        which,
        EmptyView::Photos
            | EmptyView::Recent
            | EmptyView::Favorites
            | EmptyView::Folder
            | EmptyView::Album
    ) {
        settle(&mut h, || {
            let a = app.borrow();
            a.tl.loaded && a.tl.awaiting.is_none() && a.tl.items.is_empty()
        });
    }
    h.run_steps(3);
    (app, h)
}

#[test]
fn every_view_has_an_empty_state_with_a_sentence_and_a_button() {
    let cases = [
        (
            EmptyView::NoFolders,
            "Add a folder with your photos and videos to get started.",
            "Add a folder",
        ),
        (
            EmptyView::Photos,
            "No photos or videos yet. Add a folder, or give Recollect a moment to look through \
             the ones you have.",
            "Add a folder",
        ),
        (
            EmptyView::Recent,
            "Photos and videos you add to your folders show up here.",
            "Check for new photos",
        ),
        (
            EmptyView::Favorites,
            "Click the star on any photo to add it here.",
            "Go to Photos",
        ),
        (
            EmptyView::Folder,
            "There are no photos or videos in this folder.",
            "Check for new photos",
        ),
        (
            EmptyView::Album,
            "This album is empty. Select photos and choose Add to album.",
            "Go to Photos",
        ),
        (
            EmptyView::NoResults,
            "No photos match “snow”. Try different words.",
            "Start over",
        ),
        (
            EmptyView::NoResultsDated,
            "No photos match “snow”. Try different words, or check the When filter.",
            "Clear the date filter",
        ),
        (
            EmptyView::SearchOff,
            "Smart search is off. Turn it on to find photos by describing them.",
            "Turn on smart search",
        ),
    ];
    for (which, sentence, button) in cases {
        let d = tempfile::tempdir().unwrap();
        let (_app, h) = empty_view(d.path(), which);
        h.get_by_label(sentence);
        h.get_all_by_label(button).last().unwrap();
    }
}

#[test]
fn the_buttons_of_the_empty_states_work() {
    // Favorites and album: "Go to Photos".
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::Favorites);
    h.get_by_label("Go to Photos").click();
    h.run_steps(3);
    assert_eq!(app.borrow().tl.view, View::Timeline);
    // No results: clearing the date filter keeps the words; clearing the search drops them.
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::NoResultsDated);
    h.get_by_label("Clear the date filter").click();
    h.run_steps(3);
    assert_eq!(app.borrow().date, super::when::DateFilter::default());
    assert!(app.borrow().current.is_some());
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::NoResults);
    h.get_by_label("Start over").click();
    h.run_steps(3);
    assert!(app.borrow().current.is_none() && app.borrow().query.is_empty());
    // Search off: opens Settings on smart search.
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::SearchOff);
    h.get_by_label("Turn on smart search").click();
    h.run_steps(3);
    assert!(app.borrow().settings_open);
    assert_eq!(
        app.borrow().settings_section,
        super::settings::Section::SmartSearch
    );
}

/// An app whose smart search looks installed to the interface (so the search field is on)
/// while a search still fails cleanly because no files exist.
fn searchable_app(d: &std::path::Path) -> Rc<RefCell<App>> {
    let mut app = grid_app_with(d, false);
    app.snapshot.model_installed = true;
    app.results = Arc::default();
    app.current = None;
    app.result_label.clear();
    Rc::new(RefCell::new(app))
}

#[test]
fn the_empty_search_field_offers_examples_and_remembers_searches() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    assert!(
        h.query_by_label("kids on the beach").is_none(),
        "closed until focused"
    );
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    for example in super::guide::EXAMPLE_SEARCHES {
        h.get_by_label(example);
    }
    h.get_by_label("Try searching for");
    assert!(h.query_by_label("Recent searches").is_none(), "nothing yet");
    // A chip runs that search.
    h.get_by_label("snow").click();
    h.run_steps(3);
    assert_eq!(app.borrow().query, "snow");
    settle(&mut h, || app.borrow().result_error.is_some());
    assert_eq!(
        app.borrow().current,
        Some(super::Query::Text("snow".into()))
    );
    assert_eq!(lib.settings().recent_searches, ["snow"]);
    // Typing and pressing Enter remembers the words too; newest first, no repeats.
    app.borrow_mut().clear_search();
    h.run_steps(2);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.get_by_label("Search your photos")
        .type_text("sunset boat");
    h.run_steps(2);
    assert_eq!(app.borrow().query, "sunset boat");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(lib.settings().recent_searches, ["sunset boat", "snow"]);
    app.borrow_mut().remember_search("SNOW");
    assert_eq!(lib.settings().recent_searches, ["SNOW", "sunset boat"]);
    // The most recent eight are kept.
    for i in 0..10 {
        app.borrow_mut().remember_search(&format!("word {i}"));
    }
    assert_eq!(
        lib.settings().recent_searches.len(),
        super::guide::MAX_RECENT
    );
    // They show under the field, and can be cleared.
    app.borrow_mut().clear_search();
    h.run_steps(2);
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.get_by_label("Recent searches");
    h.get_by_label("Recent search: word 9");
    h.get_by_label("Clear recent searches").click();
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
    assert!(h.query_by_label("Clear recent searches").is_none());
}

#[test]
fn search_help_stays_closed_when_smart_search_is_off_or_text_was_typed() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    h.get_by_label(super::chrome::SEARCH_OFF_LABEL).click();
    h.run_steps(3);
    assert!(h.query_by_label("kids on the beach").is_none());
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let mut h = album_harness(&app);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.get_by_label("Search your photos").type_text("dog");
    h.run_steps(3);
    assert!(h.query_by_label("kids on the beach").is_none());
}

#[test]
fn tips_show_one_at_a_time_once_each() {
    use super::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    // Photos and smart search are there: the search tip comes first, alone.
    h.get_by_label(Tip::Search.text());
    assert!(h.query_by_label(Tip::Select.text()).is_none());
    assert_eq!(h.query_all_by_label("Got it").count(), 1);
    h.get_by_label("Got it").click();
    h.run_steps(3);
    h.get_by_label(Tip::Select.text());
    assert!(h.query_by_label(Tip::Search.text()).is_none());
    h.get_by_label("Got it").click();
    h.run_steps(3);
    assert!(h.query_by_label("Got it").is_none());
    assert_eq!(lib.settings().tips_dismissed, ["search", "select"]);
    // After a restart they stay dismissed.
    drop(h);
    let mut restarted = App::new(&egui::Context::default(), Arc::clone(&lib));
    restarted.frame_stats = None;
    restarted.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    restarted.snapshot.model_installed = true;
    restarted.inject_timeline(spread_items());
    let restarted = Rc::new(RefCell::new(restarted));
    let mut h = album_harness(&restarted);
    assert!(h.query_by_label("Got it").is_none());
    // Settings can bring them back.
    open_section(&mut h, "Appearance");
    h.get_by_label("Show tips again").click();
    h.run_steps(3);
    assert!(lib.settings().tips_dismissed.is_empty());
    h.get_by_label("Close settings").click();
    h.run_steps(3);
    h.get_by_label(Tip::Search.text());
}

#[test]
fn without_smart_search_the_first_tip_is_about_selecting() {
    use super::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let h = album_harness(&app);
    h.get_by_label(Tip::Select.text());
    assert!(h.query_by_label(Tip::Search.text()).is_none());
}

#[test]
fn the_first_album_view_explains_how_to_fill_it() {
    use super::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let (lib, _app, mut h) = album_view(d.path(), 3, album_harness);
    h.get_by_label(Tip::Album.text());
    assert!(
        h.query_by_label(Tip::Select.text()).is_none(),
        "one at a time"
    );
    h.get_by_label("Got it").click();
    h.run_steps(3);
    assert!(lib.settings().tips_dismissed.contains(&"album".to_string()));
    assert!(h.query_by_label(Tip::Album.text()).is_none());
}

#[test]
fn a_failed_download_says_what_to_do_and_keeps_the_details_folded() {
    use crate::library::ErrorKind;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    lib.update_status(|s| {
        s.set_error_of(
            ErrorKind::Download,
            "Model download failed: dns error: failed to lookup address",
        );
    });
    h.run_steps(3);
    h.get_by_label("Couldn't download smart search. Check your internet connection and try again.");
    h.get_all_by_label("Try again").next().unwrap();
    assert!(h.query_by_label_contains("dns error").is_none(), "folded");
    h.get_by_label("Details").click();
    h.run_steps(3);
    h.get_by_label_contains("dns error");
    // Other failures are explained in the same way.
    for (kind, text) in [
        (ErrorKind::AddFolder, "Couldn't add that folder."),
        (
            ErrorKind::Scan,
            "Couldn't finish looking through your photos.",
        ),
        (ErrorKind::SwitchModel, "Couldn't change smart search."),
    ] {
        let (msg, _) = super::guide::describe_failure(kind);
        assert!(msg.starts_with(text), "{msg}");
        assert!(msg.ends_with('.') && msg.contains("try") || msg.contains("Try"));
    }
}

#[test]
fn other_messages_say_what_happened_and_what_to_do() {
    use crate::library::SearchError;
    use crate::store::db::AlbumError;
    let (text, details) = super::guide::album_problem(
        Some(&AlbumError::NameTaken("Trip".into())),
        "An album named “Trip” already exists",
    );
    assert_eq!(
        text,
        "An album named “Trip” already exists. Choose a different name."
    );
    assert_eq!(details, None);
    let (text, details) = super::guide::album_problem(Some(&AlbumError::NameEmpty), "");
    assert_eq!(text, "An album needs a name. Type a name and try again.");
    assert_eq!(details, None);
    let (text, details) = super::guide::album_problem(None, "database is locked");
    assert_eq!(
        text,
        "Couldn't save that album change. Nothing was changed. Please try again."
    );
    assert_eq!(details.as_deref(), Some("database is locked"));
    let p = super::guide::search_problem(SearchError::ModelMissing);
    assert!(p.text.starts_with("Search isn't ready yet.") && p.can_retry);
    let p = super::guide::search_problem(SearchError::Other);
    assert!(p.text.starts_with("Couldn't search just now.") && p.can_retry);
    let p = super::guide::search_problem(SearchError::ItemGone);
    assert_eq!(p.text, "That photo is no longer in your library.");
    assert!(!p.can_retry);
}

#[test]
fn errors_are_told_apart_by_kind_not_by_their_text() {
    use crate::library::SearchError;
    use crate::store::db::{AlbumError, check_album_name};
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    // No model installed.
    let err = lib
        .search("dog", &crate::search::SearchOptions::default())
        .unwrap_err();
    assert_eq!(SearchError::of(&err), SearchError::ModelMissing);
    // An item that is not there.
    let err = lib
        .similar(12345, &crate::search::SearchOptions::default())
        .unwrap_err();
    assert_eq!(SearchError::of(&err), SearchError::ItemGone);
    assert_eq!(check_album_name("", [], None), Err(AlbumError::NameEmpty));
    assert_eq!(
        check_album_name("Trip", [(1, "trip")], None),
        Err(AlbumError::NameTaken("trip".into()))
    );
    assert_eq!(
        check_album_name(&"x".repeat(500), [], None),
        Err(AlbumError::TooLong)
    );
    // Through the writer thread, the kind arrives with the failure.
    lib.create_album("Trip", &[]).unwrap();
    let t = lib.queue_album(crate::library::AlbumOp::Create {
        name: "trip".into(),
        add: vec![],
    });
    lib.wait_writes();
    let w = lib
        .take_album_results()
        .into_iter()
        .find(|w| w.ticket == t)
        .unwrap();
    assert!(matches!(w.error_kind, Some(AlbumError::NameTaken(_))));
}

#[test]
fn a_removed_photo_in_find_similar_has_no_try_again() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    app.borrow_mut().current = Some(super::Query::Similar {
        item_id: 999,
        label: "a.jpg".into(),
    });
    app.borrow_mut().result_label = "similar to a.jpg".into();
    app.borrow_mut().result_error = Some(super::guide::search_problem(
        crate::library::SearchError::ItemGone,
    ));
    let mut h = album_harness(&app);
    h.run_steps(2);
    // Said once: in the empty state, not also in the header.
    assert_eq!(
        h.query_all_by_label("That photo is no longer in your library.")
            .count(),
        1
    );
    assert!(h.query_by_label("Try again").is_none());
    h.get_by_label("Start over");
}

#[test]
fn a_search_error_is_not_shown_twice() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    app.borrow_mut().current = Some(super::Query::Text("snow".into()));
    app.borrow_mut().result_label = "snow".into();
    app.borrow_mut().result_error = Some(super::guide::search_problem(
        crate::library::SearchError::Other,
    ));
    let mut h = album_harness(&app);
    h.run_steps(2);
    let text = "Couldn't search just now. Please try again in a moment.";
    assert_eq!(h.query_all_by_label(text).count(), 1);
    h.get_by_label("Try again");
}

// --- Welcome: folders, ordering, wording --------------------------------------------------

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
    h.get_by_label("Continue without folders");
    h.get_by_label("You can add folders later from the sidebar.");
    h.get_by_label("Continue without folders").click();
    h.run_steps(3);
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label("Start").click();
    settle(&mut h, || lib.settings().welcome_done);
    settle(&mut h, || lib.jobs_finished() > 0);
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
        std::fs::canonicalize(pictures.path()).unwrap(),
        std::fs::canonicalize(desktop.path()).unwrap(),
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
    h.get_by_label("Add another folder…");
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
    let w = super::welcome::Welcome::with_suggestions(
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
        super::sidebar::without_nested(&["/a/b".into(), "/a".into(), "/c".into(), "/a".into()]),
        [std::path::PathBuf::from("/a"), "/c".into()]
    );
}

#[test]
fn the_computers_hardware_is_asked_once_not_every_frame() {
    use std::sync::atomic::Ordering;
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
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
    h.get_by_label("Download and turn on").click();
    h.run_steps(3);
    h.get_by_label_contains("Search works once the download finishes.");
}

#[test]
fn a_failed_model_switch_at_start_keeps_the_welcome_and_says_so() {
    let d = tempfile::tempdir().unwrap();
    let app = fresh_app(d.path());
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
    h.get_by_label("Download and turn on").click();
    h.run_steps(3);
    h.get_by_label("Start").click();
    h.run_steps(3);
    h.get_by_label("Couldn't set up smart search. Please try again in a moment.");
    assert!(app.borrow().welcome_active(), "still on the last screen");
    assert!(!lib.settings().welcome_done);
}

// --- Search suggestions with the keyboard; recent searches; waiting for a busy library ---

#[test]
fn leaving_the_search_field_without_searching_records_nothing() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.get_by_label("Search your photos").type_text("sunset");
    h.run_steps(2);
    // Before the pause in typing ends, Escape ends the typing.
    app.borrow_mut().query_changed_at = None;
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
    // So does clicking elsewhere.
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    app.borrow_mut().query_changed_at = None;
    h.get_by_label("Settings").click();
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
    // Enter does record.
    h.get_by_label("Close settings").click();
    h.run_steps(3);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(lib.settings().recent_searches, ["sunset"]);
}

#[test]
fn the_search_suggestions_work_with_the_keyboard() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let mut h = album_harness(&app);
    let (first, second) = (
        super::guide::EXAMPLE_SEARCHES[0],
        super::guide::EXAMPLE_SEARCHES[1],
    );
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.get_by_label("Try searching for");
    // Tab goes into the suggestions, which stay open.
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    h.get_by_label("Try searching for");
    assert!(h.get_by_label(first).accesskit_node().is_focused());
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert!(h.get_by_label(second).accesskit_node().is_focused());
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(3);
    assert!(h.get_by_label(first).accesskit_node().is_focused());
    h.get_by_label("Try searching for");
    // Enter runs the one that has focus.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(app.borrow().query, first);
    // Escape closes them.
    app.borrow_mut().clear_search();
    h.run_steps(2);
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    h.get_by_label("Try searching for");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Try searching for").is_none());
}

#[test]
fn asking_to_check_for_new_photos_while_busy_says_it_will_wait() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let lib = Arc::clone(&app.borrow().lib);
    let release = Arc::new(AtomicBool::new(false));
    let r = Arc::clone(&release);
    lib.spawn(move || {
        while !r.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    })
    .unwrap();
    let mut h = album_harness(&app);
    app.borrow_mut().start_indexing();
    h.run_steps(3);
    h.get_by_label("Recollect will check for new photos when it finishes the current task.");
    release.store(true, Ordering::Release);
    lib.wait();
    settle(&mut h, || lib.job_log.lock().contains(&"scan"));
    lib.wait();
}

#[test]
fn a_lost_background_answer_does_not_stall_the_snapshot_or_the_year_list() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut app = app.borrow_mut();
    // A snapshot thread that died without answering: the receiver is cleared, and a request
    // queued behind it goes out.
    let (tx, rx) = crossbeam_channel::bounded(1);
    app.snapshot_rx = Some(rx);
    app.snapshot_again = true;
    drop(tx);
    app.poll_snapshot();
    assert!(!app.snapshot_again, "the queued refresh was started");
    assert!(app.snapshot_rx.is_some(), "a fresh fetch is running");
    let fresh = app.snapshot_rx.take().unwrap();
    drop(fresh);
    let (tx, rx) = crossbeam_channel::bounded(1);
    app.snapshot_rx = Some(rx);
    drop(tx);
    app.poll_snapshot();
    assert!(app.snapshot_rx.is_none());
    // Same for the list of years.
    let (tx, rx) = crossbeam_channel::bounded::<Vec<i32>>(1);
    app.years_rx = Some(rx);
    app.years_seen = Some(7);
    drop(tx);
    app.poll_years(7);
    assert!(app.years_rx.is_none(), "the failed lookup is forgotten");
}
