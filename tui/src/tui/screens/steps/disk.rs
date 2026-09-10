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

    // The graphical installer owns alongside planning. A plan loaded from a
    // configuration file is shown so the page is not blank; it cannot be
    // edited here.
    if matches!(mode, Some(InstallationMode::Alongside)) {
        items.push(MenuItem {
            key: "",
            label: "Alongside plan",
            value: config
                .alongside
                .as_ref()
                .map(alongside_summary)
                .unwrap_or_else(|| "Missing; create it with the graphical installer (azfs)".into()),
            kind: MenuKind::SectionHeader,
        });
    }

    items
}

fn alongside_summary(request: &archinstall_zfs_core::disk::alongside::Request) -> String {
    use archinstall_zfs_core::disk::alongside::{EfiChoice, SpaceSource};
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let source = match request.source {
        SpaceSource::Shrink { partition } => format!("shrink partition {partition}"),
        SpaceSource::Unallocated { .. } => "unallocated space".to_string(),
    };
    let efi = match request.efi {
        EfiChoice::Reuse { partition } => format!("reuse EFI partition {partition}"),
        EfiChoice::CreateSeparate { .. } => "separate EFI partition".to_string(),
    };
    format!(
        "{}: {source}, {:.1} GiB for Linux, swap {:.0} GiB, {efi}",
        request.before.device.display(),
        request.allocation_bytes as f64 / GIB,
        request.swap_bytes as f64 / GIB
    )
}
