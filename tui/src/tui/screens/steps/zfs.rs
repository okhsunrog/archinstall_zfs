use archinstall_zfs_core::config::edit::{ChoiceSetting, DeviceSetting, TextSetting};
use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode, ZfsEncryptionMode};

use super::{MenuItem, MenuKind, choice_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mode = config.installation_mode;
    let has_swap_partition = config.swap_mode.uses_partition();

    let mut items = vec![
        MenuItem::new(
            TextSetting::PoolName.as_str(),
            "Pool name",
            config.pool_name.clone().unwrap_or("Not set".into()),
            if matches!(mode, Some(InstallationMode::ExistingPool)) {
                MenuKind::Custom
            } else {
                MenuKind::Text
            },
        ),
        MenuItem::text(
            TextSetting::DatasetPrefix.as_str(),
            "Dataset prefix",
            config.dataset_prefix.clone(),
        ),
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
        items.push(MenuItem::secret(
            TextSetting::EncryptionPassword.as_str(),
            "Encryption password",
            config.zfs_encryption_password.is_some(),
        ));
    }

    items.extend(choice_group(
        ChoiceSetting::SwapMode,
        "Swap",
        config.swap_mode,
    ));

    if matches!(mode, Some(InstallationMode::FullDisk)) && has_swap_partition {
        items.push(MenuItem::text(
            TextSetting::SwapPartitionSize.as_str(),
            "Swap size",
            config
                .swap_partition_size
                .clone()
                .unwrap_or("Not set".into()),
        ));
    }
    if !matches!(mode, Some(InstallationMode::FullDisk) | None) && has_swap_partition {
        items.push(MenuItem::custom(
            DeviceSetting::SwapPartition.as_str(),
            "Swap partition",
            config
                .swap_partition
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or("Not set".into()),
        ));
    }

    items.extend(choice_group(
        ChoiceSetting::InitSystem,
        "Init system",
        config.init_system,
    ));

    items
}
