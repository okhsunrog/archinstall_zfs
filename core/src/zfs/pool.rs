use std::path::{Path, PathBuf};

use color_eyre::eyre::Result;
use palimpsest::models::{ZpoolListEntry, ZpoolStatusEntry};
use palimpsest::pool::{ExportOptions, ImportOptions, PoolCreateOptions, Vdev};

/// Default pool/filesystem properties used at `zpool create` time.
///
/// `autotrim` is intentionally absent — it is set dynamically after pool
/// creation based on the detected storage type (NVMe vs SATA SSD vs HDD).
/// See `crate::system::sysinfo::StorageType` for the rationale.
fn apply_default_options(opts: PoolCreateOptions) -> PoolCreateOptions {
    opts.pool_property("ashift", "12")
        .fs_property("acltype", "posixacl")
        .fs_property("relatime", "on")
        .fs_property("xattr", "sa")
        .fs_property("dnodesize", "auto")
        .fs_property("normalization", "formD")
        .fs_property("devices", "off")
        .mountpoint("none")
}

pub async fn create_pool(
    zfs: &palimpsest::Zfs,
    name: &str,
    device: &Path,
    mountpoint: &Path,
    compression: &str,
    extra_props: &[(&str, &str)],
) -> Result<()> {
    let mut opts = apply_default_options(PoolCreateOptions::new(name))
        .force()
        .altroot(mountpoint)
        .fs_property("compression", compression);
    for (k, v) in extra_props {
        opts = opts.fs_property(*k, *v);
    }
    opts = opts.vdev(Vdev::Stripe(vec![PathBuf::from(device)]));

    zfs.create_pool(&opts).await?;
    Ok(())
}

pub async fn import_pool(zfs: &palimpsest::Zfs, name: &str, mountpoint: &Path) -> Result<()> {
    let opts = ImportOptions {
        force: true,
        altroot: Some(mountpoint.to_path_buf()),
        ..Default::default()
    };
    zfs.pool(name).import(&opts).await?;
    Ok(())
}

pub async fn import_pool_no_mount(
    zfs: &palimpsest::Zfs,
    name: &str,
    mountpoint: &Path,
) -> Result<()> {
    let opts = ImportOptions {
        no_mount: true,
        altroot: Some(mountpoint.to_path_buf()),
        ..Default::default()
    };
    zfs.pool(name).import(&opts).await?;
    Ok(())
}

pub async fn export_pool(zfs: &palimpsest::Zfs, name: &str) -> Result<()> {
    nix::unistd::sync();
    // Best-effort umount-all first; ignore errors. palimpsest's unmount_all
    // returns Err on real failures (e.g., a stuck mountpoint), but the
    // subsequent zpool export will surface the same condition more clearly.
    let _ = zfs.unmount_all(false).await;
    zfs.pool(name).export(&ExportOptions::default()).await?;
    Ok(())
}

pub async fn set_pool_property(
    zfs: &palimpsest::Zfs,
    pool: &str,
    property: &str,
    value: &str,
) -> Result<()> {
    zfs.pool(pool).set_property(property, value).await?;
    Ok(())
}

pub async fn list_pools(zfs: &palimpsest::Zfs) -> Result<Vec<ZpoolListEntry>> {
    Ok(zfs.list_pools(&Default::default()).await?)
}

pub async fn pool_status(zfs: &palimpsest::Zfs, pool: &str) -> Result<ZpoolStatusEntry> {
    Ok(zfs.pool(pool).status().await?)
}

pub async fn pool_exists(zfs: &palimpsest::Zfs, name: &str) -> bool {
    zfs.pool(name).exists().await
}

/// Public wrapper for the TUI/slint pickers — discover importable pools by
/// name. Hides palimpsest's `DiscoveredPool` struct (with id/state/status)
/// because the picker only needs names. On any error returns an empty Vec.
pub async fn discover_importable_pools() -> Vec<String> {
    palimpsest::Zfs::new()
        .discover_importable_pools()
        .await
        .map(|ps| ps.into_iter().map(|p| p.name).collect())
        .unwrap_or_default()
}
