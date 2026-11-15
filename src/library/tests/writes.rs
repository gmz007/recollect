//! Favorite marks and album changes through the writer thread.

use super::*;

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
