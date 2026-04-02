use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit};

pub fn configure_zram(target: &Path, size_expr: Option<&str>) -> Result<()> {
    let conf_dir = target.join("etc/systemd/zram-generator.conf.d");
    fs::create_dir_all(&conf_dir)?;

    // Default: min(ram/2, 4096) MB
    let size = size_expr.unwrap_or("min(ram / 2, 4096)");
    let conf = format!("[zram0]\nzram-size = {size}\ncompression-algorithm = zstd\n");

    let conf_path = target.join("etc/systemd/zram-generator.conf");
    if let Some(parent) = conf_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&conf_path, conf).wrap_err("failed to write zram-generator.conf")?;

    tracing::info!("configured zram swap");
    Ok(())
}

pub fn setup_swap_partition(
    runner: &dyn CommandRunner,
    target: &Path,
    partition: &Path,
    encrypted: bool,
) -> Result<()> {
    let part_str = partition.to_string_lossy();

    if encrypted {
        // Encrypted swap via crypttab
        crate::installer::fstab::add_cryptswap_entry(target, &part_str)?;
        tracing::info!(partition = %part_str, "configured encrypted swap partition");
    } else {
        // Direct swap
        let output = runner.run("mkswap", &[&*part_str])?;
        check_exit(&output, "mkswap")?;

        crate::installer::fstab::add_swap_entry(target, &part_str)?;
        tracing::info!(partition = %part_str, "configured swap partition");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_configure_zram() {
        let dir = tempfile::tempdir().unwrap();
        configure_zram(dir.path(), None).unwrap();

        let conf = fs::read_to_string(dir.path().join("etc/systemd/zram-generator.conf")).unwrap();
        assert!(conf.contains("[zram0]"));
        assert!(conf.contains("zram-size"));
        assert!(conf.contains("zstd"));
    }

    #[test]
    fn test_configure_zram_custom_size() {
        let dir = tempfile::tempdir().unwrap();
        configure_zram(dir.path(), Some("ram / 4")).unwrap();

        let conf = fs::read_to_string(dir.path().join("etc/systemd/zram-generator.conf")).unwrap();
        assert!(conf.contains("ram / 4"));
    }
}
