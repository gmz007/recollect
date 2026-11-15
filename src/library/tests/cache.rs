//! The index cache on disk and the index after a folder is removed.

use super::*;

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
