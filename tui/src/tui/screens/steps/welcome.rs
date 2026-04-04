use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode};

use super::radio_group;

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    radio_group(
        "installation_mode",
        "Installation mode",
        &["Full Disk", "New Pool", "Existing Pool"],
        match config.installation_mode {
            Some(InstallationMode::FullDisk) => 0,
            Some(InstallationMode::NewPool) => 1,
            Some(InstallationMode::ExistingPool) => 2,
            None => usize::MAX, // nothing selected
        },
    )
}

use super::MenuItem;
