//! Command-line interface. Without a subcommand, the GUI starts.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a duration in seconds is shown as whole seconds; saturation is intended"
)]

use crate::config::Paths;
use crate::library::{Activity, Library};
use crate::media::MediaKind;
use crate::model::{download, registry};
use crate::search::{SearchHit, SearchOptions};
use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "recollect",
    version,
    about = "Local, private semantic search over your photos and videos"
)]
pub struct Cli {
    /// Keep all Recollect data under this directory instead of the platform defaults
    /// (also settable with the `RECOLLECT_HOME` environment variable).
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    /// Use this embedding model for this run (see `recollect models list`).
    #[arg(long, global = true)]
    pub model: Option<String>,
    /// Never use the GPU.
    #[arg(long, global = true)]
    pub cpu: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the graphical app (default).
    Gui,
    #[command(flatten)]
    Lib(LibCommand),
    /// Performance measurements on synthetic data (needs --home <scratch dir>).
    Bench {
        #[command(subcommand)]
        cmd: BenchCmd,
    },
}

/// Commands that work on an opened library.
#[derive(Subcommand, Debug)]
pub enum LibCommand {
    /// List or download embedding models.
    Models {
        #[command(subcommand)]
        cmd: ModelsCmd,
    },
    /// Manage library folders.
    Folders {
        #[command(subcommand)]
        cmd: FoldersCmd,
    },
    /// Add the given folders (if any) and index everything new or changed.
    Index { paths: Vec<PathBuf> },
    /// Search the library.
    Search {
        query: String,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Print JSON lines instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Measure search quality with a query set.
    Eval {
        queries: PathBuf,
        /// Directory the expected paths in the query set are relative to.
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Show library statistics and recent errors.
    Status,
}

#[derive(Subcommand, Debug)]
pub enum BenchCmd {
    /// Startup load, memory, database size and search latency for a synthetic library.
    Search {
        #[arg(long, default_value_t = 200_000)]
        images: usize,
        #[arg(long, default_value_t = 20_000)]
        videos: usize,
        /// Segments per video (2,000 h / 20,000 videos = 6 min each; 50 = one per 7 s).
        #[arg(long, default_value_t = 50)]
        segments: usize,
        #[arg(long, default_value_t = 50)]
        queries: usize,
    },
    /// Scan and unchanged-rescan time for a tree of empty files.
    Rescan {
        #[arg(long, default_value_t = 200_000)]
        files: usize,
    },
    /// Load time and memory of the album timeline for a synthetic library without vectors.
    Timeline {
        #[arg(long, default_value_t = 200_000)]
        files: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum ModelsCmd {
    List,
    /// Download the selected model (about 0.5–0.6 GB from huggingface.co).
    Download {
        /// Don't ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum FoldersCmd {
    Add { paths: Vec<PathBuf> },
    List,
    Remove { path: PathBuf },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum KindArg {
    Image,
    Video,
}

impl From<KindArg> for MediaKind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Image => MediaKind::Image,
            KindArg::Video => MediaKind::Video,
        }
    }
}

pub fn paths(cli: &Cli) -> Result<Paths> {
    Ok(match &cli.home {
        Some(h) => Paths::under(h),
        None => Paths::from_env()?,
    })
}

/// Prints the known models and which of them are installed.
fn list_models(lib: &Library) {
    let current = lib.model().id;
    for m in registry::MODELS {
        let mark = if m.id == current { "*" } else { " " };
        let inst = if download::is_installed(&lib.paths().models_dir(), m) {
            "installed"
        } else {
            "not downloaded"
        };
        println!(
            "{mark} {:<16} {:<32} {:>5} MB  {:<11} {inst}",
            m.id,
            m.name,
            m.total_size() / 1_000_000,
            m.license
        );
    }
}

/// Prints what the library holds and how far indexing is.
fn print_status(lib: &Library) -> Result<()> {
    let s = lib.stats()?;
    println!(
        "model:   {}{}",
        lib.model().id,
        if lib.model_installed() {
            ""
        } else {
            " (not downloaded)"
        }
    );
    println!("folders: {}", s.folders);
    println!("files:   {} images, {} videos", s.images, s.videos);
    let d = lib.date_counts()?;
    println!(
        "dates:   {} from EXIF, {} from video metadata, {} from file time",
        d.exif, d.video, d.file
    );
    println!(
        "indexed: {} files ({} vectors), {} pending, {} errors",
        s.indexed, s.items, s.pending, s.errors
    );
    for (p, e) in lib.errors(10)? {
        println!("  error: {}: {e}", p.display());
    }
    Ok(())
}

/// Removes the library folder at `path` (as given or as it resolves on disk).
fn remove_folder(lib: &Library, path: &Path) -> Result<()> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Some(f) = lib
        .folders()?
        .into_iter()
        .find(|f| f.path == canon || f.path == *path)
    else {
        bail!("{} is not a library folder", path.display());
    };
    lib.remove_folder(f.id)?;
    Ok(())
}

/// Runs a command that works on an opened library.
pub fn run(lib: &Arc<Library>, command: &LibCommand) -> Result<()> {
    match command {
        LibCommand::Models {
            cmd: ModelsCmd::List,
        } => {
            list_models(lib);
        }
        LibCommand::Models {
            cmd: ModelsCmd::Download { yes },
        } => download_model(lib, *yes)?,
        LibCommand::Folders { cmd } => folders_command(lib, cmd)?,
        LibCommand::Index { paths } => {
            for p in paths {
                lib.add_folder(p)?;
            }
            index(lib)?;
        }
        LibCommand::Search {
            query,
            limit,
            kind,
            json,
        } => search_command(lib, query, *limit, *kind, *json)?,
        LibCommand::Eval {
            queries,
            root,
            json,
        } => eval_command(lib, queries, root, *json)?,
        LibCommand::Status => {
            print_status(lib)?;
        }
    }
    Ok(())
}

fn folders_command(lib: &Library, cmd: &FoldersCmd) -> Result<()> {
    match cmd {
        FoldersCmd::Add { paths } => {
            for p in paths {
                lib.add_folder(p)?;
            }
        }
        FoldersCmd::List => {
            for f in lib.folders()? {
                println!("{}", f.path.display());
            }
        }
        FoldersCmd::Remove { path } => {
            remove_folder(lib, path)?;
        }
    }
    Ok(())
}

fn search_command(
    lib: &Library,
    query: &str,
    limit: usize,
    kind: Option<KindArg>,
    json: bool,
) -> Result<()> {
    let opts = SearchOptions {
        limit,
        kind: kind.map(Into::into),
        ..Default::default()
    };
    let hits = lib.search(query, &opts)?;
    print_hits(&hits, json);
    Ok(())
}

fn eval_command(lib: &Library, queries: &Path, root: &Path, json: bool) -> Result<()> {
    let rep = crate::eval::run(lib, queries, root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&rep)?);
    } else {
        println!("model {}\n{rep}", lib.model().id);
    }
    Ok(())
}

/// Runs a benchmark (without opening a library first).
pub fn bench(cli: &Cli, cmd: &BenchCmd) -> Result<()> {
    let dir = crate::bench::require_scratch_home(cli.home.as_deref())?;
    let paths = Paths::under(dir);
    match cmd {
        BenchCmd::Search {
            images,
            videos,
            segments,
            queries,
        } => {
            let id = cli.model.as_deref().unwrap_or(registry::DEFAULT_GPU_MODEL);
            crate::bench::search(&paths, id, *images, *videos, *segments, *queries)
        }
        BenchCmd::Timeline { files } => {
            crate::bench::timeline(&Paths::under(&dir.join("timeline-home")), *files)
        }
        BenchCmd::Rescan { files } => {
            crate::bench::rescan(&Paths::under(&dir.join("rescan-home")), dir, *files)
        }
    }
}

fn download_model(lib: &Arc<Library>, yes: bool) -> Result<()> {
    let m = lib.model();
    if lib.model_installed() {
        println!("{} is already installed.", m.id);
        return Ok(());
    }
    if !yes {
        print!(
            "Download {} ({} MB, license {}) from huggingface.co/{}? [y/N] ",
            m.name,
            m.total_size() / 1_000_000,
            m.license,
            m.repo
        );
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            bail!("download declined");
        }
    }
    let mut last = 0u64;
    download::download(
        &lib.paths().models_dir(),
        m,
        &AtomicBool::new(false),
        &mut |done, total| {
            let pct = done * 100 / total.max(1);
            if pct != last {
                last = pct;
                eprint!("\rdownloading {}: {pct}%", m.id);
            }
        },
    )?;
    eprintln!("\ndone");
    Ok(())
}

fn index(lib: &Arc<Library>) -> Result<()> {
    if !lib.model_installed() {
        eprintln!("model not downloaded: only reading dates (run `recollect models download`)");
    }
    lib.start_indexing()?;
    let mut last = String::new();
    while lib.is_busy() {
        std::thread::sleep(Duration::from_millis(500));
        let line = match lib.status().activity {
            Activity::Scanning { folder } => format!("scanning {}", folder.display()),
            Activity::ReadingDates { done, total } => {
                format!("reading dates {done}/{total}")
            }
            Activity::LoadingModel => "loading model".into(),
            Activity::Indexing(p) => format!(
                "indexed {}/{} files ({} errors), {:.1} items/s on {}",
                p.done,
                p.total,
                p.errors,
                p.rate,
                p.device.map(|d| d.to_string()).unwrap_or_default()
            ),
            _ => continue,
        };
        if line != last {
            eprintln!("{line}");
            last = line;
        }
    }
    lib.wait();
    let st = lib.status();
    if let Some(e) = st.last_error {
        bail!(e);
    }
    if let Some(r) = st.last_run {
        let left = if r.deferred > 0 {
            format!(", {} left for the next run", r.deferred)
        } else {
            String::new()
        };
        eprintln!(
            "finished in {:.1}s: {} files processed, {} errors; {} new, {} changed, {} removed{left}",
            r.seconds, r.files, r.errors, r.added, r.changed, r.removed
        );
    }
    Ok(())
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

fn print_hits(hits: &[SearchHit], json: bool) {
    for h in hits {
        if json {
            let v = serde_json::json!({
                "path": h.path, "kind": crate::media::scan::kind_name(h.kind), "score": h.score,
                "t_start": h.t_start, "t_end": h.t_end, "item_id": h.item_id,
            });
            println!("{v}");
        } else {
            let t = h
                .t_start
                .map(|t| format!(" @ {}", fmt_time(t)))
                .unwrap_or_default();
            println!("{:.3}  {}{t}", h.score, h.path.display());
        }
    }
}
