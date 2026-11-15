//! The background worker: downloads, scans and what waits for a running job.

use super::*;

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
