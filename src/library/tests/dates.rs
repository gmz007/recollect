//! Capture dates in search: ranges, sorting and dates learned after indexing.

use super::*;

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
fn years_cover_the_whole_library() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    seed_dated(&lib, &["b"]);
    // b is undated: its file time (1970) counts.
    assert_eq!(lib.years().unwrap(), [2021, 2020, 2019, 1970]);
}
