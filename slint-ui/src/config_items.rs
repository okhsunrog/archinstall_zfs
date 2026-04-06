//! Build the per-step `Vec<ConfigItem>` shown in the wizard, and apply edits
//! coming back from radio/select/text widgets to the canonical `GlobalConfig`.

use slint::SharedString;

use archinstall_zfs_core::config::types::{
    AudioServer, CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, SwapMode,
    ZfsEncryptionMode,
};

use crate::ui::{ConfigItem, ItemType};

pub const TOTAL_STEPS: usize = 7;

pub const STEP_LABELS: [&str; TOTAL_STEPS] = [
    "Welcome", "Disk", "ZFS", "System", "Users", "Desktop", "Review",
];

// ── Per-step item building ──────────────────────────

pub fn build_step_items(step: usize, c: &GlobalConfig) -> Vec<ConfigItem> {
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

fn build_welcome_items(_c: &GlobalConfig) -> Vec<ConfigItem> {
    // Welcome screen is handled by dedicated UI, no config items
    vec![]
}

fn build_disk_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mode = c.installation_mode;

    let mut items = radio_group(
        "installation_mode",
        "Installation mode",
        &["Full Disk", "New Pool", "Existing Pool"],
        match mode {
            Some(InstallationMode::FullDisk) => 0,
            Some(InstallationMode::NewPool) => 1,
            Some(InstallationMode::ExistingPool) => 2,
            None => -1,
        },
    );

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
            ItemType::Text,
        ),
        ci(
            "dataset_prefix",
            "Dataset prefix",
            &c.dataset_prefix,
            ItemType::Text,
        ),
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
            ItemType::Password,
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
            ItemType::Text,
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

    items
}

fn build_system_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = vec![
        ci(
            "kernel",
            "Kernel",
            &format!(
                "{} [{}]",
                c.kernels
                    .as_ref()
                    .map(|k| k.join(", "))
                    .unwrap_or_else(|| c.primary_kernel().to_string()),
                c.zfs_module_mode
            ),
            ItemType::Select,
        ),
        ci(
            "hostname",
            "Hostname",
            &c.hostname.clone().unwrap_or("Not set".into()),
            ItemType::Text,
        ),
        ci(
            "locale",
            "Locale",
            &c.locale.clone().unwrap_or("Not set".into()),
            ItemType::Select,
        ),
        ci(
            "timezone",
            "Timezone",
            &c.timezone.clone().unwrap_or("Not set".into()),
            ItemType::Select,
        ),
        ci(
            "keyboard",
            "Keyboard layout",
            &c.keyboard_layout,
            ItemType::Select,
        ),
        ci(
            "ntp",
            "NTP (time sync)",
            if c.ntp { "Enabled" } else { "Disabled" },
            ItemType::Toggle,
        ),
    ];

    items.push(ci(
        "parallel_downloads",
        "Parallel downloads",
        &c.parallel_downloads.to_string(),
        ItemType::Text,
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
            ItemType::Password,
        ),
        ci(
            "users",
            "User accounts",
            &match &c.users {
                Some(users) if !users.is_empty() => users
                    .iter()
                    .map(|u| {
                        if u.sudo {
                            format!("{} [sudo]", u.username)
                        } else {
                            u.username.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => "None".into(),
            },
            ItemType::Text,
        ),
    ]
}

fn build_desktop_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = vec![ci(
        "profile",
        "Profile",
        c.profile.as_deref().unwrap_or("None"),
        ItemType::Select,
    )];

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
            ItemType::Toggle,
        ),
        ci(
            "packages",
            "Extra packages",
            &{
                let total = c.additional_packages.len() + c.aur_packages.len();
                if total == 0 {
                    "None".to_string()
                } else {
                    let mut parts: Vec<&str> =
                        c.additional_packages.iter().map(|s| s.as_str()).collect();
                    parts.extend(c.aur_packages.iter().map(|s| s.as_str()));
                    parts.join(", ")
                }
            },
            ItemType::Text,
        ),
        ci(
            "extra_services",
            "Extra services",
            &if c.extra_services.is_empty() {
                "None".to_string()
            } else {
                c.extra_services.join(", ")
            },
            ItemType::Text,
        ),
        ci(
            "zrepl",
            "zrepl (snapshots)",
            if c.zrepl_enabled {
                "Enabled"
            } else {
                "Disabled"
            },
            ItemType::Toggle,
        ),
    ]);

    items
}

fn build_review_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = Vec::new();

    for (step, &label) in STEP_LABELS.iter().enumerate().take(TOTAL_STEPS - 1) {
        items.push(ConfigItem {
            key: SharedString::default(),
            label: label.into(),
            value: SharedString::default(),
            item_type: ItemType::Separator,
        });

        let step_items = build_step_items(step, c);
        let mut i = 0;
        while i < step_items.len() {
            let item = &step_items[i];
            if item.item_type == ItemType::RadioHeader {
                let header_label = item.label.clone();
                let mut selected_label: SharedString = "Not set".into();
                i += 1;
                while i < step_items.len() && step_items[i].item_type == ItemType::RadioOption {
                    if step_items[i].value == "selected" {
                        selected_label = step_items[i].label.clone();
                    }
                    i += 1;
                }
                items.push(ConfigItem {
                    key: SharedString::default(),
                    label: header_label,
                    value: selected_label,
                    item_type: ItemType::Readonly,
                });
            } else {
                items.push(ConfigItem {
                    key: item.key.clone(),
                    label: item.label.clone(),
                    value: item.value.clone(),
                    item_type: ItemType::Readonly,
                });
                i += 1;
            }
        }
    }

    let errors = c.validate_for_install();
    if !errors.is_empty() {
        items.push(sep());
        for error in &errors {
            items.push(ConfigItem {
                key: SharedString::default(),
                label: SharedString::default(),
                value: error.as_str().into(),
                item_type: ItemType::Warning,
            });
        }
    }

    items.push(sep());
    items.push(ConfigItem {
        key: "install".into(),
        label: "Install".into(),
        value: SharedString::default(),
        item_type: ItemType::Action,
    });
    items.push(ConfigItem {
        key: "quit".into(),
        label: "Quit".into(),
        value: SharedString::default(),
        item_type: ItemType::Action,
    });

    items
}

fn ci(key: &str, label: &str, value: &str, item_type: ItemType) -> ConfigItem {
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
        item_type: ItemType::Separator,
    }
}

/// Emit a radio group: a header followed by clickable options.
fn radio_group(key: &str, label: &str, options: &[&str], selected: i32) -> Vec<ConfigItem> {
    let mut items = vec![ConfigItem {
        key: SharedString::default(),
        label: label.into(),
        value: SharedString::default(),
        item_type: ItemType::RadioHeader,
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
            item_type: ItemType::RadioOption,
        });
    }
    items
}

// ── Keyboard navigation helper ──────────────────────

/// Find the next selectable item, skipping non-interactive types.
pub fn next_selectable_index(items: &[ConfigItem], current: i32, dir: i32) -> i32 {
    let len = items.len() as i32;
    if len == 0 {
        return -1;
    }
    for offset in 1..=len {
        let idx = ((current + dir * offset) % len + len) % len;
        let t = items[idx as usize].item_type;
        if t != ItemType::Separator
            && t != ItemType::Readonly
            && t != ItemType::Warning
            && t != ItemType::RadioHeader
        {
            return idx;
        }
    }
    current
}

// ── Apply mutations ─────────────────────────────────

/// Apply an inline radio selection. `group_key` is e.g. "compression".
pub fn apply_radio(config: &mut GlobalConfig, group_key: &str, idx: i32) {
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
        _ => {}
    }
}

pub fn apply_text(config: &mut GlobalConfig, key: &str, val: &str) {
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
        "root_password" => config.root_password = opt,
        "encryption_password" => config.zfs_encryption_password = opt,
        "swap_partition_size" => config.swap_partition_size = opt,
        "parallel_downloads" => {
            if let Ok(n) = val.parse::<u32>() {
                config.parallel_downloads = n.clamp(1, 20);
            }
        }
        _ => {}
    }
}
