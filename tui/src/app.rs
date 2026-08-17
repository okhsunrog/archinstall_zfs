use std::sync::Arc;

use color_eyre::eyre::{Result, bail};
use tokio_util::sync::CancellationToken;

use archinstall_zfs_core::config::types::GlobalConfig;
use archinstall_zfs_core::system::cmd::{CommandRunner, RealRunner};

use crate::Cli;

pub async fn run(
    cli: Cli,
    ui_log_rx: tokio::sync::mpsc::UnboundedReceiver<(String, i32)>,
) -> Result<()> {
    let mut config = if let Some(ref path) = cli.config {
        tracing::info!(path = %path.display(), "loading config from file");
        GlobalConfig::load_from_file(path)?
    } else {
        GlobalConfig::default()
    };
    if let Some(ref path) = cli.secrets {
        tracing::info!(path = %path.display(), "loading config secrets from file");
        config.apply_secrets_from_file(path)?;
    }

    let demo = cli.demo || archinstall_zfs_core::demo::enabled_from_kernel_cmdline();

    if cli.silent {
        if demo {
            bail!("--silent is unavailable in safe demo mode");
        }
        if cli.config.is_none() {
            bail!("--silent requires --config");
        }
        let errors = config.validate_for_install();
        if !errors.is_empty() {
            let rendered: Vec<String> = errors.iter().map(ToString::to_string).collect();
            bail!("Config validation failed:\n  {}", rendered.join("\n  "));
        }
        tracing::info!("silent mode: config valid, starting installation");
        let runner: Arc<dyn CommandRunner> = Arc::new(RealRunner);
        let cancel = CancellationToken::new();
        return archinstall_zfs_core::install::run_install(runner, config, cancel, None).await;
    }

    // Interactive TUI mode
    crate::tui::run_tui(config, demo, ui_log_rx).await
}
