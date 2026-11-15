#![expect(clippy::too_many_lines, reason = "a scenario test reads top to bottom")]
use super::*;
use crate::indexer::Embedded;
use crate::media::{MediaKind, ScannedFile};
use crate::search::SearchSort;
use crate::store::db::{FileMeta, NewItem};
use std::collections::HashMap;
use std::path::Path;

mod cache;
mod dates;
mod indexing;
mod jobs;
mod model;
mod writes;

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

fn names_of(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}
