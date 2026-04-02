use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

pub fn generate_zrepl_config(pool_name: &str, dataset_prefix: &str) -> String {
    format!(
        r#"jobs:
- name: snap
  type: snap
  filesystems:
    "{pool_name}/{dataset_prefix}<": true
  snapshotting:
    type: periodic
    interval: 15m
    prefix: zrepl_
  pruning:
    keep:
    - type: grid
      grid: 4x15m | 24x1h | 3x1d
      regex: "^zrepl_"
"#
    )
}

pub fn setup_zrepl(target: &Path, pool_name: &str, dataset_prefix: &str) -> Result<()> {
    let config = generate_zrepl_config(pool_name, dataset_prefix);

    let config_dir = target.join("etc/zrepl");
    fs::create_dir_all(&config_dir)?;

    let config_path = config_dir.join("zrepl.yml");
    fs::write(&config_path, config).wrap_err("failed to write zrepl config")?;

    tracing::info!("configured zrepl");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_zrepl_config() {
        let config = generate_zrepl_config("mypool", "arch0");
        assert!(config.contains("mypool/arch0<"));
        assert!(config.contains("15m"));
        assert!(config.contains("zrepl_"));
        assert!(config.contains("grid"));
    }

    #[test]
    fn test_setup_zrepl() {
        let dir = tempfile::tempdir().unwrap();
        setup_zrepl(dir.path(), "testpool", "arch0").unwrap();

        let config_path = dir.path().join("etc/zrepl/zrepl.yml");
        assert!(config_path.exists());
        let content = fs::read_to_string(config_path).unwrap();
        assert!(content.contains("testpool/arch0"));
    }
}
