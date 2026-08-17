use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::boot_environment::BootEnvironment;

pub fn generate_zrepl_config(be: &BootEnvironment) -> String {
    let base = be.base();
    format!(
        r#"jobs:
- name: snap
  type: snap
  filesystems:
    "{base}<": true
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

pub fn setup_zrepl(target: &Path, be: &BootEnvironment) -> Result<()> {
    let config = generate_zrepl_config(be);

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
        let config = generate_zrepl_config(&BootEnvironment::new("mypool", "arch0"));
        assert!(config.contains("mypool/arch0<"));
        assert!(config.contains("15m"));
        assert!(config.contains("zrepl_"));
        assert!(config.contains("grid"));
    }

    #[test]
    fn test_setup_zrepl() {
        let dir = tempfile::tempdir().unwrap();
        setup_zrepl(dir.path(), &BootEnvironment::new("testpool", "arch0")).unwrap();

        let config_path = dir.path().join("etc/zrepl/zrepl.yml");
        assert!(config_path.exists());
        let content = fs::read_to_string(config_path).unwrap();
        assert!(content.contains("testpool/arch0"));
    }
}
