#![expect(clippy::too_many_lines, reason = "a scenario test reads top to bottom")]
use super::*;
use crate::indexer::Embedded;
use crate::media::{MediaKind, ScannedFile};
use crate::search::SearchSort;
use crate::store::db::{FileMeta, NewItem};
use std::collections::HashMap;
use std::path::Path;

fn open(dir: &Path) -> Arc<Library> {
    crate::gpu::force_cpu(true);
    Library::open(Paths::under(dir), Some(registry::CLIP_B32.id)).unwrap()
}

/// Stores one indexed image with a CLIP-sized vector; returns its item id.
fn seed(lib: &Library) -> i64 {
    let mut db = lib.db.lock();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    let file = ScannedFile {
        path: "/lib/a.jpg".into(),
        kind: MediaKind::Image,
        size: 1,
        mtime: 1,
    };
    db.sync_folder(f, &[file], &[]).unwrap();
    let id = db.pending(registry::CLIP_B32.id).unwrap()[0].id;
    let mut v = vec![0.0; 512];
    v[0] = 1.0;
    let item = NewItem {
        t_start: None,
        t_end: None,
        embedding: v,
    };
    db.put_items(
        registry::CLIP_B32.id,
        &[(id, FileMeta::default(), vec![item])],
    )
    .unwrap()[0][0]
}

#[test]
fn model_switch_rebuilds_index_and_old_vectors_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    let item = seed(&open(d.path()));
    let lib = open(d.path()); // reload: the index now holds the seeded vector
    assert_eq!(lib.indexed_items(), 1);
    assert!(
        lib.similar(item, &SearchOptions::default())
            .unwrap()
            .is_empty()
    );

    lib.set_model(registry::SIGLIP_B16.id).unwrap();
    lib.wait();
    assert_eq!(lib.status().last_error, None);
    assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
    assert_eq!(lib.indexed_items(), 0);
    let saved = Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
    // The old 512-d vector must produce an error, not a panic.
    let err = lib.similar(item, &SearchOptions::default()).unwrap_err();
    assert_eq!(SearchError::of(&err), SearchError::NotReady, "{err}");
    assert!(search::by_vector(&lib, &[0.0; 512], &SearchOptions::default(), None).is_err());
}

#[test]
fn a_settings_change_during_a_model_switch_is_kept() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    {
        // The switch job waits for the database while the user changes the theme.
        let _db = lib.db.lock();
        lib.set_model(registry::SIGLIP_B16.id).unwrap();
        lib.update_settings(|s| {
            s.theme = crate::config::Theme::Dark;
            s.use_gpu = false;
        })
        .unwrap();
    }
    lib.wait();
    assert_eq!(lib.status().last_error, None);
    assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
    let saved = Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
    assert_eq!(saved.theme, crate::config::Theme::Dark);
    assert!(!saved.use_gpu);
    assert_eq!(lib.settings(), saved);
}

#[test]
fn update_settings_leaves_the_model_choice_alone() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    let before = lib.settings().model;
    lib.update_settings(|s| {
        s.model = Some("something-else".into());
        s.search_prompt_dismissed = true;
    })
    .unwrap();
    assert_eq!(lib.settings().model, before);
    assert!(lib.settings().search_prompt_dismissed);
    let saved = Settings::load(&lib.paths().settings_path()).unwrap();
    assert!(saved.search_prompt_dismissed);
}

#[test]
fn every_recorded_error_is_counted_even_when_the_text_repeats() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    assert_eq!(lib.status().error_seq, 0);
    lib.update_status(|s| s.set_error("disk full"));
    let first = lib.status().error_seq;
    lib.update_status(|s| s.set_error("disk full"));
    let st = lib.status();
    assert_eq!(st.last_error.as_deref(), Some("disk full"));
    assert_eq!(st.error_seq, first + 1);
}

#[test]
fn long_videos_do_not_crowd_out_other_files() {
    let d = tempfile::tempdir().unwrap();
    {
        let lib = open(d.path());
        let mut db = lib.db.lock();
        let f = db.add_folder(Path::new("/lib")).unwrap();
        let mut files = vec![ScannedFile {
            path: "/lib/long.mp4".into(),
            kind: MediaKind::Video,
            size: 1,
            mtime: 1,
        }];
        for i in 0..30 {
            files.push(ScannedFile {
                path: format!("/lib/{i:02}.jpg").into(),
                kind: MediaKind::Image,
                size: 1,
                mtime: 1,
            });
        }
        db.sync_folder(f, &files, &[]).unwrap();
        // Spread over all dimensions like real embeddings (no saturating components):
        // q = constant direction, r = orthogonal alternating direction.
        let unit = |a: f32| {
            let c = 1.0 / (512f32).sqrt();
            (0..512)
                .map(|i| c * a.cos() + if i % 2 == 0 { c } else { -c } * a.sin())
                .collect::<Vec<f32>>()
        };
        let mut batch = Vec::new();
        for row in db.pending(registry::CLIP_B32.id).unwrap() {
            let items = if row.kind == MediaKind::Video {
                // 1000 segments all closer to the query than any image.
                (0..1000)
                    .map(|i| NewItem {
                        t_start: Some(f64::from(i) * 2.0),
                        t_end: Some(f64::from(i) * 2.0 + 2.0),
                        embedding: unit(0.01),
                    })
                    .collect()
            } else {
                vec![NewItem {
                    t_start: None,
                    t_end: None,
                    embedding: unit(0.5),
                }]
            };
            batch.push((row.id, FileMeta::default(), items));
        }
        db.put_items(registry::CLIP_B32.id, &batch).unwrap();
    }
    let lib = open(d.path());
    let q = vec![1.0 / (512f32).sqrt(); 512];
    let hits = search::by_vector(
        &lib,
        &q,
        &SearchOptions {
            limit: 20,
            kind: None,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(hits.len(), 20);
    assert_eq!(hits[0].kind, MediaKind::Video);
    assert_eq!(hits[0].moments.len(), 10);
    // "Find similar" from the video must skip the video's own segments.
    let hits = lib
        .similar(
            hits[0].item_id,
            &SearchOptions {
                limit: 20,
                kind: None,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(hits.len(), 20);
    assert!(hits.iter().all(|h| h.kind == MediaKind::Image));
}

#[test]
fn empty_mount_point_keeps_the_index_and_missing_files_stay_pending() {
    let d = tempfile::tempdir().unwrap();
    let media = d.path().join("mnt");
    std::fs::create_dir_all(&media).unwrap();
    let lib = open(d.path());
    lib.add_folder(&media).unwrap();
    {
        // As if indexed earlier, then the drive was unmounted (empty mount point).
        let mut db = lib.db.lock();
        let f = db.folders().unwrap()[0].id;
        let file = ScannedFile {
            path: media.join("a.jpg"),
            kind: MediaKind::Image,
            size: 1,
            mtime: 1,
        };
        db.sync_folder(f, &[file], &[]).unwrap();
        let id = db.pending(registry::CLIP_B32.id).unwrap()[0].id;
        let item = NewItem {
            t_start: None,
            t_end: None,
            embedding: vec![0.1; 512],
        };
        db.put_items(
            registry::CLIP_B32.id,
            &[(id, FileMeta::default(), vec![item])],
        )
        .unwrap();
    }
    let sum = crate::indexer::run(&lib).unwrap();
    assert_eq!(
        sum.skipped_folders,
        vec![std::fs::canonicalize(&media).unwrap()]
    );
    assert_eq!(lib.stats().unwrap().indexed, 1, "index kept");

    // A file that vanishes between scan and decode is not marked as failed.
    let row = crate::store::db::FileRow {
        id: 1,
        path: media.join("gone.jpg"),
        kind: MediaKind::Image,
        size: 1,
        mtime: 1,
        status: crate::store::db::FileStatus::Pending,
    };
    let err = anyhow!("decode failed");
    assert_eq!(
        crate::indexer::file_failed(&lib, &row, &err).unwrap(),
        crate::indexer::Failure::Retry
    );
    // A file that is still there but cannot be decoded is recorded as an error.
    let bad = media.join("bad.jpg");
    std::fs::write(&bad, b"not an image").unwrap();
    let row = crate::store::db::FileRow { path: bad, ..row };
    assert_eq!(
        crate::indexer::file_failed(&lib, &row, &err).unwrap(),
        crate::indexer::Failure::Recorded
    );
}

#[test]
fn retry_errors_and_sync_of_a_removed_folder() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    let file = ScannedFile {
        path: "/lib/a.jpg".into(),
        kind: MediaKind::Image,
        size: 1,
        mtime: 1,
    };
    db.sync_folder(f, std::slice::from_ref(&file), &[]).unwrap();
    let id = db.pending("m").unwrap()[0].id;
    db.set_error(id, "bad").unwrap();
    assert!(db.pending("m").unwrap().is_empty());
    assert_eq!(db.retry_errors().unwrap(), 1);
    assert_eq!(db.pending("m").unwrap().len(), 1);
    db.remove_folder(f).unwrap();
    // A scan that started before the removal must not resurrect rows (or hit FK errors).
    assert_eq!(
        db.sync_folder(f, &[file], &[]).unwrap(),
        crate::store::db::SyncStats::default()
    );
    assert_eq!(db.stats("m").unwrap().images, 0);
}

#[test]
fn index_cache_is_written_and_reused() {
    let d = tempfile::tempdir().unwrap();
    seed(&open(d.path()));
    let lib = open(d.path()); // rebuilt from the database
    assert_eq!(lib.indexed_items(), 1);
    lib.save_index_cache();
    let cache = index_cache_path(lib.paths(), lib.model());
    assert!(cache.exists());
    drop(lib);
    let lib = open(d.path());
    assert_eq!(lib.indexed_items(), 1, "loaded from the cache");
    assert!(lib.loaded_from_cache());
    let id = lib.db.lock().library_id().unwrap();
    let load = |id: &str, epoch: u64| {
        VectorIndex::load(
            &cache,
            lib.model().id,
            id,
            512,
            lib.model().quant_scale,
            epoch,
        )
    };
    let epoch = lib.db.lock().items_epoch().unwrap();
    assert!(load(&id, epoch).is_some());
    assert!(load("another-library", epoch).is_none(), "library id");
    // Any database change makes the cache stale.
    lib.db.lock().set_meta("items_epoch", "999").unwrap();
    let epoch = lib.db.lock().items_epoch().unwrap();
    assert!(load(&id, epoch).is_none());
}

#[test]
fn the_index_cache_is_saved_when_a_running_job_ends_not_while_it_runs() {
    let d = tempfile::tempdir().unwrap();
    seed(&open(d.path()));
    let lib = open(d.path());
    let cache = index_cache_path(lib.paths(), lib.model());
    let (release, hold) = std::sync::mpsc::channel::<()>();
    lib.spawn(move || {
        let _ = hold.recv();
    })
    .unwrap();
    lib.save_index_cache();
    assert!(!cache.exists(), "no save while a job is running");
    release.send(()).unwrap();
    lib.wait();
    assert!(cache.exists(), "the job saves when it ends");
}

#[test]
fn unchanged_index_is_not_rewritten_and_stale_temp_files_are_removed() {
    let d = tempfile::tempdir().unwrap();
    seed(&open(d.path()));
    let lib = open(d.path());
    assert!(!lib.loaded_from_cache());
    let cache = index_cache_path(lib.paths(), lib.model());
    assert!(!cache.exists(), "open must not write the cache");
    lib.save_index_cache();
    let first = std::fs::metadata(&cache).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    lib.save_index_cache();
    assert_eq!(
        std::fs::metadata(&cache).unwrap().modified().unwrap(),
        first
    );
    // A change makes the next save write again.
    lib.db.lock().set_meta("items_epoch", "999").unwrap();
    lib.save_index_cache();
    assert!(std::fs::metadata(&cache).unwrap().modified().unwrap() > first);
    // Leftovers of a crashed save are cleaned at open.
    let tmp = lib.paths().data.join("index-clip-vit-b32.1.2.tmp");
    let fresh = lib.paths().data.join("index-clip-vit-b32.3.4.tmp");
    std::fs::write(&tmp, b"x").unwrap();
    std::fs::write(&fresh, b"x").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&tmp)
        .unwrap()
        .set_modified(old)
        .unwrap();
    drop(lib);
    let _lib = open(d.path());
    assert!(!tmp.exists(), "abandoned temp file removed");
    assert!(
        fresh.exists(),
        "a save in progress (fresh temp file) is left alone"
    );
}

#[test]
fn removing_a_folder_in_the_background_clears_the_index() {
    let d = tempfile::tempdir().unwrap();
    seed(&open(d.path()));
    let lib = open(d.path());
    let folder = lib.folders().unwrap()[0].id;
    lib.start_remove_folder(folder);
    for _ in 0..200 {
        if lib.folders().unwrap().is_empty() && !lib.is_busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    lib.wait();
    assert!(lib.folders().unwrap().is_empty());
    assert_eq!(lib.indexed_items(), 0);
}

/// Writes every file of the current model with its expected size (sparse), so the download
/// finds nothing to fetch.
fn fake_install(lib: &Library) {
    let dir = lib.model_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for f in lib.model().files {
        let file = std::fs::File::create(dir.join(f.local)).unwrap();
        file.set_len(f.size).unwrap();
    }
    assert!(lib.model_installed());
}

fn wait_idle(lib: &Library) {
    for _ in 0..500 {
        if !lib.is_busy() && lib.status().last_run.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    lib.wait();
}

#[test]
fn a_finished_download_continues_with_an_indexing_run() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    let empty = d.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    lib.add_folder(&empty).unwrap();
    fake_install(&lib);
    lib.update_status(|s| {
        s.last_run = Some(RunSummary {
            files: 99,
            ..Default::default()
        });
    });
    lib.start_download().unwrap();
    wait_idle(&lib);
    let st = lib.status();
    assert_eq!(st.last_error, None);
    let run = st.last_run.expect("the download was followed by a run");
    assert_eq!(run.files, 0, "the stale summary was replaced");
    assert_eq!(st.activity, Activity::Idle);
}

#[test]
fn downloading_while_another_job_runs_cancels_it_first() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    fake_install(&lib);
    let l = Arc::clone(&lib);
    // A job that only ends when cancelled (stands in for a long dates-only scan).
    lib.spawn(move || {
        while !l.control.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    })
    .unwrap();
    lib.start_download().unwrap();
    wait_idle(&lib);
    assert_eq!(lib.status().last_error, None);
    assert!(lib.status().last_run.is_some(), "download, then indexing");
}

/// A folder with one picture, for the tests of the job queue.
fn one_photo_folder(d: &Path) -> PathBuf {
    let photos = d.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    image::RgbImage::new(4, 4)
        .save(photos.join("a.png"))
        .unwrap();
    photos
}

/// Waits until `cond` holds and no job runs (jobs may follow one another).
fn wait_until(lib: &Library, mut cond: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if cond() && !lib.is_busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    lib.wait();
    assert!(cond(), "condition not reached");
}

#[test]
fn folders_are_scanned_before_a_download_that_was_asked_for_with_them() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(&d.path().join("home"));
    fake_install(&lib);
    // (Empty, so the placeholder model files never have to embed anything.)
    let empty = d.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    lib.start_add_folders(vec![empty], AfterAdd::DownloadModel);
    wait_until(&lib, || lib.job_log.lock().len() >= 3);
    // The scan, then the download, then the scan that follows a download.
    assert_eq!(*lib.job_log.lock(), ["scan", "download", "scan"]);
    assert_eq!(lib.folders().unwrap().len(), 1);
    assert_eq!(lib.status().last_error, None);
}

#[test]
fn a_failed_download_still_leaves_the_folders_scanned() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(&d.path().join("home"));
    lib.fail_download.store(true, Ordering::Release);
    let photos = one_photo_folder(d.path());
    lib.start_add_folders(vec![photos], AfterAdd::DownloadModel);
    wait_until(&lib, || {
        lib.status().last_error.is_some() && lib.job_log.lock().len() >= 2
    });
    assert_eq!(*lib.job_log.lock(), ["scan", "download"]);
    assert_eq!(lib.status().last_error_kind, ErrorKind::Download);
    assert_eq!(lib.timeline(&TimelineFilter::default()).unwrap().len(), 1);
    assert_eq!(lib.date_counts().unwrap().file, 1, "dates were read");
}

#[test]
fn a_scan_asked_for_while_busy_waits_instead_of_being_dropped() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(&d.path().join("home"));
    let release = Arc::new(AtomicBool::new(false));
    let r = Arc::clone(&release);
    lib.spawn(move || {
        while !r.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    })
    .unwrap();
    assert_eq!(lib.request_scan().unwrap(), Request::Queued);
    assert_eq!(lib.request_download().unwrap(), Request::Queued);
    assert!(lib.job_log.lock().is_empty(), "nothing started yet");
    release.store(true, Ordering::Release);
    lib.fail_download.store(true, Ordering::Release);
    wait_until(&lib, || lib.job_log.lock().len() >= 2);
    assert_eq!(*lib.job_log.lock(), ["scan", "download"]);
    // With nothing running, a request starts at once.
    assert_eq!(lib.request_scan().unwrap(), Request::Started);
    lib.wait();
}

#[test]
fn indexing_without_a_model_reads_dates() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(&d.path().join("home"));
    assert!(!lib.model_installed());
    let photos = d.path().join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    image::RgbImage::new(4, 4)
        .save(photos.join("a.png"))
        .unwrap();
    lib.add_folder(&photos).unwrap();
    let sum = lib.index_blocking().unwrap();
    assert_eq!((sum.added, sum.dates_read, sum.files), (1, 1, 0));
    let tl = lib.timeline(&TimelineFilter::default()).unwrap();
    assert_eq!(tl.len(), 1);
    assert_eq!(
        lib.stats().unwrap().pending,
        1,
        "embedding is still waiting"
    );
    assert_eq!(lib.date_counts().unwrap().file, 1);
    // A second run has nothing left to read.
    assert_eq!(lib.index_blocking().unwrap().dates_read, 0);
}

fn day(y: i32, m: u32, d: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

/// A unit vector at `angle` radians from the first axis (the query used below).
fn at_angle(angle: f32) -> Vec<f32> {
    let mut v = vec![0.0; 512];
    v[0] = angle.cos();
    v[1] = angle.sin();
    v
}

fn query() -> Vec<f32> {
    at_angle(0.0)
}

fn names(hits: &[search::SearchHit]) -> Vec<String> {
    hits.iter()
        .map(|h| h.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// Four images captured on known days. The better the match, the earlier its name: a, b, c, d.
/// By date: b (2022) is newest, then d (2021), a (2020), c (2019).
fn seed_dated(lib: &Library, undated: &[&str]) -> Vec<Embedded> {
    let mut db = lib.db.lock();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    let spec = [
        ("a", 0.1, day(2020, 6, 1)),
        ("b", 0.2, day(2022, 12, 31)),
        ("c", 0.3, day(2019, 1, 10)),
        ("d", 0.4, day(2021, 3, 5)),
    ];
    let files: Vec<_> = spec
        .iter()
        .map(|(n, _, _)| ScannedFile {
            path: format!("/lib/{n}.jpg").into(),
            kind: MediaKind::Image,
            size: 1,
            mtime: 1,
        })
        .collect();
    db.sync_folder(f, &files, &[]).unwrap();
    let rows = db.pending(registry::CLIP_B32.id).unwrap();
    let caps: Vec<_> = rows
        .iter()
        .filter(|r| !undated.iter().any(|u| r.path.ends_with(format!("{u}.jpg"))))
        .map(|r| {
            let t = spec
                .iter()
                .find(|s| r.path.ends_with(format!("{}.jpg", s.0)));
            let t = t.unwrap().2;
            (
                r.id,
                crate::media::metadata::Capture {
                    taken_at: t,
                    source: crate::media::metadata::DateSource::Exif,
                    camera: None,
                    duration: None,
                },
            )
        })
        .collect();
    db.set_captures(&caps).unwrap();
    rows.into_iter()
        .map(|r| {
            let a = spec
                .iter()
                .find(|s| r.path.ends_with(format!("{}.jpg", s.0)))
                .unwrap()
                .1;
            let item = NewItem {
                t_start: None,
                t_end: None,
                embedding: at_angle(a),
            };
            Embedded {
                file: r,
                meta: FileMeta::default(),
                items: vec![item],
            }
        })
        .collect()
}

fn search_with(lib: &Library, range: Option<(i64, i64)>, sort: SearchSort) -> Vec<String> {
    let opts = SearchOptions {
        taken_range: range,
        sort,
        ..Default::default()
    };
    names(&search::by_vector(lib, &query(), &opts, None).unwrap())
}

fn check_date_search(lib: &Library) {
    let all = |r| search_with(lib, None, r);
    assert_eq!(
        all(SearchSort::Relevance),
        ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    );
    assert_eq!(
        all(SearchSort::Newest),
        ["b.jpg", "d.jpg", "a.jpg", "c.jpg"]
    );
    assert_eq!(
        all(SearchSort::Oldest),
        ["c.jpg", "a.jpg", "d.jpg", "b.jpg"]
    );
    // 2020 through 2021, inclusive of both whole years.
    let range = Some((day(2020, 1, 1) - 43_200, day(2021, 12, 31) + 43_199));
    assert_eq!(
        search_with(lib, range, SearchSort::Relevance),
        ["a.jpg", "d.jpg"]
    );
    assert_eq!(
        search_with(lib, range, SearchSort::Newest),
        ["d.jpg", "a.jpg"]
    );
    // The bounds themselves are inside the range.
    let exact = Some((day(2019, 1, 10), day(2019, 1, 10)));
    assert_eq!(search_with(lib, exact, SearchSort::Relevance), ["c.jpg"]);
    assert!(
        search_with(
            lib,
            Some((day(2023, 1, 1), day(2024, 1, 1))),
            SearchSort::Newest
        )
        .is_empty()
    );
    // Hits carry their capture time.
    let hits = search::by_vector(lib, &query(), &SearchOptions::default(), None).unwrap();
    assert_eq!(hits[1].taken_at, day(2022, 12, 31));
}

#[test]
fn search_by_date_range_and_sort_from_the_database_and_from_indexing() {
    // The index built from the database at startup.
    let d = tempfile::tempdir().unwrap();
    {
        let lib = open(d.path());
        let rows = seed_dated(&lib, &[]);
        let batch: Vec<_> = rows
            .iter()
            .map(|e| (e.file.id, e.meta, e.items.clone()))
            .collect();
        lib.db
            .lock()
            .put_items(registry::CLIP_B32.id, &batch)
            .unwrap();
    }
    let lib = open(d.path());
    assert_eq!(lib.indexed_items(), 4);
    check_date_search(&lib);
    // ...and through the cache.
    lib.save_index_cache();
    drop(lib);
    let lib = open(d.path());
    assert!(lib.loaded_from_cache());
    check_date_search(&lib);

    // The index filled while indexing (dates are known before embedding).
    let d2 = tempfile::tempdir().unwrap();
    let lib = open(d2.path());
    let rows = seed_dated(&lib, &[]);
    assert_eq!(lib.indexed_items(), 0);
    crate::indexer::write_items(&lib, rows).unwrap();
    assert_eq!(lib.indexed_items(), 4);
    check_date_search(&lib);
}

#[test]
fn a_date_learned_later_updates_the_index_and_its_cache() {
    let d = tempfile::tempdir().unwrap();
    let id_b = {
        let lib = open(d.path());
        // File "b" is embedded before its date could be read: only its file time (1970)
        // is known.
        let rows = seed_dated(&lib, &["b"]);
        let b = rows
            .iter()
            .find(|r| r.file.path.ends_with("b.jpg"))
            .unwrap()
            .file
            .id;
        crate::indexer::write_items(&lib, rows).unwrap();
        b
    };
    let lib = open(d.path());
    assert_eq!(
        search_with(&lib, None, SearchSort::Newest),
        ["d.jpg", "a.jpg", "c.jpg", "b.jpg"]
    );
    lib.save_index_cache();
    let epoch = lib.db.lock().items_epoch().unwrap();
    let cap = |t| crate::media::metadata::Capture {
        taken_at: t,
        source: crate::media::metadata::DateSource::Exif,
        camera: None,
        duration: None,
    };
    let times = crate::indexer::apply_captures(&lib, &[(id_b, cap(day(2022, 12, 31)))]).unwrap();
    assert!(
        lib.db.lock().items_epoch().unwrap() > epoch,
        "the cache is stale now"
    );
    // The index is patched separately, once per date pass.
    assert_eq!(times, HashMap::from([(id_b, day(2022, 12, 31))]));
    assert_eq!(search_with(&lib, None, SearchSort::Newest)[3], "b.jpg");
    crate::indexer::patch_index_dates(&lib, &times);
    assert_eq!(
        search_with(&lib, None, SearchSort::Newest),
        ["b.jpg", "d.jpg", "a.jpg", "c.jpg"]
    );
    // The rewritten cache has the new date.
    lib.save_index_cache();
    drop(lib);
    let lib = open(d.path());
    assert!(lib.loaded_from_cache());
    assert_eq!(search_with(&lib, None, SearchSort::Newest)[0], "b.jpg");
    // A file without items leaves the epoch alone.
    let epoch = lib.db.lock().items_epoch().unwrap();
    let none = crate::indexer::apply_captures(&lib, &[(9999, cap(5))]).unwrap();
    assert!(none.is_empty());
    assert_eq!(lib.db.lock().items_epoch().unwrap(), epoch);
}

#[test]
fn dates_of_files_without_items_never_touch_the_index() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    // Files are known (and undated) but nothing is embedded yet.
    let rows = seed_dated(&lib, &["a", "b", "c", "d"]);
    let cap = |t| crate::media::metadata::Capture {
        taken_at: t,
        source: crate::media::metadata::DateSource::Exif,
        camera: None,
        duration: None,
    };
    let caps: Vec<_> = rows
        .iter()
        .map(|r| (r.file.id, cap(day(2020, 1, 1))))
        .collect();
    // With a reader holding the index, any attempt to write to it would block for good.
    let guard = lib.index.read();
    let (tx, rx) = std::sync::mpsc::channel();
    let l = Arc::clone(&lib);
    std::thread::spawn(move || {
        let _ = tx.send(crate::indexer::apply_captures(&l, &caps).unwrap());
    });
    let times = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("apply_captures waited for the index");
    drop(guard);
    assert!(times.is_empty());
    crate::indexer::patch_index_dates(&lib, &times);
    assert_eq!(lib.indexed_items(), 0);
}

#[test]
fn an_undated_file_has_the_same_date_everywhere() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    let rows = seed_dated(&lib, &["b"]);
    crate::indexer::write_items(&lib, rows).unwrap();
    let expected = crate::media::metadata::effective_taken(None, 1);
    let b = lib
        .timeline(&TimelineFilter::default())
        .unwrap()
        .into_iter()
        .find(|t| t.path.ends_with("b.jpg"))
        .unwrap();
    assert_eq!(b.taken_at, expected);
    let item = b.item_id.unwrap();
    assert_eq!(
        lib.db.lock().item_info(&[item]).unwrap()[0].taken_at,
        expected
    );
    assert_eq!(
        lib.db.lock().taken_times(&[b.file_id]).unwrap()[&b.file_id],
        expected
    );
    assert_eq!(
        lib.db
            .lock()
            .file_details(b.file_id)
            .unwrap()
            .unwrap()
            .taken_at,
        expected
    );
    let mut stored = Vec::new();
    lib.db
        .lock()
        .for_each_item(lib.model().id, |i| stored.push((i.file_id, i.taken_at)))
        .unwrap();
    assert!(stored.contains(&(b.file_id, expected)));
    let cands = lib.index.read().search(&query(), 10, None);
    assert_eq!(
        cands
            .iter()
            .find(|c| c.file_id == b.file_id)
            .unwrap()
            .taken_at,
        expected
    );
    let hits = search::by_vector(&lib, &query(), &SearchOptions::default(), None).unwrap();
    assert_eq!(
        hits.iter()
            .find(|h| h.file_id == b.file_id)
            .unwrap()
            .taken_at,
        expected
    );
    // Sorting by date uses that same value: b (1970) is the oldest.
    let opts = SearchOptions {
        sort: SearchSort::Oldest,
        ..Default::default()
    };
    let oldest = search::by_vector(&lib, &query(), &opts, None).unwrap();
    assert_eq!(names(&oldest)[0], "b.jpg");
}

#[test]
fn a_file_removed_before_its_vectors_are_written_adds_none() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    let rows = seed_dated(&lib, &[]);
    let folder = lib.db.lock().folders().unwrap()[0].id;
    lib.db.lock().remove_folder(folder).unwrap();
    crate::indexer::write_items(&lib, rows).unwrap();
    assert_eq!(lib.indexed_items(), 0);
}

fn favorite_ids(lib: &Library) -> Vec<i64> {
    lib.timeline(&TimelineFilter {
        favorites_only: true,
        ..Default::default()
    })
    .unwrap()
    .iter()
    .map(|t| t.file_id)
    .collect()
}

#[test]
fn rapid_favorite_toggles_end_as_the_last_one() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &[]);
    let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
    let v0 = lib.favorites_version();
    for fav in [true, false, true] {
        lib.queue_favorite(id, fav);
    }
    lib.wait_writes();
    assert_eq!(favorite_ids(&lib), [id]);
    assert!(lib.favorites_version() > v0);
    let results = lib.take_favorite_results();
    assert!(!results.is_empty());
    assert!(results.iter().all(|w| w.error.is_none() && w.file_id == id));
    assert!(results.last().unwrap().fav);
    // And off again.
    lib.queue_favorite(id, false);
    lib.wait_writes();
    assert!(favorite_ids(&lib).is_empty());
}

#[test]
fn album_writes_keep_their_order_and_report_back() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &[]);
    let ids: Vec<i64> = lib
        .timeline(&TimelineFilter::default())
        .unwrap()
        .iter()
        .map(|t| t.file_id)
        .collect();
    let v0 = lib.albums_version();
    // Create-with-items and a duplicate name, in one burst with favorite marks between.
    let t1 = lib.queue_album(AlbumOp::Create {
        name: "Trip".into(),
        add: ids[..2].to_vec(),
    });
    lib.queue_favorite(ids[0], true);
    let t2 = lib.queue_album(AlbumOp::Create {
        name: "trip".into(),
        add: vec![],
    });
    lib.queue_favorite(ids[0], false);
    lib.wait_writes();
    let results = lib.take_album_results();
    assert_eq!(results.len(), 2);
    assert_eq!((results[0].ticket, results[1].ticket), (t1, t2));
    let album = results[0].album.unwrap();
    assert_eq!((results[0].changed, results[0].error.as_deref()), (2, None));
    assert!(
        results[1]
            .error
            .as_deref()
            .unwrap()
            .contains("already exists")
    );
    assert!(lib.albums_version() > v0);
    assert!(favorite_ids(&lib).is_empty(), "the later mark won");
    // Add, remove, rename, queued back to back: applied in that order.
    lib.queue_album(AlbumOp::Add {
        album,
        files: ids.clone(),
    });
    lib.queue_album(AlbumOp::Remove {
        album,
        files: ids[..1].to_vec(),
    });
    lib.queue_album(AlbumOp::Rename {
        id: album,
        name: "Holiday".into(),
    });
    lib.wait_writes();
    let r = lib.take_album_results();
    assert_eq!(
        r.iter().map(|w| w.changed).collect::<Vec<_>>(),
        [ids.len() - 2, 1, 0]
    );
    let info = lib.albums().unwrap();
    assert_eq!(
        (info[0].name.as_str(), info[0].count),
        ("Holiday", ids.len() - 1)
    );
    // Deleting the album leaves the files.
    lib.queue_album(AlbumOp::Delete { id: album });
    lib.wait_writes();
    assert!(lib.albums().unwrap().is_empty());
    assert_eq!(
        lib.timeline(&TimelineFilter::default()).unwrap().len(),
        ids.len()
    );
}

#[test]
fn the_album_version_moves_only_when_something_changed() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &[]);
    let ids: Vec<i64> = lib
        .timeline(&TimelineFilter::default())
        .unwrap()
        .iter()
        .map(|t| t.file_id)
        .collect();
    lib.queue_album(AlbumOp::Create {
        name: "A".into(),
        add: ids[..1].to_vec(),
    });
    lib.wait_writes();
    let album = lib.take_album_results()[0].album.unwrap();
    let v = lib.albums_version();
    // Already in it, a missing file, removing what is not there, deleting what is gone.
    lib.queue_album(AlbumOp::Add {
        album,
        files: ids[..1].to_vec(),
    });
    lib.queue_album(AlbumOp::Remove {
        album,
        files: ids[1..2].to_vec(),
    });
    lib.queue_album(AlbumOp::Delete { id: album + 100 });
    lib.wait_writes();
    let r = lib.take_album_results();
    assert!(r.iter().all(|w| w.error.is_none()));
    assert_eq!(lib.albums_version(), v);
    // A real change moves it; so do rename and delete.
    lib.queue_album(AlbumOp::Add {
        album,
        files: ids[1..2].to_vec(),
    });
    lib.wait_writes();
    assert_eq!(lib.albums_version(), v + 1);
    lib.queue_album(AlbumOp::Rename {
        id: album,
        name: "B".into(),
    });
    lib.queue_album(AlbumOp::Delete { id: album });
    lib.wait_writes();
    assert_eq!(lib.albums_version(), v + 3);
    // The album is gone: adding to it is a plain message.
    lib.queue_album(AlbumOp::Add {
        album,
        files: ids[..1].to_vec(),
    });
    lib.wait_writes();
    let r = lib.take_album_results();
    let e = r.last().unwrap().error.as_deref().unwrap();
    assert_eq!(e, "That album no longer exists");
    assert_eq!(lib.albums_version(), v + 3);
}

#[test]
fn a_failed_album_write_is_reported_and_changes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &[]);
    let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
    let album = lib.create_album("Keep", &[id]).unwrap();
    let v = lib.albums_version();
    lib.fail_albums.store(true, Ordering::Release);
    lib.queue_album(AlbumOp::Remove {
        album,
        files: vec![id],
    });
    lib.queue_album(AlbumOp::Delete { id: album });
    lib.queue_album(AlbumOp::Create {
        name: "New".into(),
        add: vec![id],
    });
    lib.wait_writes();
    let r = lib.take_album_results();
    assert_eq!(r.len(), 3);
    assert!(r.iter().all(|w| w.error.is_some() && w.changed == 0));
    assert_eq!(lib.albums_version(), v);
    let info = lib.albums().unwrap();
    assert_eq!((info.len(), info[0].count), (1, 1));
}

#[test]
fn writes_queued_right_before_exit_are_committed() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &[]);
    let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
    lib.queue_album(AlbumOp::Create {
        name: "Last".into(),
        add: vec![id],
    });
    lib.queue_favorite(id, true);
    assert!(lib.wait_writes_for(std::time::Duration::from_secs(3)));
    assert_eq!(lib.albums().unwrap()[0].count, 1);
    assert_eq!(favorite_ids(&lib), [id]);
}

#[test]
fn a_failed_favorite_write_is_reported() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &[]);
    let id = lib.timeline(&TimelineFilter::default()).unwrap()[0].file_id;
    let v0 = lib.favorites_version();
    lib.fail_favorites.store(true, Ordering::Release);
    lib.queue_favorite(id, true);
    lib.wait_writes();
    let results = lib.take_favorite_results();
    assert_eq!(results.len(), 1);
    assert_eq!((results[0].file_id, results[0].fav), (id, true));
    assert!(results[0].error.is_some());
    assert!(favorite_ids(&lib).is_empty());
    assert_eq!(lib.favorites_version(), v0, "nothing was committed");
}

#[test]
fn years_cover_the_whole_library() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &["b"]);
    // b is undated: its file time (1970) counts.
    assert_eq!(lib.years().unwrap(), [2021, 2020, 2019, 1970]);
}

#[test]
fn favorites_are_stored_and_found_in_search_hits_and_the_timeline() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    let rows = seed_dated(&lib, &[]);
    crate::indexer::write_items(&lib, rows).unwrap();
    let tl = |fav| {
        lib.timeline(&TimelineFilter {
            favorites_only: fav,
            ..Default::default()
        })
        .unwrap()
    };
    assert!(tl(true).is_empty());
    let b = tl(false)
        .into_iter()
        .find(|t| t.path.ends_with("b.jpg"))
        .unwrap();
    lib.set_favorite(&[b.file_id], true).unwrap();
    assert_eq!(names_of(&tl(true)), ["b.jpg"]);
    let hits = search::by_vector(&lib, &query(), &SearchOptions::default(), None).unwrap();
    assert_eq!(
        hits.iter()
            .filter(|h| h.favorite)
            .map(|h| h.file_id)
            .collect::<Vec<_>>(),
        [b.file_id]
    );
    lib.set_favorite(&[b.file_id], false).unwrap();
    assert!(tl(true).is_empty());
}

fn names_of(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}
