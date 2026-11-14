//! Indexing run: scan folders, reconcile with the database, embed what is new or changed.
//!
//! Pipeline for images: a rayon pool decodes and resizes files in parallel into a bounded
//! channel; this thread batches them through the vision encoder and writes results to SQLite
//! and the in-memory index. Bounded channels keep memory flat; the control object gives
//! pause/cancel at every file boundary.

use crate::library::{Activity, IndexProgress, Library, RunSummary};
use crate::media::image::{self as img, Fit};
use crate::media::metadata::{Capture, read_capture};
use crate::media::{MediaKind, scan};
use crate::model::registry::Family;
use crate::model::{Device, ModelSpec, VisionEncoder};
use crate::store::db::{FileMeta, FileRow, NewItem};
use anyhow::{Result, anyhow, ensure};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Files written to the database per transaction.
const WRITE_BATCH: usize = 64;

pub(crate) fn run(lib: &Library) -> Result<RunSummary> {
    let t0 = Instant::now();
    let mut summary = RunSummary::default();
    // Folders added while we run set `rescan_requested`; go round again until none were.
    lib.rescan_requested.store(false, Ordering::Release);
    loop {
        summary.skipped_folders.clear();
        sync_folders(lib, &mut summary)?;
        if lib.control.is_cancelled() {
            break;
        }
        read_dates(lib, &mut summary)?;
        if lib.control.is_cancelled() {
            break;
        }
        // Without a model the library is still browsable by date; embedding waits for it.
        if lib.model_installed() {
            index_pending(lib, &mut summary)?;
        }
        if lib.control.is_cancelled() || !lib.rescan_requested.swap(false, Ordering::AcqRel) {
            break;
        }
    }
    summary.cancelled = lib.control.is_cancelled();
    summary.seconds = t0.elapsed().as_secs_f32();
    Ok(summary)
}

/// Scan and reconcile only, no embedding (benchmarks).
pub(crate) fn sync_only(lib: &Library) -> Result<RunSummary> {
    let mut summary = RunSummary::default();
    sync_folders(lib, &mut summary)?;
    Ok(summary)
}

/// Rows per transaction (and progress update) in the date pass.
const DATE_BATCH: usize = 500;

/// Reads capture dates for files that have none yet. Runs before embedding so the album is
/// ordered correctly as soon as possible; it only reads headers, so it is fast even for large
/// libraries (decode threads are used because network drives make the opens slow).
fn read_dates(lib: &Library, summary: &mut RunSummary) -> Result<()> {
    use rayon::prelude::*;
    let pending = lib.db.lock().pending_dates()?;
    if pending.is_empty() {
        return Ok(());
    }
    let total = pending.len();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(lib.settings().decode_threads())
        .thread_name(|i| format!("recollect-dates-{i}"))
        .build()?;
    // Dates of files that already have items, patched into the search index in one pass at the
    // end (also when the pass stops early: the database changes are already committed).
    let mut embedded: HashMap<i64, i64> = HashMap::new();
    let result = (|| -> Result<()> {
        let mut done = 0;
        for chunk in pending.chunks(DATE_BATCH) {
            lib.update_status(|s| s.activity = Activity::ReadingDates { done, total });
            if lib.control.checkpoint() {
                return Ok(());
            }
            let caps: Vec<Option<(i64, Capture)>> = pool.install(|| {
                chunk
                    .par_iter()
                    .map(|f| {
                        if lib.control.checkpoint() {
                            return None;
                        }
                        // `None` inside: unreadable right now, left undated for the next run.
                        contain("reading the capture date", || {
                            read_capture(&f.path, f.kind, f.mtime)
                                .ok_or_else(|| anyhow!("unreadable right now"))
                        })
                        .ok()
                        .map(|c| (f.id, c))
                    })
                    .collect()
            });
            if lib.control.is_cancelled() {
                return Ok(()); // a partly read chunk is simply read again next time
            }
            let caps: Vec<(i64, Capture)> = caps.into_iter().flatten().collect();
            embedded.extend(apply_captures(lib, &caps)?);
            done += chunk.len();
            summary.dates_read += caps.len();
        }
        Ok(())
    })();
    patch_index_dates(lib, &embedded);
    result
}

/// Stores capture times. Returns the dates (file id to time) of the files that already have
/// items, which are the only ones the search index holds a time for; the caller passes them
/// to [`patch_index_dates`]. The index is not touched here, so a batch of files without items
/// never contends for its write lock.
pub(crate) fn apply_captures(lib: &Library, caps: &[(i64, Capture)]) -> Result<HashMap<i64, i64>> {
    let embedded = lib.db.lock().set_captures(caps)?;
    let times: HashMap<i64, i64> = caps.iter().map(|(id, c)| (*id, c.taken_at)).collect();
    Ok(embedded
        .into_iter()
        .filter_map(|id| times.get(&id).map(|t| (id, *t)))
        .collect())
}

/// Patches capture times into the search index in one pass over its rows. The matching
/// database change moved the epoch, so the on-disk cache is rewritten instead of reused with
/// the old times.
pub(crate) fn patch_index_dates(lib: &Library, times: &HashMap<i64, i64>) {
    if times.is_empty() {
        return;
    }
    lib.index.write().set_taken(times);
}

fn index_pending(lib: &Library, summary: &mut RunSummary) -> Result<()> {
    let spec = lib.model();
    let pending = lib.db.lock().pending(spec.id)?;
    let (images, videos): (Vec<FileRow>, Vec<FileRow>) = pending
        .into_iter()
        .partition(|f| f.kind == MediaKind::Image);
    let total = images.len() + videos.len();
    if total == 0 {
        return Ok(());
    }
    lib.update_status(|s| s.activity = Activity::LoadingModel);
    let settings = lib.settings();
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let gpu = settings.use_gpu && !crate::gpu::cpu_forced();
    let mut enc = VisionEncoder::load(spec, &lib.model_dir(), gpu, threads)?;
    summary.device = Some(enc.device());
    let mut prog = Progress::new(total, enc.device());
    index_images(lib, &mut enc, &images, settings.decode_threads(), &mut prog)?;
    crate::video_index::index_videos(lib, &mut enc, &videos, &mut prog)?;
    summary.files += prog.p.done - prog.p.errors;
    summary.errors += prog.p.errors;
    Ok(())
}

/// Runs `f`, turning a panic inside it (a decoder bug on one odd file) into an error so it
/// fails that file instead of the whole run.
pub(crate) fn contain<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        tracing::warn!("{what} panicked");
        Err(anyhow!("{what} panicked"))
    })
}

/// Fails unless the model returned one embedding per input (`what`), so zipping the two can
/// never silently drop or mismatch files.
pub(crate) fn check_count(what: &str, embeddings: usize, inputs: usize) -> Result<()> {
    ensure!(
        embeddings == inputs,
        "the model returned {embeddings} embeddings for {inputs} {what}"
    );
    Ok(())
}

/// What happened to a file that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Looks temporary; the file stays pending and the next run tries it again.
    Retry,
    /// The error was stored on the file; it is not retried until it changes.
    Recorded,
}

impl Failure {
    /// Whether the failure counts as an error in the run summary.
    pub(crate) fn is_error(self) -> bool {
        self == Self::Recorded
    }
}

/// Records a per-file failure. Failures that look temporary (the file or its drive vanished,
/// I/O errors) leave the file pending so the next run retries it; decode failures are
/// recorded as errors.
pub(crate) fn file_failed(lib: &Library, f: &FileRow, e: &anyhow::Error) -> Result<Failure> {
    let transient = !f.path.exists()
        || e.chain().any(|c| {
            c.downcast_ref::<std::io::Error>().is_some_and(|io| {
                !matches!(
                    io.kind(),
                    std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
                )
            })
        });
    if transient {
        tracing::warn!("{}: {e:#} (will retry on the next scan)", f.path.display());
    } else {
        lib.db.lock().set_error(f.id, &format!("{e:#}"))?;
    }
    Ok(if transient {
        Failure::Retry
    } else {
        Failure::Recorded
    })
}

/// Scans every folder and reconciles the files table. Folders whose root is unreachable (e.g.
/// an unplugged drive) are skipped rather than emptied.
fn sync_folders(lib: &Library, summary: &mut RunSummary) -> Result<()> {
    let folders = lib.db.lock().folders()?;
    for f in folders {
        if lib.control.checkpoint() {
            return Ok(());
        }
        lib.update_status(|s| {
            s.activity = Activity::Scanning {
                folder: f.path.clone(),
            }
        });
        if !f.path.is_dir() {
            tracing::warn!(
                "library folder {} is not reachable; skipping",
                f.path.display()
            );
            summary.skipped_folders.push(f.path);
            continue;
        }
        // `checkpoint` also honours pause while walking big trees.
        let res = scan::scan(&f.path, &|| lib.control.checkpoint());
        if lib.control.is_cancelled() {
            return Ok(());
        }
        for (p, e) in &res.problems {
            tracing::warn!("{}: {e}", p.display());
        }
        let protected: Vec<_> = res.problems.iter().map(|p| p.0.clone()).collect();
        let mut db = lib.db.lock();
        // An unmounted drive often leaves an empty mount-point directory behind: never treat
        // "everything vanished" as deletions.
        if res.files.is_empty() && db.folder_file_count(f.id)? > 0 {
            tracing::warn!(
                "{} looks empty or unmounted; keeping its index",
                f.path.display()
            );
            summary.skipped_folders.push(f.path);
            continue;
        }
        let stats = db.sync_folder(f.id, &res.files, &protected)?;
        summary.added += stats.added;
        summary.changed += stats.changed;
        summary.removed += stats.removed.len();
        let stale: HashSet<i64> = stats.removed.into_iter().chain(stats.changed_ids).collect();
        if !stale.is_empty() {
            // Still holding the db lock: lock order is db, then index.
            lib.remove_files_from_index(&stale);
        }
        drop(db);
    }
    Ok(())
}

pub(crate) struct Progress {
    pub p: IndexProgress,
    started: Instant,
    items: usize,
    last_update: Instant,
}

impl Progress {
    pub fn new(total: usize, device: Device) -> Self {
        Self {
            p: IndexProgress {
                total,
                device: Some(device),
                ..Default::default()
            },
            started: Instant::now(),
            items: 0,
            last_update: Instant::now(),
        }
    }

    pub fn file_done(&mut self, lib: &Library, f: &FileRow, items: usize, error: bool) {
        self.p.done += 1;
        self.p.errors += usize::from(error);
        self.items += items;
        self.p.current = Some(f.path.clone());
        if self.last_update.elapsed().as_millis() >= 100 || self.p.done == self.p.total {
            self.publish(lib);
        }
    }

    pub fn items_done(&mut self, lib: &Library, items: usize) {
        self.items += items;
        if self.last_update.elapsed().as_millis() >= 100 {
            self.publish(lib);
        }
    }

    fn publish(&mut self, lib: &Library) {
        self.p.rate = self.items as f32 / self.started.elapsed().as_secs_f32().max(1e-3);
        self.last_update = Instant::now();
        let p = self.p.clone();
        lib.update_status(|s| s.activity = Activity::Indexing(p));
    }
}

pub(crate) fn fit_for(spec: &ModelSpec) -> Fit {
    match spec.family {
        Family::Clip => Fit::CenterCrop,
        Family::Siglip => Fit::Stretch,
    }
}

/// Decodes an image file into the model's square RGB input.
pub(crate) fn prepare_image(
    path: &std::path::Path,
    spec: &ModelSpec,
) -> Result<(Vec<u8>, FileMeta)> {
    contain("decoding the image", || {
        let d = img::decode(path, spec.image_size)?;
        let px = img::to_square(&d.rgb, spec.image_size, fit_for(spec))?;
        Ok((
            px,
            FileMeta {
                width: Some(d.orig_width),
                height: Some(d.orig_height),
                duration: None,
            },
        ))
    })
}

type Prepared = (FileRow, Result<(Vec<u8>, FileMeta)>);

fn index_images(
    lib: &Library,
    enc: &mut VisionEncoder,
    files: &[FileRow],
    threads: usize,
    prog: &mut Progress,
) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let spec = lib.model();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("recollect-decode-{i}"))
        .build()?;
    let batch = enc.batch_size();
    let (tx, rx) = crossbeam_channel::bounded::<Prepared>(batch * 2);
    // Set when embedding or writing fails, so decoders stop instead of finishing the library.
    let abort = std::sync::atomic::AtomicBool::new(false);
    let abort = &abort;
    std::thread::scope(|s| -> Result<()> {
        s.spawn(move || {
            pool.install(|| {
                use rayon::prelude::*;
                files.par_iter().for_each_with(tx, |tx, f| {
                    if abort.load(std::sync::atomic::Ordering::Relaxed) || lib.control.checkpoint()
                    {
                        return;
                    }
                    let _ = tx.send((f.clone(), prepare_image(&f.path, spec)));
                });
            });
        });
        let mut ready: Vec<(FileRow, Vec<u8>, FileMeta)> = Vec::with_capacity(batch);
        let mut out: Vec<(FileRow, FileMeta, Vec<NewItem>)> = Vec::new();
        let mut result = Ok(());
        for (f, r) in &rx {
            if result.is_err() || lib.control.is_cancelled() {
                continue; // drain so producers can finish
            }
            match r {
                Ok((px, meta)) => ready.push((f, px, meta)),
                Err(e) => {
                    let failure = file_failed(lib, &f, &e).unwrap_or_else(|db_err| {
                        // Not recorded: the file stays pending and is retried next run.
                        tracing::warn!(
                            "{}: could not record failure: {db_err:#}",
                            f.path.display()
                        );
                        Failure::Retry
                    });
                    prog.file_done(lib, &f, 0, failure.is_error());
                }
            }
            if ready.len() >= batch {
                result = embed_ready(lib, enc, &mut ready, &mut out, prog);
            }
            if out.len() >= WRITE_BATCH && result.is_ok() {
                result = write_items(lib, &mut out);
            }
            if result.is_err() {
                abort.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if result.is_ok() && !lib.control.is_cancelled() {
            result = embed_ready(lib, enc, &mut ready, &mut out, prog);
        }
        if result.is_ok() {
            result = write_items(lib, &mut out);
        }
        result
    })
}

fn embed_ready(
    lib: &Library,
    enc: &mut VisionEncoder,
    ready: &mut Vec<(FileRow, Vec<u8>, FileMeta)>,
    out: &mut Vec<(FileRow, FileMeta, Vec<NewItem>)>,
    prog: &mut Progress,
) -> Result<()> {
    if ready.is_empty() {
        return Ok(());
    }
    let pixels: Vec<&[u8]> = ready.iter().map(|r| r.1.as_slice()).collect();
    let embs = enc.embed(&pixels)?;
    check_count("images", embs.len(), ready.len())?;
    for ((f, _, meta), e) in ready.drain(..).zip(embs) {
        prog.file_done(lib, &f, 1, false);
        out.push((
            f,
            meta,
            vec![NewItem {
                t_start: None,
                t_end: None,
                embedding: e,
            }],
        ));
    }
    Ok(())
}

/// Persists finished files and appends their vectors to the live index. (Stale vectors of
/// changed files were already dropped during the folder sync.)
pub(crate) fn write_items(
    lib: &Library,
    out: &mut Vec<(FileRow, FileMeta, Vec<NewItem>)>,
) -> Result<()> {
    if out.is_empty() {
        return Ok(());
    }
    let spec = lib.model();
    let kinds: Vec<MediaKind> = out.iter().map(|o| o.0.kind).collect();
    let batch: Vec<(i64, FileMeta, Vec<NewItem>)> = out
        .drain(..)
        .map(|(f, m, items)| (f.id, m, items))
        .collect();
    // Hold the db lock while updating the index so a concurrent folder removal can't slip in
    // between (lock order: db, then index).
    let mut db = lib.db.lock();
    let ids = db.put_items(spec.id, &batch)?;
    // Dates were read before embedding started, so the timeline's value is the right one.
    let file_ids: Vec<i64> = batch.iter().map(|b| b.0).collect();
    let taken = db.taken_times(&file_ids)?;
    let mut index = lib.index.write();
    for (((file_id, _, items), item_ids), kind) in batch.iter().zip(&ids).zip(kinds) {
        // A file that vanished meanwhile has no row (and no items) any more.
        let Some(&t) = taken.get(file_id) else {
            continue;
        };
        for (it, id) in items.iter().zip(item_ids) {
            index.add(*id, *file_id, kind, t, &it.embedding);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_an_error() {
        assert_eq!(contain("adding", || Ok(1 + 1)).unwrap(), 2);
        let err = contain::<()>("decoding", || panic!("odd file")).unwrap_err();
        assert_eq!(err.to_string(), "decoding panicked");
        let err = contain::<()>("decoding", || Err(anyhow!("plain failure"))).unwrap_err();
        assert_eq!(err.to_string(), "plain failure");
    }

    #[test]
    fn embedding_counts_must_match() {
        assert!(check_count("images", 3, 3).is_ok());
        let err = check_count("frames", 2, 3).unwrap_err().to_string();
        assert!(err.contains("2 embeddings for 3 frames"), "{err}");
        assert!(check_count("images", 4, 3).is_err());
        assert!(check_count("images", 0, 0).is_ok());
    }

    #[test]
    fn only_recorded_failures_count_as_errors() {
        assert!(Failure::Recorded.is_error());
        assert!(!Failure::Retry.is_error());
    }
}
