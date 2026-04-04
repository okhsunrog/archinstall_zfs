use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode};

use super::{MenuItem, MenuKind};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    vec![MenuItem {
        key: "installation_mode",
        label: "Installation mode",
        value: config
            .installation_mode
            .map(|m| match m {
                InstallationMode::FullDisk => "Full Disk — erase disk and create new pool",
                InstallationMode::NewPool => "New Pool — use pre-partitioned disk",
                InstallationMode::ExistingPool => "Existing Pool — use an already imported pool",
            })
            .unwrap_or("Not configured")
            .into(),
        kind: MenuKind::Select {
            options: vec!["Full Disk", "New Pool", "Existing Pool"],
            current: match config.installation_mode {
                Some(InstallationMode::FullDisk) => 0,
                Some(InstallationMode::NewPool) => 1,
                Some(InstallationMode::ExistingPool) => 2,
                None => 0,
            },
        },
    }]
}
