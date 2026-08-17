use archinstall_zfs_core::config::edit::{ChoiceSetting, DeviceSetting, TextSetting};
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
            key: TextSetting::PoolName.as_str(),
            label: "Pool name",
            value: config.pool_name.clone().unwrap_or("Not set".into()),
            kind: if matches!(mode, Some(InstallationMode::ExistingPool)) {
                MenuKind::Custom
            } else {
                MenuKind::Text
            },
        },
        MenuItem {
            key: TextSetting::DatasetPrefix.as_str(),
            label: "Dataset prefix",
            value: config.dataset_prefix.clone(),
            kind: MenuKind::Text,
        },
    ];

    items.extend(choice_group(
        ChoiceSetting::Compression,
        "Compression",
        config.compression,
    ));

    items.extend(choice_group(
        ChoiceSetting::Encryption,
        "Encryption",
        config.zfs_encryption_mode,
    ));

    if config.zfs_encryption_mode != ZfsEncryptionMode::None {
        items.push(MenuItem {
            key: TextSetting::EncryptionPassword.as_str(),
            label: "Encryption password",
            value: if config.zfs_encryption_password.is_some() {
                "Set".into()
            } else {
                "Not set".into()
            },
            kind: MenuKind::Password,
        });
    }

    items.extend(choice_group(
        ChoiceSetting::SwapMode,
        "Swap",
        config.swap_mode,
    ));

    if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
        items.push(MenuItem {
            key: TextSetting::SwapPartitionSize.as_str(),
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
            key: DeviceSetting::SwapPartition.as_str(),
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
        ChoiceSetting::InitSystem,
        "Init system",
        config.init_system,
    ));

    items
}
