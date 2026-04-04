use archinstall_zfs_core::config::types::{
    CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, SwapMode, ZfsEncryptionMode,
    ZfsModuleMode,
};

use super::{MenuItem, MenuKind};

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
        MenuItem {
            key: "compression",
            label: "Compression",
            value: config.compression.to_string(),
            kind: MenuKind::Select {
                options: vec!["lz4", "zstd", "zstd-5", "zstd-10", "off"],
                current: match config.compression {
                    CompressionAlgo::Lz4 => 0,
                    CompressionAlgo::Zstd => 1,
                    CompressionAlgo::Zstd5 => 2,
                    CompressionAlgo::Zstd10 => 3,
                    CompressionAlgo::Off => 4,
                },
            },
        },
        MenuItem {
            key: "encryption",
            label: "Encryption",
            value: config.zfs_encryption_mode.to_string(),
            kind: MenuKind::Select {
                options: vec![
                    "No encryption",
                    "Encrypt entire pool",
                    "Encrypt base dataset only",
                ],
                current: match config.zfs_encryption_mode {
                    ZfsEncryptionMode::None => 0,
                    ZfsEncryptionMode::Pool => 1,
                    ZfsEncryptionMode::Dataset => 2,
                },
            },
        },
    ];

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

    items.push(MenuItem {
        key: "swap_mode",
        label: "Swap",
        value: config.swap_mode.to_string(),
        kind: MenuKind::Select {
            options: vec![
                "None",
                "ZRAM",
                "Swap partition",
                "Swap partition (encrypted)",
            ],
            current: match config.swap_mode {
                SwapMode::None => 0,
                SwapMode::Zram => 1,
                SwapMode::ZswapPartition => 2,
                SwapMode::ZswapPartitionEncrypted => 3,
            },
        },
    });

    // Swap partition size for FullDisk + ZSWAP modes
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
    // Swap partition picker for NewPool/ExistingPool + ZSWAP modes
    if !matches!(mode, Some(InstallationMode::FullDisk) | None) && has_swap_partition {
        items.push(MenuItem {
            key: "swap_partition",
            label: "Swap partition",
            value: config
                .swap_partition_by_id
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }

    items.extend([
        MenuItem {
            key: "init_system",
            label: "Init system",
            value: config.init_system.to_string(),
            kind: MenuKind::Select {
                options: vec!["dracut", "mkinitcpio"],
                current: match config.init_system {
                    InitSystem::Dracut => 0,
                    InitSystem::Mkinitcpio => 1,
                },
            },
        },
        MenuItem {
            key: "zfs_module_mode",
            label: "ZFS module",
            value: config.zfs_module_mode.to_string(),
            kind: MenuKind::Select {
                options: vec!["precompiled", "dkms"],
                current: match config.zfs_module_mode {
                    ZfsModuleMode::Precompiled => 0,
                    ZfsModuleMode::Dkms => 1,
                },
            },
        },
    ]);

    items
}
