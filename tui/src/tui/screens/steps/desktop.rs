use archinstall_zfs_core::config::edit::{ChoiceSetting, TextSetting};
use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, choice_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let sel = config.profile_selection.as_ref();
    let profile_def = sel.and_then(|s| s.profile_def());

    let mut items = vec![
        MenuItem::custom(
            "profile",
            "Profile",
            profile_def
                .as_ref()
                .map(|p| p.display_name.to_string())
                .unwrap_or_else(|| "Not set".into()),
        ),
        MenuItem::custom(
            "display_manager",
            "Display manager",
            sel.and_then(|s| s.effective_display_manager())
                .map(|d| d.display_name().to_string())
                .unwrap_or_else(|| "Profile default".into()),
        ),
    ];

    items.extend(choice_group(
        ChoiceSetting::SeatAccess,
        "Seat access",
        sel.and_then(|s| s.seat_access),
    ));

    // GPU driver is only meaningful for graphical profiles. Hide the row
    // for Server / Minimal selections to keep the step focused.
    if profile_def
        .as_ref()
        .is_some_and(|p| p.supports_gfx_driver())
    {
        items.push(MenuItem::custom(
            "gpu_driver",
            "GPU driver",
            config
                .gfx_driver
                .map(|d| d.to_string())
                .unwrap_or("None".into()),
        ));
    }

    items.extend(choice_group(ChoiceSetting::Audio, "Audio", config.audio));

    items.extend([
        MenuItem::toggle("bluetooth", "Bluetooth", config.bluetooth),
        MenuItem::custom("packages", "Extra packages", {
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
        }),
        MenuItem::text(
            TextSetting::ExtraServices.as_str(),
            "Extra services",
            if config.extra_services.is_empty() {
                "None".into()
            } else {
                config.extra_services.join(", ")
            },
        ),
        MenuItem::toggle("zrepl", "zrepl (snapshots)", config.zrepl_enabled),
    ]);

    items
}
