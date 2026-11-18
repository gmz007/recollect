//! Headless UI tests (`egui_kittest`). No model or GPU needed.
#![expect(clippy::too_many_lines, reason = "a scenario test reads top to bottom")]
#![expect(clippy::cast_possible_wrap, reason = "small test fixture indices")]

mod albums;
mod favorites;
mod features;
mod guide;
mod photo_grid;
mod pill;
mod results;
mod ruler;
mod search;
mod settings;
mod timeline;
mod viewer;
mod welcome;

use super::App;
use super::timeline::View;
use crate::Library;
use crate::config::Paths;
use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::model::{download, registry};
use crate::store::db::TimelineItem;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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

fn library(dir: &std::path::Path) -> Arc<Library> {
    crate::gpu::force_cpu(true);
    Library::open(Paths::under(dir), Some(registry::CLIP_B32.id)).unwrap()
}

/// Creates empty placeholder model files so the app believes the model is installed.
fn fake_install(lib: &Library) {
    download::PLACEHOLDERS_COUNT.store(true, std::sync::atomic::Ordering::Relaxed);
    let dir = download::model_dir(&lib.paths().models_dir(), lib.model());
    std::fs::create_dir_all(&dir).unwrap();
    for f in lib.model().files {
        std::fs::File::create(dir.join(f.local)).unwrap();
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

/// Opens Settings on `section` by its list entry.
fn open_section(h: &mut Harness<'_>, section: &str) {
    if h.query_by_label("Close settings").is_none() {
        h.get_by_label("Settings").click();
        h.run();
    }
    h.get_by_label(&format!("{section} settings")).click();
    h.run();
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

/// Whether the tile labelled `label` is selected, as assistive technology sees it.
fn is_selected(h: &Harness<'_>, label: &str) -> bool {
    h.get_by_label(label)
        .accesskit_node()
        .toggled()
        .is_some_and(|t| format!("{t:?}") == "True")
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
    app.res.hits = Arc::new(
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
    app.res.current = Some(super::Query::Text("a dragon".into()));
    app.res.label = "a dragon".into();
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

/// How many frames `Harness::run` may take to settle. The default (4) is too tight: the app
/// asks for a repaint half a second after the library was last busy, which is 30 frames at
/// 60 fps. A screen that never stops repainting still fails.
const MAX_STEPS: u64 = 64;

/// Runs frames until the screen settles. While a library job runs, the app polls it every half
/// second, so the screen cannot settle (on a slow machine the first scan may still be going
/// when a test starts): this waits up to about 5 s for such a job, then carries on without
/// failing, since a test may hold a job on purpose.
fn run_settled(h: &mut Harness<'_>, app: &Rc<RefCell<App>>) {
    let lib = Arc::clone(&app.borrow().lib);
    for _ in 0..250 {
        if !lib.is_busy() && h.try_run().is_ok() {
            return;
        }
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn album_harness(app: &Rc<RefCell<App>>) -> Harness<'static> {
    let shown = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_max_steps(MAX_STEPS)
        .build_ui(move |ui| shown.borrow_mut().show(ui));
    run_settled(&mut h, app);
    h
}

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
    let shown = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_step_dt(0.05)
        .with_max_steps(MAX_STEPS)
        .build_ui(move |ui| shown.borrow_mut().show(ui));
    run_settled(&mut h, app);
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

fn file_id_of(lib: &Library, name: &str) -> i64 {
    lib.timeline(&crate::store::db::TimelineFilter::default())
        .unwrap()
        .into_iter()
        .find(|t| t.path.ends_with(name))
        .unwrap()
        .file_id
}

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
                        taken_at: 1_500_000_000 + i / 40 * 86_400 + i * 60,
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

/// `spread_items` with the first 24 photos taken on one day (under one heading) and small
/// tiles, so the first twelve photos are all on screen.
fn small_tiles_harness(app: &Rc<RefCell<App>>) -> Harness<'static> {
    {
        let mut a = app.borrow_mut();
        let mut items = spread_items();
        for it in items.iter_mut().take(24) {
            it.taken_at = titem(0, 2019, 12, 15).taken_at;
        }
        a.inject_timeline(items);
        a.tl.tile_size = 96.0;
    }
    album_harness(app)
}

fn tl_selection(app: &Rc<RefCell<App>>) -> Vec<i64> {
    app.borrow().selected_file_ids()
}

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
        // `awaiting`: the album's own list has arrived, not just the previous view's.
        matches!(a.tl.view, View::Album(_))
            && a.tl.items.len() == n
            && a.tl.loaded
            && a.tl.awaiting.is_none()
    });
    h.run_steps(3);
    (lib, app, h)
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
        // File system paths are shown as they are (Settings > Storage), on Windows too.
        if std::path::Path::new(t).is_absolute() {
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

/// Like `quick_harness` for a window `width` points wide.
fn quick_harness_wide(app: &Rc<RefCell<App>>, width: f32) -> Harness<'static> {
    let shown = Rc::clone(app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(width, 800.0))
        .with_step_dt(0.05)
        .with_max_steps(MAX_STEPS)
        .build_ui(move |ui| shown.borrow_mut().show(ui));
    run_settled(&mut h, app);
    h
}

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
            app.res.current = Some(super::Query::Text("snow".into()));
            app.res.label = "snow".into();
            if matches!(which, EmptyView::NoResultsDated) {
                app.date = DateFilter {
                    when: When::ThisYear,
                    from: 0,
                    to: 0,
                };
            }
            if matches!(which, EmptyView::SearchOff) {
                app.res.error = Some(super::guide::search_problem(
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

/// An app whose smart search looks installed to the interface (so the search field is on)
/// while a search still fails cleanly because no files exist.
fn searchable_app(d: &std::path::Path) -> Rc<RefCell<App>> {
    let mut app = grid_app_with(d, false);
    app.snapshot.model_installed = true;
    app.res.hits = Arc::default();
    app.res.current = None;
    app.res.label.clear();
    Rc::new(RefCell::new(app))
}
