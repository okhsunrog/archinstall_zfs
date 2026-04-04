use archinstall_zfs_core::config::types::{AudioServer, GlobalConfig};

use super::{MenuItem, MenuKind};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    vec![
        MenuItem {
            key: "profile",
            label: "Profile",
            value: config.profile.clone().unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "audio",
            label: "Audio",
            value: config.audio.map(|a| a.to_string()).unwrap_or("None".into()),
            kind: MenuKind::Select {
                options: vec!["None", "pipewire", "pulseaudio"],
                current: match config.audio {
                    None => 0,
                    Some(AudioServer::Pipewire) => 1,
                    Some(AudioServer::Pulseaudio) => 2,
                },
            },
        },
        MenuItem {
            key: "bluetooth",
            label: "Bluetooth",
            value: if config.bluetooth {
                "Enabled"
            } else {
                "Disabled"
            }
            .into(),
            kind: MenuKind::Toggle,
        },
        MenuItem {
            key: "additional_packages",
            label: "Additional packages",
            value: if config.additional_packages.is_empty() {
                "None".into()
            } else {
                config.additional_packages.join(", ")
            },
            kind: MenuKind::Text,
        },
        MenuItem {
            key: "aur_packages",
            label: "AUR packages",
            value: if config.aur_packages.is_empty() {
                "None".into()
            } else {
                config.aur_packages.join(", ")
            },
            kind: MenuKind::Text,
        },
        MenuItem {
            key: "extra_services",
            label: "Extra services",
            value: if config.extra_services.is_empty() {
                "None".into()
            } else {
                config.extra_services.join(", ")
            },
            kind: MenuKind::Text,
        },
        MenuItem {
            key: "zrepl",
            label: "zrepl (snapshots)",
            value: if config.zrepl_enabled {
                "Enabled"
            } else {
                "Disabled"
            }
            .into(),
            kind: MenuKind::Toggle,
        },
    ]
}
