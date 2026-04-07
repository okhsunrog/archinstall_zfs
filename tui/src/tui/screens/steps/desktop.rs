use archinstall_zfs_core::config::types::{AudioServer, GlobalConfig, SeatAccess};

use super::{MenuItem, MenuKind, radio_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let sel = config.profile_selection.as_ref();
    let profile_def = sel.and_then(|s| s.profile_def());

    let mut items = vec![
        MenuItem {
            key: "profile",
            label: "Profile",
            value: profile_def
                .as_ref()
                .map(|p| p.display_name.to_string())
                .unwrap_or_else(|| "Not set".into()),
            kind: MenuKind::Custom,
        },
        MenuItem {
            key: "display_manager",
            label: "Display manager",
            value: sel
                .and_then(|s| s.effective_display_manager())
                .map(|d| d.display_name().to_string())
                .unwrap_or_else(|| "Profile default".into()),
            kind: MenuKind::Custom,
        },
    ];

    items.extend(radio_group(
        "seat_access",
        "Seat access",
        &["None", "seatd", "polkit"],
        match sel.and_then(|s| s.seat_access) {
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
            key: "packages",
            label: "Extra packages",
            value: {
                let total = config.additional_packages.len() + config.aur_packages.len();
                if total == 0 {
                    "None".into()
                } else {
                    let mut parts: Vec<&str> = config
                        .additional_packages
                        .iter()
                        .map(|s| s.as_str())
                        .collect();
                    parts.extend(config.aur_packages.iter().map(|s| s.as_str()));
                    parts.join(", ")
                }
            },
            kind: MenuKind::Custom,
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
