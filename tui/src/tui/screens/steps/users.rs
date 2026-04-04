use archinstall_zfs_core::config::types::GlobalConfig;

use super::{MenuItem, MenuKind};

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    vec![
        MenuItem {
            key: "root_password",
            label: "Root password",
            value: if config.root_password.is_some() {
                "Set".into()
            } else {
                "Not set".into()
            },
            kind: MenuKind::Password,
        },
        MenuItem {
            key: "users",
            label: "User accounts",
            value: match &config.users {
                Some(users) if !users.is_empty() => {
                    let names: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
                    names.join(", ")
                }
                _ => "None".into(),
            },
            kind: MenuKind::Custom,
        },
    ]
}
