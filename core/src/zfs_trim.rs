//! Post-install TRIM configuration. Runs *after* `Installer::perform_installation`
//! so it can be a regular `async fn` — there's no Alpm involvement, and the
//! ZFS-side work goes through zfskit directly without a `block_on` bridge.
//!
//! Strategy:
//!   - NVMe → set the pool's `autotrim=on` property (kernel TRIMs continuously)
//!   - SATA SSD → enable `zfs-trim-weekly@<pool>.timer` on the install target
//!   - HDD → nothing
//!
//! `fstrim.timer` is intentionally never enabled — it is a VFS-level tool
//! unaware of ZFS internals and silently skips ZFS pools on every run.

use std::path::Path;

use color_eyre::eyre::Result;

use crate::config::types::{GlobalConfig, InstallationMode};
use crate::installer::services;
use crate::system::cmd::CommandRunner;
use crate::system::sysinfo::{StorageType, detect_storage_type};

pub async fn configure_zfs_trim(
    runner: &dyn CommandRunner,
    target: &Path,
    pool_name: &str,
    config: &GlobalConfig,
) -> Result<()> {
    // Only configure TRIM when we created (or know) the disk. ExistingPool
    // mode leaves the pool's autotrim property and any timer untouched.
    let disk_path = match config.installation_mode {
        Some(InstallationMode::FullDisk) => config.disk.as_deref(),
        Some(InstallationMode::NewPool) => config.zfs_partition.as_deref(),
        _ => None,
    };

    let Some(disk_path) = disk_path else {
        tracing::debug!("no disk path available for TRIM detection, skipping");
        return Ok(());
    };

    match detect_storage_type(disk_path) {
        StorageType::Nvme => {
            tracing::info!(pool = pool_name, "NVMe detected — enabling autotrim");
            zfskit::Zfs::new()
                .pool(pool_name)?
                .set_property("autotrim", "on")
                .await?;
        }
        StorageType::SataSsd => {
            let timer = format!("zfs-trim-weekly@{pool_name}.timer");
            tracing::info!(
                pool = pool_name,
                timer,
                "SATA SSD detected — enabling periodic zpool trim timer"
            );
            services::enable_service(runner, target, &timer)?;
        }
        StorageType::Hdd => {
            tracing::debug!(pool = pool_name, "HDD detected — no TRIM configured");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::types::InstallationMode;
    use crate::system::cmd::tests::RecordingRunner;

    fn config_for(mode: InstallationMode, disk: Option<&str>) -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(mode),
            disk: disk.map(PathBuf::from),
            zfs_partition: disk.map(PathBuf::from),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_existing_pool_keeps_whatever_trim_it_already_had() {
        let runner = RecordingRunner::new(vec![]);
        let config = config_for(
            InstallationMode::ExistingPool,
            Some("/dev/disk/by-id/some-disk"),
        );

        configure_zfs_trim(&runner, Path::new("/mnt"), "zroot", &config)
            .await
            .expect("existing pools are left alone");

        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn no_disk_means_nothing_to_detect() {
        let runner = RecordingRunner::new(vec![]);
        let config = config_for(InstallationMode::FullDisk, None);

        configure_zfs_trim(&runner, Path::new("/mnt"), "zroot", &config)
            .await
            .expect("a missing disk path is not an error");

        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn an_unrecognised_device_gets_no_trim_configuration() {
        // Nothing under /sys to say it is solid state, so it is treated as
        // rotational and left without a TRIM strategy rather than guessing.
        let runner = RecordingRunner::new(vec![]);
        let config = config_for(
            InstallationMode::FullDisk,
            Some("/dev/disk/by-id/definitely-not-a-real-device"),
        );

        configure_zfs_trim(&runner, Path::new("/mnt"), "zroot", &config)
            .await
            .expect("an undetectable device is not an error");

        assert!(runner.calls().is_empty());
    }
}
