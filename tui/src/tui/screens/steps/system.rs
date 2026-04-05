use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, MenuKind, radio_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem {
            key: "kernel",
            label: "Kernel",
            value: config
                .kernels
                .as_ref()
                .map(|k| k.join(", "))
                .unwrap_or_else(|| config.primary_kernel().to_string()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "hostname",
            label: "Hostname",
            value: config.hostname.clone().unwrap_or("Not set".into()),
            kind: MenuKind::Text,
        },
        MenuItem {
            key: "locale",
            label: "Locale",
            value: config.locale.clone().unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "timezone",
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
        "network",
        "Network",
        &["Copy from ISO", "Manual"],
        if config.network_copy_iso { 0 } else { 1 },
    ));

    items.push(MenuItem {
        key: "parallel_downloads",
        label: "Parallel downloads",
        value: config.parallel_downloads.to_string(),
        kind: MenuKind::Text,
    });

    items
}
