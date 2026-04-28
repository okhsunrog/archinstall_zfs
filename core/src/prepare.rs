//! Pre-installation orchestration: disk partitioning and ZFS pool/dataset setup.
//!
//! These helpers are the phase 1 (disk) and phase 2 (ZFS) steps shared by the
//! TUI and Slint UIs. They dispatch on `InstallationMode` to create new
//! partitions, build a new pool, or create a fresh boot-environment inside an
//! existing pool.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, eyre};
use palimpsest::pool::{ExportOptions, ImportOptions, PoolCreateOptions, Vdev};

use crate::config::types::{GlobalConfig, InstallationMode, SwapMode, ZfsEncryptionMode};
use crate::system::cmd::CommandRunner;

/// Partitions selected or created for the installation.
///
/// `zfs` is `None` for [`InstallationMode::ExistingPool`] — the pool is already
/// present, so no partition is consumed.
pub struct PreparedPartitions {
    pub efi: PathBuf,
    pub zfs: Option<PathBuf>,
    pub swap: Option<PathBuf>,
}

/// Phase 1: partition the disk (full-disk mode) or resolve partitions from
/// config (new/existing pool).
pub fn prepare_disk(
    runner: &dyn CommandRunner,
    config: &GlobalConfig,
) -> Result<PreparedPartitions> {
    let mode = config
        .installation_mode
        .ok_or_else(|| eyre!("installation mode not set"))?;

    match mode {
        InstallationMode::FullDisk => {
            let disk = config
                .disk_by_id
                .as_ref()
                .ok_or_else(|| eyre!("disk not selected for full disk mode"))?;
            crate::disk::partition::zap_disk(runner, disk)?;

            let swap_size = match config.swap_mode {
                SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted => {
                    config.swap_partition_size.as_deref()
                }
                _ => None,
            };
            let layout = crate::disk::partition::create_partitions(runner, disk, swap_size)?;
            let parts = crate::disk::partition::wait_for_by_id_partitions(disk, &layout);
            let efi = parts[0].clone();
            let zfs = parts[1].clone();
            let swap = parts.get(2).cloned();
            Ok(PreparedPartitions {
                efi,
                zfs: Some(zfs),
                swap,
            })
        }
        InstallationMode::NewPool => {
            let efi = config
                .efi_partition_by_id
                .clone()
                .ok_or_else(|| eyre!("EFI partition not selected"))?;
            let zfs = config
                .zfs_partition_by_id
                .clone()
                .ok_or_else(|| eyre!("ZFS partition not selected"))?;
            let swap = config.swap_partition_by_id.clone();
            Ok(PreparedPartitions {
                efi,
                zfs: Some(zfs),
                swap,
            })
        }
        InstallationMode::ExistingPool => {
            let efi = config
                .efi_partition_by_id
                .clone()
                .ok_or_else(|| eyre!("EFI partition not selected"))?;
            let swap = config.swap_partition_by_id.clone();
            Ok(PreparedPartitions {
                efi,
                zfs: None,
                swap,
            })
        }
    }
}

/// Phase 2: ZFS setup — hostid, cache, optional key file, pool/dataset creation
/// or import, and mounting.
///
/// For [`InstallationMode::ExistingPool`] this creates a fresh boot-environment
/// (base dataset + children) inside the existing pool, mirroring the Python
/// installer's semantics. `create_base_dataset` errors if `pool/prefix`
/// already exists, so the user must pick a new prefix.
///
/// Sync `runner` is used only for non-ZFS commands (systemctl, hostid, cache
/// file management). All ZFS operations route through palimpsest, with a
/// `Zfs::new()` handle constructed locally.
pub async fn prepare_zfs(
    runner: &dyn CommandRunner,
    config: &GlobalConfig,
    zfs_partition: Option<&Path>,
    mountpoint: &Path,
) -> Result<()> {
    let mode = config
        .installation_mode
        .ok_or_else(|| eyre!("installation mode not set"))?;
    let pool_name = config
        .pool_name
        .as_deref()
        .ok_or_else(|| eyre!("pool name not set"))?;
    let prefix = config.dataset_prefix.as_str();
    let compression = config.compression.to_string();
    let encryption = config.zfs_encryption_mode;

    let zfs = palimpsest::Zfs::new();

    crate::zfs_target_files::create_hostid(runner)?;
    crate::zfs_target_files::prepare_zfs_cache(Path::new("/"), pool_name)?;
    let _ = runner.run("systemctl", &["enable", "--now", "zfs-zed.service"]);

    if encryption != ZfsEncryptionMode::None
        && let Some(pw) = config.zfs_encryption_password.as_deref()
    {
        crate::zfs_keyfile::write_key_file(Path::new("/"), pw)?;
    }
    let key_path = crate::zfs_keyfile::key_file_path(Path::new("/"));

    match mode {
        InstallationMode::FullDisk | InstallationMode::NewPool => {
            let zfs_partition =
                zfs_partition.ok_or_else(|| eyre!("zfs partition required for new pool modes"))?;

            let enc_props: Vec<(&str, String)> = match encryption {
                ZfsEncryptionMode::Pool => {
                    crate::zfs_keyfile::pool_encryption_properties(&key_path)
                }
                _ => Vec::new(),
            };
            let enc_refs: Vec<(&str, &str)> =
                enc_props.iter().map(|(k, v)| (*k, v.as_str())).collect();

            create_pool(
                &zfs,
                pool_name,
                zfs_partition,
                mountpoint,
                &compression,
                &enc_refs,
            )
            .await?;
            tracing::info!("Created pool: {pool_name}");

            zfs.pool(pool_name)
                .set_property("cachefile", "none")
                .await?;

            let base_refs = base_dataset_props(encryption, &key_path, &compression);
            let base_refs_view: Vec<(&str, &str)> =
                base_refs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            crate::dataset_layout::create_base_dataset(&zfs, pool_name, prefix, &base_refs_view)
                .await?;

            let datasets = crate::dataset_layout::default_datasets();
            crate::dataset_layout::create_child_datasets(&zfs, pool_name, prefix, &datasets)
                .await?;
            tracing::info!("Created datasets");

            export_pool(&zfs, pool_name).await?;
            import_pool_no_mount(&zfs, pool_name, mountpoint).await?;
            let key_loc = format!("file://{}", key_path.display());
            match encryption {
                ZfsEncryptionMode::Pool => {
                    zfs.dataset(pool_name)
                        .load_key_with_keylocation(&key_loc)
                        .await?;
                }
                ZfsEncryptionMode::Dataset => {
                    let base = format!("{pool_name}/{prefix}");
                    zfs.dataset(&base)
                        .load_key_with_keylocation(&key_loc)
                        .await?;
                }
                ZfsEncryptionMode::None => {}
            }
        }
        InstallationMode::ExistingPool => {
            import_pool_no_mount(&zfs, pool_name, mountpoint).await?;

            // Pool-level encryption: load the pool key so the new BE can be
            // created as an encrypted child. Dataset-level encryption applies
            // only to the new base dataset; the pool itself is not encrypted.
            if encryption == ZfsEncryptionMode::Pool {
                let key_loc = format!("file://{}", key_path.display());
                zfs.dataset(pool_name)
                    .load_key_with_keylocation(&key_loc)
                    .await?;
            }

            let base_refs = base_dataset_props(encryption, &key_path, &compression);
            let base_refs_view: Vec<(&str, &str)> =
                base_refs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            crate::dataset_layout::create_base_dataset(&zfs, pool_name, prefix, &base_refs_view)
                .await?;

            let datasets = crate::dataset_layout::default_datasets();
            crate::dataset_layout::create_child_datasets(&zfs, pool_name, prefix, &datasets)
                .await?;
            tracing::info!("Created new BE in existing pool");
        }
    }

    let datasets = crate::dataset_layout::default_datasets();
    crate::dataset_layout::mount_datasets_ordered(&zfs, pool_name, prefix, &datasets).await?;
    tracing::info!("Datasets mounted");

    Ok(())
}

fn base_dataset_props(
    encryption: ZfsEncryptionMode,
    key_path: &Path,
    compression: &str,
) -> Vec<(&'static str, String)> {
    match encryption {
        ZfsEncryptionMode::Dataset => {
            let mut p = crate::zfs_keyfile::dataset_encryption_properties(key_path);
            p.push(("mountpoint", "none".to_string()));
            p.push(("compression", compression.to_string()));
            p
        }
        _ => vec![
            ("mountpoint", "none".to_string()),
            ("compression", compression.to_string()),
        ],
    }
}

/// Defaults applied at every `zpool create`. `autotrim` is intentionally
/// absent — it is set dynamically post-create based on storage type
/// (NVMe / SATA SSD / HDD) by `Installer::configure_zfs_trim`.
fn apply_default_pool_options(opts: PoolCreateOptions) -> PoolCreateOptions {
    opts.pool_property("ashift", "12")
        .fs_property("acltype", "posixacl")
        .fs_property("relatime", "on")
        .fs_property("xattr", "sa")
        .fs_property("dnodesize", "auto")
        .fs_property("normalization", "formD")
        .fs_property("devices", "off")
        .mountpoint("none")
}

async fn create_pool(
    zfs: &palimpsest::Zfs,
    name: &str,
    device: &Path,
    mountpoint: &Path,
    compression: &str,
    extra_props: &[(&str, &str)],
) -> Result<()> {
    let mut opts = apply_default_pool_options(PoolCreateOptions::new(name))
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

async fn import_pool_no_mount(zfs: &palimpsest::Zfs, name: &str, mountpoint: &Path) -> Result<()> {
    let opts = ImportOptions {
        no_mount: true,
        altroot: Some(mountpoint.to_path_buf()),
        ..Default::default()
    };
    zfs.pool(name).import(&opts).await?;
    Ok(())
}

async fn export_pool(zfs: &palimpsest::Zfs, name: &str) -> Result<()> {
    nix::unistd::sync();
    // Best-effort umount-all first; ignore errors. palimpsest's unmount_all
    // returns Err on real failures (e.g., a stuck mountpoint), but the
    // subsequent zpool export will surface the same condition more clearly.
    let _ = zfs.unmount_all(false).await;
    zfs.pool(name).export(&ExportOptions::default()).await?;
    Ok(())
}
