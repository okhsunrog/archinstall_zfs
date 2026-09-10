use archinstall_zfs_core::config::edit::{ChoiceSetting, EditorSetting, TextSetting};
use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, radio_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = vec![
        // Before the kernel: which kernels exist depends on the answer here.
        MenuItem::custom(
            EditorSetting::Distribution.as_str(),
            "Distribution",
            config.distribution().display_name.to_string(),
        ),
        MenuItem::custom(
            EditorSetting::Kernel.as_str(),
            "Kernel",
            format!(
                "{} [{}]",
                config
                    .kernels
                    .as_ref()
                    .map(|k| k.join(", "))
                    .unwrap_or_else(|| config.primary_kernel().to_string()),
                config.zfs_module_mode
            ),
        ),
        MenuItem::text(
            TextSetting::Hostname.as_str(),
            "Hostname",
            config.hostname.clone().unwrap_or("Not set".into()),
        ),
        MenuItem::custom(
            EditorSetting::Locale.as_str(),
            "Locale",
            config.locale.clone().unwrap_or("Not set".into()),
        ),
        MenuItem::custom(
            EditorSetting::Timezone.as_str(),
            "Timezone",
            config.timezone.clone().unwrap_or("Not set".into()),
        ),
        MenuItem::custom(
            EditorSetting::Keyboard.as_str(),
            "Keyboard layout",
            config.keyboard_layout.clone(),
        ),
        MenuItem::toggle("ntp", "NTP (time sync)", config.ntp),
    ];

    items.extend(radio_group(
        ChoiceSetting::NetworkCopyIso.as_str(),
        "Network",
        &["Copy from ISO", "Manual"],
        if config.network_copy_iso { 0 } else { 1 },
    ));

    items.push(MenuItem::text(
        TextSetting::ParallelDownloads.as_str(),
        "Parallel downloads",
        config.parallel_downloads.to_string(),
    ));

    items
}
