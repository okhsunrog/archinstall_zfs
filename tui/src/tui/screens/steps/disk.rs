use archinstall_zfs_core::config::edit::DeviceSetting;
use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode};

use super::{MenuItem, MenuKind};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mode = config.installation_mode;
    let mut items = Vec::new();

    // Show disk picker for FullDisk mode
    if matches!(mode, Some(InstallationMode::FullDisk) | None) {
        items.push(MenuItem {
            key: DeviceSetting::Disk.as_str(),
            label: "Disk",
            value: config
                .disk
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }

    // Show partition pickers for NewPool/ExistingPool
    if matches!(
        mode,
        Some(InstallationMode::NewPool) | Some(InstallationMode::ExistingPool)
    ) {
        items.push(MenuItem {
            key: DeviceSetting::EfiPartition.as_str(),
            label: "EFI partition",
            value: config
                .efi_partition
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }
    if matches!(mode, Some(InstallationMode::NewPool)) {
        items.push(MenuItem {
            key: DeviceSetting::ZfsPartition.as_str(),
            label: "ZFS partition",
            value: config
                .zfs_partition
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }

    items
}
