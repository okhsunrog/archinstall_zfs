use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode};

use super::{MenuItem, MenuKind};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mode = config.installation_mode;
    let mut items = Vec::new();

    // Show disk picker for FullDisk mode
    if matches!(mode, Some(InstallationMode::FullDisk) | None) {
        items.push(MenuItem {
            key: "disk_by_id",
            label: "Disk",
            value: config
                .disk_by_id
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
            key: "efi_partition",
            label: "EFI partition",
            value: config
                .efi_partition_by_id
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }
    if matches!(mode, Some(InstallationMode::NewPool)) {
        items.push(MenuItem {
            key: "zfs_partition",
            label: "ZFS partition",
            value: config
                .zfs_partition_by_id
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }

    items
}
