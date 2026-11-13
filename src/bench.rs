//! `recollect bench`: measurements for the PRD performance targets on synthetic data.
//! Always run with `--home <scratch dir>`; it writes a large throwaway library there.

use crate::Library;
use crate::config::Paths;
use crate::media::{MediaKind, ScannedFile};
use crate::model::registry;
use crate::search::{self, SearchOptions};
use crate::store::Db;
use crate::store::db::{FileMeta, NewItem};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Resident memory of this process in MB (Linux; 0 elsewhere).
pub fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }

    fn unit(&mut self, dim: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dim).map(|_| self.next_f32()).collect();
        crate::model::embedder::normalize(&mut v);
        v
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Builds (if needed) a synthetic library of `images` images and `videos` videos with
/// `segments` segments each, then measures load time, memory, DB size and search latency.
pub fn search(
    paths: &Paths,
    model_id: &str,
    images: usize,
    videos: usize,
    segments: usize,
    queries: usize,
) -> Result<()> {
    let spec =
        registry::get(model_id).ok_or_else(|| anyhow::anyhow!("unknown model {model_id}"))?;
    let marker = paths.data.join("bench-library.txt");
    let wanted = format!("{model_id} {images} {videos} {segments}");
    if std::fs::read_to_string(&marker).ok().as_deref() != Some(wanted.as_str()) {
        if Db::open(&paths.db_path())?.stats(model_id)?.folders > 0 {
            bail!(
                "{} already holds a different library; use an empty --home",
                paths.data.display()
            );
        }
        generate(paths, spec, images, videos, segments)?;
        std::fs::write(&marker, &wanted)?;
    }
    let db_mb = std::fs::metadata(paths.db_path())?.len() / 1_000_000
        + std::fs::metadata(paths.db_path().with_extension("db-wal"))
            .map(|m| m.len() / 1_000_000)
            .unwrap_or(0);

    let rss0 = rss_mb();
    let t = Instant::now();
    let lib = Library::open(paths.clone(), Some(model_id))?;
    let load = t.elapsed();
    let rss = rss_mb();
    println!(
        "library: {} vectors ({images} images + {videos} videos × {segments}), dim {}",
        lib.indexed_items(),
        spec.dim
    );
    println!(
        "T6 startup (open + index load): {:.2} s",
        load.as_secs_f64()
    );
    println!("T4 RSS after load: {rss} MB (process baseline before open {rss0} MB)");
    println!("T5 database size: {db_mb} MB");

    let mut rng = Rng(0x5EED);
    let opts = SearchOptions {
        limit: 200,
        kind: None,
    };
    search::by_vector(&lib, &rng.unit(spec.dim), &opts, None)?; // warm caches
    let mut lat = Vec::with_capacity(queries);
    for _ in 0..queries {
        let q = rng.unit(spec.dim);
        let t = Instant::now();
        let hits = search::by_vector(&lib, &q, &opts, None)?;
        lat.push(t.elapsed());
        assert!(!hits.is_empty());
    }
    lat.sort();
    let mut scan = Vec::new();
    for _ in 0..queries.min(20) {
        let q = rng.unit(spec.dim);
        let t = Instant::now();
        let c = lib.index.read().search(&q, 900, None);
        scan.push(t.elapsed());
        assert!(!c.is_empty());
    }
    scan.sort();
    println!(
        "  of which i8 scan (top 900): p50 {:.1} ms, p95 {:.1} ms",
        percentile(&scan, 0.5).as_secs_f64() * 1e3,
        percentile(&scan, 0.95).as_secs_f64() * 1e3
    );
    println!(
        "vector search (scan + group + info, limit 200): p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        percentile(&lat, 0.5).as_secs_f64() * 1e3,
        percentile(&lat, 0.95).as_secs_f64() * 1e3,
        lat.last().unwrap().as_secs_f64() * 1e3
    );
    if lib.model_installed() {
        lib.embed_text("warm up")?;
        let mut tl = Vec::new();
        for q in [
            "kids on the beach at sunset",
            "a dog playing in the snow",
            "birthday cake with candles",
        ] {
            for _ in 0..5 {
                let t = Instant::now();
                lib.embed_text(q)?;
                tl.push(t.elapsed());
            }
        }
        tl.sort();
        println!(
            "text encoding: p50 {:.1} ms, p95 {:.1} ms",
            percentile(&tl, 0.5).as_secs_f64() * 1e3,
            percentile(&tl, 0.95).as_secs_f64() * 1e3
        );
        println!(
            "T1 estimate (text p95 + search p95): {:.1} ms",
            (percentile(&tl, 0.95) + percentile(&lat, 0.95)).as_secs_f64() * 1e3
        );
    } else {
        println!("(model not installed in this home: text encoding not measured)");
    }
    println!("RSS after searches: {} MB", rss_mb());
    Ok(())
}

fn generate(
    paths: &Paths,
    spec: &registry::ModelSpec,
    images: usize,
    videos: usize,
    segments: usize,
) -> Result<()> {
    let t = Instant::now();
    paths.create_all()?;
    let mut db = Db::open(&paths.db_path())?;
    let root = Path::new("/bench-library");
    let folder = db.add_folder(root)?;
    let mut files: Vec<ScannedFile> = (0..images)
        .map(|i| ScannedFile {
            path: root.join(format!("photos/{:03}/img_{i:07}.jpg", i / 1000)),
            kind: MediaKind::Image,
            size: 3_000_000,
            mtime: 1,
        })
        .collect();
    files.extend((0..videos).map(|i| ScannedFile {
        path: root.join(format!("videos/{:03}/vid_{i:06}.mp4", i / 1000)),
        kind: MediaKind::Video,
        size: 500_000_000,
        mtime: 1,
    }));
    db.sync_folder(folder, &files, &[])?;
    drop(files);
    let pending = db.pending(spec.id)?;
    let mut rng = Rng(0xBEEF);
    let mut batch = Vec::new();
    let mut done = 0usize;
    for f in pending {
        let items: Vec<NewItem> = if f.kind == MediaKind::Video {
            (0..segments)
                .map(|s| NewItem {
                    t_start: Some(s as f64 * 6.0),
                    t_end: Some(s as f64 * 6.0 + 6.0),
                    embedding: rng.unit(spec.dim),
                })
                .collect()
        } else {
            vec![NewItem {
                t_start: None,
                t_end: None,
                embedding: rng.unit(spec.dim),
            }]
        };
        done += items.len();
        let meta = FileMeta {
            width: Some(4000),
            height: Some(3000),
            duration: (f.kind == MediaKind::Video).then_some(segments as f64 * 6.0),
        };
        batch.push((f.id, meta, items));
        if batch.iter().map(|b| b.2.len()).sum::<usize>() >= 20_000 {
            db.put_items(spec.id, &batch)?;
            batch.clear();
            eprint!("\rgenerating: {done} vectors");
        }
    }
    db.put_items(spec.id, &batch)?;
    eprintln!(
        "\rgenerated {done} vectors in {:.0} s",
        t.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Creates `files` empty media files under `dir` (if missing) and measures the first scan and
/// an unchanged rescan.
pub fn rescan(paths: &Paths, dir: &Path, files: usize) -> Result<()> {
    let tree = dir.join("tree");
    if !tree.join(".complete").exists() {
        let t = Instant::now();
        for i in 0..files {
            let d = tree.join(format!("{:03}/{:03}", i / 100_000, (i / 1000) % 100));
            if i % 1000 == 0 {
                std::fs::create_dir_all(&d)?;
            }
            let ext = if i % 50 == 0 { "mp4" } else { "jpg" };
            std::fs::File::create(d.join(format!("f{i:07}.{ext}")))?;
        }
        std::fs::write(tree.join(".complete"), b"")?;
        eprintln!(
            "created {files} files in {:.0} s",
            t.elapsed().as_secs_f64()
        );
    }
    let lib = Library::open(paths.clone(), Some(registry::CLIP_B32.id))?;
    lib.add_folder(&tree)?;
    for label in ["first scan (inserts)", "T7 unchanged rescan"] {
        let t = Instant::now();
        let s = crate::indexer::sync_only(&lib)?;
        println!(
            "{label}: {:.2} s ({} new, {} changed, {} removed)",
            t.elapsed().as_secs_f64(),
            s.added,
            s.changed,
            s.removed
        );
    }
    println!("RSS: {} MB", rss_mb());
    Ok(())
}

/// Default bench home if none given: refuse, to never write GBs into the user's real library.
pub fn require_scratch_home(home: Option<&PathBuf>) -> Result<()> {
    if home.is_none() {
        bail!("benchmarks need --home <scratch dir> (they create a large throwaway library)");
    }
    Ok(())
}
