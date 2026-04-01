mod app;
mod config;
mod disk;
mod installer;
mod kernel;
mod profile;
mod swap;
mod system;
mod tui;
mod zfs;
mod zrepl;

use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::Result;

#[derive(Parser, Debug)]
#[command(
    name = "archinstall-zfs",
    about = "Arch Linux installer with ZFS support"
)]
pub struct Cli {
    /// Path to a JSON configuration file
    #[arg(long)]
    config: Option<PathBuf>,

    /// Run installation without interactive prompts (requires --config)
    #[arg(long)]
    silent: bool,

    /// Preview commands without executing them
    #[arg(long)]
    dry_run: bool,
}

fn setup_logging() -> Result<()> {
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let file_appender = tracing_appender::rolling::never("/tmp", "archinstall-zfs.log");
    let file_layer = fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_target(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .init();

    Ok(())
}

fn main() -> Result<()> {
    color_eyre::install()?;
    setup_logging()?;

    let cli = Cli::parse();
    tracing::info!(?cli, "starting archinstall-zfs");

    app::run(cli)
}
