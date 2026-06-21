//! Build the per-step `Vec<ConfigItem>` shown in the wizard, and apply edits
//! coming back from radio/select/text widgets to the canonical `GlobalConfig`.

use slint::SharedString;
use std::path::PathBuf;

use archinstall_zfs_core::config::types::{
    AudioServer, CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, ProfileSelection,
    SeatAccess, SwapMode, ZfsEncryptionMode,
};
use archinstall_zfs_core::disk::device::DeviceChoice;

use crate::ui::{ConfigItem, ItemType};

pub const TOTAL_STEPS: usize = 7;

pub const STEP_LABELS: [&str; TOTAL_STEPS] = [
    "Welcome", "Disk", "ZFS", "System", "Users", "Desktop", "Review",
];

#[derive(Debug, Clone)]
struct ChoiceRow {
    path: PathBuf,
    label: String,
    icon: String,
    model: String,
    serial: String,
    size: String,
    transport: String,
    media: String,
    removable: bool,
    persistent_path: String,
    persistent_kind: String,
    group_label: String,
    group_model: String,
    group_serial: String,
    group_size: String,
    group_transport: String,
    group_media: String,
    group_removable: bool,
}

impl From<DeviceChoice> for ChoiceRow {
    fn from(choice: DeviceChoice) -> Self {
        Self {
            path: choice.path,
            label: choice.label,
            icon: choice.icon,
            model: choice.model,
            serial: choice.serial,
            size: choice.size,
            transport: choice.transport,
            media: choice.media,
            removable: choice.removable,
            persistent_path: choice.persistent_path,
            persistent_kind: choice.persistent_kind,
            group_label: choice.group_label,
            group_model: choice.group_model,
            group_serial: choice.group_serial,
            group_size: choice.group_size,
            group_transport: choice.group_transport,
            group_media: choice.group_media,
            group_removable: choice.group_removable,
        }
    }
}

impl ChoiceRow {
    fn path_only(path: PathBuf) -> Self {
        let label = path.display().to_string();
        Self {
            path,
            label,
            icon: "hard-drive".to_string(),
            model: String::new(),
            serial: String::new(),
            size: String::new(),
            transport: String::new(),
            media: String::new(),
            removable: false,
            persistent_path: String::new(),
            persistent_kind: String::new(),
            group_label: String::new(),
            group_model: String::new(),
            group_serial: String::new(),
            group_size: String::new(),
            group_transport: String::new(),
            group_media: String::new(),
            group_removable: false,
        }
    }

    fn group_key(&self) -> &str {
        if self.group_label.is_empty() {
            self.label.as_str()
        } else {
            self.group_label.as_str()
        }
    }
}

// ── Per-step item building ──────────────────────────

pub fn build_step_items(step: usize, c: &GlobalConfig) -> Vec<ConfigItem> {
    let mut items = match step {
        0 => build_welcome_items(c),
        1 => build_disk_items(c),
        2 => build_zfs_items(c),
        3 => build_system_items(c),
        4 => build_users_items(c),
        5 => build_desktop_items(c),
        6 => build_review_items(c),
        _ => vec![],
    };
    mark_section_boundaries(&mut items);
    items
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
            None => 0,
        },
    );

    if matches!(mode, Some(InstallationMode::FullDisk) | None) {
        let disks = disk_choices();
        let selected = c
            .disk
            .as_ref()
            .and_then(|sel| disks.iter().position(|choice| &choice.path == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_choice_group("disk", "Disk", &disks, selected));
    }

    if matches!(
        mode,
        Some(InstallationMode::NewPool) | Some(InstallationMode::ExistingPool)
    ) {
        let parts = partition_choices();

        let efi_selected = c
            .efi_partition
            .as_ref()
            .and_then(|sel| parts.iter().position(|choice| &choice.path == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_partition_choice_group(
            "efi_partition",
            "EFI partition",
            &parts,
            efi_selected,
        ));

        if matches!(mode, Some(InstallationMode::NewPool)) {
            let zfs_selected = c
                .zfs_partition
                .as_ref()
                .and_then(|sel| parts.iter().position(|choice| &choice.path == sel))
                .map(|i| i as i32)
                .unwrap_or(-1);
            items.extend(radio_partition_choice_group(
                "zfs_partition",
                "ZFS partition",
                &parts,
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
        section_header("Pool"),
        ci_opt(
            "pool_name",
            "Pool name",
            c.pool_name.as_deref(),
            ItemType::Text,
        ),
        ci(
            "dataset_prefix",
            "Dataset prefix",
            &c.dataset_prefix,
            ItemType::Text,
        ),
    ];

    items.extend(radio_group_with_off(
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
        4,
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
        items.push(ci_opt(
            "encryption_password",
            "Encryption password",
            c.zfs_encryption_password.as_ref().map(|_| "Set"),
            ItemType::Password,
        ));
    }

    items.extend(radio_group_with_off(
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
        0,
    ));

    if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
        items.push(ci_opt(
            "swap_partition_size",
            "Swap size",
            c.swap_partition_size.as_deref(),
            ItemType::Text,
        ));
    }
    if !matches!(mode, Some(InstallationMode::FullDisk) | None) && has_swap_partition {
        let parts = partition_choices();
        let swap_selected = c
            .swap_partition
            .as_ref()
            .and_then(|sel| parts.iter().position(|choice| &choice.path == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_choice_group(
            "swap_partition",
            "Swap partition",
            &parts,
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
    vec![
        section_header("System"),
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
        ci_opt(
            "hostname",
            "Hostname",
            c.hostname.as_deref(),
            ItemType::Text,
        ),
        ci_toggle("ntp", "NTP (time sync)", c.ntp),
        ci(
            "parallel_downloads",
            "Parallel downloads",
            &c.parallel_downloads.to_string(),
            ItemType::Text,
        ),
        section_header("Locale"),
        ci_opt("locale", "Locale", c.locale.as_deref(), ItemType::Select),
        ci_opt(
            "timezone",
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
            "root_password",
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
        section_header("Desktop"),
        ConfigItem {
            key: "profile".into(),
            label: "Profile".into(),
            value: profile_name.clone().unwrap_or_else(|| "None".into()).into(),
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

        // Seat access (Wayland compositors). Its own section card via
        // radio_group, like Audio.
        if p.needs_seat_access() {
            items.extend(radio_group_with_off(
                "seat_access",
                "Seat access",
                &["None", "seatd", "polkit"],
                match sel.seat_access {
                    None => 0,
                    Some(SeatAccess::Seatd) => 1,
                    Some(SeatAccess::Polkit) => 2,
                },
                0,
            ));
        }
    }

    items.extend(radio_group_with_off(
        "audio",
        "Audio",
        &["None", "pipewire", "pulseaudio"],
        match c.audio {
            None => 0,
            Some(AudioServer::Pipewire) => 1,
            Some(AudioServer::Pulseaudio) => 2,
        },
        0,
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

    for (step, &label) in STEP_LABELS.iter().enumerate().take(TOTAL_STEPS - 1) {
        // Each step becomes a section in the review screen.
        items.push(section_header(label));

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
                ItemType::SectionHeader => {
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
                        value: item.value.clone(),
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

    let errors = c.validate_for_install();
    if !errors.is_empty() {
        items.push(section_header("Validation"));
        for error in &errors {
            items.push(ConfigItem {
                value: error.as_str().into(),
                item_type: ItemType::Warning,
                ..Default::default()
            });
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

/// Emit a radio group: a `RadioHeader` followed by clickable `RadioOption`
/// rows. The header is a distinct `ItemType` from a plain `SectionHeader`
/// so the review screen knows to collapse the header + options into one
/// summary row, while bare section headers (used as visual dividers) get
/// dropped in review entirely.
fn radio_group(key: &str, label: &str, options: &[&str], selected: i32) -> Vec<ConfigItem> {
    radio_group_inner(key, label, options, selected, None)
}

/// Variant of [`radio_group`] that marks one option as the semantic "off"
/// state (e.g. compression "off", audio "None"). The off row's `is_empty`
/// flag is propagated to the review screen's collapsed Readonly row when
/// it's the selected option, so it renders muted instead of green.
fn radio_group_with_off(
    key: &str,
    label: &str,
    options: &[&str],
    selected: i32,
    off_index: usize,
) -> Vec<ConfigItem> {
    radio_group_inner(key, label, options, selected, Some(off_index))
}

fn radio_group_inner(
    key: &str,
    label: &str,
    options: &[&str],
    selected: i32,
    off_index: Option<usize>,
) -> Vec<ConfigItem> {
    let mut items = vec![ConfigItem {
        label: label.into(),
        item_type: ItemType::RadioHeader,
        ..Default::default()
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
            is_empty: off_index == Some(i),
            ..Default::default()
        });
    }
    items
}

fn radio_choice_group(
    key: &str,
    label: &str,
    options: &[ChoiceRow],
    selected: i32,
) -> Vec<ConfigItem> {
    let mut items = vec![ConfigItem {
        label: label.into(),
        item_type: ItemType::RadioHeader,
        ..Default::default()
    }];
    for (i, option) in options.iter().enumerate() {
        items.push(ConfigItem {
            key: format!("radio:{key}:{i}").into(),
            label: option.label.as_str().into(),
            icon: option.icon.as_str().into(),
            detail_model: option.model.as_str().into(),
            detail_serial: option.serial.as_str().into(),
            detail_size: option.size.as_str().into(),
            detail_transport: option.transport.as_str().into(),
            detail_media: option.media.as_str().into(),
            is_removable: option.removable,
            persistent_path: option.persistent_path.as_str().into(),
            persistent_kind: option.persistent_kind.as_str().into(),
            group_label: option.group_label.as_str().into(),
            group_model: option.group_model.as_str().into(),
            group_serial: option.group_serial.as_str().into(),
            group_size: option.group_size.as_str().into(),
            group_transport: option.group_transport.as_str().into(),
            group_media: option.group_media.as_str().into(),
            group_removable: option.group_removable,
            value: if i as i32 == selected {
                "selected".into()
            } else {
                SharedString::default()
            },
            item_type: ItemType::RadioOption,
            ..Default::default()
        });
    }
    items
}

fn radio_partition_choice_group(
    key: &str,
    label: &str,
    options: &[ChoiceRow],
    selected: i32,
) -> Vec<ConfigItem> {
    let mut items = vec![ConfigItem {
        label: label.into(),
        item_type: ItemType::RadioHeader,
        ..Default::default()
    }];
    let mut current_group = "";

    for (i, option) in options.iter().enumerate() {
        let group_key = option.group_key();
        if group_key != current_group {
            current_group = group_key;
            items.push(ConfigItem {
                label: option.group_key().into(),
                icon: "hard-drive".into(),
                detail_model: option.group_model.as_str().into(),
                detail_serial: option.group_serial.as_str().into(),
                detail_size: option.group_size.as_str().into(),
                detail_transport: option.group_transport.as_str().into(),
                detail_media: option.group_media.as_str().into(),
                is_removable: option.group_removable,
                item_type: ItemType::RadioSubheader,
                ..Default::default()
            });
        }

        items.push(ConfigItem {
            key: format!("radio:{key}:{i}").into(),
            label: option.label.as_str().into(),
            detail_size: option.size.as_str().into(),
            persistent_path: option.persistent_path.as_str().into(),
            persistent_kind: option.persistent_kind.as_str().into(),
            group_label: option.group_label.as_str().into(),
            group_model: option.group_model.as_str().into(),
            group_serial: option.group_serial.as_str().into(),
            group_size: option.group_size.as_str().into(),
            group_transport: option.group_transport.as_str().into(),
            group_media: option.group_media.as_str().into(),
            group_removable: option.group_removable,
            value: if i as i32 == selected {
                "selected".into()
            } else {
                SharedString::default()
            },
            item_type: ItemType::RadioOption,
            ..Default::default()
        });
    }
    items
}

// ── Section boundary marking ────────────────────────

/// Walk a list of items after it's built and set `is_first_in_section` /
/// `is_last_in_section` on each field row, based on adjacent SectionHeaders
/// and Separators. Field types (text/select/password/toggle/radio-option/
/// readonly) are part of section cards; everything else is a standalone
/// element and gets neither flag set.
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
        if t != ItemType::Separator
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
pub fn apply_radio(config: &mut GlobalConfig, group_key: &str, idx: i32) {
    match group_key {
        "installation_mode" => {
            let new_mode = match idx {
                0 => InstallationMode::FullDisk,
                1 => InstallationMode::NewPool,
                _ => InstallationMode::ExistingPool,
            };
            if config.installation_mode != Some(new_mode) {
                config.disk = None;
                config.efi_partition = None;
                config.zfs_partition = None;
                config.swap_partition = None;
            }
            config.installation_mode = Some(new_mode);
        }
        "disk" => {
            let disks = disk_choices();
            if let Some(choice) = disks.get(idx as usize) {
                config.installation_mode = Some(InstallationMode::FullDisk);
                config.disk = Some(choice.path.clone());
            }
        }
        "efi_partition" => {
            let parts = partition_choices();
            if let Some(choice) = parts.get(idx as usize) {
                config.efi_partition = Some(choice.path.clone());
            }
        }
        "zfs_partition" => {
            let parts = partition_choices();
            if let Some(choice) = parts.get(idx as usize) {
                config.zfs_partition = Some(choice.path.clone());
            }
        }
        "swap_partition" => {
            let parts = partition_choices();
            if let Some(choice) = parts.get(idx as usize) {
                config.swap_partition = Some(choice.path.clone());
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
            config.profile_selection = if idx == 0 {
                None
            } else {
                profiles
                    .get((idx - 1) as usize)
                    .and_then(|p| ProfileSelection::new(p.name))
            };
        }
        "audio" => {
            config.audio = match idx {
                0 => None,
                1 => Some(AudioServer::Pipewire),
                _ => Some(AudioServer::Pulseaudio),
            }
        }
        "seat_access" => {
            if let Some(sel) = config.profile_selection.as_mut() {
                sel.seat_access = match idx {
                    0 => None,
                    1 => Some(SeatAccess::Seatd),
                    _ => Some(SeatAccess::Polkit),
                };
            }
        }
        _ => {}
    }
}

fn disk_choices() -> Vec<ChoiceRow> {
    archinstall_zfs_core::disk::device::disk_choices()
        .map(|choices| choices.into_iter().map(ChoiceRow::from).collect())
        .unwrap_or_else(|_| {
            archinstall_zfs_core::disk::by_id::list_disks_by_id()
                .unwrap_or_default()
                .into_iter()
                .map(ChoiceRow::path_only)
                .collect()
        })
}

fn partition_choices() -> Vec<ChoiceRow> {
    archinstall_zfs_core::disk::device::partition_choices()
        .map(|choices| choices.into_iter().map(ChoiceRow::from).collect())
        .unwrap_or_else(|_| {
            archinstall_zfs_core::disk::by_id::list_partitions_by_id()
                .unwrap_or_default()
                .into_iter()
                .map(ChoiceRow::path_only)
                .collect()
        })
}

pub fn apply_text(config: &mut GlobalConfig, key: &str, val: &str) {
    let opt = if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    };
    match key {
        "pool_name" => config.pool_name = opt,
        "dataset_prefix" if !val.is_empty() => {
            config.dataset_prefix = val.to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GlobalConfig {
        GlobalConfig::default()
    }

    // ── apply_radio ─────────────────────────────────────

    #[test]
    fn radio_installation_mode_sets_and_clears_dependents() {
        let mut c = cfg();
        c.disk = Some("/dev/sda".into());
        c.efi_partition = Some("/dev/sda1".into());

        // Switching mode should clear all device selections
        apply_radio(&mut c, "installation_mode", 1);
        assert_eq!(c.installation_mode, Some(InstallationMode::NewPool));
        assert!(c.disk.is_none());
        assert!(c.efi_partition.is_none());
        assert!(c.zfs_partition.is_none());
        assert!(c.swap_partition.is_none());
    }

    #[test]
    fn radio_installation_mode_no_clear_when_unchanged() {
        let mut c = cfg();
        c.installation_mode = Some(InstallationMode::FullDisk);
        c.disk = Some("/dev/sda".into());

        apply_radio(&mut c, "installation_mode", 0); // Same mode (FullDisk)
        assert_eq!(c.installation_mode, Some(InstallationMode::FullDisk));
        // Selections should be preserved when mode doesn't actually change
        assert!(c.disk.is_some());
    }

    #[test]
    fn radio_installation_mode_indices() {
        let cases = [
            (0, InstallationMode::FullDisk),
            (1, InstallationMode::NewPool),
            (2, InstallationMode::ExistingPool),
        ];
        for (idx, expected) in cases {
            let mut c = cfg();
            apply_radio(&mut c, "installation_mode", idx);
            assert_eq!(c.installation_mode, Some(expected), "idx={idx}");
        }
    }

    #[test]
    fn radio_compression_indices() {
        let cases = [
            (0, CompressionAlgo::Lz4),
            (1, CompressionAlgo::Zstd),
            (2, CompressionAlgo::Zstd5),
            (3, CompressionAlgo::Zstd10),
            (4, CompressionAlgo::Off),
        ];
        for (idx, expected) in cases {
            let mut c = cfg();
            apply_radio(&mut c, "compression", idx);
            assert_eq!(c.compression, expected, "idx={idx}");
        }
    }

    #[test]
    fn radio_encryption_clears_password_when_set_to_none() {
        let mut c = cfg();
        c.zfs_encryption_mode = ZfsEncryptionMode::Pool;
        c.zfs_encryption_password = Some("hunter2".into());

        apply_radio(&mut c, "encryption", 0); // None
        assert_eq!(c.zfs_encryption_mode, ZfsEncryptionMode::None);
        assert!(c.zfs_encryption_password.is_none());
    }

    #[test]
    fn radio_encryption_keeps_password_when_set_to_pool() {
        let mut c = cfg();
        c.zfs_encryption_mode = ZfsEncryptionMode::Dataset;
        c.zfs_encryption_password = Some("hunter2".into());

        apply_radio(&mut c, "encryption", 1); // Pool
        assert_eq!(c.zfs_encryption_mode, ZfsEncryptionMode::Pool);
        assert_eq!(c.zfs_encryption_password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn radio_swap_mode_indices() {
        let cases = [
            (0, SwapMode::None),
            (1, SwapMode::Zram),
            (2, SwapMode::ZswapPartition),
            (3, SwapMode::ZswapPartitionEncrypted),
        ];
        for (idx, expected) in cases {
            let mut c = cfg();
            apply_radio(&mut c, "swap_mode", idx);
            assert_eq!(c.swap_mode, expected, "idx={idx}");
        }
    }

    #[test]
    fn radio_init_system_indices() {
        let mut c = cfg();
        apply_radio(&mut c, "init_system", 0);
        assert_eq!(c.init_system, InitSystem::Dracut);
        apply_radio(&mut c, "init_system", 1);
        assert_eq!(c.init_system, InitSystem::Mkinitcpio);
    }

    #[test]
    fn radio_audio_indices() {
        let cases = [
            (0, None),
            (1, Some(AudioServer::Pipewire)),
            (2, Some(AudioServer::Pulseaudio)),
        ];
        for (idx, expected) in cases {
            let mut c = cfg();
            apply_radio(&mut c, "audio", idx);
            assert_eq!(c.audio, expected, "idx={idx}");
        }
    }

    #[test]
    fn radio_unknown_key_is_noop() {
        let mut c = cfg();
        let before_mode = c.installation_mode;
        let before_compression = c.compression;
        let before_swap = c.swap_mode;
        apply_radio(&mut c, "totally_made_up", 5);
        assert_eq!(c.installation_mode, before_mode);
        assert_eq!(c.compression, before_compression);
        assert_eq!(c.swap_mode, before_swap);
    }

    // ── apply_text ──────────────────────────────────────

    #[test]
    fn text_pool_name_sets_and_clears() {
        let mut c = cfg();
        apply_text(&mut c, "pool_name", "rpool");
        assert_eq!(c.pool_name.as_deref(), Some("rpool"));

        // Empty string clears the field
        apply_text(&mut c, "pool_name", "");
        assert!(c.pool_name.is_none());
    }

    #[test]
    fn text_dataset_prefix_does_not_clear_on_empty() {
        let mut c = cfg();
        let original = c.dataset_prefix.clone();
        apply_text(&mut c, "dataset_prefix", "myprefix");
        assert_eq!(c.dataset_prefix, "myprefix");

        // Empty string is a no-op (must not blank the prefix)
        apply_text(&mut c, "dataset_prefix", "");
        assert_eq!(c.dataset_prefix, "myprefix");

        // Restoring the default works
        apply_text(&mut c, "dataset_prefix", &original);
        assert_eq!(c.dataset_prefix, original);
    }

    #[test]
    fn text_hostname_sets_and_clears() {
        let mut c = cfg();
        apply_text(&mut c, "hostname", "archbox");
        assert_eq!(c.hostname.as_deref(), Some("archbox"));
        apply_text(&mut c, "hostname", "");
        assert!(c.hostname.is_none());
    }

    #[test]
    fn text_timezone_sets_and_clears() {
        let mut c = cfg();
        apply_text(&mut c, "timezone", "Europe/Berlin");
        assert_eq!(c.timezone.as_deref(), Some("Europe/Berlin"));
        apply_text(&mut c, "timezone", "");
        assert!(c.timezone.is_none());
    }

    #[test]
    fn text_root_password_sets_and_clears() {
        let mut c = cfg();
        apply_text(&mut c, "root_password", "hunter2");
        assert_eq!(c.root_password.as_deref(), Some("hunter2"));
        apply_text(&mut c, "root_password", "");
        assert!(c.root_password.is_none());
    }

    #[test]
    fn text_encryption_password_sets_and_clears() {
        let mut c = cfg();
        apply_text(&mut c, "encryption_password", "secret");
        assert_eq!(c.zfs_encryption_password.as_deref(), Some("secret"));
        apply_text(&mut c, "encryption_password", "");
        assert!(c.zfs_encryption_password.is_none());
    }

    #[test]
    fn text_parallel_downloads_clamps_and_rejects_garbage() {
        let mut c = cfg();

        apply_text(&mut c, "parallel_downloads", "8");
        assert_eq!(c.parallel_downloads, 8);

        // Above 20 → clamped to 20
        apply_text(&mut c, "parallel_downloads", "100");
        assert_eq!(c.parallel_downloads, 20);

        // Zero → clamped to 1
        apply_text(&mut c, "parallel_downloads", "0");
        assert_eq!(c.parallel_downloads, 1);

        // Garbage / non-numeric → previous value preserved
        c.parallel_downloads = 5;
        apply_text(&mut c, "parallel_downloads", "abc");
        assert_eq!(c.parallel_downloads, 5);

        // Negative parses fail (it's u32) → preserved
        apply_text(&mut c, "parallel_downloads", "-3");
        assert_eq!(c.parallel_downloads, 5);
    }

    #[test]
    fn text_unknown_key_is_noop() {
        let mut c = cfg();
        let before_pool = c.pool_name.clone();
        let before_hostname = c.hostname.clone();
        let before_parallel = c.parallel_downloads;
        apply_text(&mut c, "totally_made_up", "value");
        assert_eq!(c.pool_name, before_pool);
        assert_eq!(c.hostname, before_hostname);
        assert_eq!(c.parallel_downloads, before_parallel);
    }

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
