use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};
use serde::{Deserialize, Serialize};

use super::types::GlobalConfig;
use crate::system::fs::write_file_with_mode;

const ZFS_CONFIG_KEY: &str = "archinstall_zfs";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zfs_encryption_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_password: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<UserSecrets>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserSecrets {
    pub username: String,
    pub password: String,
}

impl GlobalConfig {
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read config: {}", path.display()))?;
        Self::load_from_str(&content)
    }

    pub fn load_from_str(json: &str) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(json).wrap_err("failed to parse config JSON")?;

        // Check if there's an archinstall_zfs sub-key
        if let Some(zfs_block) = value.get(ZFS_CONFIG_KEY) {
            serde_json::from_value(zfs_block.clone())
                .wrap_err("failed to deserialize archinstall_zfs config block")
        } else {
            // Try parsing the whole file as GlobalConfig
            serde_json::from_value(value).wrap_err("failed to deserialize config")
        }
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let json = self.to_redacted_json_string()?;
        write_private_file(path, &json, "config")
    }

    pub fn save_secrets_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.secrets())
            .wrap_err("failed to serialize config secrets")?;
        write_private_file(path, &json, "config secrets")
    }

    pub fn apply_secrets_from_file(&mut self, path: &Path) -> Result<()> {
        let content = fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read config secrets: {}", path.display()))?;
        let secrets: ConfigSecrets =
            serde_json::from_str(&content).wrap_err("failed to parse config secrets JSON")?;
        self.apply_secrets(secrets);
        Ok(())
    }

    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string_pretty(self).wrap_err("failed to serialize config")
    }

    pub fn to_redacted_json_string(&self) -> Result<String> {
        let mut redacted = self.clone();
        redacted.zfs_encryption_password = None;
        redacted.root_password = None;
        if let Some(users) = redacted.users.as_mut() {
            for user in users {
                user.password = None;
            }
        }
        redacted.to_json_string()
    }

    pub fn has_secrets(&self) -> bool {
        self.zfs_encryption_password.is_some()
            || self.root_password.is_some()
            || self
                .users
                .as_ref()
                .is_some_and(|users| users.iter().any(|user| user.password.is_some()))
    }

    pub fn secrets(&self) -> ConfigSecrets {
        ConfigSecrets {
            zfs_encryption_password: self.zfs_encryption_password.clone(),
            root_password: self.root_password.clone(),
            users: self
                .users
                .iter()
                .flatten()
                .filter_map(|user| {
                    user.password.as_ref().map(|password| UserSecrets {
                        username: user.username.clone(),
                        password: password.clone(),
                    })
                })
                .collect(),
        }
    }

    pub fn apply_secrets(&mut self, secrets: ConfigSecrets) {
        if secrets.zfs_encryption_password.is_some() {
            self.zfs_encryption_password = secrets.zfs_encryption_password;
        }
        if secrets.root_password.is_some() {
            self.root_password = secrets.root_password;
        }
        if let Some(users) = self.users.as_mut() {
            for user_secret in secrets.users {
                if let Some(user) = users
                    .iter_mut()
                    .find(|user| user.username == user_secret.username)
                {
                    user.password = Some(user_secret.password);
                }
            }
        }
    }

    pub fn to_combined_json(&self) -> Result<String> {
        let value = serde_json::to_value(self).wrap_err("failed to serialize config")?;
        let combined = serde_json::json!({
            ZFS_CONFIG_KEY: value
        });
        serde_json::to_string_pretty(&combined).wrap_err("failed to serialize combined config")
    }
}

fn write_private_file(path: &Path, contents: &str, description: &str) -> Result<()> {
    write_file_with_mode(path, contents.as_bytes(), 0o600, description)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use crate::config::types::{GlobalConfig, InstallationMode, UserConfig};

    #[test]
    fn test_load_direct_format() {
        let json = r#"{
            "installation_mode": "full_disk",
            "pool_name": "testpool",
            "dataset_prefix": "arch0"
        }"#;

        let cfg = GlobalConfig::load_from_str(json).unwrap();
        assert_eq!(cfg.installation_mode, Some(InstallationMode::FullDisk));
        assert_eq!(cfg.pool_name.as_deref(), Some("testpool"));
    }

    #[test]
    fn test_load_combined_format() {
        let json = r#"{
            "archinstall_zfs": {
                "installation_mode": "new_pool",
                "pool_name": "zfsroot"
            }
        }"#;

        let cfg = GlobalConfig::load_from_str(json).unwrap();
        assert_eq!(cfg.installation_mode, Some(InstallationMode::NewPool));
        assert_eq!(cfg.pool_name.as_deref(), Some("zfsroot"));
    }

    #[test]
    fn test_to_combined_json() {
        let cfg = GlobalConfig {
            pool_name: Some("mypool".to_string()),
            ..Default::default()
        };

        let json = cfg.to_combined_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("archinstall_zfs").is_some());
        assert_eq!(
            value["archinstall_zfs"]["pool_name"].as_str(),
            Some("mypool")
        );
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_config.json");

        let cfg = GlobalConfig {
            installation_mode: Some(InstallationMode::ExistingPool),
            pool_name: Some("roundtrip".to_string()),
            hostname: Some("testhost".to_string()),
            ..Default::default()
        };

        cfg.save_to_file(&path).unwrap();
        let loaded = GlobalConfig::load_from_file(&path).unwrap();

        assert_eq!(
            loaded.installation_mode,
            Some(InstallationMode::ExistingPool)
        );
        assert_eq!(loaded.pool_name.as_deref(), Some("roundtrip"));
        assert_eq!(loaded.hostname.as_deref(), Some("testhost"));
    }

    #[test]
    fn saved_config_omits_all_passwords_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = GlobalConfig {
            zfs_encryption_password: Some("pool-secret".into()),
            root_password: Some("root-secret".into()),
            users: Some(vec![UserConfig {
                username: "alice".into(),
                password: Some("user-secret".into()),
                sudo: true,
                shell: None,
                groups: None,
                ssh_authorized_keys: Vec::new(),
                autologin: false,
            }]),
            ..Default::default()
        };

        fs::write(&path, "old-secret-that-must-be-removed").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        cfg.save_to_file(&path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("old-secret-that-must-be-removed"));
        assert!(!content.contains("pool-secret"));
        assert!(!content.contains("root-secret"));
        assert!(!content.contains("user-secret"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn secrets_file_roundtrip_restores_passwords() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let cfg = GlobalConfig {
            zfs_encryption_password: Some("pool-secret".into()),
            root_password: Some("root-secret".into()),
            users: Some(vec![UserConfig {
                username: "alice".into(),
                password: Some("user-secret".into()),
                sudo: true,
                shell: None,
                groups: None,
                ssh_authorized_keys: Vec::new(),
                autologin: false,
            }]),
            ..Default::default()
        };

        cfg.save_secrets_to_file(&path).unwrap();
        let mut redacted = cfg.clone();
        redacted.zfs_encryption_password = None;
        redacted.root_password = None;
        redacted.users.as_mut().unwrap()[0].password = None;
        redacted.apply_secrets_from_file(&path).unwrap();

        assert_eq!(
            redacted.zfs_encryption_password.as_deref(),
            Some("pool-secret")
        );
        assert_eq!(redacted.root_password.as_deref(), Some("root-secret"));
        assert_eq!(
            redacted.users.unwrap()[0].password.as_deref(),
            Some("user-secret")
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn save_refuses_to_follow_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.json");
        let link = dir.path().join("config.json");
        fs::write(&victim, "do not overwrite").unwrap();
        std::os::unix::fs::symlink(&victim, &link).unwrap();

        let result = GlobalConfig::default().save_to_file(&link);

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(victim).unwrap(), "do not overwrite");
    }
}
