use color_eyre::eyre::{bail, Result};

use crate::config::types::GlobalConfig;
use crate::Cli;

#[derive(Debug)]
pub enum AppState {
    Menu,
    Installing,
    Done,
}

pub fn run(cli: Cli) -> Result<()> {
    let config = if let Some(ref path) = cli.config {
        tracing::info!(path = %path.display(), "loading config from file");
        GlobalConfig::load_from_file(path)?
    } else {
        GlobalConfig::default()
    };

    if cli.silent {
        if cli.config.is_none() {
            bail!("--silent requires --config");
        }
        let errors = config.validate_for_install();
        if !errors.is_empty() {
            bail!("Config validation failed:\n  {}", errors.join("\n  "));
        }
        tracing::info!("silent mode: config valid, starting installation");
        // TODO: run installation pipeline directly
        return Ok(());
    }

    // Interactive TUI mode
    crate::tui::run_tui(config, cli.dry_run)
}
