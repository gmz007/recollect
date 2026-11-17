//! Indexing runs: what is embedded, retried, left pending or counted.

use super::*;

#[test]
fn long_videos_do_not_crowd_out_other_files() {
    let d = tempfile::tempdir().unwrap();
    {
        let lib = open(d.path());
        let mut db = lib.db.lock();
        let f = db.add_folder(Path::new("/nowhere")).unwrap();
        let mut files = vec![ScannedFile {
            path: "/nowhere/long.mp4".into(),
            kind: MediaKind::Video,
            size: 1,
            mtime: 1,
        }];
        for i in 0..30 {
            files.push(ScannedFile {
                path: format!("/nowhere/{i:02}.jpg").into(),
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
        crate::indexer::Outcome::Deferred
    );
    // A file that is still there but cannot be decoded is recorded as an error.
    let bad = media.join("bad.jpg");
    std::fs::write(&bad, b"not an image").unwrap();
    let row = crate::store::db::FileRow { path: bad, ..row };
    assert_eq!(
        crate::indexer::file_failed(&lib, &row, &err).unwrap(),
        crate::indexer::Outcome::Failed
    );
}

#[test]
fn retry_errors_and_sync_of_a_removed_folder() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/nowhere")).unwrap();
    let file = ScannedFile {
        path: "/nowhere/a.jpg".into(),
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

#[test]
fn progress_counts_indexed_failed_and_deferred_files() {
    use crate::indexer::{Outcome, Progress};
    let dir = tempfile::tempdir().unwrap();
    let lib = open(dir.path());
    let row = crate::store::db::FileRow {
        id: 1,
        path: "/x.jpg".into(),
        kind: MediaKind::Image,
        size: 1,
        mtime: 1,
        status: crate::store::db::FileStatus::Pending,
    };
    let mut prog = Progress::new(3, crate::model::Device::Cpu);
    prog.file_done(&lib, &row, 1, Outcome::Indexed);
    prog.file_done(&lib, &row, 0, Outcome::Failed);
    prog.file_done(&lib, &row, 0, Outcome::Deferred);
    assert_eq!((prog.p.done, prog.p.errors, prog.deferred), (3, 1, 1));
}
