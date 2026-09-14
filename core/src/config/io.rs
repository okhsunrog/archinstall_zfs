use std::fs;
use std::path::{Path, PathBuf};

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

/// Where the passwords for `config` live: the same name with `.secrets`
/// before the extension, so a saved pair is `azfs.json` and
/// `azfs.secrets.json` and the two travel together without the
/// configuration itself ever holding a password.
pub fn secrets_path_for(config: &Path) -> PathBuf {
    let stem = config
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".into());
    let extension = config
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    config.with_file_name(format!("{stem}.secrets{extension}"))
}

impl GlobalConfig {
    /// Save the configuration, and the passwords beside it when there are
    /// any. Returns what was written, in order.
    pub fn save_with_secrets(&self, path: &Path) -> Result<Vec<PathBuf>> {
        self.save_to_file(path)?;
        let mut written = vec![path.to_path_buf()];
        if self.has_secrets() {
            let secrets = secrets_path_for(path);
            self.save_secrets_to_file(&secrets)?;
            written.push(secrets);
        }
        Ok(written)
    }

    /// Load a configuration, applying the passwords beside it when that
    /// file exists. A missing companion is not an error: a configuration
    /// shared without its passwords is the point of the split.
    pub fn load_with_secrets(path: &Path) -> Result<Self> {
        let mut config = Self::load_from_file(path)?;
        let secrets = secrets_path_for(path);
        if secrets.is_file() {
            config.apply_secrets_from_file(&secrets)?;
        }
        Ok(config)
    }

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

    /// Serialize including every password. Private on purpose: the only
    /// caller is the redacting path below, and an accidental public use is a
    /// secret written somewhere it should not be.
    fn to_json_string(&self) -> Result<String> {
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
}

fn write_private_file(path: &Path, contents: &str, description: &str) -> Result<()> {
    write_file_with_mode(path, contents.as_bytes(), 0o600, description)
}

#[cfg(test)]
mod pair_tests {
    use super::*;
    use crate::config::types::UserConfig;

    fn config_with_secrets() -> GlobalConfig {
        GlobalConfig {
            hostname: Some("archzfs".into()),
            root_password: Some("root secret".into()),
            users: Some(vec![UserConfig {
                username: "nika".into(),
                password: Some("user secret".into()),
                sudo: true,
                shell: None,
                groups: None,
                ssh_authorized_keys: Vec::new(),
                autologin: false,
            }]),
            ..GlobalConfig::default()
        }
    }

    #[test]
    fn the_companion_keeps_the_name_and_the_extension() {
        assert_eq!(
            secrets_path_for(Path::new("/root/azfs.json")),
            Path::new("/root/azfs.secrets.json")
        );
        assert_eq!(
            secrets_path_for(Path::new("azfs")),
            Path::new("azfs.secrets")
        );
    }

    #[test]
    fn passwords_are_written_beside_the_configuration_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("azfs.json");
        let written = config_with_secrets().save_with_secrets(&path).unwrap();
        assert_eq!(written, vec![path.clone(), secrets_path_for(&path)]);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(
            !saved.contains("root secret"),
            "the config holds a password"
        );
        assert!(
            !saved.contains("user secret"),
            "the config holds a password"
        );

        let loaded = GlobalConfig::load_with_secrets(&path).unwrap();
        assert_eq!(loaded.root_password.as_deref(), Some("root secret"));
        assert_eq!(
            loaded.users.unwrap()[0].password.as_deref(),
            Some("user secret")
        );
    }

    #[test]
    fn a_configuration_without_passwords_writes_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("azfs.json");
        let config = GlobalConfig {
            hostname: Some("archzfs".into()),
            ..GlobalConfig::default()
        };
        assert_eq!(config.save_with_secrets(&path).unwrap(), vec![path.clone()]);
        assert!(!secrets_path_for(&path).exists());
        let loaded = GlobalConfig::load_with_secrets(&path).unwrap();
        assert_eq!(loaded.hostname.as_deref(), Some("archzfs"));
        assert!(loaded.root_password.is_none());
    }
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
