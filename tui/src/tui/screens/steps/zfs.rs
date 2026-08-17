use archinstall_zfs_core::config::types::{
    GlobalConfig, InstallationMode, SwapMode, ZfsEncryptionMode,
};

use super::{MenuItem, MenuKind, choice_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mode = config.installation_mode;
    let has_swap_partition = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );

    let mut items = vec![
        MenuItem {
            key: "pool_name",
            label: "Pool name",
            value: config.pool_name.clone().unwrap_or("Not set".into()),
            kind: if matches!(mode, Some(InstallationMode::ExistingPool)) {
                MenuKind::Custom
            } else {
                MenuKind::Text
            },
        },
        MenuItem {
            key: "dataset_prefix",
            label: "Dataset prefix",
            value: config.dataset_prefix.clone(),
            kind: MenuKind::Text,
        },
    ];

    items.extend(choice_group(
        "compression",
        "Compression",
        config.compression,
    ));

    items.extend(choice_group(
        "encryption",
        "Encryption",
        config.zfs_encryption_mode,
    ));

    if config.zfs_encryption_mode != ZfsEncryptionMode::None {
        items.push(MenuItem {
            key: "encryption_password",
            label: "Encryption password",
            value: if config.zfs_encryption_password.is_some() {
                "Set".into()
            } else {
                "Not set".into()
            },
            kind: MenuKind::Password,
        });
    }

    items.extend(choice_group("swap_mode", "Swap", config.swap_mode));

    if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
        items.push(MenuItem {
            key: "swap_partition_size",
            label: "Swap size",
            value: config
                .swap_partition_size
                .clone()
                .unwrap_or("Not set".into()),
            kind: MenuKind::Text,
        });
    }
    if !matches!(mode, Some(InstallationMode::FullDisk) | None) && has_swap_partition {
        items.push(MenuItem {
            key: "swap_partition",
            label: "Swap partition",
            value: config
                .swap_partition
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }

    items.extend(choice_group(
        "init_system",
        "Init system",
        config.init_system,
    ));

    items
}
