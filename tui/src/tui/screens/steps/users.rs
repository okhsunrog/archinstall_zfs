use archinstall_zfs_core::config::edit::TextSetting;
use archinstall_zfs_core::config::types::GlobalConfig;

use super::MenuItem;

pub fn items(config: &GlobalConfig) -> Vec<MenuItem> {
    vec![
        MenuItem::secret(
            TextSetting::RootPassword.as_str(),
            "Root password",
            config.root_password.is_some(),
        ),
        MenuItem::custom(
            "users",
            "User accounts",
            match &config.users {
                Some(users) if !users.is_empty() => {
                    let names: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
                    names.join(", ")
                }
                _ => "None".into(),
            },
        ),
    ]
}
