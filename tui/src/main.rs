mod app;
mod tui;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

#[derive(Parser, Debug)]
#[command(
    name = "archinstall-zfs",
    about = "Arch Linux installer with ZFS support"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to a JSON configuration file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Run installation without interactive prompts (requires --config)
    #[arg(long, global = true)]
    silent: bool,

    /// Preview commands without executing them
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Render archiso profile templates for ISO building
    RenderProfile {
        /// Source profile directory containing .j2 templates
        #[arg(long)]
        profile_dir: PathBuf,

        /// Output directory for rendered profile
        #[arg(long)]
        out_dir: PathBuf,

        /// Kernel package (linux, linux-lts, linux-zen)
        #[arg(long, default_value = "linux-lts")]
        kernel: String,

        /// ZFS module mode (precompiled or dkms)
        #[arg(long, default_value = "precompiled")]
        zfs: String,

        /// Include kernel headers (auto, true, false)
        #[arg(long, default_value = "auto")]
        headers: String,

        /// Fast build mode (minimal packages, erofs)
        #[arg(long)]
        fast: bool,
    },
}

fn setup_logging(ui_log_tx: tokio::sync::mpsc::UnboundedSender<(String, i32)>) -> Result<()> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    // Channel layer for UI log display — included globally so all threads see it
    let channel_layer = tui::tracing_layer::ChannelLayer::new(ui_log_tx);
    let ui_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // File layer — trace for our code, warn for noisy deps
    let file_appender = tracing_appender::rolling::never("/tmp", "archinstall-zfs.log");
    let file_filter =
        EnvFilter::new("trace,h2=warn,hyper=warn,reqwest=warn,rustls=warn,pacman=info");
    let file_layer = fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_target(true)
        .with_filter(file_filter);

    tracing_subscriber::registry()
        .with(channel_layer.with_filter(ui_filter))
        .with(file_layer)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let (ui_log_tx, ui_log_rx) = tokio::sync::mpsc::unbounded_channel();
    setup_logging(ui_log_tx)?;

    let cli = Cli::parse();
    tracing::info!(?cli, "starting archinstall-zfs");

    match &cli.command {
        Some(Commands::RenderProfile {
            profile_dir,
            out_dir,
            kernel,
            zfs,
            headers,
            fast,
        }) => archinstall_zfs_core::iso::render_profile(
            profile_dir,
            out_dir,
            kernel,
            zfs,
            headers,
            *fast,
        ),
        None => app::run(cli, ui_log_rx).await,
    }
}
