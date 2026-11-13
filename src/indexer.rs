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
use anyhow::Result;
use std::collections::HashSet;
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
                    read_capture(&f.path, f.kind, f.mtime).map(|c| (f.id, c))
                })
                .collect()
        });
        if lib.control.is_cancelled() {
            return Ok(()); // a partly read chunk is simply read again next time
        }
        let caps: Vec<(i64, Capture)> = caps.into_iter().flatten().collect();
        apply_captures(lib, &caps)?;
        done += chunk.len();
        summary.dates_read += caps.len();
    }
    Ok(())
}

/// Stores capture times. Files that were embedded before their date was known (it was
/// unreadable at the time) have the fallback time in the search index; the in-memory column is
/// patched in one pass, and the database write moves the epoch so the on-disk cache is
/// rewritten instead of reused with the old time.
pub(crate) fn apply_captures(lib: &Library, caps: &[(i64, Capture)]) -> Result<()> {
    // Lock order: db, then index.
    let mut db = lib.db.lock();
    db.set_captures(caps)?;
    let mut index = lib.index.write();
    if !index.is_empty() {
        let times = caps.iter().map(|(id, c)| (*id, c.taken_at)).collect();
        index.set_taken(&times);
    }
    Ok(())
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
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
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

/// Records a per-file failure. Failures that look temporary (the file or its drive vanished,
/// I/O errors) leave the file pending so the next run retries it; decode failures are
/// recorded as errors.
pub(crate) fn file_failed(lib: &Library, f: &FileRow, e: &anyhow::Error) -> Result<bool> {
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
    Ok(!transient)
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
            summary.skipped_folders.push(f.path.clone());
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
            summary.skipped_folders.push(f.path.clone());
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
        for (f, r) in rx.iter() {
            if result.is_err() || lib.control.is_cancelled() {
                continue; // drain so producers can finish
            }
            match r {
                Ok((px, meta)) => ready.push((f, px, meta)),
                Err(e) => {
                    let failed = file_failed(lib, &f, &e).unwrap_or(true);
                    prog.file_done(lib, &f, 0, failed);
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
        let t = taken.get(file_id).copied().unwrap_or_default();
        for (it, id) in items.iter().zip(item_ids) {
            index.add(*id, *file_id, kind, t, &it.embedding);
        }
    }
    Ok(())
}
