mod install;
mod tracing_layer;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{bail, Result};
use slint::{ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::GlobalConfig;

slint::include_modules!();

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
        }) => {
            return archinstall_zfs_core::iso::render_profile(
                profile_dir,
                out_dir,
                kernel,
                zfs,
                headers,
                *fast,
            );
        }
        None => {}
    }

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
        let runner = archinstall_zfs_core::system::cmd::RealRunner;
        return install::run_install(&runner, &config);
    }

    run_gui(config)
}

fn run_gui(config: GlobalConfig) -> Result<()> {
    let app = App::new().unwrap();

    refresh_config_items(&app, &config);
    app.set_status_text(SharedString::from("Click an item to edit"));

    // ── Item activated (user clicks a config field) ───
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_item_activated(move |key| {
            let Some(app) = weak.upgrade() else { return };
            let key = key.to_string();
            handle_item_activated(&app, &key, &cfg);
        });
    }

    // ── Select confirmed ──────────────────────────────
    {
        let weak = app.as_weak();
        app.on_select_confirmed(move |_key, _idx| {
            let Some(_app) = weak.upgrade() else { return };
            // TODO: update config based on key+idx, refresh items
        });
    }

    // ── Install requested ─────────────────────────────
    {
        let weak = app.as_weak();
        app.on_install_requested(move || {
            let Some(app) = weak.upgrade() else { return };
            // TODO: validate config, start install
            app.set_install_state(1);
        });
    }

    // ── Quit ──────────────────────────────────────────
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

fn refresh_config_items(app: &App, config: &GlobalConfig) {
    let items: Vec<ConfigItem> = build_config_items(config);
    let model = ModelRc::new(VecModel::from(items));
    app.set_config_items(model);
}

fn build_config_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    vec![
        config_item("installation_mode", "Installation mode",
            &c.installation_mode.map(|m| m.to_string()).unwrap_or("Not set".into()), 1),
        config_item("disk_by_id", "Disk",
            &c.disk_by_id.as_ref().map(|p| p.display().to_string()).unwrap_or("Not set".into()), 0),
        config_item("pool_name", "Pool name",
            &c.pool_name.clone().unwrap_or("Not set".into()), 0),
        config_item("dataset_prefix", "Dataset prefix", &c.dataset_prefix, 0),
        config_item("encryption", "Encryption", &c.zfs_encryption_mode.to_string(), 1),
        config_item("compression", "Compression", &c.compression.to_string(), 1),
        config_item("swap_mode", "Swap", &c.swap_mode.to_string(), 1),
        separator(),
        config_item("init_system", "Init system", &c.init_system.to_string(), 1),
        config_item("zfs_module_mode", "ZFS module", &c.zfs_module_mode.to_string(), 1),
        config_item("hostname", "Hostname",
            &c.hostname.clone().unwrap_or("Not set".into()), 0),
        config_item("locale", "Locale",
            &c.locale.clone().unwrap_or("Not set".into()), 0),
        config_item("timezone", "Timezone",
            &c.timezone.clone().unwrap_or("Not set".into()), 0),
        config_item("keyboard", "Keyboard layout", &c.keyboard_layout, 0),
        config_item("ntp", "NTP (time sync)",
            if c.ntp { "Enabled" } else { "Disabled" }, 3),
        separator(),
        config_item("root_password", "Root password",
            if c.root_password.is_some() { "Set" } else { "Not set" }, 2),
        config_item("profile", "Profile",
            &c.profile.clone().unwrap_or("Not set".into()), 1),
        config_item("audio", "Audio",
            &c.audio.map(|a| a.to_string()).unwrap_or("None".into()), 1),
        config_item("bluetooth", "Bluetooth",
            if c.bluetooth { "Enabled" } else { "Disabled" }, 3),
        {
            let pkgs = if c.additional_packages.is_empty() { "None".to_string() } else { c.additional_packages.join(", ") };
            config_item("additional_packages", "Additional packages", &pkgs, 0)
        },
        config_item("zrepl", "zrepl (snapshots)",
            if c.zrepl_enabled { "Enabled" } else { "Disabled" }, 3),
        separator(),
        action_item("install", "Install"),
        action_item("quit", "Quit"),
    ]
}

fn config_item(key: &str, label: &str, value: &str, item_type: i32) -> ConfigItem {
    ConfigItem {
        key: SharedString::from(key),
        label: SharedString::from(label),
        value: SharedString::from(value),
        item_type,
    }
}

fn separator() -> ConfigItem {
    ConfigItem {
        key: SharedString::default(),
        label: SharedString::default(),
        value: SharedString::default(),
        item_type: 4,
    }
}

fn action_item(key: &str, label: &str) -> ConfigItem {
    ConfigItem {
        key: SharedString::from(key),
        label: SharedString::from(label),
        value: SharedString::default(),
        item_type: 5,
    }
}

fn handle_item_activated(app: &App, key: &str, _config: &GlobalConfig) {
    let (title, options, current) = match key {
        "installation_mode" => (
            "Installation Mode",
            vec!["Full Disk", "New Pool", "Existing Pool"],
            0,
        ),
        "encryption" => (
            "Encryption",
            vec!["No encryption", "Encrypt entire pool", "Encrypt base dataset only"],
            0,
        ),
        "compression" => (
            "Compression",
            vec!["lz4", "zstd", "zstd-5", "zstd-10", "off"],
            0,
        ),
        "swap_mode" => (
            "Swap Mode",
            vec!["None", "ZRAM", "Swap partition", "Swap partition (encrypted)"],
            0,
        ),
        "init_system" => (
            "Init System",
            vec!["dracut", "mkinitcpio"],
            0,
        ),
        "zfs_module_mode" => (
            "ZFS Module",
            vec!["precompiled", "dkms"],
            0,
        ),
        "profile" => (
            "Profile",
            vec![
                "None", "gnome", "plasma", "xfce", "sway", "hyprland", "i3",
                "budgie", "cinnamon", "mate", "lxqt", "deepin", "minimal",
            ],
            0,
        ),
        "audio" => (
            "Audio",
            vec!["None", "pipewire", "pulseaudio"],
            0,
        ),
        // Toggle items
        "ntp" | "bluetooth" | "zrepl" => {
            // Handled differently — toggle inline
            // TODO: implement toggle + refresh
            return;
        }
        // Text/password items
        _ => {
            // TODO: implement text input popup
            return;
        }
    };

    let select_options: Vec<SelectOption> = options
        .iter()
        .map(|s| SelectOption {
            text: SharedString::from(*s),
        })
        .collect();

    app.set_select_title(SharedString::from(title));
    app.set_select_options(ModelRc::new(VecModel::from(select_options)));
    app.set_select_index(current);
    app.set_select_key(SharedString::from(key));
    app.set_select_visible(true);
}
