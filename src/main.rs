use clap::Parser;
use recollect::Library;
use recollect::cli::{self, Cli, Command};

fn main() -> anyhow::Result<()> {
    // Frame statistics are logged at info level; make them visible without extra setup.
    let default_filter = if std::env::var_os("RECOLLECT_FRAME_STATS").is_some() {
        "warn,recollect::ui::frame_stats=info"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RECOLLECT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Cli::parse();
    if args.cpu {
        recollect::gpu::force_cpu(true);
    }
    let lib_cmd = match &args.command {
        // Benchmarks open their own libraries; don't load one up front.
        Some(Command::Bench { cmd }) => return cli::bench(&args, cmd),
        Some(Command::Lib(cmd)) => Some(cmd),
        None | Some(Command::Gui) => None,
    };
    let gui = lib_cmd.is_none();
    let lib = match cli::paths(&args).and_then(|p| Library::open(p, args.model.as_deref())) {
        Ok(lib) => lib,
        Err(e) if gui => {
            let msg = format!("{e:#}");
            eprintln!("error: {msg}");
            return recollect::run_error(msg);
        }
        Err(e) => return Err(e),
    };
    match lib_cmd {
        None => recollect::run(lib),
        Some(cmd) => {
            let res = cli::run(&lib, cmd);
            // No-op unless something changed, so CLI runs leave a valid cache behind.
            lib.save_index_cache();
            res
        }
    }
}
