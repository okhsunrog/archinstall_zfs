mod config_items;
mod controllers;
mod editing_models;
mod format;
mod install;
mod refresh;
mod tracing_layer;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use clap::Parser;
use color_eyre::eyre::Result;
use slint::ComponentHandle;

use archinstall_zfs_core::config::types::GlobalConfig;

pub mod ui {
    slint::include_modules!();
}
use ui::*;

use refresh::refresh_items;

#[derive(Parser, Debug)]
#[command(
    name = "archinstall-zfs",
    about = "Arch Linux installer with ZFS support (Slint UI)"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Path to a JSON secrets file merged into the configuration
    #[arg(long, global = true)]
    secrets: Option<PathBuf>,

    #[arg(long, global = true)]
    silent: bool,

    /// UI scale factor for the GUI (e.g. 1.5, 2.0). On linuxkms this maps to
    /// the SLINT_SCALE_FACTOR env var since the backend cannot auto-detect
    /// physical DPI; on desktop builds the OS value is used unless overridden.
    #[arg(long, global = true)]
    ui_scale: Option<f32>,

    /// Run the full UI and hardware backends while disabling installation and
    /// destructive storage operations.
    #[arg(long, global = true)]
    demo: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let demo = cli.demo || kernel_cmdline_enables_demo();

    if let Some(scale) = cli.ui_scale
        && scale > 0.0
    {
        // Must be set before any Slint window is created.
        // SAFETY: single-threaded at this point in startup.
        unsafe {
            std::env::set_var("SLINT_SCALE_FACTOR", scale.to_string());
        }
    }

    let mut config = if let Some(ref path) = cli.config {
        GlobalConfig::load_from_file(path)?
    } else {
        GlobalConfig::default()
    };
    if let Some(ref path) = cli.secrets {
        config.apply_secrets_from_file(path)?;
    }

    if cli.silent {
        use color_eyre::eyre::bail;
        if demo {
            bail!("--silent is unavailable in safe demo mode");
        }
        if cli.config.is_none() {
            bail!("--silent requires --config");
        }
        let errors = config.validate_for_install();
        if !errors.is_empty() {
            bail!("Config validation failed:\n  {}", errors.join("\n  "));
        }
        let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
            Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
        install::run_install(
            runner,
            &config,
            tokio_util::sync::CancellationToken::new(),
            None,
        )
    } else {
        run_gui(config, demo)
    }
}

fn run_gui(config: GlobalConfig, demo: bool) -> Result<()> {
    let app = App::new()?;
    let config = Rc::new(RefCell::new(config));
    let kernel_scan = controllers::welcome::KernelScan::new();

    let models = editing_models::EditingModels::new();
    models.attach(&app);
    models.seed(&config.borrow());

    refresh_items(&app, &config.borrow());

    controllers::welcome::setup(&app, &config, &kernel_scan, demo);
    controllers::lists::setup(&app, &config, &models);
    controllers::wizard::setup(&app, &config, &kernel_scan);
    controllers::install::setup(&app, &config, demo);
    controllers::wifi::setup(&app);
    controllers::quit::setup(&app);

    let demo_session = demo.then(controllers::demo::DemoSession::new);
    if let Some(session) = &demo_session {
        controllers::demo::setup(&app, &config, session);
    }

    app.run()?;
    if let Some(session) = demo_session {
        session.export_all_blocking();
    }
    Ok(())
}

fn kernel_cmdline_enables_demo() -> bool {
    cmdline_enables_demo(&std::fs::read_to_string("/proc/cmdline").unwrap_or_default())
}

fn cmdline_enables_demo(cmdline: &str) -> bool {
    cmdline
        .split_whitespace()
        .any(|arg| matches!(arg, "archinstall_zfs.demo" | "archinstall_zfs.demo=1"))
}

#[cfg(test)]
mod tests {
    use super::cmdline_enables_demo;

    #[test]
    fn demo_kernel_argument_is_detected() {
        assert!(cmdline_enables_demo(
            "quiet archinstall_zfs.demo=1 console=tty1"
        ));
        assert!(cmdline_enables_demo("archinstall_zfs.demo"));
        assert!(!cmdline_enables_demo("archinstall_zfs.demo=0"));
    }
}
