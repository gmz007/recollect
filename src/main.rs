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
    let lib = Library::open(cli::paths(&args)?, args.model.as_deref())?;
    match args.command {
        None | Some(Command::Gui) => recollect::ui::run(lib),
        Some(_) => cli::run(&args, &lib),
    }
}
