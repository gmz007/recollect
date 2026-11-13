//! Command-line interface. Without a subcommand, the GUI starts.

use crate::config::Paths;
use crate::library::{Activity, Library};
use crate::media::MediaKind;
use crate::model::{download, registry};
use crate::search::{SearchHit, SearchOptions};
use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write;
use std::path::PathBuf;
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
    /// (also settable with the RECOLLECT_HOME environment variable).
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
pub enum ModelsCmd {
    List,
    /// Download the selected model (about 0.6–0.8 GB from huggingface.co).
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

/// Runs a non-GUI command.
pub fn run(cli: &Cli, lib: &Arc<Library>) -> Result<()> {
    match cli.command.as_ref().expect("GUI handled by caller") {
        Command::Gui => unreachable!(),
        Command::Models {
            cmd: ModelsCmd::List,
        } => {
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
        Command::Models {
            cmd: ModelsCmd::Download { yes },
        } => download_model(lib, *yes)?,
        Command::Folders { cmd } => match cmd {
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
                let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                let Some(f) = lib
                    .folders()?
                    .into_iter()
                    .find(|f| f.path == canon || f.path == *path)
                else {
                    bail!("{} is not a library folder", path.display());
                };
                lib.remove_folder(f.id)?;
            }
        },
        Command::Index { paths } => {
            for p in paths {
                lib.add_folder(p)?;
            }
            index(lib)?;
        }
        Command::Search {
            query,
            limit,
            kind,
            json,
        } => {
            let opts = SearchOptions {
                limit: *limit,
                kind: kind.map(Into::into),
            };
            let hits = lib.search(query, &opts)?;
            print_hits(&hits, *json);
        }
        Command::Eval {
            queries,
            root,
            json,
        } => {
            let rep = crate::eval::run(lib, queries, root)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                println!("model {}\n{rep}", lib.model().id);
            }
        }
        Command::Status => {
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
            println!(
                "indexed: {} files ({} vectors), {} pending, {} errors",
                s.indexed, s.items, s.pending, s.errors
            );
            for (p, e) in lib.errors(10)? {
                println!("  error: {}: {e}", p.display());
            }
        }
    }
    Ok(())
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
    lib.start_indexing()?;
    let mut last = String::new();
    while lib.is_busy() {
        std::thread::sleep(Duration::from_millis(500));
        let line = match lib.status().activity {
            Activity::Scanning { folder } => format!("scanning {}", folder.display()),
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
        eprintln!(
            "finished in {:.1}s: {} files processed, {} errors; {} new, {} changed, {} removed",
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
