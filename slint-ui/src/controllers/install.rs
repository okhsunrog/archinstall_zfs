//! Install pipeline controller: validate config, spawn the install thread,
//! pump tracing log lines and download progress into the InstallState global.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::GlobalConfig;
use archinstall_zfs_core::system::async_download::{PackageProgress, PackageState};

use crate::format::{format_duration, format_speed, truncate_str};
use crate::install;
use crate::tracing_layer;
use crate::ui::{App, DownloadInfo, InstallState, LogMessage, WizardState};

const MAX_LOG_LINES: usize = 2000;

pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.on_install_requested(move || {
        let Some(app) = weak.upgrade() else { return };
        let c = cfg.borrow().clone();

        let errors = c.validate_for_install();
        if !errors.is_empty() {
            app.global::<WizardState>()
                .set_status_text(SharedString::from(format!("Validation: {}", errors[0])));
            return;
        }

        app.global::<InstallState>().set_state(1);
        app.global::<InstallState>()
            .set_log_messages(ModelRc::new(VecModel::<LogMessage>::default()));

        let (log_tx, log_rx) = crossbeam_channel::bounded::<(String, i32)>(512);
        spawn_log_pump(&app, log_rx);

        // Download progress channel
        let (download_tx, download_rx) = tokio::sync::watch::channel(PackageProgress::default());
        let download_tx = Arc::new(download_tx);
        spawn_download_pump(&app, download_rx);

        spawn_install_thread(&app, c, log_tx, download_tx);
    });
}

fn spawn_log_pump(app: &App, log_rx: crossbeam_channel::Receiver<(String, i32)>) {
    let weak = app.as_weak();
    thread::spawn(move || {
        while let Ok((text, level)) = log_rx.recv() {
            // Extract phase info from log messages like "[INFO ] Phase 4: Installing..."
            let phase_update = if text.contains("Phase ") {
                let after_phase = text.split("Phase ").nth(1).unwrap_or("");
                let num_str: String = after_phase
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                let phase_num = num_str.parse::<i32>().unwrap_or(-1);
                let label: String = after_phase
                    .split(": ")
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if phase_num >= 0 && !label.is_empty() {
                    Some((phase_num, label))
                } else {
                    None
                }
            } else {
                None
            };

            let text = SharedString::from(&text);
            let _ = weak.upgrade_in_event_loop(move |app| {
                if let Some((phase_num, label)) = phase_update {
                    app.global::<InstallState>().set_phase(phase_num);
                    app.global::<InstallState>()
                        .set_phase_label(SharedString::from(&label));
                }

                let model = app.global::<InstallState>().get_log_messages();
                let vec_model = model
                    .as_any()
                    .downcast_ref::<VecModel<LogMessage>>()
                    .expect("log_messages model is always VecModel<LogMessage>");
                vec_model.push(LogMessage { text, level });
                if vec_model.row_count() > MAX_LOG_LINES {
                    let to_remove = vec_model.row_count() - MAX_LOG_LINES + MAX_LOG_LINES / 4;
                    for _ in 0..to_remove {
                        vec_model.remove(0);
                    }
                }
            });
        }
    });
}

fn spawn_download_pump(app: &App, mut rx: tokio::sync::watch::Receiver<PackageProgress>) {
    let weak = app.as_weak();
    thread::spawn(move || {
        loop {
            thread::sleep(std::time::Duration::from_millis(100));

            // Check if sender is gone
            match rx.has_changed() {
                Err(_) => {
                    let _ = weak.upgrade_in_event_loop(|app| {
                        app.global::<InstallState>().set_download_active(false);
                    });
                    break;
                }
                Ok(false) => continue,
                Ok(true) => {}
            }

            let progress = rx.borrow_and_update().clone();

            match progress {
                PackageProgress::Downloading {
                    ref packages,
                    total_bytes,
                    downloaded_bytes,
                    active_downloads,
                    completed,
                    failed,
                } => {
                    let is_active = total_bytes > 0
                        && (active_downloads > 0
                            || (completed + failed < packages.len() && !packages.is_empty()));

                    let pct = if total_bytes > 0 {
                        (downloaded_bytes as f64 / total_bytes as f64 * 100.0) as i32
                    } else {
                        0
                    };

                    let speed = progress.total_speed_bps();
                    let speed_str = format_speed(speed);
                    let eta_str = progress
                        .eta()
                        .map(format_duration)
                        .unwrap_or_else(|| "--:--".to_string());
                    let status = format!(
                        "Downloads {}/{} | {} | ETA {}",
                        completed,
                        packages.len(),
                        speed_str,
                        eta_str,
                    );

                    let mut dl_items = Vec::new();
                    for pkg in packages {
                        match pkg {
                            PackageState::Downloading {
                                filename,
                                downloaded,
                                total,
                                speed_bps,
                                ..
                            } => {
                                let pkg_pct = if *total > 0 {
                                    (*downloaded as f64 / *total as f64 * 100.0) as i32
                                } else {
                                    0
                                };
                                dl_items.push(DownloadInfo {
                                    filename: truncate_str(filename, 30).into(),
                                    pct: pkg_pct,
                                    speed: format_speed(*speed_bps).into(),
                                    state: 0,
                                });
                            }
                            PackageState::Verifying { filename } => {
                                dl_items.push(DownloadInfo {
                                    filename: truncate_str(filename, 30).into(),
                                    pct: 100,
                                    speed: SharedString::default(),
                                    state: 1,
                                });
                            }
                            _ => {}
                        }
                    }

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let s = app.global::<InstallState>();
                        s.set_download_active(is_active);
                        s.set_download_pct(pct);
                        s.set_download_status(SharedString::from(&status));
                        s.set_download_items(ModelRc::new(VecModel::from(dl_items)));
                    });
                }
                PackageProgress::Installing {
                    package,
                    current,
                    total,
                    percent,
                } => {
                    let status = format!("Installing {current}/{total}: {package}");
                    let pct = percent as i32;
                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let s = app.global::<InstallState>();
                        s.set_download_active(true);
                        s.set_download_pct(pct);
                        s.set_download_status(SharedString::from(&status));
                        s.set_download_items(ModelRc::default());
                    });
                }
                PackageProgress::Done => {
                    let _ = weak.upgrade_in_event_loop(|app| {
                        app.global::<InstallState>().set_download_active(false);
                    });
                }
            }
        }
    });
}

fn spawn_install_thread(
    app: &App,
    config: GlobalConfig,
    log_tx: crossbeam_channel::Sender<(String, i32)>,
    download_tx: Arc<tokio::sync::watch::Sender<PackageProgress>>,
) {
    let weak = app.as_weak();
    tokio::task::spawn_blocking(move || {
        use tracing_subscriber::Layer as _;
        use tracing_subscriber::layer::SubscriberExt as _;

        let layer = tracing_layer::UiLogLayer::new(log_tx);
        let ui_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

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
                .expect("failed to open metrics file");

        let subscriber = tracing_subscriber::registry()
            .with(layer.with_filter(ui_filter))
            .with(file_layer)
            .with(metrics_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
            Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
        let result = install::run_install(runner, &config, Some(download_tx));

        let state = if result.is_ok() { 2 } else { 3 };
        let _ = weak.upgrade_in_event_loop(move |app| {
            app.global::<InstallState>().set_state(state);
        });
    });
}
