use archinstall_zfs_core::config::types::{
    CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, SwapMode, ZfsEncryptionMode,
    ZfsModuleMode,
};

use super::{MenuItem, MenuKind, radio_group};

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

    items.extend(radio_group(
        "compression",
        "Compression",
        &["lz4", "zstd", "zstd-5", "zstd-10", "off"],
        match config.compression {
            CompressionAlgo::Lz4 => 0,
            CompressionAlgo::Zstd => 1,
            CompressionAlgo::Zstd5 => 2,
            CompressionAlgo::Zstd10 => 3,
            CompressionAlgo::Off => 4,
        },
    ));

    items.extend(radio_group(
        "encryption",
        "Encryption",
        &[
            "No encryption",
            "Encrypt entire pool",
            "Encrypt base dataset only",
        ],
        match config.zfs_encryption_mode {
            ZfsEncryptionMode::None => 0,
            ZfsEncryptionMode::Pool => 1,
            ZfsEncryptionMode::Dataset => 2,
        },
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

    items.extend(radio_group(
        "swap_mode",
        "Swap",
        &[
            "None",
            "ZRAM",
            "Swap partition",
            "Swap partition (encrypted)",
        ],
        match config.swap_mode {
            SwapMode::None => 0,
            SwapMode::Zram => 1,
            SwapMode::ZswapPartition => 2,
            SwapMode::ZswapPartitionEncrypted => 3,
        },
    ));

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
                .swap_partition_by_id
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
            kind: MenuKind::Custom,
        });
    }

    items.extend(radio_group(
        "init_system",
        "Init system",
        &["dracut", "mkinitcpio"],
        match config.init_system {
            InitSystem::Dracut => 0,
            InitSystem::Mkinitcpio => 1,
        },
    ));

    items.extend(radio_group(
        "zfs_module_mode",
        "ZFS module",
        &["precompiled", "dkms"],
        match config.zfs_module_mode {
            ZfsModuleMode::Precompiled => 0,
            ZfsModuleMode::Dkms => 1,
        },
    ));

    items
}
