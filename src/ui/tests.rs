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
    h.get_by_label("No model yet");
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
    app.results = vec![
        hit("/nowhere/clip.mp4", MediaKind::Video, Some(42.0)),
        hit("/nowhere/photo.jpg", MediaKind::Image, None),
    ];
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
    app.results = ["a.jpg", "b.jpg", "c.jpg"]
        .iter()
        .map(|n| hit(&format!("/nowhere/{n}"), MediaKind::Image, None))
        .collect();
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
    use crate::media::MediaKind;
    let lib = library(d);
    fake_install(&lib);
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.results = ["a.jpg", "b.jpg", "c.jpg"]
        .iter()
        .map(|n| hit(&format!("/nowhere/{n}"), MediaKind::Image, None))
        .collect();
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
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
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
