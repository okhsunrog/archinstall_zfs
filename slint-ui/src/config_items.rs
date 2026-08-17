//! Build the per-step `Vec<ConfigItem>` shown in the wizard, and apply edits
//! coming back from radio/select/text widgets to the canonical `GlobalConfig`.

use slint::SharedString;
use std::path::PathBuf;

use archinstall_zfs_core::config::choices::Choice;
use archinstall_zfs_core::config::edit::{
    ChoiceSetting, DeviceSetting, EditorSetting, TextSetting,
};
use archinstall_zfs_core::config::types::{
    CompressionAlgo, GlobalConfig, InstallationMode, SwapMode, ZfsEncryptionMode,
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

    let mut items = choice_group(
        ChoiceSetting::InstallationMode,
        "Installation mode",
        mode.unwrap_or(InstallationMode::FullDisk),
    );

    if matches!(mode, Some(InstallationMode::FullDisk) | None) {
        let disks = disk_choices();
        let selected = c
            .disk
            .as_ref()
            .and_then(|sel| disks.iter().position(|choice| &choice.path == sel))
            .map(|i| i as i32)
            .unwrap_or(-1);
        items.extend(radio_choice_group(
            DeviceSetting::Disk,
            "Disk",
            &disks,
            selected,
        ));
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
            DeviceSetting::EfiPartition,
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
                DeviceSetting::ZfsPartition,
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
            TextSetting::PoolName.as_str(),
            "Pool name",
            c.pool_name.as_deref(),
            ItemType::Text,
        ),
        ci(
            TextSetting::DatasetPrefix.as_str(),
            "Dataset prefix",
            &c.dataset_prefix,
            ItemType::Text,
        ),
    ];

    items.extend(choice_group_with_off(
        ChoiceSetting::Compression,
        "Compression",
        c.compression,
        CompressionAlgo::Off,
    ));

    items.extend(choice_group(
        ChoiceSetting::Encryption,
        "Encryption",
        c.zfs_encryption_mode,
    ));

    if c.zfs_encryption_mode != ZfsEncryptionMode::None {
        items.push(ci_opt(
            TextSetting::EncryptionPassword.as_str(),
            "Encryption password",
            c.zfs_encryption_password.as_ref().map(|_| "Set"),
            ItemType::Password,
        ));
    }

    items.extend(choice_group_with_off(
        ChoiceSetting::SwapMode,
        "Swap",
        c.swap_mode,
        SwapMode::None,
    ));

    if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
        items.push(ci_opt(
            TextSetting::SwapPartitionSize.as_str(),
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
            DeviceSetting::SwapPartition,
            "Swap partition",
            &parts,
            swap_selected,
        ));
    }

    items.extend(choice_group(
        ChoiceSetting::InitSystem,
        "Init system",
        c.init_system,
    ));

    items
}

fn build_system_items(c: &GlobalConfig) -> Vec<ConfigItem> {
    vec![
        section_header("System"),
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
        section_header("Locale"),
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
            items.extend(choice_group_with_off(
                ChoiceSetting::SeatAccess,
                "Seat access",
                sel.seat_access,
                None,
            ));
        }
    }

    items.extend(choice_group_with_off(
        ChoiceSetting::Audio,
        "Audio",
        c.audio,
        None,
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
                value: error.to_string().into(),
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

/// Build a radio group from a [`Choice`] enum, so the order, the labels and
/// the selected index all come from one table rather than being spelled out
/// here and inverted again in [`apply_radio`].
fn choice_group<T: Choice>(setting: ChoiceSetting, label: &str, current: T) -> Vec<ConfigItem> {
    radio_group(
        setting.as_str(),
        label,
        &T::labels(),
        current.index() as i32,
    )
}

/// [`choice_group`] for lists with a semantic "off" alternative, named by
/// value rather than by index.
fn choice_group_with_off<T: Choice>(
    setting: ChoiceSetting,
    label: &str,
    current: T,
    off: T,
) -> Vec<ConfigItem> {
    radio_group_with_off(
        setting.as_str(),
        label,
        &T::labels(),
        current.index() as i32,
        off.index(),
    )
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
    setting: DeviceSetting,
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
            key: device_key(setting, &option.path),
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
    setting: DeviceSetting,
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
            key: device_key(setting, &option.path),
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
/// Key for a device row.
///
/// The device's path travels with the row rather than its position in the
/// list. Resolving a position means enumerating the block devices a second
/// time when the click arrives, and the set can change in between — a stick
/// plugged in, udev still settling — which silently shifts every index after
/// it. For a screen whose next step erases the chosen disk, selecting by
/// identity rather than by position is the only version that is safe to be
/// wrong about.
fn device_key(setting: DeviceSetting, path: &std::path::Path) -> SharedString {
    format!("device:{}:{}", setting.as_str(), path.display()).into()
}

fn disk_choices() -> Vec<ChoiceRow> {
    archinstall_zfs_core::disk::device::disk_choices()
        .map(|choices| choices.into_iter().map(ChoiceRow::from).collect())
        .unwrap_or_default()
}

fn partition_choices() -> Vec<ChoiceRow> {
    archinstall_zfs_core::disk::device::partition_choices()
        .map(|choices| choices.into_iter().map(ChoiceRow::from).collect())
        .unwrap_or_default()
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
