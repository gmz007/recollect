use clap::Parser;
use recollect::Library;
use recollect::cli::{self, Cli, Command};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RECOLLECT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Cli::parse();
    if args.cpu {
        recollect::gpu::force_cpu(true);
    }
    let gui = matches!(args.command, None | Some(Command::Gui));
    let lib = match cli::paths(&args).and_then(|p| Library::open(p, args.model.as_deref())) {
        Ok(lib) => lib,
        Err(e) if gui => {
            let msg = format!("{e:#}");
            eprintln!("error: {msg}");
            return recollect::ui::run_error(msg);
        }
        Err(e) => return Err(e),
    };
    match args.command {
        None | Some(Command::Gui) => recollect::ui::run(lib),
        Some(_) => cli::run(&args, &lib),
    }
}
