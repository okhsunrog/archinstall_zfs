//! Post-install TRIM configuration. Runs *after* `Installer::perform_installation`
//! so it can be a regular `async fn` — there's no Alpm involvement, and the
//! ZFS-side work goes through palimpsest directly without a `block_on` bridge.
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
        Some(InstallationMode::FullDisk) => config.disk_by_id.as_deref(),
        Some(InstallationMode::NewPool) => config.zfs_partition_by_id.as_deref(),
        _ => None,
    };

    let Some(disk_path) = disk_path else {
        tracing::debug!("no disk path available for TRIM detection, skipping");
        return Ok(());
    };

    match detect_storage_type(disk_path) {
        StorageType::Nvme => {
            tracing::info!(pool = pool_name, "NVMe detected — enabling autotrim");
            palimpsest::Zfs::new()
                .pool(pool_name)
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
