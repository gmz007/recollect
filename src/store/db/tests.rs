use super::*;
use crate::media::metadata::{Capture, DateSource, local_wall_clock};
use crate::media::{MediaKind, ScannedFile};
use std::path::PathBuf;

fn sf(p: &str, size: u64, mtime: i64) -> ScannedFile {
    ScannedFile {
        path: PathBuf::from(p),
        kind: crate::media::scan::kind_of(Path::new(p)).unwrap(),
        size,
        mtime,
    }
}

fn item(v: f32) -> NewItem {
    NewItem {
        t_start: None,
        t_end: None,
        embedding: vec![v, 1.0 - v],
    }
}

#[test]
fn folder_nesting_rules() {
    let mut db = Db::open_in_memory().unwrap();
    let a = db.add_folder(Path::new("/m/photos/2020")).unwrap();
    assert_eq!(db.add_folder(Path::new("/m/photos/2020")).unwrap(), a);
    assert!(db.add_folder(Path::new("/m/photos/2020/x")).is_err());
    db.sync_folder(a, &[sf("/m/photos/2020/a.jpg", 1, 1)], &[])
        .unwrap();
    // Adding a parent absorbs the child folder and its files.
    let p = db.add_folder(Path::new("/m/photos")).unwrap();
    let f = db.folders().unwrap();
    assert_eq!(
        f,
        vec![Folder {
            id: p,
            path: PathBuf::from("/m/photos")
        }]
    );
    assert_eq!(db.stats("m").unwrap().images, 1);
    // Sibling with common string prefix is not "inside".
    assert!(db.add_folder(Path::new("/m/photos2")).is_ok());
}

#[test]
fn epoch_only_moves_on_real_changes() {
    let mut db = Db::open_in_memory().unwrap();
    let e0 = db.items_epoch().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    assert_eq!(db.items_epoch().unwrap(), e0, "add_folder");
    let files = [sf("/lib/a.jpg", 1, 1)];
    db.sync_folder(f, &files, &[]).unwrap();
    let e1 = db.items_epoch().unwrap();
    assert!(e1 > e0, "new file");
    db.sync_folder(f, &files, &[]).unwrap();
    assert_eq!(db.items_epoch().unwrap(), e1, "no-op sync");
    db.sync_folder(f, &[sf("/lib/a.jpg", 2, 1)], &[]).unwrap();
    let e2 = db.items_epoch().unwrap();
    assert!(e2 > e1, "changed file");
    db.sync_folder(f, &[], &[]).unwrap();
    assert!(db.items_epoch().unwrap() > e2, "removed file");
}

#[test]
fn library_id_is_created_once() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db.sqlite");
    let id = Db::open(&p).unwrap().library_id().unwrap();
    assert_eq!(id.len(), 16);
    assert_eq!(Db::open(&p).unwrap().library_id().unwrap(), id);
    assert_ne!(Db::open_in_memory().unwrap().library_id().unwrap(), id);
}

#[test]
fn sync_detects_add_change_remove() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    let s = db
        .sync_folder(f, &[sf("/lib/a.jpg", 1, 1), sf("/lib/b.mp4", 2, 2)], &[])
        .unwrap();
    assert_eq!(
        (s.added, s.changed, s.unchanged, s.removed.len()),
        (2, 0, 0, 0)
    );
    let pend = db.pending("m").unwrap();
    assert_eq!(pend.len(), 2);
    assert_eq!(pend[0].kind, MediaKind::Image);
    let ids = db
        .put_items("m", &[(pend[0].id, FileMeta::default(), vec![item(0.5)])])
        .unwrap();
    assert_eq!(ids[0].len(), 1);
    assert_eq!(db.pending("m").unwrap().len(), 1);
    // Another model needs everything again.
    assert_eq!(db.pending("other").unwrap().len(), 2);

    let s = db.sync_folder(f, &[sf("/lib/a.jpg", 1, 9)], &[]).unwrap();
    assert_eq!((s.added, s.changed, s.unchanged), (0, 1, 0));
    assert_eq!(s.removed, vec![pend[1].id]);
    assert_eq!(db.pending("m").unwrap().len(), 1);
    assert_eq!(
        db.stats("m").unwrap().items,
        0,
        "stale items of changed files are dropped"
    );
}

#[test]
fn unreadable_subfolders_are_not_emptied() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    db.sync_folder(
        f,
        &[sf("/lib/a.jpg", 1, 1), sf("/lib/sub/b.jpg", 1, 1)],
        &[],
    )
    .unwrap();
    let s = db
        .sync_folder(f, &[sf("/lib/a.jpg", 1, 1)], &[PathBuf::from("/lib/sub")])
        .unwrap();
    assert!(s.removed.is_empty());
    let s = db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1)], &[]).unwrap();
    assert_eq!(s.removed.len(), 1);
}

#[test]
fn items_round_trip_and_errors() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1), sf("/lib/v.mp4", 1, 1)], &[])
        .unwrap();
    let p = db.pending("m").unwrap();
    let v = NewItem {
        t_start: Some(1.0),
        t_end: Some(3.0),
        embedding: vec![0.25, -0.5],
    };
    let meta = FileMeta {
        width: Some(640),
        height: Some(480),
        duration: Some(10.0),
    };
    let ids = db
        .put_items(
            "m",
            &[
                (p[0].id, FileMeta::default(), vec![item(0.1)]),
                (p[1].id, meta, vec![v.clone()]),
            ],
        )
        .unwrap();
    let mut loaded = Vec::new();
    db.for_each_item("m", |it| loaded.push(it)).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[1].embedding, v.embedding);
    assert_eq!(loaded[1].kind, MediaKind::Video);
    let info = db.item_info(&[ids[1][0]]).unwrap();
    assert_eq!(info[0].t_start, Some(1.0));
    assert_eq!(info[0].width, Some(640));
    assert_eq!(db.embeddings(&ids[1]).unwrap()[&ids[1][0]], v.embedding);
    db.set_error(p[0].id, "boom").unwrap();
    assert_eq!(
        db.errors(10).unwrap(),
        vec![(PathBuf::from("/lib/a.jpg"), "boom".to_string())]
    );
    let st = db.stats("m").unwrap();
    assert_eq!((st.errors, st.indexed, st.items), (1, 1, 1));
    let removed = db.remove_folder(f).unwrap();
    assert_eq!(removed.len(), 2);
    assert_eq!(db.stats("m").unwrap(), Stats::default());
}

#[test]
fn put_items_for_deleted_file_is_dropped() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    let g = db.add_folder(Path::new("/other")).unwrap();
    db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1)], &[]).unwrap();
    db.sync_folder(g, &[sf("/other/b.jpg", 1, 1)], &[]).unwrap();
    let p = db.pending("m").unwrap();
    db.remove_folder(f).unwrap();
    // A batch mixing a removed file with a live one: the live one is stored, the other skipped.
    let ids = db
        .put_items(
            "m",
            &[
                (p[0].id, FileMeta::default(), vec![item(0.1)]),
                (p[1].id, FileMeta::default(), vec![item(0.2)]),
            ],
        )
        .unwrap();
    assert!(ids[0].is_empty());
    assert_eq!(ids[1].len(), 1);
    assert_eq!(db.stats("m").unwrap().items, 1);
}

#[test]
fn reopen_keeps_data() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("x/lib.db");
    {
        let mut db = Db::open(&p).unwrap();
        db.add_folder(Path::new("/lib")).unwrap();
        db.set_meta("model", "m").unwrap();
    }
    let db = Db::open(&p).unwrap();
    assert_eq!(db.folders().unwrap().len(), 1);
    assert_eq!(db.meta("model").unwrap().as_deref(), Some("m"));
}

/// The schema as shipped in version 1, to prove old libraries keep working.
const V1_SCHEMA: &str = r"
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE folders (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, added_at INTEGER NOT NULL);
    CREATE TABLE files (
        id INTEGER PRIMARY KEY,
        folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        path TEXT NOT NULL UNIQUE,
        kind INTEGER NOT NULL,
        size INTEGER NOT NULL,
        mtime INTEGER NOT NULL,
        status INTEGER NOT NULL DEFAULT 0,
        model TEXT,
        error TEXT,
        width INTEGER,
        height INTEGER,
        duration REAL,
        indexed_at INTEGER
    );
    CREATE INDEX files_folder ON files(folder_id);
    CREATE INDEX files_status ON files(status);
    CREATE TABLE items (
        id INTEGER PRIMARY KEY,
        file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
        t_start REAL,
        t_end REAL,
        embedding BLOB NOT NULL
    );
    CREATE INDEX items_file ON items(file_id);
    INSERT INTO meta VALUES ('schema_version', '1'), ('library_id', 'abc'), ('items_epoch', '4');
    INSERT INTO folders VALUES (1, '/lib', 10);
    INSERT INTO files VALUES (1, 1, '/lib/a.jpg', 0, 5, 1000, 1, 'm', NULL, 640, 480, NULL, 555);
    INSERT INTO files VALUES (2, 1, '/lib/b.mp4', 1, 6, 2000, 0, NULL, NULL, NULL, NULL, NULL, NULL);
    INSERT INTO items VALUES (1, 1, NULL, NULL, x'0000803f');
";

#[test]
fn v1_database_is_migrated() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("lib.db");
    Connection::open(&p)
        .unwrap()
        .execute_batch(V1_SCHEMA)
        .unwrap();
    let db = Db::open(&p).unwrap();
    assert_eq!(db.meta("schema_version").unwrap().as_deref(), Some("2"));
    assert_eq!(db.library_id().unwrap(), "abc");
    assert_eq!(db.items_epoch().unwrap(), 4);
    assert_eq!(db.folders().unwrap().len(), 1);
    let st = db.stats("m").unwrap();
    assert_eq!((st.images, st.videos, st.indexed, st.items), (1, 1, 1, 1));
    let tl = db.timeline(&TimelineFilter::default()).unwrap();
    assert_eq!(tl.len(), 2);
    assert!(
        tl.iter()
            .all(|i| i.date_source == DateSource::File && !i.favorite)
    );
    let added: Vec<(i64, i64)> = db
        .conn
        .prepare("SELECT id, added_at FROM files ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(added[0], (1, 555), "uses indexed_at");
    assert!(added[1].1 >= now() - 60, "falls back to now");
    assert_eq!(db.pending_dates().unwrap().len(), 2);
    db.conn
        .execute("INSERT INTO albums(name, created_at) VALUES ('x', 1)", [])
        .unwrap();
    drop(db);
    // Reopening a migrated database is a no-op.
    assert_eq!(Db::open(&p).unwrap().folders().unwrap().len(), 1);
}

#[test]
fn v1_database_without_a_version_row_is_still_migrated() {
    // An old build was killed after creating its tables, before it recorded a version.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("lib.db");
    Connection::open(&p)
        .unwrap()
        .execute_batch(&V1_SCHEMA.replace(
            "INSERT INTO meta VALUES ('schema_version', '1'), ('library_id', 'abc'), ('items_epoch', '4');",
            "",
        ))
        .unwrap();
    let db = Db::open(&p).unwrap();
    assert_eq!(db.meta("schema_version").unwrap().as_deref(), Some("2"));
    assert_eq!(db.stats("m").unwrap().images, 1, "existing rows are kept");
    assert_eq!(db.pending_dates().unwrap().len(), 2);
    db.conn
        .execute("INSERT INTO albums(name, created_at) VALUES ('x', 1)", [])
        .unwrap();
}

#[test]
fn migration_keeps_one_backup_of_the_old_database() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("lib.db");
    Connection::open(&p)
        .unwrap()
        .execute_batch(V1_SCHEMA)
        .unwrap();
    let bak = d.path().join("lib.db.v1.bak");
    drop(Db::open(&p).unwrap());
    assert!(bak.exists());
    // The backup is a complete version 1 database.
    let old = Connection::open(&bak).unwrap();
    let (v, files): (String, i64) = old
        .query_row(
            "SELECT (SELECT value FROM meta WHERE key = 'schema_version'),
                    (SELECT COUNT(*) FROM files)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((v.as_str(), files), ("1", 2));
    drop(old);
    // An existing backup is never overwritten, and a fresh database gets none.
    std::fs::write(&bak, b"keep me").unwrap();
    Connection::open(&p)
        .unwrap()
        .execute(
            "UPDATE meta SET value = '1' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
    drop(Db::open(&p).unwrap());
    assert_eq!(std::fs::read(&bak).unwrap(), b"keep me");
    let fresh = d.path().join("fresh.db");
    drop(Db::open(&fresh).unwrap());
    assert!(!d.path().join("fresh.db.v1.bak").exists());
}

#[test]
fn captures_store_a_video_duration_only_when_unknown() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    db.sync_folder(f, &[sf("/lib/a.mp4", 1, 1)], &[]).unwrap();
    let id = db.pending_dates().unwrap()[0].id;
    let mut c = capture(100_000_000, DateSource::Video);
    c.duration = Some(12.5);
    db.set_captures(&[(id, c.clone())]).unwrap();
    c.duration = Some(99.0);
    db.set_captures(&[(id, c)]).unwrap();
    let tl = db.timeline(&TimelineFilter::default()).unwrap();
    assert_eq!(tl[0].duration, Some(12.5));
}

#[test]
fn newer_schema_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("lib.db");
    drop(Db::open(&p).unwrap());
    Connection::open(&p)
        .unwrap()
        .execute(
            "UPDATE meta SET value = '3' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
    let e = Db::open(&p).expect_err("must refuse").to_string();
    assert!(e.contains("schema version 3"), "{e}");
}

fn capture(t: i64, source: DateSource) -> Capture {
    Capture {
        taken_at: t,
        source,
        camera: Some("cam".into()),
        duration: None,
    }
}

#[test]
fn captures_are_stored_and_reset_when_a_file_changes() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    db.sync_folder(f, &[sf("/lib/a.jpg", 1, 1), sf("/lib/b.jpg", 1, 1)], &[])
        .unwrap();
    let pend = db.pending_dates().unwrap();
    assert_eq!(pend.len(), 2);
    db.set_captures(&[
        (pend[0].id, capture(100_000_000, DateSource::Exif)),
        (pend[1].id, capture(200_000_000, DateSource::File)),
    ])
    .unwrap();
    assert!(db.pending_dates().unwrap().is_empty());
    assert_eq!(
        db.date_counts().unwrap(),
        DateCounts {
            exif: 1,
            video: 0,
            file: 1
        }
    );
    db.sync_folder(f, &[sf("/lib/a.jpg", 2, 1), sf("/lib/b.jpg", 1, 1)], &[])
        .unwrap();
    let pend = db.pending_dates().unwrap();
    assert_eq!(pend.len(), 1);
    assert_eq!(pend[0].path, PathBuf::from("/lib/a.jpg"));
}

fn timeline_db() -> (Db, i64, i64) {
    let mut db = Db::open_in_memory().unwrap();
    let f1 = db.add_folder(Path::new("/one")).unwrap();
    let f2 = db.add_folder(Path::new("/two")).unwrap();
    db.sync_folder(f1, &[sf("/one/a.jpg", 1, 10), sf("/one/b.mp4", 1, 10)], &[])
        .unwrap();
    db.sync_folder(f2, &[sf("/two/c.jpg", 1, 10), sf("/two/d.jpg", 1, 10)], &[])
        .unwrap();
    (db, f1, f2)
}

fn id_of(db: &Db, path: &str) -> i64 {
    db.conn
        .query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0))
        .unwrap()
}

fn names(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn timeline_order_and_ties() {
    let (mut db, _, _) = timeline_db();
    let ids: Vec<i64> = ["/one/a.jpg", "/one/b.mp4", "/two/c.jpg", "/two/d.jpg"]
        .iter()
        .map(|p| id_of(&db, p))
        .collect();
    db.set_captures(&[
        (ids[0], capture(300_000_000, DateSource::Exif)),
        (ids[1], capture(100_000_000, DateSource::Video)),
        (ids[2], capture(300_000_000, DateSource::Exif)), // ties with a.jpg
        (ids[3], capture(200_000_000, DateSource::File)),
    ])
    .unwrap();
    let desc = db.timeline(&TimelineFilter::default()).unwrap();
    assert_eq!(names(&desc), ["a.jpg", "c.jpg", "d.jpg", "b.mp4"]);
    let asc = db
        .timeline(&TimelineFilter {
            order: TimelineOrder::TakenAsc,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(names(&asc), ["b.mp4", "d.jpg", "a.jpg", "c.jpg"]);
    assert_eq!(asc[0].camera.as_deref(), Some("cam"));
    assert_eq!(asc[0].date_source, DateSource::Video);
    // Added order: everything was added in the same second, so path order decides; make
    // one file "newer" explicitly.
    db.conn
        .execute(
            "UPDATE files SET added_at = added_at + 100 WHERE id = ?1",
            [ids[3]],
        )
        .unwrap();
    let added = db
        .timeline(&TimelineFilter {
            order: TimelineOrder::AddedDesc,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(names(&added), ["d.jpg", "a.jpg", "b.mp4", "c.jpg"]);
}

#[test]
fn timeline_filters() {
    let (db, f1, _) = timeline_db();
    let a = id_of(&db, "/one/a.jpg");
    let c = id_of(&db, "/two/c.jpg");
    db.conn
        .execute("UPDATE files SET favorite = 1 WHERE id IN (?1, ?2)", [a, c])
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO albums(id, name, created_at) VALUES (7, 'trip', 1)",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO album_items(album_id, file_id, added_at) VALUES (7, ?1, 1), (7, ?2, 1)",
            [c, id_of(&db, "/two/d.jpg")],
        )
        .unwrap();
    let q = |f: TimelineFilter| names(&db.timeline(&f).unwrap());
    let all = TimelineFilter {
        order: TimelineOrder::TakenAsc,
        ..Default::default()
    };
    assert_eq!(q(all.clone()).len(), 4);
    assert_eq!(
        q(TimelineFilter {
            folder: Some(f1),
            ..all
        }),
        ["a.jpg", "b.mp4"]
    );
    assert_eq!(
        q(TimelineFilter {
            kind: Some(MediaKind::Video),
            ..all
        }),
        ["b.mp4"]
    );
    assert_eq!(
        q(TimelineFilter {
            favorites_only: true,
            ..all
        }),
        ["a.jpg", "c.jpg"]
    );
    assert_eq!(
        q(TimelineFilter {
            album: Some(7),
            ..all
        }),
        ["c.jpg", "d.jpg"]
    );
    assert_eq!(
        q(TimelineFilter {
            album: Some(7),
            favorites_only: true,
            folder: Some(f1),
            ..all
        }),
        Vec::<String>::new()
    );
    // Deleting a file or an album cleans up memberships.
    db.conn
        .execute("DELETE FROM files WHERE id = ?1", [c])
        .unwrap();
    assert_eq!(
        q(TimelineFilter {
            album: Some(7),
            ..all
        }),
        ["d.jpg"]
    );
    db.conn
        .execute("DELETE FROM albums WHERE id = 7", [])
        .unwrap();
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM album_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn timeline_uses_file_time_when_the_date_is_unknown() {
    let mut db = Db::open_in_memory().unwrap();
    let f = db.add_folder(Path::new("/lib")).unwrap();
    db.sync_folder(
        f,
        &[sf("/lib/a.jpg", 1, 1_500_000_000), sf("/lib/b.jpg", 1, 1)],
        &[],
    )
    .unwrap();
    let b = id_of(&db, "/lib/b.jpg");
    db.set_captures(&[(b, capture(1_600_000_000, DateSource::Exif))])
        .unwrap();
    let tl = db.timeline(&TimelineFilter::default()).unwrap();
    assert_eq!(names(&tl), ["b.jpg", "a.jpg"]);
    assert_eq!(tl[1].taken_at, local_wall_clock(1_500_000_000));
    assert_eq!(tl[1].date_source, DateSource::File);
}

#[test]
fn album_crud_and_name_rules() {
    let (mut db, _, _) = timeline_db();
    let trip = db.create_album("  Trip  ").unwrap();
    assert_eq!(db.albums().unwrap()[0].name, "Trip");
    assert!(db.create_album("   ").is_err());
    assert!(db.create_album("trip").is_err(), "case-insensitive");
    let b = db.create_album("beach").unwrap();
    // Albums are listed by name.
    let names: Vec<_> = db.albums().unwrap().into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["beach", "Trip"]);
    assert!(db.rename_album(b, "TRIP").is_err());
    assert!(db.rename_album(b, " ").is_err());
    db.rename_album(trip, "TRIP").unwrap();
    db.rename_album(b, "Beach days").unwrap();
    assert!(db.rename_album(9999, "x").is_err());
    let names: Vec<_> = db.albums().unwrap().into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["Beach days", "TRIP"]);
}

#[test]
fn album_items_are_idempotent_and_the_cover_is_the_newest() {
    let (mut db, _, _) = timeline_db();
    let (a, c) = (id_of(&db, "/one/a.jpg"), id_of(&db, "/two/c.jpg"));
    db.conn
        .execute("UPDATE files SET taken_at = 100 WHERE id = ?1", [a])
        .unwrap();
    db.conn
        .execute("UPDATE files SET taken_at = 200 WHERE id = ?1", [c])
        .unwrap();
    let album = db.create_album("x").unwrap();
    assert_eq!(db.albums().unwrap()[0].cover, None);
    assert_eq!(db.add_to_album(album, &[a, c]).unwrap(), 2);
    assert_eq!(db.add_to_album(album, &[a, c, 424_242]).unwrap(), 0);
    let info = &db.albums().unwrap()[0];
    assert_eq!(info.count, 2);
    assert_eq!(info.cover.as_ref().unwrap().file_id, c);
    assert_eq!(db.remove_from_album(album, &[c, 777]).unwrap(), 1);
    let info = &db.albums().unwrap()[0];
    assert_eq!((info.count, info.cover.as_ref().unwrap().file_id), (1, a));
}

#[test]
fn album_names_have_one_shared_rule_including_a_length_limit() {
    let (mut db, _, _) = timeline_db();
    let longest = "é".repeat(MAX_ALBUM_NAME_CHARS);
    db.create_album(&longest).unwrap();
    let too_long = format!("{longest}x");
    let e = db.create_album(&too_long).unwrap_err().to_string();
    assert!(e.contains("at most 100"), "{e}");
    assert!(
        db.rename_album(db.albums().unwrap()[0].id, &too_long)
            .is_err()
    );
    // The same function serves the UI's pre-check.
    let known = [(1, "Trip")];
    assert_eq!(check_album_name("  new ", known, None).unwrap(), "new");
    assert!(check_album_name("TRIP", known, None).is_err());
    assert!(check_album_name("TRIP", known, Some(1)).is_ok());
    assert!(check_album_name(" ", known, None).is_err());
    assert!(check_album_name(&too_long, [], None).is_err());
}

#[test]
fn creating_with_items_is_all_or_nothing() {
    let (mut db, _, _) = timeline_db();
    let (a, c) = (id_of(&db, "/one/a.jpg"), id_of(&db, "/two/c.jpg"));
    let (id, n) = db.create_album_with("Good", &[a, c, 424_242]).unwrap();
    assert_eq!((n, db.albums().unwrap()[0].count), (2, 2));
    // A failure while adding leaves no album behind.
    db.conn
        .execute_batch(
            "CREATE TRIGGER boom BEFORE INSERT ON album_items
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
    assert!(db.create_album_with("Bad", &[a]).is_err());
    db.conn.execute_batch("DROP TRIGGER boom").unwrap();
    let names: Vec<_> = db.albums().unwrap().into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["Good"]);
    // A bad name adds nothing either.
    assert!(db.create_album_with("good", &[a]).is_err());
    assert_eq!(db.albums().unwrap()[0].count, 2);
    // Adding to an album that is gone is a plain message, not a constraint error.
    assert!(db.delete_album(id).unwrap());
    assert!(!db.delete_album(id).unwrap(), "already gone");
    let e = db.add_to_album(id, &[a]).unwrap_err().to_string();
    assert_eq!(e, "That album no longer exists");
}

#[test]
fn the_album_list_comes_from_grouped_queries() {
    let (mut db, _, _) = timeline_db();
    let (a, b, c) = (
        id_of(&db, "/one/a.jpg"),
        id_of(&db, "/one/b.mp4"),
        id_of(&db, "/two/c.jpg"),
    );
    for (f, t) in [(a, 100), (b, 300), (c, 200)] {
        db.conn
            .execute(
                "UPDATE files SET taken_at = ?2 WHERE id = ?1",
                params![f, t],
            )
            .unwrap();
    }
    let x = db.create_album("x").unwrap();
    let y = db.create_album("y").unwrap();
    let empty = db.create_album("z").unwrap();
    db.add_to_album(x, &[a, b, c]).unwrap();
    db.add_to_album(y, &[a, c]).unwrap();
    let list = db.albums().unwrap();
    let by = |id| list.iter().find(|i| i.id == id).unwrap();
    assert_eq!((by(x).count, by(x).cover.as_ref().unwrap().file_id), (3, b));
    assert_eq!((by(y).count, by(y).cover.as_ref().unwrap().file_id), (2, c));
    assert_eq!((by(empty).count, by(empty).cover.clone()), (0, None));
}

#[test]
fn deleting_an_album_keeps_files_and_removing_a_folder_keeps_albums() {
    let (mut db, f1, _) = timeline_db();
    let (a, c) = (id_of(&db, "/one/a.jpg"), id_of(&db, "/two/c.jpg"));
    let keep = db.create_album("keep").unwrap();
    let gone = db.create_album("gone").unwrap();
    db.add_to_album(keep, &[a, c]).unwrap();
    db.add_to_album(gone, &[a]).unwrap();
    db.delete_album(gone).unwrap();
    assert_eq!(db.timeline(&TimelineFilter::default()).unwrap().len(), 4);
    assert_eq!(db.albums().unwrap().len(), 1);
    // Removing a library folder drops its files' entries, not the album.
    db.remove_folder(f1).unwrap();
    let info = &db.albums().unwrap()[0];
    assert_eq!((info.name.as_str(), info.count), ("keep", 1));
    let rows: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM album_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}
