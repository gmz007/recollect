//! Headless UI tests (egui_kittest). No model or GPU needed.

use super::App;
use crate::Library;
use crate::config::Paths;
use crate::model::{download, registry};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.show(ui));
    h.run();
    h
}

#[test]
fn first_run_asks_before_downloading_the_model() {
    let d = tempfile::tempdir().unwrap();
    let h = harness(library(d.path()));
    h.get_by_label_contains("One-time setup");
    h.get_by_label("Download model");
    h.get_by_label("Not now");
}

#[test]
fn declining_the_download_shows_a_hint() {
    let d = tempfile::tempdir().unwrap();
    let mut h = harness(library(d.path()));
    h.get_by_label("Not now").click();
    h.run();
    h.get_by_label("Search is off until the model is downloaded.");
    h.get_by_label("Download model…");
    assert!(h.query_by_label_contains("One-time setup").is_none());
}

#[test]
fn empty_library_invites_adding_a_folder() {
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let h = harness(lib);
    h.get_by_label("Add a folder to get started");
    h.get_by_label("Add folder…");
}

#[test]
fn settings_window_lists_models() {
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(lib);
    h.get_by_label("Settings").click();
    h.run();
    h.get_by_label("Search model");
    for m in registry::MODELS {
        h.get_by_label_contains(&format!("{}  (", m.name));
    }
    h.get_by_label("Use the GPU when available");
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
fn results_and_video_detail_panel() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    // No real folder: adding one would start a background rescan that races the test.
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.results = Arc::new(vec![
        hit("/nowhere/clip.mp4", MediaKind::Video, Some(42.0)),
        hit("/nowhere/photo.jpg", MediaKind::Image, None),
    ]);
    app.current = Some(super::Query::Text("a dragon".into()));
    app.result_label = "a dragon".into();
    app.selected = Some(0);
    app.selected_t = Some(42.0);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.show(ui));
    h.run();
    h.get_by_label_contains("Best 2 matches for “a dragon”");
    h.get_by_label("clip.mp4");
    h.get_by_label("Play at 0:42");
    h.get_by_label("Matching moments");
    h.get_by_label_contains("1:40 – 1:44");
    h.get_by_label("Find similar");
    h.get_by_label("Show in folder");
}

#[test]
fn arrow_keys_move_the_selection() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.results = Arc::new(
        ["a.jpg", "b.jpg", "c.jpg"]
            .iter()
            .map(|n| hit(&format!("/nowhere/{n}"), MediaKind::Image, None))
            .collect(),
    );
    app.current = Some(super::Query::Text("a dragon".into()));
    app.result_label = "a dragon".into();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| app.show(ui));
    h.run();
    assert!(h.query_by_label("c.jpg").is_none());
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.get_by_label("a.jpg");
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.get_by_label("c.jpg");
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    h.get_by_label("b.jpg");
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
            .map(|n| hit(&format!("/nowhere/{n}"), MediaKind::Image, None))
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
    assert!(h.query_by_label("a.jpg").is_none());
    h.key_press(egui::Key::Escape);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.get_by_label("a.jpg");
}

#[test]
fn modified_arrows_do_not_move_the_selection() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    h.run();
    assert!(h.query_by_label("a.jpg").is_none());
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.get_by_label("a.jpg");
}

#[test]
fn arrows_are_ignored_while_settings_is_open() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Settings").click();
    h.run();
    h.get_by_label("Search model");
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(h.query_by_label("a.jpg").is_none());
}

#[test]
fn clicking_a_tile_keeps_arrows_working() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Result a.jpg").click();
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.get_by_label("b.jpg");
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
    h.get_by_label("240 items");
    h.get_by_label("Group by");
    h.get_by_label("Tile size");
    h.get_by_label("Year").click();
    h.run();
    h.get_by_label("2019 · 12");
    assert!(h.query_by_label("December 2019 · 1").is_none());
    h.get_by_label("Day").click();
    h.run();
    h.get_by_label("Sunday 15 December 2019 · 1");
}

#[test]
fn navigator_jumps_to_a_year() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    assert!(h.query_by_label("December 2005 · 1").is_none());
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
    assert!(h.query_by_label("Group by").is_none());
}

#[test]
fn banner_shows_while_the_timeline_stays_visible() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let h = album_harness(&app);
    h.get_by_label_contains("One-time setup");
    h.get_by_label("Download model");
    h.get_by_label("Not now");
    h.get_by_label("December 2019 · 1");
    assert_eq!(app.borrow().tl.group, Group::Month);
}

#[test]
fn selecting_a_timeline_item_shows_its_details() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Item img1.jpg").click();
    h.run();
    h.get_by_label("img1.jpg");
    h.get_by_label_contains("Sunday 15 December 2019");
    h.get_by_label("Test Camera");
    h.get_by_label("4000×3000");
    h.get_by_label("Find similar");
    h.get_by_label("Open");
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert_eq!(app.borrow().tl.selected, Some(1));
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

/// Like `album_harness` with short frames, so two clicks a frame apart still count as a double
/// click (egui allows 0.3 s between them).
fn quick_harness(app: &Rc<RefCell<App>>) -> Harness<'static> {
    let app = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_step_dt(0.05)
        .build_ui(move |ui| app.borrow_mut().show(ui));
    h.run();
    h
}

/// Two clicks on a tile, a frame apart, near its top left corner (the tile may move a little
/// when the first click opens the detail panel).
fn double_click(h: &mut Harness<'_>, label: &str) {
    let pos = h.get_by_label(label).rect().min + egui::vec2(12.0, 12.0);
    h.event(egui::Event::PointerMoved(pos));
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.step();
    }
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
    h.get_by_label("Item img1.jpg").click();
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    // The viewer replaces the album: no grid, no sidebar.
    h.get_by_label("img1.jpg");
    h.get_by_label("1 / 240");
    h.get_by_label("Close");
    h.get_by_label("Next");
    h.get_by_label("Info");
    assert!(
        h.query_by_label("Previous").is_none(),
        "nothing before the first item"
    );
    assert!(h.query_by_label("Item img1.jpg").is_none());
    assert!(h.query_by_label("Timeline").is_none());
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("img2.jpg");
    h.get_by_label("Previous");
    assert!(h.query_by_label("img1.jpg").is_none());
    h.key_press(egui::Key::End);
    h.run_steps(3);
    h.get_by_label("240 / 240");
    assert!(h.query_by_label("Next").is_none());
    h.key_press(egui::Key::Home);
    h.run_steps(3);
    h.get_by_label("1 / 240");
    h.get_by_label("Next").click();
    h.run_steps(3);
    h.get_by_label("img2.jpg");
    // Esc returns to the grid with the item selected.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    assert_eq!(app.borrow().tl.selected, Some(1));
    h.get_by_label("Item img2.jpg");
    h.get_by_label("Timeline");
}

#[test]
fn double_click_opens_the_viewer_and_the_info_panel_toggles() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    double_click(&mut h, "Item img1.jpg");
    h.run_steps(3);
    h.get_by_label("img1.jpg");
    h.get_by_label_contains("Sunday 15 December 2019");
    h.get_by_label("Test Camera");
    h.get_by_label("4000×3000");
    h.get_by_label("Open with default app");
    h.get_by_label("Show in folder");
    h.get_by_label("Find similar");
    h.get_by_label("Favorite");
    // `I` hides the panel (the name moves to the top bar) and shows it again.
    h.key_press(egui::Key::I);
    h.run_steps(3);
    h.get_by_label("img1.jpg");
    assert!(h.query_by_label("Open with default app").is_none());
    h.get_by_label("Favorite");
    h.key_press(egui::Key::I);
    h.run_steps(3);
    h.get_by_label("Open with default app");
    // Zoom toggles with Z and with the button.
    h.get_by_label("Zoom to 100%");
    h.key_press(egui::Key::Z);
    h.run_steps(3);
    h.get_by_label("Fit to window").click();
    h.run_steps(3);
    h.get_by_label("Zoom to 100%");
    // Close button.
    h.get_by_label("Close").click();
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
fn f_marks_a_favorite_and_the_favorites_view_lists_it() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 3);
    h.get_by_label("Item b.jpg").click();
    h.run_steps(3);
    h.get_by_label("Favorite");
    h.key_press(egui::Key::F);
    h.run_steps(3);
    // The grid and the detail panel change at once...
    h.get_by_label("Unfavorite");
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
    h.get_by_label("Item b.jpg");
    assert!(h.query_by_label("Item a.jpg").is_none());
    // F again removes the mark; the item leaves the view right away.
    h.get_by_label("Item b.jpg").click();
    h.run_steps(3);
    h.key_press(egui::Key::F);
    h.run_steps(3);
    assert!(h.query_by_label("Item b.jpg").is_none());
    h.get_by_label("No favorites yet");
    assert_eq!(app.borrow().tl.selected, None);
    settle(&mut h, || lib.timeline(&fav).unwrap().is_empty());
    // ...and it stays gone after the view is reloaded.
    settle(&mut h, || app.borrow().fav_marks.is_empty());
    assert!(app.borrow().tl.items.is_empty());
}

#[test]
fn favorites_toggle_in_the_viewer() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = quick_harness(&app);
    double_click(&mut h, "Item img1.jpg");
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
fn search_shows_the_when_and_sort_selectors() {
    let d = tempfile::tempdir().unwrap();
    let app = Rc::new(RefCell::new(grid_app_with(d.path(), false)));
    let mut h = album_harness(&app);
    h.get_by_label("When");
    h.get_by_label("Sort");
    assert_eq!(app.borrow().date.when, super::when::When::Any);
    h.get_by_label("When").click();
    h.run_steps(3);
    h.get_by_label("Last year").click();
    h.run_steps(3);
    assert_eq!(app.borrow().date.when, super::when::When::LastYear);
}

#[test]
fn browsing_has_no_when_and_sort_selectors() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = album_harness(&app);
    assert!(h.query_by_label("When").is_none());
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
    h.get_by_label("1 / 20");
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("2 / 20");
    h.get_by_label("img22.jpg"); // March 2018
    // Not in the other views.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.get_by_label("Recently added").click();
    h.run_steps(3);
    assert!(h.query_by_label("On this day").is_none());
}

#[test]
fn the_status_bar_does_not_blame_waiting_without_a_model() {
    let r = crate::library::RunSummary::default();
    let without = super::run_summary_text(&r, 168, false);
    assert!(
        without.starts_with("168 files will be searchable after the model download"),
        "{without}"
    );
    assert!(!without.contains("waiting"));
    assert!(super::run_summary_text(&r, 168, true).contains("168 files still waiting"));
    assert!(super::run_summary_text(&r, 0, false).starts_with("Up to date"));
    assert_eq!(
        super::waiting_for_model_text(1),
        "1 file will be searchable after the model download"
    );
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
            .notice
            .as_ref()
            .is_some_and(|n| n.0.contains("Could not save"))
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
    double_click(&mut h, "Item img1.jpg");
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
    double_click(&mut h, "Item img1.jpg");
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
    double_click(&mut h, "Item img1.jpg");
    h.run_steps(5);
    assert!(app.borrow().previews.cached() > 0);
    h.get_by_label("Find similar").click();
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
    h.get_by_label("Item img2.jpg").click();
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [2]);
    h.get_by_label("img2.jpg");
    click_with(&mut h, "Item img4.jpg", ctrl);
    assert_eq!(tl_selection(&app), [2, 4]);
    h.get_by_label("2 selected");
    h.get_by_label("2 items selected");
    // Shift+click selects from the anchor (img4) to the clicked item.
    click_with(&mut h, "Item img7.jpg", shift);
    assert_eq!(tl_selection(&app), [4, 5, 6, 7]);
    h.get_by_label("4 selected");
    // Ctrl+click toggles one item off again.
    click_with(&mut h, "Item img5.jpg", ctrl);
    assert_eq!(tl_selection(&app), [4, 6, 7]);
    // Plain click collapses to one and shows its details.
    h.get_by_label("Item img3.jpg").click();
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [3]);
    assert!(h.query_by_label("Clear selection").is_none());
    // Ctrl+A selects the whole list.
    h.key_press_modifiers(ctrl, egui::Key::A);
    h.run_steps(3);
    assert_eq!(tl_selection(&app).len(), 240);
    h.get_by_label("240 selected");
    // A plain arrow collapses to one selection, moved.
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [4]);
    // Shift+arrows extend it.
    h.key_press_modifiers(shift, egui::Key::ArrowRight);
    h.run_steps(3);
    h.key_press_modifiers(shift, egui::Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(tl_selection(&app), [4, 5, 6]);
    h.get_by_label("3 selected");
    // Escape clears everything.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
    assert_eq!(app.borrow().tl.selected, None);
    assert!(h.query_by_label("Clear selection").is_none());
}

#[test]
fn the_selection_survives_a_reload() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = small_tiles_harness(&app);
    h.get_by_label("Item img1.jpg").click();
    h.run_steps(3);
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
fn clear_selection_button_and_search_results_multi_select() {
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
    h.get_by_label("Result r1.jpg").click();
    h.run_steps(3);
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::SHIFT);
    assert_eq!(tl_selection(&app), [1, 2, 3]);
    h.get_by_label("3 selected");
    h.get_by_label("3 items selected");
    h.get_by_label("Add to album…");
    h.get_by_label("Favorite");
    h.get_by_label("Unfavorite");
    assert!(h.query_by_label("Remove from album").is_none());
    h.get_by_label("Clear selection").click();
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
    settle(&mut h, || app.borrow().snapshot.albums.len() == 1);
    h.get_by_label("Trip (0)");

    // Add a multi-selection through the selection bar.
    h.get_by_label("Item p4.jpg").click();
    h.run_steps(3);
    click_with(&mut h, "Item p2.jpg", ctrl);
    click_with(&mut h, "Item p0.jpg", ctrl);
    h.get_by_label("3 selected");
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Trip").click();
    settle(&mut h, || {
        lib.albums().unwrap()[0].count == 3 && app.borrow().snapshot.albums[0].count == 3
    });
    h.get_by_label_contains("Added 3 items to “Trip”");
    // Adding them again changes nothing.
    h.get_by_label("Add to album…").click();
    h.run_steps(3);
    h.get_by_label("Trip").click();
    settle(&mut h, || {
        app.borrow()
            .notice
            .as_ref()
            .is_some_and(|n| n.0.contains("Already in"))
    });
    assert_eq!(lib.albums().unwrap()[0].count, 3);

    // The album view lists exactly those items.
    h.get_by_label("Trip (3)").click();
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.view == View::Album(a.snapshot.albums[0].id) && a.tl.items.len() == 3 && a.tl.loaded
    });
    h.run_steps(3);
    h.get_by_label("Item p4.jpg");
    h.get_by_label("Item p2.jpg");
    h.get_by_label("Item p0.jpg");
    assert!(h.query_by_label("Item p3.jpg").is_none());
    h.get_by_label("Rename…");
    h.get_by_label("Delete album…");

    // Remove one from the album with the detail panel's button.
    h.get_by_label("Item p2.jpg").click();
    h.run_steps(3);
    h.get_by_label("Remove from album").click();
    h.run_steps(3);
    settle(&mut h, || lib.albums().unwrap()[0].count == 2);
    assert!(h.query_by_label("Item p2.jpg").is_none());
    // ...and two with the selection bar.
    h.get_by_label("Item p4.jpg").click();
    h.run_steps(3);
    click_with(&mut h, "Item p0.jpg", shift);
    h.get_by_label("Remove from album").click();
    settle(&mut h, || lib.albums().unwrap()[0].count == 0);
    settle(&mut h, || {
        let a = app.borrow();
        a.tl.items.is_empty() && a.tl.awaiting.is_none() && a.tl.loaded
    });
    h.run_steps(3);
    h.get_by_label("This album is empty");
    assert_eq!(lib.timeline(&Default::default()).unwrap().len(), 5);

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
        lib.albums().unwrap()[0].name == "Holiday"
            && app.borrow().snapshot.albums[0].name == "Holiday"
    });
    h.get_by_label("Holiday");
    h.get_by_label_contains("Holiday (");

    // Delete needs a confirmation that says the photos stay.
    h.get_by_label("Delete album…").click();
    h.run_steps(3);
    h.get_by_label("The photos and videos in it are not deleted.");
    h.get_by_label("Cancel").click();
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
    app.borrow_mut().refresh_snapshot(true);
    settle(&mut h, || app.borrow().snapshot.albums.len() == 1);
    h.get_by_label("Item p3.jpg").click();
    h.run_steps(3);
    // Single item: detail panel menu; "New album…" creates an album holding the item.
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
        a.tl.items.len() == 3 && a.snapshot.albums.len() == 1
    });
    app.borrow_mut().refresh_snapshot(true);
    settle(&mut h, || !app.borrow().snapshot.albums.is_empty());
    double_click(&mut h, "Item p1.jpg");
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
