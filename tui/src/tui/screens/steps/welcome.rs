use archinstall_zfs_core::config::edit::ChoiceSetting;
use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode};

use super::{MenuItem, choice_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    choice_group(
        ChoiceSetting::InstallationMode,
        "Installation mode",
        config
            .installation_mode
            .unwrap_or(InstallationMode::FullDisk),
    )
}
