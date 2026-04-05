mod install;
mod tracing_layer;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, bail};
use slint::{Model, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::{
    AudioServer, CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, SwapMode,
    ZfsEncryptionMode, ZfsModuleMode,
};

slint::include_modules!();

const MAX_LOG_LINES: usize = 2000;
const TOTAL_STEPS: usize = 7;

// ── Step definitions ────────────────────────────────

const STEP_LABELS: [&str; TOTAL_STEPS] = [
    "Welcome", "Disk", "ZFS", "System", "Users", "Desktop", "Review",
];

#[derive(Parser, Debug)]
#[command(
    name = "archinstall-zfs",
    about = "Arch Linux installer with ZFS support (Slint UI)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[arg(long, global = true)]
    silent: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    RenderProfile {
        #[arg(long)]
        profile_dir: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long, default_value = "linux-lts")]
        kernel: String,
        #[arg(long, default_value = "precompiled")]
        zfs: String,
        #[arg(long, default_value = "auto")]
        headers: String,
        #[arg(long)]
        fast: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

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
        None => {
            let config = if let Some(ref path) = cli.config {
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
                let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
                    Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
                install::run_install(runner, &config, None)
            } else {
                run_gui(config)
            }
        }
    }
}

// ── Wizard state ────────────────────────────────────

struct WizardState {
    current_step: usize,
    max_visited: usize,
}

impl WizardState {
    fn new() -> Self {
        Self {
            current_step: 0,
            max_visited: 0,
        }
    }

    fn go_to(&mut self, step: usize) {
        if step < TOTAL_STEPS {
            self.current_step = step;
            if step > self.max_visited {
                self.max_visited = step;
            }
        }
    }

    fn next(&mut self) {
        if self.current_step < TOTAL_STEPS - 1 {
            self.go_to(self.current_step + 1);
        }
    }

    fn prev(&mut self) {
        if self.current_step > 0 {
            self.current_step -= 1;
        }
    }
}

fn run_gui(config: GlobalConfig) -> Result<()> {
    let app = App::new()?;
    let config = Rc::new(RefCell::new(config));
    let wizard = Rc::new(RefCell::new(WizardState::new()));

    app.set_total_steps(TOTAL_STEPS as i32);
    refresh_ui(&app, &config.borrow(), &wizard.borrow());

    // ── Item activated ───────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_item_activated(move |key| {
            let Some(app) = weak.upgrade() else { return };

            // Handle inline radio option clicks: "radio:{group_key}:{index}"
            if let Some(rest) = key.strip_prefix("radio:") {
                if let Some((group_key, idx_str)) = rest.rsplit_once(':')
                    && let Ok(idx) = idx_str.parse::<i32>()
                {
                    let mut c = cfg.borrow_mut();
                    apply_radio(&mut c, group_key, idx);
                    refresh_ui(&app, &c, &wiz.borrow());
                }
                return;
            }

            handle_item_activated(&app, &key, &cfg.borrow(), &wiz.borrow());
        });
    }

    // ── Toggle activated ─────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_toggle_activated(move |key| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            match key.as_str() {
                "ntp" => c.ntp = !c.ntp,
                "bluetooth" => c.bluetooth = !c.bluetooth,
                "zrepl" => c.zrepl_enabled = !c.zrepl_enabled,
                _ => return,
            }
            refresh_ui(&app, &c, &wiz.borrow());
        });
    }

    // ── Select confirmed ─────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_select_confirmed(move |key, idx| {
            let Some(app) = weak.upgrade() else { return };

            if key == "timezone_region" {
                let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
                if let Some(&region) = regions.get(idx as usize) {
                    let cities =
                        archinstall_zfs_core::installer::locale::list_timezone_cities(region);
                    let city_strs: Vec<&str> = cities.iter().map(|s| s.as_str()).collect();
                    let tz_key = format!("timezone_city:{region}");
                    show_select(&app, &tz_key, &format!("{region} /"), &city_strs, 0);
                }
                return;
            }

            if key.starts_with("timezone_city:") {
                let region = key.strip_prefix("timezone_city:").unwrap();
                let cities = archinstall_zfs_core::installer::locale::list_timezone_cities(region);
                if let Some(city) = cities.get(idx as usize) {
                    cfg.borrow_mut().timezone = Some(format!("{region}/{city}"));
                    refresh_ui(&app, &cfg.borrow(), &wiz.borrow());
                }
                return;
            }

            if key == "kernel_select" {
                let kernels = archinstall_zfs_core::kernel::AVAILABLE_KERNELS;
                if let Some(info) = kernels.get(idx as usize) {
                    cfg.borrow_mut().kernels = Some(vec![info.name.to_string()]);
                    refresh_ui(&app, &cfg.borrow(), &wiz.borrow());
                }
                return;
            }

            if key == "locale_select" {
                let locales = archinstall_zfs_core::installer::locale::list_locales();
                if let Some(loc) = locales.get(idx as usize) {
                    cfg.borrow_mut().locale = Some(loc.clone());
                    refresh_ui(&app, &cfg.borrow(), &wiz.borrow());
                }
                return;
            }

            let mut c = cfg.borrow_mut();
            apply_select(&mut c, &key, idx);
            refresh_ui(&app, &c, &wiz.borrow());
        });
    }

    // ── Text confirmed ───────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_text_confirmed(move |key, val| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            apply_text(&mut c, &key, &val);
            refresh_ui(&app, &c, &wiz.borrow());
        });
    }

    // ── Step navigation ──────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_next_step(move || {
            let Some(app) = weak.upgrade() else { return };
            wiz.borrow_mut().next();
            refresh_ui(&app, &cfg.borrow(), &wiz.borrow());
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_prev_step(move || {
            let Some(app) = weak.upgrade() else { return };
            wiz.borrow_mut().prev();
            refresh_ui(&app, &cfg.borrow(), &wiz.borrow());
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_step_clicked(move |idx| {
            let Some(app) = weak.upgrade() else { return };
            let mut w = wiz.borrow_mut();
            if (idx as usize) <= w.max_visited {
                w.go_to(idx as usize);
                refresh_ui(&app, &cfg.borrow(), &w);
            }
        });
    }

    // ── Install requested ────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_install_requested(move || {
            let Some(app) = weak.upgrade() else { return };
            let c = cfg.borrow().clone();

            let errors = c.validate_for_install();
            if !errors.is_empty() {
                app.set_status_text(SharedString::from(format!("Validation: {}", errors[0])));
                return;
            }

            app.set_install_state(1);
            app.set_log_messages(ModelRc::new(VecModel::<LogMessage>::default()));

            let (log_tx, log_rx) = crossbeam_channel::bounded::<(String, i32)>(512);

            let weak_log = app.as_weak();
            thread::spawn(move || {
                while let Ok((text, level)) = log_rx.recv() {
                    // Extract phase info from log messages like "[INFO ] Phase 4: Installing..."
                    let phase_update = if text.contains("Phase ") {
                        // Parse "Phase N:" or "Phase N-M:"
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
                    let _ = weak_log.upgrade_in_event_loop(move |app| {
                        // Update phase if detected
                        if let Some((phase_num, label)) = phase_update {
                            app.set_install_phase(phase_num);
                            app.set_install_phase_label(SharedString::from(&label));
                        }

                        let model = app.get_log_messages();
                        let vec_model = model
                            .as_any()
                            .downcast_ref::<VecModel<LogMessage>>()
                            .expect("log_messages model is always VecModel<LogMessage>");
                        vec_model.push(LogMessage { text, level });
                        if vec_model.row_count() > MAX_LOG_LINES {
                            let to_remove =
                                vec_model.row_count() - MAX_LOG_LINES + MAX_LOG_LINES / 4;
                            for _ in 0..to_remove {
                                vec_model.remove(0);
                            }
                        }
                    });
                }
            });

            // Download progress channel
            let (download_tx, download_rx) = tokio::sync::watch::channel(
                archinstall_zfs_core::system::async_download::PackageProgress::default(),
            );
            let download_tx = Arc::new(download_tx);

            // Download progress consumer thread — polls every 100ms
            let weak_dl = app.as_weak();
            thread::spawn(move || {
                let mut rx = download_rx;
                loop {
                    thread::sleep(std::time::Duration::from_millis(100));

                    // Check if sender is gone
                    match rx.has_changed() {
                        Err(_) => {
                            let _ = weak_dl.upgrade_in_event_loop(|app| {
                                app.set_download_active(false);
                            });
                            break;
                        }
                        Ok(false) => continue,
                        Ok(true) => {}
                    }

                    use archinstall_zfs_core::system::async_download::{
                        PackageProgress, PackageState,
                    };

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
                                    || (completed + failed < packages.len()
                                        && !packages.is_empty()));

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

                            let _ = weak_dl.upgrade_in_event_loop(move |app| {
                                app.set_download_active(is_active);
                                app.set_download_pct(pct);
                                app.set_download_status(SharedString::from(&status));
                                app.set_download_items(ModelRc::new(VecModel::from(dl_items)));
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
                            let _ = weak_dl.upgrade_in_event_loop(move |app| {
                                app.set_download_active(true);
                                app.set_download_pct(pct);
                                app.set_download_status(SharedString::from(&status));
                                app.set_download_items(ModelRc::default());
                            });
                        }
                        PackageProgress::Done => {
                            let _ = weak_dl.upgrade_in_event_loop(|app| {
                                app.set_download_active(false);
                            });
                        }
                    }
                }
            });

            let weak_install = app.as_weak();
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

                let subscriber = tracing_subscriber::registry()
                    .with(layer.with_filter(ui_filter))
                    .with(file_layer);
                let _guard = tracing::subscriber::set_default(subscriber);

                let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
                    Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
                let result = install::run_install(runner, &c, Some(download_tx));

                let state = if result.is_ok() { 2 } else { 3 };
                let _ = weak_install.upgrade_in_event_loop(move |app| {
                    app.set_install_state(state);
                });
            });
        });
    }

    // ── Keyboard navigation ────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_key_nav_down(move || {
            let Some(app) = weak.upgrade() else { return };
            let items = build_step_items(wiz.borrow().current_step, &cfg.borrow());
            let current = app.get_focused_index();
            let next = next_selectable_index(&items, current, 1);
            app.set_focused_index(next);
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_key_nav_up(move || {
            let Some(app) = weak.upgrade() else { return };
            let items = build_step_items(wiz.borrow().current_step, &cfg.borrow());
            let current = app.get_focused_index();
            let next = next_selectable_index(&items, current, -1);
            app.set_focused_index(next);
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let wiz = wizard.clone();
        app.on_key_nav_activate(move || {
            let Some(app) = weak.upgrade() else { return };
            let idx = app.get_focused_index();
            let items = build_step_items(wiz.borrow().current_step, &cfg.borrow());
            if idx < 0 || idx as usize >= items.len() {
                return;
            }
            let item = &items[idx as usize];
            let item_type = item.item_type;
            let key = item.key.clone();
            if item_type == 5 {
                if key == "install" {
                    app.invoke_install_requested();
                } else if key == "quit" {
                    let _ = app.window().hide();
                }
            } else if item_type == 3 {
                app.invoke_toggle_activated(key);
            } else if item_type != 4 && item_type != 6 && item_type != 7 && item_type != 8 {
                app.invoke_item_activated(key);
            }
        });
    }

    // ── Select filter changed (fuzzy search for locale etc.) ──
    {
        let weak = app.as_weak();
        app.on_select_filter_changed(move |key, filter_text| {
            let Some(app) = weak.upgrade() else { return };
            let filter = filter_text.to_lowercase();

            if key == "locale_select" {
                let all_locales = archinstall_zfs_core::installer::locale::list_locales();
                let filtered: Vec<SelectOption> = if filter.is_empty() {
                    all_locales
                        .iter()
                        .map(|s| SelectOption {
                            text: SharedString::from(s.as_str()),
                        })
                        .collect()
                } else {
                    all_locales
                        .iter()
                        .filter(|s| s.to_lowercase().contains(&filter))
                        .map(|s| SelectOption {
                            text: SharedString::from(s.as_str()),
                        })
                        .collect()
                };
                app.set_select_options(ModelRc::new(VecModel::from(filtered)));
                app.set_select_index(-1);
            }
        });
    }

    // ── Text input edited (password strength) ───────
    {
        let weak = app.as_weak();
        app.on_text_input_edited(move |key, value| {
            let Some(app) = weak.upgrade() else { return };

            if key == "root_password" || key == "encryption_password" {
                if value.is_empty() {
                    app.set_password_strength_score(-1);
                    return;
                }
                let entropy = zxcvbn::zxcvbn(value.as_str(), &[]);
                let score = u8::from(entropy.score());
                let theme = app.global::<Theme>().get_c();
                let (label, color) = match score {
                    0 => ("Very weak", theme.red),
                    1 => ("Weak", theme.red),
                    2 => ("Fair", theme.yellow),
                    3 => ("Strong", theme.green),
                    _ => ("Very strong", theme.teal),
                };
                app.set_password_strength_score(score as i32);
                app.set_password_strength_label(SharedString::from(label));
                app.set_password_strength_color(color);
            }
        });
    }

    // ── Quit ─────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_quit_requested(move || {
            if let Some(app) = weak.upgrade() {
                let should_reboot = app.get_install_state() == 2;
                let _ = app.window().hide();
                if should_reboot {
                    let _ = std::process::Command::new("systemctl")
                        .arg("reboot")
                        .spawn();
                }
            }
        });
    }

    app.run()?;
    Ok(())
}

// ── UI refresh ──────────────────────────────────────

fn refresh_ui(app: &App, config: &GlobalConfig, wizard: &WizardState) {
    let items = build_step_items(wizard.current_step, config);
    app.set_current_step(wizard.current_step as i32);
    app.set_steps(ModelRc::new(VecModel::from(build_steps(wizard))));
    // Reset focused index to first selectable item
    let first = next_selectable_index(&items, -1, 1);
    app.set_focused_index(first);
    app.set_config_items(ModelRc::new(VecModel::from(items)));
    app.set_status_text(SharedString::default());
}

fn build_steps(wizard: &WizardState) -> Vec<StepInfo> {
    STEP_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let state = if i == wizard.current_step {
                1 // current
            } else if i <= wizard.max_visited {
                2 // done/visited
            } else {
                0 // pending
            };
            StepInfo {
                label: SharedString::from(*label),
                state,
            }
        })
        .collect()
}

// ── Per-step item building ──────────────────────────

fn build_step_items(step: usize, c: &GlobalConfig) -> Vec<ConfigItem> {
    match step {
        0 => build_welcome_items(c),
        1 => build_disk_items(c),
        2 => build_zfs_items(c),
        3 => build_system_items(c),
        4 => build_users_items(c),
        5 => build_desktop_items(c),
        6 => build_review_items(c),
        _ => vec![],
    }
}

fn build_welcome_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    radio_group(
        "installation_mode",
        "Installation mode",
        &["Full Disk", "New Pool", "Existing Pool"],
        match c.installation_mode {
            Some(InstallationMode::FullDisk) => 0,
            Some(InstallationMode::NewPool) => 1,
            Some(InstallationMode::ExistingPool) => 2,
            None => -1,
        },
    )
}

fn build_disk_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mode = c.installation_mode;
    let mut items = Vec::new();

    if matches!(mode, Some(InstallationMode::FullDisk) | None) {
        let disks = archinstall_zfs_core::disk::by_id::list_disks_by_id().unwrap_or_default();
        let disk_strs: Vec<String> = disks.iter().map(|p| p.display().to_string()).collect();
        let disk_refs: Vec<&str> = disk_strs.iter().map(|s| s.as_str()).collect();
        let selected = c
            .disk_by_id
            .as_ref()
            .and_then(|sel| disks.iter().position(|d| d == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_group("disk_by_id", "Disk", &disk_refs, selected));
    }

    if matches!(
        mode,
        Some(InstallationMode::NewPool) | Some(InstallationMode::ExistingPool)
    ) {
        let parts = archinstall_zfs_core::disk::by_id::list_partitions_by_id().unwrap_or_default();
        let part_strs: Vec<String> = parts.iter().map(|p| p.display().to_string()).collect();
        let part_refs: Vec<&str> = part_strs.iter().map(|s| s.as_str()).collect();

        let efi_selected = c
            .efi_partition_by_id
            .as_ref()
            .and_then(|sel| parts.iter().position(|p| p == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_group(
            "efi_partition",
            "EFI partition",
            &part_refs,
            efi_selected,
        ));

        if matches!(mode, Some(InstallationMode::NewPool)) {
            let zfs_selected = c
                .zfs_partition_by_id
                .as_ref()
                .and_then(|sel| parts.iter().position(|p| p == sel))
                .map(|i| i as i32)
                .unwrap_or(-1);
            items.extend(radio_group(
                "zfs_partition",
                "ZFS partition",
                &part_refs,
                zfs_selected,
            ));
        }
    }

    items
}

fn build_zfs_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mode = c.installation_mode;
    let has_swap_partition = matches!(
        c.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );

    let mut items = vec![
        ci(
            "pool_name",
            "Pool name",
            &c.pool_name.clone().unwrap_or("Not set".into()),
            0,
        ),
        ci("dataset_prefix", "Dataset prefix", &c.dataset_prefix, 0),
    ];

    items.extend(radio_group(
        "compression",
        "Compression",
        &["lz4", "zstd", "zstd-5", "zstd-10", "off"],
        match c.compression {
            CompressionAlgo::Lz4 => 0,
            CompressionAlgo::Zstd => 1,
            CompressionAlgo::Zstd5 => 2,
            CompressionAlgo::Zstd10 => 3,
            CompressionAlgo::Off => 4,
        },
    ));

    items.extend(radio_group(
        "encryption",
        "Encryption",
        &[
            "No encryption",
            "Encrypt entire pool",
            "Encrypt base dataset only",
        ],
        match c.zfs_encryption_mode {
            ZfsEncryptionMode::None => 0,
            ZfsEncryptionMode::Pool => 1,
            ZfsEncryptionMode::Dataset => 2,
        },
    ));

    if c.zfs_encryption_mode != ZfsEncryptionMode::None {
        items.push(ci(
            "encryption_password",
            "Encryption password",
            if c.zfs_encryption_password.is_some() {
                "Set"
            } else {
                "Not set"
            },
            2,
        ));
    }

    items.extend(radio_group(
        "swap_mode",
        "Swap",
        &[
            "None",
            "ZRAM",
            "Swap partition",
            "Swap partition (encrypted)",
        ],
        match c.swap_mode {
            SwapMode::None => 0,
            SwapMode::Zram => 1,
            SwapMode::ZswapPartition => 2,
            SwapMode::ZswapPartitionEncrypted => 3,
        },
    ));

    if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
        items.push(ci(
            "swap_partition_size",
            "Swap size",
            &c.swap_partition_size.clone().unwrap_or("Not set".into()),
            0,
        ));
    }
    if !matches!(mode, Some(InstallationMode::FullDisk) | None) && has_swap_partition {
        let parts = archinstall_zfs_core::disk::by_id::list_partitions_by_id().unwrap_or_default();
        let part_strs: Vec<String> = parts.iter().map(|p| p.display().to_string()).collect();
        let part_refs: Vec<&str> = part_strs.iter().map(|s| s.as_str()).collect();
        let swap_selected = c
            .swap_partition_by_id
            .as_ref()
            .and_then(|sel| parts.iter().position(|p| p == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_group(
            "swap_partition",
            "Swap partition",
            &part_refs,
            swap_selected,
        ));
    }

    items.extend(radio_group(
        "init_system",
        "Init system",
        &["dracut", "mkinitcpio"],
        match c.init_system {
            InitSystem::Dracut => 0,
            InitSystem::Mkinitcpio => 1,
        },
    ));

    items.extend(radio_group(
        "zfs_module_mode",
        "ZFS module",
        &["precompiled", "dkms"],
        match c.zfs_module_mode {
            ZfsModuleMode::Precompiled => 0,
            ZfsModuleMode::Dkms => 1,
        },
    ));

    items
}

fn build_system_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = vec![
        ci(
            "kernel",
            "Kernel",
            &c.kernels
                .as_ref()
                .map(|k| k.join(", "))
                .unwrap_or_else(|| c.primary_kernel().to_string()),
            1,
        ),
        ci(
            "hostname",
            "Hostname",
            &c.hostname.clone().unwrap_or("Not set".into()),
            0,
        ),
        ci(
            "locale",
            "Locale",
            &c.locale.clone().unwrap_or("Not set".into()),
            1,
        ),
        ci(
            "timezone",
            "Timezone",
            &c.timezone.clone().unwrap_or("Not set".into()),
            1,
        ),
        ci("keyboard", "Keyboard layout", &c.keyboard_layout, 0),
        ci(
            "ntp",
            "NTP (time sync)",
            if c.ntp { "Enabled" } else { "Disabled" },
            3,
        ),
    ];

    let dl_options: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
    let dl_refs: Vec<&str> = dl_options.iter().map(|s| s.as_str()).collect();
    items.extend(radio_group(
        "parallel_downloads",
        "Parallel downloads",
        &dl_refs,
        (c.parallel_downloads as i32) - 1,
    ));

    items
}

fn build_users_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    vec![
        ci(
            "root_password",
            "Root password",
            if c.root_password.is_some() {
                "Set"
            } else {
                "Not set"
            },
            2,
        ),
        // Users are shown as read-only summary (no user management popup in slint yet)
        ci(
            "users",
            "User accounts",
            &match &c.users {
                Some(users) if !users.is_empty() => users
                    .iter()
                    .map(|u| u.username.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => "None".into(),
            },
            0, // text input for now
        ),
    ]
}

fn build_desktop_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let profiles = archinstall_zfs_core::profile::all_profiles();
    let mut profile_names: Vec<String> = vec!["None".to_string()];
    profile_names.extend(profiles.iter().map(|p| p.name.to_string()));
    let profile_refs: Vec<&str> = profile_names.iter().map(|s| s.as_str()).collect();
    let profile_selected = c
        .profile
        .as_ref()
        .and_then(|sel| profiles.iter().position(|p| p.name == *sel))
        .map(|i| (i + 1) as i32) // +1 because "None" is at index 0
        .unwrap_or(0);

    let mut items = radio_group("profile", "Profile", &profile_refs, profile_selected);

    items.extend(radio_group(
        "audio",
        "Audio",
        &["None", "pipewire", "pulseaudio"],
        match c.audio {
            None => 0,
            Some(AudioServer::Pipewire) => 1,
            Some(AudioServer::Pulseaudio) => 2,
        },
    ));

    items.extend([
        ci(
            "bluetooth",
            "Bluetooth",
            if c.bluetooth { "Enabled" } else { "Disabled" },
            3,
        ),
        ci(
            "additional_packages",
            "Additional packages",
            &if c.additional_packages.is_empty() {
                "None".to_string()
            } else {
                c.additional_packages.join(", ")
            },
            0,
        ),
        ci(
            "aur_packages",
            "AUR packages",
            &if c.aur_packages.is_empty() {
                "None".to_string()
            } else {
                c.aur_packages.join(", ")
            },
            0,
        ),
        ci(
            "extra_services",
            "Extra services",
            &if c.extra_services.is_empty() {
                "None".to_string()
            } else {
                c.extra_services.join(", ")
            },
            0,
        ),
        ci(
            "zrepl",
            "zrepl (snapshots)",
            if c.zrepl_enabled {
                "Enabled"
            } else {
                "Disabled"
            },
            3,
        ),
    ]);

    items
}

fn build_review_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = Vec::new();

    // Collect summary from all steps
    for (step, &label) in STEP_LABELS.iter().enumerate().take(TOTAL_STEPS - 1) {
        // Section header
        items.push(ConfigItem {
            key: SharedString::default(),
            label: label.into(),
            value: SharedString::default(),
            item_type: 4, // separator used as section label
        });

        let step_items = build_step_items(step, c);
        let mut i = 0;
        while i < step_items.len() {
            let item = &step_items[i];
            if item.item_type == 8 {
                // Radio header: find the selected option and show as "Header: Selected"
                let header_label = item.label.clone();
                let mut selected_label: SharedString = "Not set".into();
                i += 1;
                while i < step_items.len() && step_items[i].item_type == 9 {
                    if step_items[i].value == "selected" {
                        selected_label = step_items[i].label.clone();
                    }
                    i += 1;
                }
                items.push(ConfigItem {
                    key: SharedString::default(),
                    label: header_label,
                    value: selected_label,
                    item_type: 6,
                });
            } else {
                items.push(ConfigItem {
                    key: item.key.clone(),
                    label: item.label.clone(),
                    value: item.value.clone(),
                    item_type: 6,
                });
                i += 1;
            }
        }
    }

    // Validation errors
    let errors = c.validate_for_install();
    if !errors.is_empty() {
        items.push(sep());
        for error in &errors {
            items.push(ConfigItem {
                key: SharedString::default(),
                label: SharedString::default(),
                value: error.as_str().into(),
                item_type: 7, // warning
            });
        }
    }

    items.push(sep());
    items.push(ConfigItem {
        key: "install".into(),
        label: "Install".into(),
        value: SharedString::default(),
        item_type: 5,
    });
    items.push(ConfigItem {
        key: "quit".into(),
        label: "Quit".into(),
        value: SharedString::default(),
        item_type: 5,
    });

    items
}

fn ci(key: &str, label: &str, value: &str, item_type: i32) -> ConfigItem {
    ConfigItem {
        key: key.into(),
        label: label.into(),
        value: value.into(),
        item_type,
    }
}

fn sep() -> ConfigItem {
    ConfigItem {
        key: SharedString::default(),
        label: SharedString::default(),
        value: SharedString::default(),
        item_type: 4,
    }
}

/// Emit a radio group: a header (item_type 8) followed by clickable options (item_type 9).
/// `key` is the logical group key (e.g. "compression").
/// `selected` is the currently selected index.
fn radio_group(key: &str, label: &str, options: &[&str], selected: i32) -> Vec<ConfigItem> {
    let mut items = vec![ConfigItem {
        key: SharedString::default(),
        label: label.into(),
        value: SharedString::default(),
        item_type: 8, // radio header
    }];
    for (i, opt) in options.iter().enumerate() {
        items.push(ConfigItem {
            key: format!("radio:{key}:{i}").into(),
            label: (*opt).into(),
            value: if i as i32 == selected {
                "selected".into()
            } else {
                SharedString::default()
            },
            item_type: 9, // radio option
        });
    }
    items
}

// ── Item activation (open popup) ─────────────────────

fn handle_item_activated(app: &App, key: &str, config: &GlobalConfig, _wizard: &WizardState) {
    match key {
        // Popup selects — only for items with too many options or async scan
        "kernel" => {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let results = rt.block_on(archinstall_zfs_core::kernel::scanner::scan_all_kernels());
            let mut options = Vec::new();
            for (info, result) in archinstall_zfs_core::kernel::AVAILABLE_KERNELS
                .iter()
                .zip(&results)
            {
                let compat = if result.precompiled_compatible || result.dkms_compatible {
                    "OK"
                } else {
                    "INCOMPATIBLE"
                };
                let ver = result.kernel_version.as_deref().unwrap_or("?");
                options.push(format!("{} ({ver}) [{compat}]", info.display_name));
            }
            let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
            let current_kernel = config.primary_kernel();
            let current_idx = archinstall_zfs_core::kernel::AVAILABLE_KERNELS
                .iter()
                .position(|k| k.name == current_kernel)
                .unwrap_or(0);
            show_select(
                app,
                "kernel_select",
                "Kernel",
                &opt_refs,
                current_idx as i32,
            );
        }
        "timezone" => {
            let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
            show_select(app, "timezone_region", "Timezone region", &regions, 0);
        }
        "locale" => {
            let locales = archinstall_zfs_core::installer::locale::list_locales();
            let locale_strs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
            let current_idx = config
                .locale
                .as_ref()
                .and_then(|l| locales.iter().position(|x| x == l))
                .map(|i| i as i32)
                .unwrap_or(0);
            show_select_with_filter(
                app,
                "locale_select",
                "Locale (type to filter)",
                &locale_strs,
                current_idx,
                true,
            );
        }
        // Text input popups
        "pool_name"
        | "dataset_prefix"
        | "hostname"
        | "keyboard"
        | "additional_packages"
        | "aur_packages"
        | "extra_services"
        | "swap_partition_size"
        | "users" => {
            let current = match key {
                "pool_name" => config.pool_name.clone().unwrap_or_default(),
                "dataset_prefix" => config.dataset_prefix.clone(),
                "hostname" => config.hostname.clone().unwrap_or_default(),
                "keyboard" => config.keyboard_layout.clone(),
                "additional_packages" => config.additional_packages.join(" "),
                "aur_packages" => config.aur_packages.join(" "),
                "extra_services" => config.extra_services.join(" "),
                "swap_partition_size" => config.swap_partition_size.clone().unwrap_or_default(),
                "users" => config
                    .users
                    .as_ref()
                    .map(|u| {
                        u.iter()
                            .map(|u| u.username.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            };
            show_text_input(app, key, key, &current, false);
        }
        "root_password" | "encryption_password" => {
            show_text_input(app, key, key, "", true);
        }
        _ => {}
    }
}

fn show_select(app: &App, key: &str, title: &str, options: &[&str], current: i32) {
    show_select_with_filter(app, key, title, options, current, false);
}

fn show_select_with_filter(
    app: &App,
    key: &str,
    title: &str,
    options: &[&str],
    current: i32,
    filterable: bool,
) {
    let opts: Vec<SelectOption> = options
        .iter()
        .map(|s| SelectOption {
            text: SharedString::from(*s),
        })
        .collect();
    app.set_select_key(key.into());
    app.set_select_title(title.into());
    app.set_select_options(ModelRc::new(VecModel::from(opts)));
    app.set_select_index(current);
    app.set_select_show_filter(filterable);
    app.set_select_visible(true);
}

fn show_text_input(app: &App, key: &str, title: &str, current: &str, password: bool) {
    app.set_text_input_key(key.into());
    app.set_text_input_title(title.into());
    app.set_text_input_value(current.into());
    app.set_text_input_password(password);
    app.set_password_strength_score(-1);
    app.set_text_input_visible(true);
}

/// Find the next selectable item index, skipping separators (4), readonly (6), warnings (7).
fn next_selectable_index(items: &[ConfigItem], current: i32, dir: i32) -> i32 {
    let len = items.len() as i32;
    if len == 0 {
        return -1;
    }
    for offset in 1..=len {
        let idx = ((current + dir * offset) % len + len) % len;
        let t = items[idx as usize].item_type;
        // Skip separator (4), readonly (6), warning (7), radio-header (8)
        if t != 4 && t != 6 && t != 7 && t != 8 {
            return idx;
        }
    }
    current
}

// ── Apply mutations ──────────────────────────────────

fn apply_select(config: &mut GlobalConfig, key: &str, idx: i32) {
    // Fallback for any popup selects not handled explicitly in on_select_confirmed.
    // Most selects are now inline radio groups handled by apply_radio.
    apply_radio(config, key, idx);
}

/// Apply an inline radio selection. `group_key` is e.g. "compression", `idx` is the option index.
fn apply_radio(config: &mut GlobalConfig, group_key: &str, idx: i32) {
    match group_key {
        "installation_mode" => {
            let new_mode = match idx {
                0 => InstallationMode::FullDisk,
                1 => InstallationMode::NewPool,
                _ => InstallationMode::ExistingPool,
            };
            if config.installation_mode != Some(new_mode) {
                config.disk_by_id = None;
                config.efi_partition_by_id = None;
                config.zfs_partition_by_id = None;
                config.swap_partition_by_id = None;
            }
            config.installation_mode = Some(new_mode);
        }
        "disk_by_id" => {
            if let Ok(disks) = archinstall_zfs_core::disk::by_id::list_disks_by_id()
                && let Some(path) = disks.get(idx as usize)
            {
                config.disk_by_id = Some(path.clone());
            }
        }
        "efi_partition" => {
            if let Ok(parts) = archinstall_zfs_core::disk::by_id::list_partitions_by_id()
                && let Some(path) = parts.get(idx as usize)
            {
                config.efi_partition_by_id = Some(path.clone());
            }
        }
        "zfs_partition" => {
            if let Ok(parts) = archinstall_zfs_core::disk::by_id::list_partitions_by_id()
                && let Some(path) = parts.get(idx as usize)
            {
                config.zfs_partition_by_id = Some(path.clone());
            }
        }
        "swap_partition" => {
            if let Ok(parts) = archinstall_zfs_core::disk::by_id::list_partitions_by_id()
                && let Some(path) = parts.get(idx as usize)
            {
                config.swap_partition_by_id = Some(path.clone());
            }
        }
        "compression" => {
            config.compression = match idx {
                0 => CompressionAlgo::Lz4,
                1 => CompressionAlgo::Zstd,
                2 => CompressionAlgo::Zstd5,
                3 => CompressionAlgo::Zstd10,
                _ => CompressionAlgo::Off,
            }
        }
        "encryption" => {
            config.zfs_encryption_mode = match idx {
                0 => ZfsEncryptionMode::None,
                1 => ZfsEncryptionMode::Pool,
                _ => ZfsEncryptionMode::Dataset,
            };
            if config.zfs_encryption_mode == ZfsEncryptionMode::None {
                config.zfs_encryption_password = None;
            }
        }
        "swap_mode" => {
            config.swap_mode = match idx {
                0 => SwapMode::None,
                1 => SwapMode::Zram,
                2 => SwapMode::ZswapPartition,
                _ => SwapMode::ZswapPartitionEncrypted,
            }
        }
        "init_system" => {
            config.init_system = match idx {
                0 => InitSystem::Dracut,
                _ => InitSystem::Mkinitcpio,
            }
        }
        "zfs_module_mode" => {
            config.zfs_module_mode = match idx {
                0 => ZfsModuleMode::Precompiled,
                _ => ZfsModuleMode::Dkms,
            }
        }
        "profile" => {
            let profiles = archinstall_zfs_core::profile::all_profiles();
            config.profile = if idx == 0 {
                None
            } else {
                profiles.get((idx - 1) as usize).map(|p| p.name.to_string())
            };
        }
        "audio" => {
            config.audio = match idx {
                0 => None,
                1 => Some(AudioServer::Pipewire),
                _ => Some(AudioServer::Pulseaudio),
            }
        }
        "parallel_downloads" => {
            config.parallel_downloads = (idx + 1) as u32;
        }
        _ => {}
    }
}

fn apply_text(config: &mut GlobalConfig, key: &str, val: &str) {
    let opt = if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    };
    match key {
        "pool_name" => config.pool_name = opt,
        "dataset_prefix" => {
            if !val.is_empty() {
                config.dataset_prefix = val.to_string();
            }
        }
        "hostname" => config.hostname = opt,
        "locale" => config.locale = opt,
        "timezone" => config.timezone = opt,
        "keyboard" => {
            if !val.is_empty() {
                config.keyboard_layout = val.to_string();
            }
        }
        "root_password" => config.root_password = opt,
        "encryption_password" => config.zfs_encryption_password = opt,
        "swap_partition_size" => config.swap_partition_size = opt,
        "additional_packages" => {
            config.additional_packages = val
                .split_whitespace()
                .map(|s| s.trim_matches(',').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        "aur_packages" => {
            config.aur_packages = val
                .split_whitespace()
                .map(|s| s.trim_matches(',').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        "extra_services" => {
            config.extra_services = val
                .split_whitespace()
                .map(|s| s.trim_matches(',').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        _ => {}
    }
}

// ── Download progress helpers ───────────────────────

fn format_speed(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.1} MB/s", bps as f64 / 1_000_000.0)
    } else if bps >= 1_000 {
        format!("{:.0} KB/s", bps as f64 / 1_000.0)
    } else if bps > 0 {
        format!("{bps} B/s")
    } else {
        "-- B/s".to_string()
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        match s.char_indices().nth(max) {
            Some((idx, _)) => &s[..idx],
            None => s,
        }
    }
}
