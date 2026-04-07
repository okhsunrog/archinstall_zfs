mod app;
mod tui;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};

#[derive(Parser, Debug)]
#[command(
    name = "archinstall-zfs",
    about = "Arch Linux installer with ZFS support"
)]
pub struct Cli {
    /// Path to a JSON configuration file
    #[arg(long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Run installation without interactive prompts (requires --config)
    #[arg(long, global = true)]
    pub silent: bool,

    /// Preview commands without executing them
    #[arg(long, global = true)]
    pub dry_run: bool,
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

    let metrics_layer =
        archinstall_zfs_core::metrics::MetricsLayer::open("/tmp/archinstall-metrics.jsonl")
            .wrap_err("failed to open metrics file")?;

    tracing_subscriber::registry()
        .with(channel_layer.with_filter(ui_filter))
        .with(file_layer)
        .with(metrics_layer)
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

    app::run(cli, ui_log_rx).await
}
