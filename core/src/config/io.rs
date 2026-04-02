use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use super::types::GlobalConfig;

const ZFS_CONFIG_KEY: &str = "archinstall_zfs";

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
        let json = self.to_json_string()?;
        fs::write(path, json)
            .wrap_err_with(|| format!("failed to write config: {}", path.display()))?;
        Ok(())
    }

    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string_pretty(self).wrap_err("failed to serialize config")
    }

    pub fn to_combined_json(&self) -> Result<String> {
        let value = serde_json::to_value(self).wrap_err("failed to serialize config")?;
        let combined = serde_json::json!({
            ZFS_CONFIG_KEY: value
        });
        serde_json::to_string_pretty(&combined).wrap_err("failed to serialize combined config")
    }
}

#[cfg(test)]
mod tests {
    use crate::config::types::{GlobalConfig, InstallationMode};

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
}
