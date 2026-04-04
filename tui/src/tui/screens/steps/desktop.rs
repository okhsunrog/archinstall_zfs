use archinstall_zfs_core::config::types::{AudioServer, GlobalConfig, SeatAccess};

use super::{MenuItem, MenuKind, radio_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem {
            key: "profile",
            label: "Profile",
            value: config
                .profile
                .as_deref()
                .and_then(archinstall_zfs_core::profile::get_profile)
                .map(|p| p.display_name.to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "display_manager",
            label: "Display manager",
            value: config.display_manager_override.clone().unwrap_or_else(|| {
                config
                    .profile
                    .as_deref()
                    .and_then(archinstall_zfs_core::profile::get_profile)
                    .and_then(|p| p.display_manager().map(str::to_string))
                    .unwrap_or("Profile default".into())
            }),
            kind: MenuKind::Custom,
        },
    ];

    items.extend(radio_group(
        "seat_access",
        "Seat access",
        &["None", "seatd", "polkit"],
        match config.seat_access {
            None => 0,
            Some(SeatAccess::Seatd) => 1,
            Some(SeatAccess::Polkit) => 2,
        },
    ));

    items.push(MenuItem {
        key: "gpu_driver",
        label: "GPU driver",
        value: config
            .gfx_driver
            .map(|d| d.to_string())
            .unwrap_or("None".into()),
        kind: MenuKind::Custom,
    });

    items.extend(radio_group(
        "audio",
        "Audio",
        &["None", "pipewire", "pulseaudio"],
        match config.audio {
            None => 0,
            Some(AudioServer::Pipewire) => 1,
            Some(AudioServer::Pulseaudio) => 2,
        },
    ));

    items.extend([
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
    ]);

    items
}
