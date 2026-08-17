use archinstall_zfs_core::config::edit::{ChoiceSetting, TextSetting};
use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, MenuKind, radio_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem {
            key: "kernel",
            label: "Kernel",
            value: format!(
                "{} [{}]",
                config
                    .kernels
                    .as_ref()
                    .map(|k| k.join(", "))
                    .unwrap_or_else(|| config.primary_kernel().to_string()),
                config.zfs_module_mode
            ),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: TextSetting::Hostname.as_str(),
            label: "Hostname",
            value: config.hostname.clone().unwrap_or("Not set".into()),
            kind: MenuKind::Text,
        },
        MenuItem {
            key: TextSetting::Locale.as_str(),
            label: "Locale",
            value: config.locale.clone().unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: TextSetting::Timezone.as_str(),
            label: "Timezone",
            value: config.timezone.clone().unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "keyboard",
            label: "Keyboard layout",
            value: config.keyboard_layout.clone(),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "ntp",
            label: "NTP (time sync)",
            value: if config.ntp { "Enabled" } else { "Disabled" }.into(),
            kind: MenuKind::Toggle,
        },
    ];

    items.extend(radio_group(
        ChoiceSetting::NetworkCopyIso.as_str(),
        "Network",
        &["Copy from ISO", "Manual"],
        if config.network_copy_iso { 0 } else { 1 },
    ));

    items.push(MenuItem {
        key: TextSetting::ParallelDownloads.as_str(),
        label: "Parallel downloads",
        value: config.parallel_downloads.to_string(),
        kind: MenuKind::Text,
    });

    items
}
