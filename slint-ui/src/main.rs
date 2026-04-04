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

fn main() -> Result<()> {
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
                install::run_install(runner, &config)
            } else {
                run_gui(config)
            }
        }
    }
}

fn run_gui(config: GlobalConfig) -> Result<()> {
    let app = App::new().unwrap();
    let config = Rc::new(RefCell::new(config));

    refresh_config_items(&app, &config.borrow());
    app.set_status_text("Click an item to edit".into());

    // ── Item activated ───────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_item_activated(move |key| {
            let Some(app) = weak.upgrade() else { return };
            handle_item_activated(&app, &key, &cfg.borrow());
        });
    }

    // ── Toggle activated ─────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_toggle_activated(move |key| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            match key.as_str() {
                "ntp" => c.ntp = !c.ntp,
                "bluetooth" => c.bluetooth = !c.bluetooth,
                "zrepl" => c.zrepl_enabled = !c.zrepl_enabled,
                _ => return,
            }
            refresh_config_items(&app, &c);
        });
    }

    // ── Select confirmed ─────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_select_confirmed(move |key, idx| {
            let Some(app) = weak.upgrade() else { return };

            // Timezone two-step: region selected -> show cities
            if key == "timezone_region" {
                let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
                if let Some(&region) = regions.get(idx as usize) {
                    let cities =
                        archinstall_zfs_core::installer::locale::list_timezone_cities(region);
                    let city_strs: Vec<&str> = cities.iter().map(|s| s.as_str()).collect();
                    // Store region for the next callback via the select_key
                    let tz_key = format!("timezone_city:{region}");
                    show_select(&app, &tz_key, &format!("{region} /"), &city_strs, 0);
                }
                return;
            }

            // Timezone two-step: city selected -> set timezone
            if key.starts_with("timezone_city:") {
                let region = key.strip_prefix("timezone_city:").unwrap();
                let cities = archinstall_zfs_core::installer::locale::list_timezone_cities(region);
                if let Some(city) = cities.get(idx as usize) {
                    cfg.borrow_mut().timezone = Some(format!("{region}/{city}"));
                    refresh_config_items(&app, &cfg.borrow());
                }
                return;
            }

            // Disk select
            if key == "disk_select" {
                if let Ok(disks) = archinstall_zfs_core::disk::by_id::list_disks_by_id()
                    && let Some(disk) = disks.get(idx as usize)
                {
                    cfg.borrow_mut().disk_by_id = Some(disk.clone());
                    refresh_config_items(&app, &cfg.borrow());
                }
                return;
            }

            // Kernel select
            if key == "kernel_select" {
                let kernels = archinstall_zfs_core::kernel::AVAILABLE_KERNELS;
                if let Some(info) = kernels.get(idx as usize) {
                    cfg.borrow_mut().kernels = Some(vec![info.name.to_string()]);
                    refresh_config_items(&app, &cfg.borrow());
                }
                return;
            }

            // Profile select
            if key == "profile_select" {
                let profiles = archinstall_zfs_core::profile::all_profiles();
                cfg.borrow_mut().profile = if idx == 0 {
                    None
                } else {
                    profiles.get((idx - 1) as usize).map(|p| p.name.to_string())
                };
                refresh_config_items(&app, &cfg.borrow());
                return;
            }

            // Locale select
            if key == "locale_select" {
                let locales = archinstall_zfs_core::installer::locale::list_locales();
                if let Some(loc) = locales.get(idx as usize) {
                    cfg.borrow_mut().locale = Some(loc.clone());
                    refresh_config_items(&app, &cfg.borrow());
                }
                return;
            }

            let mut c = cfg.borrow_mut();
            apply_select(&mut c, &key, idx);
            refresh_config_items(&app, &c);
        });
    }

    // ── Text confirmed ───────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_text_confirmed(move |key, val| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            apply_text(&mut c, &key, &val);
            refresh_config_items(&app, &c);
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

            // Set up tracing channel
            let (log_tx, log_rx) = crossbeam_channel::bounded::<(String, i32)>(512);

            // Spawn log consumer thread
            let weak_log = app.as_weak();
            thread::spawn(move || {
                while let Ok((text, level)) = log_rx.recv() {
                    let text = SharedString::from(&text);
                    let _ = weak_log.upgrade_in_event_loop(move |app| {
                        let model = app.get_log_messages();
                        let vec_model = model
                            .as_any()
                            .downcast_ref::<VecModel<LogMessage>>()
                            .unwrap();
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

            // Spawn install thread with tracing layer
            let weak_install = app.as_weak();
            thread::spawn(move || {
                use tracing_subscriber::layer::SubscriberExt;

                let layer = tracing_layer::UiLogLayer::new(log_tx);
                let filter = tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("trace"));

                let file_appender = tracing_appender::rolling::never("/tmp", "archinstall-zfs.log");
                let file_layer = tracing_subscriber::fmt::layer()
                    .with_writer(file_appender)
                    .with_ansi(false)
                    .with_target(true);

                let subscriber = tracing_subscriber::registry()
                    .with(filter)
                    .with(file_layer)
                    .with(layer);
                let _guard = tracing::subscriber::set_default(subscriber);

                let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
                    Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
                let result = install::run_install(runner, &c);

                let state = if result.is_ok() { 2 } else { 3 };
                let _ = weak_install.upgrade_in_event_loop(move |app| {
                    app.set_install_state(state);
                });
            });
        });
    }

    // ── Quit ─────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_quit_requested(move || {
            if let Some(app) = weak.upgrade() {
                let _ = app.window().hide();
            }
        });
    }

    app.run().unwrap();
    Ok(())
}

// ── Config item building ─────────────────────────────

fn refresh_config_items(app: &App, config: &GlobalConfig) {
    app.set_config_items(ModelRc::new(VecModel::from(build_config_items(config))));
}

fn build_config_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let pkg_str = if c.additional_packages.is_empty() {
        "None".to_string()
    } else {
        c.additional_packages.join(", ")
    };
    let aur_str = if c.aur_packages.is_empty() {
        "None".to_string()
    } else {
        c.aur_packages.join(", ")
    };

    vec![
        ci(
            "installation_mode",
            "Installation mode",
            &c.installation_mode
                .map(|m| m.to_string())
                .unwrap_or("Not set".into()),
            1,
        ),
        ci(
            "disk_by_id",
            "Disk",
            &c.disk_by_id
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            1,
        ),
        ci(
            "pool_name",
            "Pool name",
            &c.pool_name.clone().unwrap_or("Not set".into()),
            0,
        ),
        ci("dataset_prefix", "Dataset prefix", &c.dataset_prefix, 0),
        ci(
            "encryption",
            "Encryption",
            &c.zfs_encryption_mode.to_string(),
            1,
        ),
        ci("compression", "Compression", &c.compression.to_string(), 1),
        ci("swap_mode", "Swap", &c.swap_mode.to_string(), 1),
        sep(),
        ci("init_system", "Init system", &c.init_system.to_string(), 1),
        ci(
            "zfs_module_mode",
            "ZFS module",
            &c.zfs_module_mode.to_string(),
            1,
        ),
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
        sep(),
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
        ci(
            "profile",
            "Profile",
            &c.profile.clone().unwrap_or("Not set".into()),
            1,
        ),
        ci(
            "audio",
            "Audio",
            &c.audio.map(|a| a.to_string()).unwrap_or("None".into()),
            1,
        ),
        ci(
            "bluetooth",
            "Bluetooth",
            if c.bluetooth { "Enabled" } else { "Disabled" },
            3,
        ),
        ci("additional_packages", "Additional packages", &pkg_str, 0),
        ci("aur_packages", "AUR packages", &aur_str, 0),
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
        sep(),
        ConfigItem {
            key: "install".into(),
            label: "Install".into(),
            value: SharedString::default(),
            item_type: 5,
        },
        ConfigItem {
            key: "quit".into(),
            label: "Quit".into(),
            value: SharedString::default(),
            item_type: 5,
        },
    ]
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

// ── Item activation (open popup) ─────────────────────

fn handle_item_activated(app: &App, key: &str, config: &GlobalConfig) {
    match key {
        // Select items
        "installation_mode" => show_select(
            app,
            key,
            "Installation Mode",
            &["Full Disk", "New Pool", "Existing Pool"],
            match config.installation_mode {
                Some(InstallationMode::FullDisk) => 0,
                Some(InstallationMode::NewPool) => 1,
                Some(InstallationMode::ExistingPool) => 2,
                None => 0,
            },
        ),
        "encryption" => show_select(
            app,
            key,
            "Encryption",
            &[
                "No encryption",
                "Encrypt entire pool",
                "Encrypt base dataset only",
            ],
            match config.zfs_encryption_mode {
                ZfsEncryptionMode::None => 0,
                ZfsEncryptionMode::Pool => 1,
                ZfsEncryptionMode::Dataset => 2,
            },
        ),
        "compression" => show_select(
            app,
            key,
            "Compression",
            &["lz4", "zstd", "zstd-5", "zstd-10", "off"],
            match config.compression {
                CompressionAlgo::Lz4 => 0,
                CompressionAlgo::Zstd => 1,
                CompressionAlgo::Zstd5 => 2,
                CompressionAlgo::Zstd10 => 3,
                CompressionAlgo::Off => 4,
            },
        ),
        "swap_mode" => show_select(
            app,
            key,
            "Swap Mode",
            &[
                "None",
                "ZRAM",
                "Swap partition",
                "Swap partition (encrypted)",
            ],
            match config.swap_mode {
                SwapMode::None => 0,
                SwapMode::Zram => 1,
                SwapMode::ZswapPartition => 2,
                SwapMode::ZswapPartitionEncrypted => 3,
            },
        ),
        "init_system" => show_select(
            app,
            key,
            "Init System",
            &["dracut", "mkinitcpio"],
            match config.init_system {
                InitSystem::Dracut => 0,
                InitSystem::Mkinitcpio => 1,
            },
        ),
        "zfs_module_mode" => show_select(
            app,
            key,
            "ZFS Module",
            &["precompiled", "dkms"],
            match config.zfs_module_mode {
                ZfsModuleMode::Precompiled => 0,
                ZfsModuleMode::Dkms => 1,
            },
        ),
        "kernel" => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
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
        "profile" => {
            let profiles = archinstall_zfs_core::profile::all_profiles();
            let mut names: Vec<String> = vec!["None".to_string()];
            names.extend(profiles.iter().map(|p| p.name.to_string()));
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            show_select(app, "profile_select", "Profile", &name_refs, 0);
        }
        "audio" => show_select(
            app,
            key,
            "Audio",
            &["None", "pipewire", "pulseaudio"],
            match config.audio {
                None => 0,
                Some(AudioServer::Pipewire) => 1,
                Some(AudioServer::Pulseaudio) => 2,
            },
        ),

        // Timezone: two-step select (region, then city)
        "timezone" => {
            let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
            show_select(app, "timezone_region", "Timezone region", &regions, 0);
        }

        // Locale: select from available UTF-8 locales
        "locale" => {
            let locales = archinstall_zfs_core::installer::locale::list_locales();
            let locale_strs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
            show_select(app, "locale_select", "Locale", &locale_strs, 0);
        }

        // Disk: select from /dev/disk/by-id/
        "disk_by_id" => {
            if let Ok(disks) = archinstall_zfs_core::disk::by_id::list_disks_by_id() {
                let disk_strs: Vec<String> =
                    disks.iter().map(|p| p.display().to_string()).collect();
                let disk_refs: Vec<&str> = disk_strs.iter().map(|s| s.as_str()).collect();
                show_select(app, "disk_select", "Select disk", &disk_refs, 0);
            }
        }

        // Text items
        "pool_name"
        | "dataset_prefix"
        | "hostname"
        | "keyboard"
        | "additional_packages"
        | "aur_packages" => {
            let current = match key {
                "pool_name" => config.pool_name.clone().unwrap_or_default(),
                "dataset_prefix" => config.dataset_prefix.clone(),
                "hostname" => config.hostname.clone().unwrap_or_default(),
                "keyboard" => config.keyboard_layout.clone(),
                "additional_packages" => config.additional_packages.join(" "),
                "aur_packages" => config.aur_packages.join(" "),
                _ => String::new(),
            };
            show_text_input(app, key, key, &current, false);
        }

        // Password
        "root_password" => {
            show_text_input(app, key, "Root password", "", true);
        }

        _ => {}
    }
}

fn show_select(app: &App, key: &str, title: &str, options: &[&str], current: i32) {
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
    app.set_select_visible(true);
}

fn show_text_input(app: &App, key: &str, title: &str, current: &str, password: bool) {
    app.set_text_input_key(key.into());
    app.set_text_input_title(title.into());
    app.set_text_input_value(current.into());
    app.set_text_input_password(password);
    app.set_text_input_visible(true);
}

// ── Apply mutations ──────────────────────────────────

fn apply_select(config: &mut GlobalConfig, key: &str, idx: i32) {
    match key {
        "installation_mode" => {
            config.installation_mode = Some(match idx {
                0 => InstallationMode::FullDisk,
                1 => InstallationMode::NewPool,
                _ => InstallationMode::ExistingPool,
            })
        }
        "encryption" => {
            config.zfs_encryption_mode = match idx {
                0 => ZfsEncryptionMode::None,
                1 => ZfsEncryptionMode::Pool,
                _ => ZfsEncryptionMode::Dataset,
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
            // profile handled via profile_select
        }
        "audio" => {
            config.audio = match idx {
                0 => None,
                1 => Some(AudioServer::Pipewire),
                _ => Some(AudioServer::Pulseaudio),
            }
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
        // disk_by_id handled via disk_select
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
        _ => {}
    }
}
