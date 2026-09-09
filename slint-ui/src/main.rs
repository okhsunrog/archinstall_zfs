mod completion;
mod config_items;
#[cfg(feature = "linuxkms")]
mod console_session;
mod controllers;
mod editing_models;
mod format;
#[cfg(feature = "linuxkms")]
mod installed_shell;
mod preview;
mod refresh;
mod tracing_layer;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};
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

    /// Review a simulated installation-ready system (desktop-mock builds only).
    #[arg(long, conflicts_with_all = ["demo", "silent", "config", "secrets"])]
    preview: Option<preview::Scene>,

    /// Physical preview window size, for example 1920x1080.
    #[arg(long, requires = "preview", default_value = "1280x800")]
    preview_size: preview::Size,
}

/// Install the process-wide tracing subscriber.
///
/// Global rather than per-install-thread: the pipeline does its work on tokio
/// worker threads and blocking-pool threads, and a thread-local subscriber is
/// invisible to all of them. Set as a thread default, everything the package
/// downloader, the AUR builds and the ZFSBootMenu step emitted was dropped —
/// missing from both the on-screen log and the log file.
fn setup_logging(ui_log_tx: crossbeam_channel::Sender<(String, i32)>) -> Result<()> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let ui_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let ui_layer = tracing_layer::UiLogLayer::new(ui_log_tx).with_filter(ui_filter);

    let file_appender = tracing_appender::rolling::never("/tmp", "archinstall-zfs.log");
    let file_filter = tracing_subscriber::EnvFilter::new(
        "trace,h2=warn,hyper=warn,reqwest=warn,rustls=warn,pacman=info",
    );
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_target(true)
        .with_filter(file_filter);

    let metrics_layer =
        archinstall_zfs_core::metrics::MetricsLayer::open("/tmp/archinstall-metrics.jsonl")
            .wrap_err("failed to open metrics file")?;

    tracing_subscriber::registry()
        .with(ui_layer)
        .with(file_layer)
        .with(metrics_layer)
        .init();

    Ok(())
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    #[cfg(feature = "linuxkms")]
    let resumed = if !cli.silent && cli.preview.is_none() {
        console_session::supervise()?
    } else {
        None
    };
    #[cfg(not(feature = "linuxkms"))]
    let resumed = None;
    if cli.preview.is_some() {
        color_eyre::eyre::ensure!(
            cfg!(feature = "desktop-mock"),
            "--preview requires a desktop-mock build"
        );
        preview::enable();
    }
    let demo = !preview::enabled()
        && (cli.demo || archinstall_zfs_core::demo::enabled_from_kernel_cmdline());

    // Bounded: the UI cannot keep up with trace-level output, and dropping
    // lines it will never render is preferable to slowing the installation.
    let (log_tx, log_rx) = crossbeam_channel::bounded::<(String, i32)>(512);
    setup_logging(log_tx)?;

    if let Some(scale) = cli.ui_scale {
        color_eyre::eyre::ensure!(
            scale.is_finite() && scale > 0.0,
            "UI scale must be finite and positive"
        );
        // Must be set before any Slint window is created.
        // SAFETY: single-threaded at this point in startup.
        unsafe {
            std::env::set_var("SLINT_SCALE_FACTOR", scale.to_string());
        }
    }

    let mut config = if preview::enabled() {
        preview::config(cli.preview.unwrap())
    } else if let Some(ref path) = cli.config {
        GlobalConfig::load_from_file(path)?
    } else {
        GlobalConfig::default()
    };
    if let Some(ref path) = cli.secrets {
        config.apply_secrets_from_file(path)?;
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            if cli.silent {
                use color_eyre::eyre::bail;
                if demo {
                    bail!("--silent is unavailable in safe demo mode");
                }
                if cli.config.is_none() {
                    bail!("--silent requires --config");
                }
                let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
                    Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
                Ok(archinstall_zfs_core::install::run_install(
                    runner,
                    config,
                    tokio_util::sync::CancellationToken::new(),
                    None,
                )
                .await?)
            } else {
                run_gui(config, demo, log_rx, cli.preview, cli.preview_size, resumed)
            }
        })
}

fn run_gui(
    config: GlobalConfig,
    demo: bool,
    log_rx: crossbeam_channel::Receiver<(String, i32)>,
    scene: Option<preview::Scene>,
    size: preview::Size,
    resumed: Option<completion::Completion>,
) -> Result<()> {
    let app = App::new()?;
    let completion = Arc::new(std::sync::Mutex::new(resumed.clone().unwrap_or_default()));
    controllers::quit::setup(&app, &completion);
    if let Some(resumed) = resumed {
        completion::show(&app, &resumed);
        // Resume only the result screen: no discovery, editable wizard or install callbacks.
        let log = std::fs::read_to_string("/tmp/archinstall-zfs.log").unwrap_or_default();
        let mut lines: Vec<_> = log
            .lines()
            .rev()
            .filter(|line| {
                line.contains(" INFO ") || line.contains(" WARN ") || line.contains("ERROR")
            })
            .take(2000)
            .collect();
        lines.reverse();
        let messages: Vec<_> = lines
            .into_iter()
            .map(|text| LogMessage {
                text: text.into(),
                level: 0,
            })
            .collect();
        app.global::<InstallState>()
            .set_log_messages(slint::ModelRc::new(slint::VecModel::from(messages)));
        app.run()?;
        return Ok(());
    }

    let config = Rc::new(RefCell::new(config));
    let kernel_scan = controllers::welcome::KernelScan::new();

    let models = editing_models::EditingModels::new();
    models.attach(&app);
    models.seed(&config.borrow());

    refresh_items(&app, &config.borrow());

    controllers::welcome::setup(&app, &config, &kernel_scan, demo);
    controllers::lists::setup(&app, &config, &models);
    controllers::wizard::setup(&app, &config, &kernel_scan);
    controllers::install::setup(&app, &config, demo, log_rx, &completion);
    controllers::wifi::setup(&app);

    let demo_session = demo.then(controllers::demo::DemoSession::new);
    if let Some(session) = &demo_session {
        controllers::demo::setup(&app, &config, session);
    }

    if let Some(scene) = scene {
        preview::show(&app, scene, size);
    }

    app.run()?;
    if let Some(session) = demo_session {
        session.export_all_blocking();
    }
    Ok(())
}
