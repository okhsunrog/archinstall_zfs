use archinstall_zfs_core::config::choices::Choice;
use archinstall_zfs_core::config::edit::ChoiceSetting;
use archinstall_zfs_core::config::types::{GlobalConfig, InstallationMode};

use super::{MenuItem, MenuKind, choice_group};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    let mut items = choice_group(
        ChoiceSetting::InstallationMode,
        "Installation mode",
        config
            .installation_mode
            .unwrap_or(InstallationMode::FullDisk),
    );
    // The graphical allocation editor owns alongside plan construction. Do not
    // expose an option that this frontend cannot configure.
    items.retain(|item| !matches!(item.kind, MenuKind::RadioOption { index, .. } if index == InstallationMode::Alongside.index()));
    items.push(MenuItem::header(
        "",
        "For installation alongside another OS, use the graphical installer (azfs).",
        String::new(),
    ));
    items
}
