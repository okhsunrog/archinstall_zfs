use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use archinstall_zfs_core::config::types::{
    AudioServer, CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, SwapMode, UserConfig,
    ZfsEncryptionMode, ZfsModuleMode,
};

use crate::tui::theme;
use crate::tui::Action;

use super::edit::run_edit;
use super::select::run_select;

// ── Menu item definition ──────────────────────────────

#[derive(Clone)]
enum MenuKind {
    /// Separator line (not selectable)
    Separator,
    /// Select from a list of options
    Select {
        options: Vec<&'static str>,
        current: usize,
    },
    /// Free-form text input
    Text,
    /// Masked text input (password)
    Password,
    /// Boolean toggle
    Toggle,
    /// Custom handler (disk, timezone, locale, profile — shows value)
    Custom,
    /// Action button (install, quit — no value shown)
    Action,
}

#[derive(Clone)]
struct MenuItem {
    key: &'static str,
    label: &'static str,
    value: String,
    kind: MenuKind,
}

impl MenuItem {
    fn is_selectable(&self) -> bool {
        !matches!(self.kind, MenuKind::Separator)
    }
}

// ── Main menu state ───────────────────────────────────

pub struct MainMenu {
    config: GlobalConfig,
    selected: usize,
    scroll_offset: usize,
}

impl MainMenu {
    pub fn new(config: GlobalConfig) -> Self {
        Self {
            config,
            selected: 0,
            scroll_offset: 0,
        }
    }

    pub fn into_config(self) -> GlobalConfig {
        self.config
    }

    fn items(&self) -> Vec<MenuItem> {
        let c = &self.config;
        let mode = c.installation_mode;
        let has_swap_partition = matches!(
            c.swap_mode,
            SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
        );

        let mut items = vec![
            // ── Storage & ZFS ──
            MenuItem {
                key: "installation_mode",
                label: "Installation mode",
                value: c
                    .installation_mode
                    .map(|m| m.to_string())
                    .unwrap_or("Not configured".into()),
                kind: MenuKind::Select {
                    options: vec!["Full Disk", "New Pool", "Existing Pool"],
                    current: match c.installation_mode {
                        Some(InstallationMode::FullDisk) => 0,
                        Some(InstallationMode::NewPool) => 1,
                        Some(InstallationMode::ExistingPool) => 2,
                        None => 0,
                    },
                },
            },
        ];

        // Show disk picker for FullDisk mode
        if matches!(mode, Some(InstallationMode::FullDisk) | None) {
            items.push(MenuItem {
                key: "disk_by_id",
                label: "Disk",
                value: c
                    .disk_by_id
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            });
        }

        // Show partition pickers for NewPool/ExistingPool
        if matches!(
            mode,
            Some(InstallationMode::NewPool) | Some(InstallationMode::ExistingPool)
        ) {
            items.push(MenuItem {
                key: "efi_partition",
                label: "EFI partition",
                value: c
                    .efi_partition_by_id
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            });
        }
        if matches!(mode, Some(InstallationMode::NewPool)) {
            items.push(MenuItem {
                key: "zfs_partition",
                label: "ZFS partition",
                value: c
                    .zfs_partition_by_id
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            });
        }

        items.push(MenuItem {
            key: "pool_name",
            label: "Pool name",
            value: c.pool_name.clone().unwrap_or("Not set".into()),
            kind: if matches!(mode, Some(InstallationMode::ExistingPool)) {
                MenuKind::Custom
            } else {
                MenuKind::Text
            },
        });
        items.push(MenuItem {
            key: "dataset_prefix",
            label: "Dataset prefix",
            value: c.dataset_prefix.clone(),
            kind: MenuKind::Text,
        });
        items.push(MenuItem {
            key: "encryption",
            label: "Encryption",
            value: c.zfs_encryption_mode.to_string(),
            kind: MenuKind::Select {
                options: vec![
                    "No encryption",
                    "Encrypt entire pool",
                    "Encrypt base dataset only",
                ],
                current: match c.zfs_encryption_mode {
                    ZfsEncryptionMode::None => 0,
                    ZfsEncryptionMode::Pool => 1,
                    ZfsEncryptionMode::Dataset => 2,
                },
            },
        });
        // Show encryption password status when encryption is enabled
        if c.zfs_encryption_mode != ZfsEncryptionMode::None {
            items.push(MenuItem {
                key: "encryption_password",
                label: "Encryption password",
                value: if c.zfs_encryption_password.is_some() {
                    "Set".into()
                } else {
                    "Not set".into()
                },
                kind: MenuKind::Password,
            });
        }
        items.push(MenuItem {
            key: "compression",
            label: "Compression",
            value: c.compression.to_string(),
            kind: MenuKind::Select {
                options: vec!["lz4", "zstd", "zstd-5", "zstd-10", "off"],
                current: match c.compression {
                    CompressionAlgo::Lz4 => 0,
                    CompressionAlgo::Zstd => 1,
                    CompressionAlgo::Zstd5 => 2,
                    CompressionAlgo::Zstd10 => 3,
                    CompressionAlgo::Off => 4,
                },
            },
        });
        items.push(MenuItem {
            key: "swap_mode",
            label: "Swap",
            value: c.swap_mode.to_string(),
            kind: MenuKind::Select {
                options: vec![
                    "None",
                    "ZRAM",
                    "Swap partition",
                    "Swap partition (encrypted)",
                ],
                current: match c.swap_mode {
                    SwapMode::None => 0,
                    SwapMode::Zram => 1,
                    SwapMode::ZswapPartition => 2,
                    SwapMode::ZswapPartitionEncrypted => 3,
                },
            },
        });

        // Swap partition size for FullDisk + ZSWAP modes
        if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
            items.push(MenuItem {
                key: "swap_partition_size",
                label: "Swap size",
                value: c.swap_partition_size.clone().unwrap_or("Not set".into()),
                kind: MenuKind::Text,
            });
        }
        // Swap partition picker for NewPool/ExistingPool + ZSWAP modes
        if !matches!(mode, Some(InstallationMode::FullDisk) | None) && has_swap_partition {
            items.push(MenuItem {
                key: "swap_partition",
                label: "Swap partition",
                value: c
                    .swap_partition_by_id
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            });
        }

        items.push(MenuItem {
            key: "sep1",
            label: "",
            value: String::new(),
            kind: MenuKind::Separator,
        });

        // ── System ──
        items.extend([
            MenuItem {
                key: "init_system",
                label: "Init system",
                value: c.init_system.to_string(),
                kind: MenuKind::Select {
                    options: vec!["dracut", "mkinitcpio"],
                    current: match c.init_system {
                        InitSystem::Dracut => 0,
                        InitSystem::Mkinitcpio => 1,
                    },
                },
            },
            MenuItem {
                key: "zfs_module_mode",
                label: "ZFS module",
                value: c.zfs_module_mode.to_string(),
                kind: MenuKind::Select {
                    options: vec!["precompiled", "dkms"],
                    current: match c.zfs_module_mode {
                        ZfsModuleMode::Precompiled => 0,
                        ZfsModuleMode::Dkms => 1,
                    },
                },
            },
            MenuItem {
                key: "kernel",
                label: "Kernel",
                value: c
                    .kernels
                    .as_ref()
                    .map(|k| k.join(", "))
                    .unwrap_or_else(|| c.primary_kernel().to_string()),
                kind: MenuKind::Custom,
            },
            MenuItem {
                key: "hostname",
                label: "Hostname",
                value: c.hostname.clone().unwrap_or("Not set".into()),
                kind: MenuKind::Text,
            },
            MenuItem {
                key: "locale",
                label: "Locale",
                value: c.locale.clone().unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            },
            MenuItem {
                key: "timezone",
                label: "Timezone",
                value: c.timezone.clone().unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            },
            MenuItem {
                key: "keyboard",
                label: "Keyboard layout",
                value: c.keyboard_layout.clone(),
                kind: MenuKind::Text,
            },
            MenuItem {
                key: "ntp",
                label: "NTP (time sync)",
                value: if c.ntp { "Enabled" } else { "Disabled" }.into(),
                kind: MenuKind::Toggle,
            },
            MenuItem {
                key: "network",
                label: "Network",
                value: if c.network_copy_iso {
                    "Copy from ISO"
                } else {
                    "Manual"
                }
                .into(),
                kind: MenuKind::Select {
                    options: vec!["Copy from ISO", "Manual"],
                    current: if c.network_copy_iso { 0 } else { 1 },
                },
            },
            MenuItem {
                key: "parallel_downloads",
                label: "Parallel downloads",
                value: c.parallel_downloads.to_string(),
                kind: MenuKind::Custom,
            },
            MenuItem {
                key: "sep2",
                label: "",
                value: String::new(),
                kind: MenuKind::Separator,
            },
        ]);

        // ── Auth & packages ──
        items.push(MenuItem {
            key: "root_password",
            label: "Root password",
            value: if c.root_password.is_some() {
                "Set".into()
            } else {
                "Not set".into()
            },
            kind: MenuKind::Password,
        });
        items.push(MenuItem {
            key: "users",
            label: "User accounts",
            value: match &c.users {
                Some(users) if !users.is_empty() => {
                    let names: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
                    names.join(", ")
                }
                _ => "None".into(),
            },
            kind: MenuKind::Custom,
        });
        items.extend([
            MenuItem {
                key: "profile",
                label: "Profile",
                value: c.profile.clone().unwrap_or("Not set".into()),
                kind: MenuKind::Custom,
            },
            MenuItem {
                key: "audio",
                label: "Audio",
                value: c.audio.map(|a| a.to_string()).unwrap_or("None".into()),
                kind: MenuKind::Select {
                    options: vec!["None", "pipewire", "pulseaudio"],
                    current: match c.audio {
                        None => 0,
                        Some(AudioServer::Pipewire) => 1,
                        Some(AudioServer::Pulseaudio) => 2,
                    },
                },
            },
            MenuItem {
                key: "bluetooth",
                label: "Bluetooth",
                value: if c.bluetooth { "Enabled" } else { "Disabled" }.into(),
                kind: MenuKind::Toggle,
            },
            MenuItem {
                key: "additional_packages",
                label: "Additional packages",
                value: if c.additional_packages.is_empty() {
                    "None".into()
                } else {
                    c.additional_packages.join(", ")
                },
                kind: MenuKind::Text,
            },
            MenuItem {
                key: "aur_packages",
                label: "AUR packages",
                value: if c.aur_packages.is_empty() {
                    "None".into()
                } else {
                    c.aur_packages.join(", ")
                },
                kind: MenuKind::Text,
            },
            MenuItem {
                key: "extra_services",
                label: "Extra services",
                value: if c.extra_services.is_empty() {
                    "None".into()
                } else {
                    c.extra_services.join(", ")
                },
                kind: MenuKind::Text,
            },
            MenuItem {
                key: "zrepl",
                label: "zrepl (snapshots)",
                value: if c.zrepl_enabled {
                    "Enabled"
                } else {
                    "Disabled"
                }
                .into(),
                kind: MenuKind::Toggle,
            },
            MenuItem {
                key: "sep3",
                label: "",
                value: String::new(),
                kind: MenuKind::Separator,
            },
            // ── Actions ──
            MenuItem {
                key: "save",
                label: "Save configuration",
                value: String::new(),
                kind: MenuKind::Action,
            },
            MenuItem {
                key: "install",
                label: "Install",
                value: String::new(),
                kind: MenuKind::Action,
            },
            MenuItem {
                key: "quit",
                label: "Quit",
                value: String::new(),
                kind: MenuKind::Action,
            },
        ]);

        items
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.items()
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_selectable())
            .map(|(i, _)| i)
            .collect()
    }

    fn move_up(&mut self) {
        let indices = self.selectable_indices();
        if let Some(pos) = indices.iter().position(|&i| i == self.selected) {
            let new_pos = if pos == 0 { indices.len() - 1 } else { pos - 1 };
            self.selected = indices[new_pos];
        }
    }

    fn move_down(&mut self) {
        let indices = self.selectable_indices();
        if let Some(pos) = indices.iter().position(|&i| i == self.selected) {
            let new_pos = if pos >= indices.len() - 1 { 0 } else { pos + 1 };
            self.selected = indices[new_pos];
        }
    }

    pub fn handle_event(
        &mut self,
        ev: Event,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Action> {
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    return Ok(Action::Quit);
                }
                (KeyCode::Up | KeyCode::Char('k'), _) => self.move_up(),
                (KeyCode::Down | KeyCode::Char('j'), _) => self.move_down(),
                (KeyCode::Enter | KeyCode::Right, _) => {
                    return self.activate_item(terminal);
                }
                (KeyCode::Home, _) => {
                    let indices = self.selectable_indices();
                    if let Some(&first) = indices.first() {
                        self.selected = first;
                    }
                }
                (KeyCode::End, _) => {
                    let indices = self.selectable_indices();
                    if let Some(&last) = indices.last() {
                        self.selected = last;
                    }
                }
                _ => {}
            }
        }
        Ok(Action::Continue)
    }

    fn activate_item(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Action> {
        let items = self.items();
        let item = &items[self.selected];
        let key = item.key;

        match &item.kind {
            MenuKind::Action => match key {
                "install" => {
                    // Pre-install validation
                    let errors = self.config.validate_for_install();
                    if !errors.is_empty() {
                        let msg = errors.join("\n");
                        let lines: Vec<&str> = msg.lines().collect();
                        let _ = run_select(terminal, "Validation errors", &lines, 0);
                        return Ok(Action::Continue);
                    }
                    return Ok(Action::Install);
                }
                "save" => {
                    let result = run_edit(terminal, "Save config to file", "config.json", false)?;
                    if let Some(path) = result.value {
                        if !path.is_empty() {
                            match self.config.save_to_file(std::path::Path::new(&path)) {
                                Ok(()) => {
                                    let _ = run_select(
                                        terminal,
                                        &format!("Saved to {path}"),
                                        &["OK"],
                                        0,
                                    );
                                }
                                Err(e) => {
                                    let msg = format!("Save failed: {e}");
                                    let _ = run_select(terminal, &msg, &["OK"], 0);
                                }
                            }
                        }
                    }
                }
                "quit" => return Ok(Action::Quit),
                _ => {}
            },
            MenuKind::Custom => match key {
                "timezone" => {
                    if let Some(tz) = self.pick_timezone(terminal)? {
                        self.config.timezone = Some(tz);
                    }
                }
                "locale" => {
                    if let Some(loc) = self.pick_locale(terminal)? {
                        self.config.locale = Some(loc);
                    }
                }
                "disk_by_id" => {
                    if let Some(disk) = self.pick_disk(terminal)? {
                        self.config.disk_by_id = Some(disk);
                    }
                }
                "efi_partition" => {
                    if let Some(part) = self.pick_partition(terminal, "EFI partition")? {
                        self.config.efi_partition_by_id = Some(part);
                    }
                }
                "zfs_partition" => {
                    if let Some(part) = self.pick_partition(terminal, "ZFS partition")? {
                        self.config.zfs_partition_by_id = Some(part);
                    }
                }
                "swap_partition" => {
                    if let Some(part) = self.pick_partition(terminal, "Swap partition")? {
                        self.config.swap_partition_by_id = Some(part);
                    }
                }
                "kernel" => {
                    if let Some(kernels) = self.pick_kernel(terminal)? {
                        self.config.kernels = Some(kernels);
                    }
                }
                "profile" => {
                    let profiles = archinstall_zfs_core::profile::all_profiles();
                    let mut names: Vec<&str> = vec!["None"];
                    names.extend(profiles.iter().map(|p| p.name));
                    let result = run_select(terminal, "Profile", &names, 0)?;
                    if let Some(idx) = result.selected {
                        self.config.profile = if idx == 0 {
                            None
                        } else {
                            Some(names[idx].to_string())
                        };
                    }
                }
                "users" => {
                    self.manage_users(terminal)?;
                }
                "parallel_downloads" => {
                    let options: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
                    let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
                    let current = (self.config.parallel_downloads as usize).saturating_sub(1);
                    let result = run_select(terminal, "Parallel downloads", &opt_refs, current)?;
                    if let Some(idx) = result.selected {
                        self.config.parallel_downloads = (idx + 1) as u32;
                    }
                }
                "pool_name" => {
                    // ExistingPool mode: discover pools, detect encryption, verify passphrase
                    self.pick_existing_pool(terminal)?;
                }
                _ => {}
            },
            MenuKind::Toggle => {
                self.apply_toggle(key);
            }
            MenuKind::Select { options, current } => {
                let result = run_select(terminal, item.label, options, *current)?;
                if let Some(idx) = result.selected {
                    self.apply_select(key, idx, terminal)?;
                }
            }
            MenuKind::Text => {
                let current = &item.value;
                let initial = if current == "Not set" || current == "None" {
                    ""
                } else {
                    current
                };
                let result = run_edit(terminal, item.label, initial, false)?;
                if let Some(val) = result.value {
                    self.apply_text(key, &val);
                }
            }
            MenuKind::Password => {
                let result = run_edit(terminal, item.label, "", true)?;
                if let Some(val) = result.value {
                    if !val.is_empty() {
                        self.apply_text(key, &val);
                    }
                }
            }
            MenuKind::Separator => {}
        }
        Ok(Action::Continue)
    }

    // ── Pickers ─────────────────────────────────────────

    fn pick_timezone(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Option<String>> {
        use archinstall_zfs_core::installer::locale;

        let regions = locale::list_timezone_regions();
        let region_strs: Vec<&str> = regions.iter().copied().collect();
        let result = run_select(terminal, "Timezone region", &region_strs, 0)?;
        let Some(region_idx) = result.selected else {
            return Ok(None);
        };
        let region = regions[region_idx];

        let cities = locale::list_timezone_cities(region);
        let city_strs: Vec<&str> = cities.iter().map(|s| s.as_str()).collect();
        let result = run_select(terminal, &format!("{region} /"), &city_strs, 0)?;
        let Some(city_idx) = result.selected else {
            return Ok(None);
        };

        Ok(Some(format!("{region}/{}", cities[city_idx])))
    }

    fn pick_disk(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Option<std::path::PathBuf>> {
        let disks = archinstall_zfs_core::disk::by_id::list_disks_by_id()?;
        if disks.is_empty() {
            return Ok(None);
        }
        let disk_strs: Vec<String> = disks.iter().map(|p| p.display().to_string()).collect();
        let disk_refs: Vec<&str> = disk_strs.iter().map(|s| s.as_str()).collect();
        let result = run_select(terminal, "Select disk", &disk_refs, 0)?;
        match result.selected {
            Some(idx) => Ok(Some(disks[idx].clone())),
            None => Ok(None),
        }
    }

    fn pick_partition(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        title: &str,
    ) -> color_eyre::eyre::Result<Option<std::path::PathBuf>> {
        let parts = archinstall_zfs_core::disk::by_id::list_partitions_by_id()?;
        if parts.is_empty() {
            return Ok(None);
        }
        let part_strs: Vec<String> = parts.iter().map(|p| p.display().to_string()).collect();
        let part_refs: Vec<&str> = part_strs.iter().map(|s| s.as_str()).collect();
        let result = run_select(terminal, title, &part_refs, 0)?;
        match result.selected {
            Some(idx) => Ok(Some(parts[idx].clone())),
            None => Ok(None),
        }
    }

    /// Existing-pool picker: discover importable pools, let the user select one,
    /// then detect encryption and verify passphrase (matching the Python flow).
    fn pick_existing_pool(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<()> {
        use archinstall_zfs_core::system::cmd::RealRunner;
        use archinstall_zfs_core::zfs::{encryption, pool};

        let runner = RealRunner;

        // Discover importable pools, offer Refresh and manual entry
        let pool_name = loop {
            let mut pools = pool::discover_importable_pools(&runner);
            let mut options: Vec<String> = pools.iter().map(|p| p.clone()).collect();
            options.push("Refresh".into());
            options.push("Enter manually".into());
            let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();

            let current = if let Some(ref name) = self.config.pool_name {
                pools.iter().position(|p| p == name).unwrap_or(0)
            } else {
                0
            };

            let result = run_select(terminal, "Select importable ZFS pool", &opt_refs, current)?;
            let Some(idx) = result.selected else {
                return Ok(()); // cancelled
            };

            if idx == options.len() - 2 {
                // Refresh
                continue;
            } else if idx == options.len() - 1 {
                // Enter manually
                let result = run_edit(terminal, "Pool name", "", false)?;
                match result.value {
                    Some(name) if !name.is_empty() => break name,
                    _ => return Ok(()),
                }
            } else {
                break pools.swap_remove(idx);
            }
        };

        self.config.pool_name = Some(pool_name.clone());

        // Detect encryption via ephemeral import/export
        if encryption::detect_pool_encryption(&runner, &pool_name) {
            // Pool is encrypted — verify passphrase
            loop {
                let result = run_edit(terminal, "Enter pool passphrase", "", true)?;
                let Some(pw) = result.value else {
                    // User cancelled — leave encryption unconfigured
                    break;
                };
                if pw.is_empty() {
                    break;
                }

                if encryption::verify_pool_passphrase(&runner, &pool_name, &pw) {
                    self.config.zfs_encryption_mode = ZfsEncryptionMode::Pool;
                    self.config.zfs_encryption_password = Some(pw);
                    break;
                } else {
                    let _ = run_select(
                        terminal,
                        "Passphrase verification failed. Try again.",
                        &["OK"],
                        0,
                    );
                }
            }
        } else {
            // Pool is not encrypted — offer to encrypt the new base dataset
            let result = run_select(
                terminal,
                "Encrypt the new base dataset?",
                &["No - Skip encryption", "Yes - Encrypt new base dataset"],
                0,
            )?;
            if result.selected == Some(1) {
                // Prompt for new encryption password with confirmation
                loop {
                    let pw1 = run_edit(terminal, "Encryption password (min 8 chars)", "", true)?;
                    let Some(pw1) = pw1.value.filter(|p| !p.is_empty()) else {
                        break;
                    };
                    let pw2 = run_edit(terminal, "Verify password", "", true)?;
                    let Some(pw2) = pw2.value else {
                        break;
                    };
                    if pw1 == pw2 {
                        self.config.zfs_encryption_mode = ZfsEncryptionMode::Dataset;
                        self.config.zfs_encryption_password = Some(pw1);
                        break;
                    } else {
                        let _ =
                            run_select(terminal, "Passwords do not match. Try again.", &["OK"], 0);
                    }
                }
            } else {
                self.config.zfs_encryption_mode = ZfsEncryptionMode::None;
                self.config.zfs_encryption_password = None;
            }
        }

        Ok(())
    }

    fn pick_locale(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Option<String>> {
        use archinstall_zfs_core::installer::locale;

        let locales = locale::list_locales();
        let locale_strs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
        let result = run_select(terminal, "Locale", &locale_strs, 0)?;
        match result.selected {
            Some(idx) => Ok(Some(locales[idx].clone())),
            None => Ok(None),
        }
    }

    fn pick_kernel(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<Option<Vec<String>>> {
        use archinstall_zfs_core::kernel::scanner::scan_all_kernels;
        use archinstall_zfs_core::kernel::AVAILABLE_KERNELS;

        let results = scan_all_kernels();

        let mut options = Vec::new();
        let mut kernel_names = Vec::new();
        for (info, result) in AVAILABLE_KERNELS.iter().zip(&results) {
            let compat = match self.config.zfs_module_mode {
                ZfsModuleMode::Precompiled => {
                    if result.precompiled_compatible {
                        "OK"
                    } else {
                        "INCOMPATIBLE"
                    }
                }
                ZfsModuleMode::Dkms => {
                    if result.dkms_compatible {
                        "OK"
                    } else {
                        "INCOMPATIBLE"
                    }
                }
            };
            let ver = result.kernel_version.as_deref().unwrap_or("?");
            options.push(format!("{} ({ver}) [{compat}]", info.display_name));
            kernel_names.push(info.name);
        }

        let option_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        let current_kernel = self.config.primary_kernel();
        let current_idx = kernel_names
            .iter()
            .position(|&n| n == current_kernel)
            .unwrap_or(0);

        let result = run_select(terminal, "Kernel", &option_refs, current_idx)?;
        match result.selected {
            Some(idx) => Ok(Some(vec![kernel_names[idx].to_string()])),
            None => Ok(None),
        }
    }

    // ── User management ─────────────────────────────────

    fn manage_users(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<()> {
        loop {
            let users = self.config.users.clone().unwrap_or_default();
            let mut options: Vec<String> = users
                .iter()
                .map(|u| {
                    let sudo = if u.sudo { " [sudo]" } else { "" };
                    format!("{}{sudo}", u.username)
                })
                .collect();
            options.push("+ Add user".to_string());
            if !users.is_empty() {
                options.push("- Remove user".to_string());
            }
            options.push("Done".to_string());

            let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
            let result = run_select(terminal, "User accounts", &opt_refs, 0)?;
            let Some(idx) = result.selected else {
                break;
            };

            if idx < users.len() {
                // Edit existing user — toggle sudo
                if let Some(ref mut user_list) = self.config.users {
                    if let Some(user) = user_list.get_mut(idx) {
                        user.sudo = !user.sudo;
                    }
                }
            } else if options[idx] == "+ Add user" {
                let result = run_edit(terminal, "Username", "", false)?;
                if let Some(username) = result.value {
                    if !username.is_empty() {
                        let pw_result =
                            run_edit(terminal, "Password (empty=no password)", "", true)?;
                        let password = pw_result.value.filter(|p| !p.is_empty());

                        let sudo_opts = ["No", "Yes"];
                        let sudo_result = run_select(terminal, "Enable sudo?", &sudo_opts, 1)?;
                        let sudo = sudo_result.selected == Some(1);

                        let user = UserConfig {
                            username,
                            password,
                            sudo,
                            shell: None,
                            groups: None,
                        };
                        self.config.users.get_or_insert_with(Vec::new).push(user);
                    }
                }
            } else if options[idx].starts_with("- Remove") {
                // Pick which user to remove
                let user_names: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
                let result = run_select(terminal, "Remove user", &user_names, 0)?;
                if let Some(rm_idx) = result.selected {
                    if let Some(ref mut user_list) = self.config.users {
                        user_list.remove(rm_idx);
                        if user_list.is_empty() {
                            self.config.users = None;
                        }
                    }
                }
            } else {
                // Done
                break;
            }
        }
        Ok(())
    }

    // ── Apply handlers ──────────────────────────────────

    fn apply_toggle(&mut self, key: &str) {
        match key {
            "ntp" => self.config.ntp = !self.config.ntp,
            "bluetooth" => self.config.bluetooth = !self.config.bluetooth,
            "zrepl" => self.config.zrepl_enabled = !self.config.zrepl_enabled,
            _ => {}
        }
    }

    fn apply_select(
        &mut self,
        key: &str,
        idx: usize,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> color_eyre::eyre::Result<()> {
        match key {
            "installation_mode" => {
                let new_mode = match idx {
                    0 => InstallationMode::FullDisk,
                    1 => InstallationMode::NewPool,
                    2 => InstallationMode::ExistingPool,
                    _ => return Ok(()),
                };
                // Reset mode-dependent fields when switching modes
                if self.config.installation_mode != Some(new_mode) {
                    self.config.disk_by_id = None;
                    self.config.efi_partition_by_id = None;
                    self.config.zfs_partition_by_id = None;
                    self.config.swap_partition_by_id = None;
                }
                self.config.installation_mode = Some(new_mode);
            }
            "encryption" => {
                let new_mode = match idx {
                    0 => ZfsEncryptionMode::None,
                    1 => ZfsEncryptionMode::Pool,
                    2 => ZfsEncryptionMode::Dataset,
                    _ => return Ok(()),
                };
                self.config.zfs_encryption_mode = new_mode;
                // Prompt for password when enabling encryption
                if new_mode != ZfsEncryptionMode::None
                    && self.config.zfs_encryption_password.is_none()
                {
                    let result = run_edit(terminal, "Encryption password (min 8 chars)", "", true)?;
                    if let Some(pw) = result.value {
                        if !pw.is_empty() {
                            self.config.zfs_encryption_password = Some(pw);
                        }
                    }
                }
                if new_mode == ZfsEncryptionMode::None {
                    self.config.zfs_encryption_password = None;
                }
            }
            "compression" => {
                self.config.compression = match idx {
                    0 => CompressionAlgo::Lz4,
                    1 => CompressionAlgo::Zstd,
                    2 => CompressionAlgo::Zstd5,
                    3 => CompressionAlgo::Zstd10,
                    4 => CompressionAlgo::Off,
                    _ => return Ok(()),
                };
            }
            "swap_mode" => {
                self.config.swap_mode = match idx {
                    0 => SwapMode::None,
                    1 => SwapMode::Zram,
                    2 => SwapMode::ZswapPartition,
                    3 => SwapMode::ZswapPartitionEncrypted,
                    _ => return Ok(()),
                };
            }
            "init_system" => {
                self.config.init_system = match idx {
                    0 => InitSystem::Dracut,
                    1 => InitSystem::Mkinitcpio,
                    _ => return Ok(()),
                };
            }
            "zfs_module_mode" => {
                self.config.zfs_module_mode = match idx {
                    0 => ZfsModuleMode::Precompiled,
                    1 => ZfsModuleMode::Dkms,
                    _ => return Ok(()),
                };
            }
            "audio" => {
                self.config.audio = match idx {
                    0 => None,
                    1 => Some(AudioServer::Pipewire),
                    2 => Some(AudioServer::Pulseaudio),
                    _ => return Ok(()),
                };
            }
            "network" => {
                self.config.network_copy_iso = idx == 0;
            }
            "profile" => {
                self.config.profile = match idx {
                    0 => None,
                    _ => {
                        let profiles = archinstall_zfs_core::profile::all_profiles();
                        profiles.get(idx - 1).map(|p| p.name.to_string())
                    }
                };
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_text(&mut self, key: &str, val: &str) {
        let val_opt = if val.is_empty() {
            None
        } else {
            Some(val.to_string())
        };
        match key {
            "pool_name" => self.config.pool_name = val_opt,
            "dataset_prefix" => {
                if !val.is_empty() {
                    self.config.dataset_prefix = val.to_string();
                }
            }
            "hostname" => self.config.hostname = val_opt,
            "locale" => self.config.locale = val_opt,
            "timezone" => self.config.timezone = val_opt,
            "keyboard" => {
                if !val.is_empty() {
                    self.config.keyboard_layout = val.to_string();
                }
            }
            "root_password" => self.config.root_password = val_opt,
            "encryption_password" => self.config.zfs_encryption_password = val_opt,
            "swap_partition_size" => self.config.swap_partition_size = val_opt,
            "additional_packages" => {
                self.config.additional_packages = val
                    .split_whitespace()
                    .map(|s| s.trim_matches(',').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "aur_packages" => {
                self.config.aur_packages = val
                    .split_whitespace()
                    .map(|s| s.trim_matches(',').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "extra_services" => {
                self.config.extra_services = val
                    .split_whitespace()
                    .map(|s| s.trim_matches(',').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }

    // ── Render ───────────────────────────────────────────

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let items = self.items();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        // Title
        let title = Paragraph::new(Line::from(vec![Span::styled(
            " archinstall-zfs ",
            theme::TITLE_STYLE,
        )]))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .style(theme::BORDER_STYLE),
        );
        frame.render_widget(title, chunks[0]);

        // Menu block
        let menu_block = Block::default()
            .title(" Configuration ")
            .title_style(theme::HEADER_STYLE)
            .borders(Borders::ALL)
            .style(theme::BORDER_STYLE);

        let inner = menu_block.inner(chunks[1]);
        frame.render_widget(menu_block, chunks[1]);

        // Render items with scrolling
        let visible_height = inner.height as usize;
        let total_items = items.len();

        // Adjust scroll to keep selected visible
        let mut scroll = self.scroll_offset;
        if self.selected >= scroll + visible_height {
            scroll = self.selected - visible_height + 1;
        }
        if self.selected < scroll {
            scroll = self.selected;
        }

        for (vi, item) in items.iter().enumerate().skip(scroll).take(visible_height) {
            let y = inner.y + (vi - scroll) as u16;
            let line_area = Rect::new(inner.x, y, inner.width, 1);

            if matches!(item.kind, MenuKind::Separator) {
                let sep = Paragraph::new(Line::from(Span::styled(
                    "─".repeat(inner.width as usize),
                    theme::BORDER_STYLE,
                )));
                frame.render_widget(sep, line_area);
                continue;
            }

            let is_selected = vi == self.selected;
            let is_action = matches!(item.kind, MenuKind::Action);

            let label_style = if is_selected {
                theme::SELECTED_STYLE
            } else {
                theme::NORMAL_STYLE
            };

            let value_style = if is_selected {
                theme::SELECTED_STYLE
            } else if is_action {
                theme::TITLE_STYLE
            } else if item.value.contains("Not") || item.value == "None" {
                theme::UNSET_STYLE
            } else {
                theme::VALUE_STYLE
            };

            let cursor = if is_selected { "> " } else { "  " };

            let line = if is_action {
                Line::from(vec![
                    Span::styled(cursor, label_style),
                    Span::styled(item.label, value_style),
                ])
            } else {
                Line::from(vec![
                    Span::styled(cursor, label_style),
                    Span::styled(format!("{:<22}", item.label), label_style),
                    Span::styled(&item.value, value_style),
                ])
            };

            frame.render_widget(Paragraph::new(line), line_area);
        }

        // Scrollbar if needed
        if total_items > visible_height {
            let mut scrollbar_state = ScrollbarState::new(total_items).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                chunks[1],
                &mut scrollbar_state,
            );
        }

        // Footer
        let footer = Paragraph::new(Line::from(vec![Span::styled(
            " j/k: navigate | Enter: edit | q: quit ",
            theme::DIMMED_STYLE,
        )]))
        .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[2]);
    }
}
