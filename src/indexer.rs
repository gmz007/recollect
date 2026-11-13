//! Indexing run: scan folders, reconcile with the database, embed what is new or changed.
//!
//! Pipeline for images: a rayon pool decodes and resizes files in parallel into a bounded
//! channel; this thread batches them through the vision encoder and writes results to SQLite
//! and the in-memory index. Bounded channels keep memory flat; the control object gives
//! pause/cancel at every file boundary.

use crate::library::{Activity, IndexProgress, Library, RunSummary};
use crate::media::image::{self as img, Fit};
use crate::media::{MediaKind, scan};
use crate::model::registry::Family;
use crate::model::{Device, ModelSpec, VisionEncoder};
use crate::store::db::{FileMeta, FileRow, NewItem};
use anyhow::Result;
use std::collections::HashSet;
use std::time::Instant;

/// Files written to the database per transaction.
const WRITE_BATCH: usize = 64;

pub(crate) fn run(lib: &Library) -> Result<RunSummary> {
    let t0 = Instant::now();
    let mut summary = RunSummary::default();
    sync_folders(lib, &mut summary)?;
    if lib.control.is_cancelled() {
        summary.cancelled = true;
        return Ok(summary);
    }

    let spec = lib.model();
    let pending = lib.db.lock().pending(spec.id)?;
    let (images, videos): (Vec<FileRow>, Vec<FileRow>) = pending
        .into_iter()
        .partition(|f| f.kind == MediaKind::Image);
    let total = images.len() + videos.len();
    if total > 0 {
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
        summary.files = prog.p.done;
        summary.errors = prog.p.errors;
    }
    summary.cancelled = lib.control.is_cancelled();
    summary.seconds = t0.elapsed().as_secs_f32();
    Ok(summary)
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
            continue;
        }
        let res = scan::scan(&f.path, &|| lib.control.is_cancelled());
        if lib.control.is_cancelled() {
            return Ok(());
        }
        for (p, e) in &res.problems {
            tracing::warn!("{}: {e}", p.display());
        }
        let stats = lib.db.lock().sync_folder(f.id, &res.files)?;
        summary.added += stats.added;
        summary.changed += stats.changed;
        summary.removed += stats.removed.len();
        let stale: HashSet<i64> = stats.removed.into_iter().chain(stats.changed_ids).collect();
        if !stale.is_empty() {
            lib.remove_files_from_index(&stale);
        }
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
    std::thread::scope(|s| -> Result<()> {
        s.spawn(move || {
            pool.install(|| {
                use rayon::prelude::*;
                files.par_iter().for_each_with(tx, |tx, f| {
                    if lib.control.checkpoint() {
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
                    let _ = lib.db.lock().set_error(f.id, &format!("{e:#}"));
                    prog.file_done(lib, &f, 0, true);
                }
            }
            if ready.len() >= batch {
                result = embed_ready(lib, enc, &mut ready, &mut out, prog);
            }
            if out.len() >= WRITE_BATCH && result.is_ok() {
                result = write_items(lib, &mut out);
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
    let ids = lib.db.lock().put_items(spec.id, &batch)?;
    let mut index = lib.index.write();
    for (((file_id, _, items), item_ids), kind) in batch.iter().zip(&ids).zip(kinds) {
        for (it, id) in items.iter().zip(item_ids) {
            index.add(*id, *file_id, kind, &it.embedding);
        }
    }
    Ok(())
}
