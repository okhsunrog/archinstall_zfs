//! Build the per-step `Vec<ConfigItem>` shown in the wizard, and apply edits
//! coming back from radio/select/text widgets to the canonical `GlobalConfig`.

use slint::SharedString;

use archinstall_zfs_core::config::choices::Choice;
use archinstall_zfs_core::config::edit::{ChoiceSetting, EditorSetting, TextSetting};
use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode, ZfsEncryptionMode};

use crate::ui::{ConfigItem, ItemType};
#[cfg(test)]
use archinstall_zfs_core::config::edit::DeviceSetting;

pub const TOTAL_STEPS: usize = 7;

pub const STEP_LABELS: [&str; TOTAL_STEPS] = [
    "Welcome", "Disk", "ZFS", "System", "Users", "Desktop", "Review",
];

// ── Per-step item building ──────────────────────────

pub fn build_step_items(step: usize, c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = match step {
        0 => build_welcome_items(c),
        1 if c.installation_mode == Some(InstallationMode::Alongside) => vec![],
        1 => build_disk_items(c),
        2 => build_zfs_items(c),
        3 => build_system_items(c),
        4 => build_users_items(c),
        5 => build_desktop_items(c),
        6 => build_review_items(c),
        _ => vec![],
    };
    if step != 6 {
        for item in &mut items {
            let description = match item.key.as_str() {
                "root_password" => {
                    "Controls direct login as root. A regular administrator account can use sudo instead."
                }
                "users" => {
                    "Create a daily-use account and choose who can run administrator commands with sudo."
                }
                "profile" => {
                    "Choose a desktop environment, a window manager, or a console-only system."
                }
                "display_manager" => {
                    "The graphical sign-in screen. The profile default is used unless you override it."
                }
                _ => continue,
            };
            item.description = description.into();
        }
    }
    mark_section_boundaries(&mut items);
    items
}

fn build_welcome_items(_c: &GlobalConfig) -> Vec<ConfigItem> {
    // Welcome screen is handled by dedicated UI, no config items
    vec![]
}

fn build_disk_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = vec![section_header("Storage assignments")];
    match c.installation_mode {
        Some(InstallationMode::Alongside) => {
            if let Some(r) = &c.alongside {
                items.push(ConfigItem {
                    label: "Planned allocation".into(),
                    value: format!(
                        "{} — {:.0} GiB",
                        r.before.device.display(),
                        r.allocation_bytes as f64 / 1073741824.0
                    )
                    .into(),
                    description: {
                        use archinstall_zfs_core::disk::alongside::{EfiChoice, SpaceSource};
                        let source = match r.source {
                            SpaceSource::Shrink { partition } => format!(
                                "Shrink partition {partition} from its end; preserve its data"
                            ),
                            SpaceSource::Unallocated { .. } => {
                                "Use unallocated space; preserve existing partitions".into()
                            }
                        };
                        let efi = match r.efi {
                            EfiChoice::Reuse { partition } => {
                                format!("Reuse EFI partition {partition} without formatting")
                            }
                            EfiChoice::CreateSeparate { .. } => {
                                "Create the explicitly selected separate 512 MiB EFI partition"
                                    .into()
                            }
                        };
                        format!("{source}. {efi}. {} GiB reserved for swap; the remainder is the new ZFS pool.", r.swap_bytes / 1073741824).into()
                    },
                    item_type: ItemType::Storage,
                    ..Default::default()
                });
            }
        }
        Some(InstallationMode::FullDisk) => {
            items.push(storage_item("disk", "Disk to erase", c.disk.as_deref(), c))
        }
        Some(InstallationMode::NewPool) | Some(InstallationMode::ExistingPool) => {
            if c.installation_mode == Some(InstallationMode::ExistingPool) {
                items.push(ConfigItem {
                    key: "storage:pool".into(),
                    label: "Existing ZFS pool".into(),
                    value: c.pool_name.as_deref().unwrap_or("Choose a pool").into(),
                    description: "Create a new boot environment in this pool".into(),
                    item_type: ItemType::Storage,
                    is_empty: c.pool_name.is_none(),
                    ..Default::default()
                });
            }
            items.push(storage_item(
                "efi_partition",
                "EFI boot partition",
                c.efi_partition.as_deref(),
                c,
            ));
            if c.installation_mode == Some(InstallationMode::NewPool) {
                items.push(storage_item(
                    "zfs_partition",
                    "New ZFS pool",
                    c.zfs_partition.as_deref(),
                    c,
                ));
            }
        }
        None => {}
    }
    items
}

fn storage_item(
    role: &str,
    label: &str,
    selected: Option<&std::path::Path>,
    c: &GlobalConfig,
) -> ConfigItem {
    let choices = crate::storage::choices(role);
    let choice = selected.and_then(|p| choices.iter().find(|d| d.path == p));
    let consequence = match role {
        "disk" if c.swap_mode.uses_partition() => {
            "Erase all partitions; create EFI, ZFS and swap partitions"
        }
        "disk" => "Erase all partitions; create EFI and ZFS partitions",
        "efi_partition" => "Reuse filesystem; write bootloader files",
        "zfs_partition" => "Replace this partition's contents with a new ZFS pool",
        _ => "Replace this partition's contents with swap",
    };
    let mut item = ConfigItem {
        key: format!("storage:{role}").into(),
        label: label.into(),
        value: choice
            .map(|d| d.label.clone())
            .or_else(|| selected.map(|p| p.display().to_string()))
            .unwrap_or_else(|| "Choose a device".into())
            .into(),
        description: consequence.into(),
        destructive: role != "efi_partition",
        item_type: ItemType::Storage,
        is_empty: selected.is_none(),
        ..Default::default()
    };
    if let Some(d) = choice {
        item.detail_model = format!("{}  {}  {}", d.model, d.usage.filesystem, d.usage.label)
            .trim()
            .into();
        item.detail_size = d.size.clone().into();
        item.persistent_path = d.path.display().to_string().into();
        let blocked = crate::storage::unavailable(d, role, c);
        if !blocked.is_empty() {
            item.description = format!("Unavailable: {blocked}").into();
            item.destructive = true;
        }
    }
    item
}

fn inline_text(key: TextSetting, label: &str, value: &str, description: &str) -> ConfigItem {
    ConfigItem {
        key: key.as_str().into(),
        label: label.into(),
        value: value.into(),
        description: description.into(),
        item_type: ItemType::InlineText,
        ..Default::default()
    }
}
fn compact_choice<T: Choice>(
    key: ChoiceSetting,
    label: &str,
    current: T,
    description: &str,
) -> ConfigItem {
    ConfigItem {
        key: key.as_str().into(),
        label: label.into(),
        value: T::labels()[current.index()].into(),
        choices: slint::ModelRc::new(slint::VecModel::from(
            T::labels()
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )),
        choice_index: current.index() as i32,
        description: description.into(),
        item_type: ItemType::CompactChoice,
        ..Default::default()
    }
}
fn build_zfs_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = vec![section_header("Pool and boot environment")];
    if c.installation_mode == Some(InstallationMode::ExistingPool) {
        items.push(ConfigItem {
            key: "storage:pool".into(),
            label: "Existing pool".into(),
            value: c.pool_name.as_deref().unwrap_or("Choose a pool").into(),
            description: "Select the existing pool; its name will not be changed".into(),
            is_empty: c.pool_name.is_none(),
            item_type: ItemType::Storage,
            ..Default::default()
        });
    } else {
        items.push(inline_text(
            TextSetting::PoolName,
            "New pool name",
            c.pool_name.as_deref().unwrap_or(""),
            "",
        ));
    }
    items.push(inline_text(
        TextSetting::DatasetPrefix,
        "Boot environment",
        &c.dataset_prefix,
        &format!(
            "Create {}",
            archinstall_zfs_core::boot_environment::BootEnvironment::new(
                c.pool_name.as_deref().unwrap_or("?"),
                &c.dataset_prefix
            )
            .base()
        ),
    ));
    items.push(section_header("Data protection"));
    let mut encryption = compact_choice(
        ChoiceSetting::Encryption,
        "Encryption",
        c.zfs_encryption_mode,
        if c.installation_mode == Some(InstallationMode::ExistingPool) {
            "Use the existing pool passphrase, or give the new boot environment its own encryption."
        } else {
            "Encrypt the entire pool, or just this boot environment."
        },
    );
    if c.installation_mode == Some(InstallationMode::ExistingPool) {
        let labels = [
            "No additional encryption",
            "Unlock encrypted pool",
            "Encrypt new boot environment",
        ];
        encryption.choices = slint::ModelRc::new(slint::VecModel::from(
            labels
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        ));
        encryption.value = labels[c.zfs_encryption_mode.index()].into();
    }
    items.push(encryption);
    if c.zfs_encryption_mode != ZfsEncryptionMode::None {
        items.push(ConfigItem {
            key: TextSetting::EncryptionPassword.as_str().into(),
            label: "Encryption passphrase".into(),
            value: if c.zfs_encryption_password.is_some() {
                "Set — enter to replace"
            } else {
                "Required"
            }
            .into(),
            description:
                "At least 8 characters. Keep a copy: a lost passphrase cannot be recovered.".into(),
            item_type: ItemType::InlinePassword,
            ..Default::default()
        });
    }
    if c.installation_mode == Some(InstallationMode::Alongside) {
        let value = if let Some(r) = &c.alongside
            && r.swap_bytes > 0
        {
            format!("{} — {} GiB", c.swap_mode, r.swap_bytes / 1073741824)
        } else {
            c.swap_mode.to_string()
        };
        items.push(ConfigItem {
            label: "Swap method".into(),
            value: value.into(),
            description: "Configured with the space allocation on the Disk page".into(),
            item_type: ItemType::Readonly,
            ..Default::default()
        });
    } else {
        items.push(section_header("Swap"));
        items.push(compact_choice(
            ChoiceSetting::SwapMode,
            "Swap method",
            c.swap_mode,
            "ZRAM uses compressed RAM. A swap partition uses disk space.",
        ));
        if c.swap_mode.uses_partition() {
            if c.installation_mode == Some(InstallationMode::FullDisk) {
                items.push(inline_text(
                    TextSetting::SwapPartitionSize,
                    "Swap size",
                    c.swap_partition_size.as_deref().unwrap_or(""),
                    "Created on the selected disk, for example 8G",
                ));
            } else {
                items.push(storage_item(
                    "swap_partition",
                    "Swap partition",
                    c.swap_partition.as_deref(),
                    c,
                ));
            }
        }
    }
    let mut compression = compact_choice(
        ChoiceSetting::Compression,
        "Compression",
        c.compression,
        "lz4 prioritizes speed; zstd levels trade CPU time for compression.",
    );
    compression.advanced = true;
    items.push(compression);
    let mut init = compact_choice(
        ChoiceSetting::InitSystem,
        "Initramfs generator",
        c.init_system,
        "Builds the early boot image used to load the system.",
    );
    init.advanced = true;
    items.push(init);
    items
}

fn build_system_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    vec![
        section_header("Base system"),
        // Before the kernel row on purpose: which kernels exist is the
        // distribution's answer, so choosing one first is the order that makes
        // sense on screen.
        ci(
            EditorSetting::Distribution.as_str(),
            "Distribution",
            c.distribution().display_name,
            ItemType::Select,
        ),
        ci(
            EditorSetting::Kernel.as_str(),
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
        ci_opt(
            TextSetting::Hostname.as_str(),
            "Hostname",
            c.hostname.as_deref(),
            ItemType::Text,
        ),
        ci_toggle("ntp", "NTP (time sync)", c.ntp),
        ci(
            TextSetting::ParallelDownloads.as_str(),
            "Parallel downloads",
            &c.parallel_downloads.to_string(),
            ItemType::Text,
        ),
        section_header("Language and region"),
        ci_opt("locale", "Locale", c.locale.as_deref(), ItemType::Select),
        ci_opt(
            EditorSetting::Timezone.as_str(),
            "Timezone",
            c.timezone.as_deref(),
            ItemType::Select,
        ),
        ci(
            "keyboard",
            "Keyboard layout",
            &c.keyboard_layout,
            ItemType::Select,
        ),
    ]
}

fn build_users_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    vec![
        section_header("Authentication"),
        ci_opt(
            TextSetting::RootPassword.as_str(),
            "Root password",
            c.root_password.as_ref().map(|_| "Set"),
            ItemType::Password,
        ),
        section_header("Accounts"),
        {
            let summary = match &c.users {
                Some(users) if !users.is_empty() => Some(
                    users
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
                ),
                _ => None,
            };
            // ci_opt's None → "Not set"; users semantically wants "None".
            // Construct directly so we keep the established label.
            ConfigItem {
                key: "users".into(),
                label: "User accounts".into(),
                value: summary.clone().unwrap_or_else(|| "None".into()).into(),
                item_type: ItemType::Text,
                is_empty: summary.is_none(),
                ..Default::default()
            }
        },
    ]
}

fn build_desktop_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let sel = c.profile_selection.as_ref();
    let profile_def = sel.and_then(|s| s.profile_def());

    let profile_name = profile_def.as_ref().map(|p| p.display_name.to_string());
    let mut items = vec![
        section_header("Environment"),
        ConfigItem {
            key: "profile".into(),
            label: "Profile".into(),
            value: profile_name
                .clone()
                .unwrap_or_else(|| "Console only".into())
                .into(),
            item_type: ItemType::Select,
            is_empty: profile_name.is_none(),
            ..Default::default()
        },
    ];

    // ── Profile configuration: only when a desktop profile is active ──
    if let (Some(sel), Some(p)) = (sel, profile_def.as_ref())
        && p.is_desktop()
    {
        items.push(section_header("Profile configuration"));

        // Optional packages: "N of M"
        let total = p.optional_packages().len();
        if total > 0 {
            let chosen = sel.optional_packages.len();
            items.push(ConfigItem {
                key: "optional_packages".into(),
                label: "Optional packages".into(),
                value: format!("{chosen} of {total}").into(),
                item_type: ItemType::Select,
                is_empty: chosen == 0,
                ..Default::default()
            });
        }

        // Display manager: shows the effective DM with (default) or
        // (override) suffix so the user can tell at a glance whether they
        // diverged from the profile.
        let (value, dm_is_empty) = match (sel.display_manager_override, p.default_display_manager())
        {
            (Some(over), _) => (format!("{} (override)", over.display_name()), false),
            (None, Some(def)) => (format!("{} (default)", def.display_name()), false),
            (None, None) => ("None".to_string(), true),
        };
        items.push(ConfigItem {
            key: "display_manager".into(),
            label: "Display manager".into(),
            value: value.into(),
            item_type: ItemType::Select,
            is_empty: dm_is_empty,
            ..Default::default()
        });

        // Seat access is only relevant to Wayland compositor profiles.
        if p.needs_seat_access() {
            items.push(compact_choice(
                ChoiceSetting::SeatAccess,
                "Seat access",
                sel.seat_access,
                "How the compositor accesses input and display devices.",
            ));
        }
    }

    items.push(section_header("Sound"));
    items.push(compact_choice(
        ChoiceSetting::Audio,
        "Audio",
        c.audio,
        "Choose the sound server installed with the system.",
    ));

    items.push(section_header("Hardware"));
    // GPU driver — only shown for graphical profiles (mirrors upstream
    // archinstall's `is_graphic_driver_supported` gate). Headless installs
    // skip the row entirely.
    if profile_def
        .as_ref()
        .is_some_and(|p| p.supports_gfx_driver())
    {
        items.push({
            let driver = c.gfx_driver.map(|d| d.to_string());
            ConfigItem {
                key: "gpu_driver".into(),
                label: "GPU driver".into(),
                value: driver.clone().unwrap_or_else(|| "None".into()).into(),
                item_type: ItemType::Select,
                is_empty: driver.is_none(),
                ..Default::default()
            }
        });

        // Inline warning when the proprietary NVIDIA driver is paired with
        // a Wayland-only compositor. The TUI shows a confirmation dialog;
        // the GUI surfaces it as a Warning row inside the same section so
        // the user sees it without opening a popup.
        if profile_def.as_ref().is_some_and(|p| p.is_wayland_only())
            && c.gfx_driver == Some(archinstall_zfs_core::system::gpu::GfxDriver::NvidiaOpen)
        {
            items.push(ConfigItem {
                value: "Proprietary NVIDIA driver is known-problematic on \
                        Wayland-only compositors."
                    .into(),
                item_type: ItemType::Warning,
                ..Default::default()
            });
        }
    }
    items.push(ci_toggle("bluetooth", "Bluetooth", c.bluetooth));

    items.push(section_header("Software"));
    items.push({
        let parts: Vec<&str> = c
            .additional_packages
            .iter()
            .chain(c.aur_packages.iter())
            .map(|s| s.as_str())
            .collect();
        let joined = if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        };
        ConfigItem {
            key: "packages".into(),
            label: "Extra packages".into(),
            value: joined.clone().unwrap_or_else(|| "None".into()).into(),
            item_type: ItemType::Text,
            is_empty: joined.is_none(),
            ..Default::default()
        }
    });
    items.push({
        let joined = if c.extra_services.is_empty() {
            None
        } else {
            Some(c.extra_services.join(", "))
        };
        ConfigItem {
            key: "extra_services".into(),
            label: "Extra services".into(),
            value: joined.clone().unwrap_or_else(|| "None".into()).into(),
            item_type: ItemType::Text,
            is_empty: joined.is_none(),
            ..Default::default()
        }
    });
    items.push(ci_toggle("zrepl", "zrepl (snapshots)", c.zrepl_enabled));

    items
}

fn build_review_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = Vec::new();
    let mut errors: Vec<String> = c
        .validate_for_install()
        .iter()
        .map(ToString::to_string)
        .collect();
    errors.extend(crate::storage::issues(c));
    if !errors.is_empty() {
        items.push(section_header("Complete setup before installing"));
        for error in errors {
            items.push(ConfigItem {
                value: error.to_string().into(),
                item_type: ItemType::Warning,
                ..Default::default()
            });
        }
    }
    let mut storage_header = section_header("Storage changes during installation");
    storage_header.key = "edit:1".into();
    items.push(storage_header);
    for mut item in build_disk_items(c)
        .into_iter()
        .filter(|i| i.item_type == ItemType::Storage)
    {
        item.key = "".into();
        items.push(item);
    }
    if matches!(
        c.installation_mode,
        Some(InstallationMode::NewPool | InstallationMode::ExistingPool)
    ) && c.swap_mode.uses_partition()
    {
        let mut item = storage_item(
            "swap_partition",
            "Swap partition",
            c.swap_partition.as_deref(),
            c,
        );
        item.key = "".into();
        items.push(item);
    }
    items.push(ConfigItem {
        label: "Boot environment".into(),
        value: archinstall_zfs_core::boot_environment::BootEnvironment::new(
            c.pool_name.as_deref().unwrap_or("?"),
            &c.dataset_prefix,
        )
        .base()
        .into(),
        description: "Create this environment and its child datasets".into(),
        item_type: ItemType::Readonly,
        ..Default::default()
    });

    for (step, &label) in STEP_LABELS.iter().enumerate().take(TOTAL_STEPS - 1).skip(2) {
        let mut header = section_header(label);
        header.key = format!("edit:{step}").into();
        items.push(header);
        let step_items = build_step_items(step, c);
        let mut i = 0;
        while i < step_items.len() {
            let item = &step_items[i];
            match item.item_type {
                ItemType::RadioHeader => {
                    // Collapse `radio-header + N radio-options` into a single
                    // readonly row showing "Group: Selected option".
                    let header_label = item.label.clone();
                    let mut selected_label: SharedString = "Not set".into();
                    let mut selected_detail_model = SharedString::default();
                    let mut selected_detail_serial = SharedString::default();
                    let mut selected_detail_size = SharedString::default();
                    let mut selected_detail_transport = SharedString::default();
                    let mut selected_detail_media = SharedString::default();
                    let mut selected_is_removable = false;
                    let mut selected_persistent_path = SharedString::default();
                    let mut selected_persistent_kind = SharedString::default();
                    // Default empty: nothing selected. Overwritten when we
                    // find the selected option, taking its is_empty value.
                    let mut selected_is_empty = true;
                    i += 1;
                    while i < step_items.len()
                        && matches!(
                            step_items[i].item_type,
                            ItemType::RadioOption | ItemType::RadioSubheader
                        )
                    {
                        if step_items[i].item_type == ItemType::RadioOption
                            && step_items[i].value == "selected"
                        {
                            selected_label = step_items[i].label.clone();
                            selected_is_empty = step_items[i].is_empty;
                            selected_detail_model = step_items[i].detail_model.clone();
                            selected_detail_serial = step_items[i].detail_serial.clone();
                            selected_detail_size = step_items[i].detail_size.clone();
                            selected_detail_transport = step_items[i].detail_transport.clone();
                            selected_detail_media = step_items[i].detail_media.clone();
                            selected_is_removable = step_items[i].is_removable;
                            selected_persistent_path = step_items[i].persistent_path.clone();
                            selected_persistent_kind = step_items[i].persistent_kind.clone();
                        }
                        i += 1;
                    }
                    items.push(ConfigItem {
                        label: header_label,
                        value: selected_label,
                        detail_model: selected_detail_model,
                        detail_serial: selected_detail_serial,
                        detail_size: selected_detail_size,
                        detail_transport: selected_detail_transport,
                        detail_media: selected_detail_media,
                        is_removable: selected_is_removable,
                        persistent_path: selected_persistent_path,
                        persistent_kind: selected_persistent_kind,
                        item_type: ItemType::Readonly,
                        is_empty: selected_is_empty,
                        ..Default::default()
                    });
                }
                ItemType::SectionHeader | ItemType::Storage => {
                    // Visual section divider — the step-level header above
                    // already groups things on the review screen, so the
                    // inner divider would just produce an empty Readonly
                    // row ("Not set"). Drop it.
                    i += 1;
                }
                _ => {
                    items.push(ConfigItem {
                        key: item.key.clone(),
                        label: item.label.clone(),
                        value: if item.item_type == ItemType::InlinePassword {
                            if c.zfs_encryption_password.is_some() {
                                "Set".into()
                            } else {
                                "Required".into()
                            }
                        } else {
                            item.value.clone()
                        },
                        description: item.description.clone(),
                        item_type: ItemType::Readonly,
                        is_empty: item.is_empty,
                        ..Default::default()
                    });
                    i += 1;
                }
            }
        }
    }

    items
}

fn ci(key: &str, label: &str, value: &str, item_type: ItemType) -> ConfigItem {
    ConfigItem {
        key: key.into(),
        label: label.into(),
        value: value.into(),
        item_type,
        ..Default::default()
    }
}

/// Variant of [`ci`] that takes an `Option<&str>`. `None` is rendered as
/// "Not set" with `is_empty: true` so the Slint side colors the value muted
/// without string-matching the sentinel.
fn ci_opt(key: &str, label: &str, value: Option<&str>, item_type: ItemType) -> ConfigItem {
    let (display, is_empty) = match value {
        Some(v) => (v, false),
        None => ("Not set", true),
    };
    ConfigItem {
        key: key.into(),
        label: label.into(),
        value: display.into(),
        item_type,
        is_empty,
        ..Default::default()
    }
}

/// Toggle row helper. `enabled=false` is rendered as the "off" state with
/// `is_empty: true` so the value reads muted, matching how unset fields
/// look on the rest of the wizard.
fn ci_toggle(key: &str, label: &str, enabled: bool) -> ConfigItem {
    ConfigItem {
        key: key.into(),
        label: label.into(),
        value: if enabled { "Enabled" } else { "Disabled" }.into(),
        item_type: ItemType::Toggle,
        is_empty: !enabled,
        ..Default::default()
    }
}

#[cfg(test)]
fn sep() -> ConfigItem {
    ConfigItem {
        item_type: ItemType::Separator,
        ..Default::default()
    }
}

fn section_header(label: &str) -> ConfigItem {
    ConfigItem {
        label: label.into(),
        item_type: ItemType::SectionHeader,
        ..Default::default()
    }
}

fn mark_section_boundaries(items: &mut [ConfigItem]) {
    fn is_field(t: ItemType) -> bool {
        matches!(
            t,
            ItemType::Text
                | ItemType::Select
                | ItemType::Password
                | ItemType::Toggle
                | ItemType::RadioSubheader
                | ItemType::RadioOption
                | ItemType::Readonly
        )
    }
    // SectionHeader and RadioHeader both break sections; everything that
    // isn't a field naturally is a "non-field" and breaks the section, so
    // no extra check needed beyond is_field above (RadioHeader != any
    // field variant).

    let n = items.len();
    for i in 0..n {
        let t = items[i].item_type;
        if !is_field(t) {
            continue;
        }
        let prev_breaks = i == 0 || !is_field(items[i - 1].item_type);
        let next_breaks = i + 1 == n || !is_field(items[i + 1].item_type);
        items[i].is_first_in_section = prev_breaks;
        items[i].is_last_in_section = next_breaks;
    }
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
        if !(t == ItemType::Storage && items[idx as usize].key.is_empty())
            && t != ItemType::Separator
            && t != ItemType::Readonly
            && t != ItemType::Warning
            && t != ItemType::SectionHeader
            && t != ItemType::RadioHeader
            && t != ItemType::RadioSubheader
        {
            return idx;
        }
    }
    current
}

// ── Apply mutations ─────────────────────────────────

/// Apply an inline radio selection. `group_key` is e.g. "compression".
/// Key for a device row.
///
/// The device's path travels with the row rather than its position in the
/// list. Resolving a position means enumerating the block devices a second
/// time when the click arrives, and the set can change in between — a stick
/// plugged in, udev still settling — which silently shifts every index after
/// it. For a screen whose next step erases the chosen disk, selecting by
/// identity rather than by position is the only version that is safe to be
/// wrong about.
#[cfg(test)]
fn device_key(setting: DeviceSetting, path: &std::path::Path) -> SharedString {
    format!("device:{}:{}", setting.as_str(), path.display()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── apply_radio ─────────────────────────────────────

    #[test]
    fn a_device_row_carries_the_path_it_selects() {
        let path = std::path::Path::new("/dev/disk/by-path/pci-0000:00:04.0");
        // Colons in persistent paths must survive the round trip through the
        // key, so the dispatcher has to split on the first one only.
        let key = device_key(DeviceSetting::Disk, path);
        let rest = key.strip_prefix("device:").expect("device prefix");
        let (group, payload) = rest.split_once(':').expect("group and payload");

        assert_eq!(group, "disk");
        assert_eq!(std::path::Path::new(payload), path);
    }

    // ── apply_text ──────────────────────────────────────

    // ── next_selectable_index ───────────────────────────

    fn typed(label: &str, item_type: ItemType) -> ConfigItem {
        ConfigItem {
            key: label.into(),
            label: label.into(),
            item_type,
            ..Default::default()
        }
    }

    #[test]
    fn next_selectable_skips_non_interactive_types() {
        let items = vec![
            typed("A", ItemType::SectionHeader),
            typed("B", ItemType::RadioOption),
            typed("C", ItemType::Separator),
            typed("D", ItemType::Text),
        ];

        // From -1, going forward, the first selectable is index 1 (RadioOption)
        assert_eq!(next_selectable_index(&items, -1, 1), 1);
        // From 1, forward, skip Separator(2), land on Text(3)
        assert_eq!(next_selectable_index(&items, 1, 1), 3);
        // From 3, backward, skip Separator(2), land on RadioOption(1)
        assert_eq!(next_selectable_index(&items, 3, -1), 1);
    }

    #[test]
    fn next_selectable_wraps_around() {
        let items = vec![typed("a", ItemType::Text), typed("b", ItemType::Toggle)];
        // From last item, forward → wraps to first
        assert_eq!(next_selectable_index(&items, 1, 1), 0);
        // From first item, backward → wraps to last
        assert_eq!(next_selectable_index(&items, 0, -1), 1);
    }

    #[test]
    fn next_selectable_returns_minus_one_for_empty() {
        let items: Vec<ConfigItem> = vec![];
        assert_eq!(next_selectable_index(&items, -1, 1), -1);
        assert_eq!(next_selectable_index(&items, 5, -1), -1);
    }

    #[test]
    fn next_selectable_returns_current_when_no_interactive_items() {
        let items = vec![
            typed("", ItemType::Separator),
            typed("", ItemType::Readonly),
        ];
        assert_eq!(next_selectable_index(&items, 0, 1), 0);
    }

    // ── mark_section_boundaries ─────────────────────────

    #[test]
    fn mark_boundaries_simple_section() {
        let mut items = vec![
            section_header("Pool"),
            ci("a", "A", "", ItemType::Text),
            ci("b", "B", "", ItemType::Text),
            ci("c", "C", "", ItemType::Text),
        ];
        mark_section_boundaries(&mut items);

        // Header itself stays unmarked.
        assert!(!items[0].is_first_in_section);
        assert!(!items[0].is_last_in_section);
        // First field after header.
        assert!(items[1].is_first_in_section);
        assert!(!items[1].is_last_in_section);
        // Middle field.
        assert!(!items[2].is_first_in_section);
        assert!(!items[2].is_last_in_section);
        // Last field (end of list).
        assert!(!items[3].is_first_in_section);
        assert!(items[3].is_last_in_section);
    }

    #[test]
    fn mark_boundaries_two_adjacent_sections() {
        let mut items = vec![
            section_header("Pool"),
            ci("a", "A", "", ItemType::Text),
            section_header("Compression"),
            ci("b", "B", "", ItemType::RadioOption),
            ci("c", "C", "", ItemType::RadioOption),
        ];
        mark_section_boundaries(&mut items);

        // Pool's only field: first AND last in section.
        assert!(items[1].is_first_in_section);
        assert!(items[1].is_last_in_section);
        // First Compression option.
        assert!(items[3].is_first_in_section);
        assert!(!items[3].is_last_in_section);
        // Last Compression option.
        assert!(!items[4].is_first_in_section);
        assert!(items[4].is_last_in_section);
    }

    #[test]
    fn mark_boundaries_radio_followed_by_text_in_same_section() {
        // Encryption: 3 radio options followed by an optional password text.
        // All four belong to the same section card.
        let mut items = vec![
            section_header("Encryption"),
            ci("none", "None", "selected", ItemType::RadioOption),
            ci("pool", "Pool", "", ItemType::RadioOption),
            ci("dataset", "Dataset", "", ItemType::RadioOption),
            ci("password", "Password", "Set", ItemType::Password),
        ];
        mark_section_boundaries(&mut items);

        assert!(items[1].is_first_in_section);
        assert!(!items[1].is_last_in_section);
        assert!(!items[2].is_first_in_section);
        assert!(!items[2].is_last_in_section);
        assert!(!items[3].is_first_in_section);
        assert!(!items[3].is_last_in_section);
        assert!(!items[4].is_first_in_section);
        assert!(items[4].is_last_in_section);
    }

    #[test]
    fn mark_boundaries_separator_breaks_section() {
        let mut items = vec![
            ci("a", "A", "", ItemType::Text),
            sep(),
            ci("b", "B", "", ItemType::Text),
        ];
        mark_section_boundaries(&mut items);

        // First Text: is_first (no prev) and is_last (Separator after).
        assert!(items[0].is_first_in_section);
        assert!(items[0].is_last_in_section);
        // Second Text: is_first (Separator before) and is_last (end of list).
        assert!(items[2].is_first_in_section);
        assert!(items[2].is_last_in_section);
    }

    #[test]
    fn mark_boundaries_action_does_not_join_section() {
        // Actions are standalone, not part of a section card. A field
        // followed by an Action terminates the section.
        let mut items = vec![
            ci("a", "A", "", ItemType::Text),
            ConfigItem {
                key: "install".into(),
                label: "Install".into(),
                item_type: ItemType::Action,
                ..Default::default()
            },
        ];
        mark_section_boundaries(&mut items);
        assert!(items[0].is_first_in_section);
        assert!(items[0].is_last_in_section);
    }
}

#[cfg(test)]
mod storage_design_tests {
    use super::*;
    #[test]
    fn assignments_do_not_expand_device_catalogues() {
        for mode in [
            InstallationMode::FullDisk,
            InstallationMode::NewPool,
            InstallationMode::ExistingPool,
        ] {
            let c = GlobalConfig {
                installation_mode: Some(mode),
                ..Default::default()
            };
            let items = build_disk_items(&c);
            assert!(!items.iter().any(|i| matches!(
                i.item_type,
                ItemType::RadioOption | ItemType::RadioSubheader
            )));
            assert_eq!(
                items
                    .iter()
                    .filter(|i| i.item_type == ItemType::Storage)
                    .count(),
                if mode == InstallationMode::FullDisk {
                    1
                } else {
                    2
                }
            );
        }
    }
    #[test]
    fn review_uses_canonical_environment_path_and_never_copies_passphrases() {
        let c = GlobalConfig {
            installation_mode: Some(InstallationMode::NewPool),
            pool_name: Some("tank".into()),
            dataset_prefix: "newbe".into(),
            zfs_encryption_mode: ZfsEncryptionMode::Dataset,
            zfs_encryption_password: Some("do-not-display-this".into()),
            ..Default::default()
        };
        let items = build_review_items(&c);
        assert!(items.iter().any(|i| i.value == "tank/newbe"));
        assert!(!items.iter().any(|i| i.value.contains("do-not-display")));
        assert!(items.iter().any(|i| i.item_type == ItemType::Warning));
    }
}
