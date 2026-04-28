use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, eyre};
use tokio_util::sync::CancellationToken;

use archinstall_zfs_core::config::types::{GlobalConfig, SwapMode};
use archinstall_zfs_core::system::async_download::DownloadProgress;
use archinstall_zfs_core::system::cmd::CommandRunner;

/// Full installation pipeline. All progress reported via tracing.
/// Must be called from a thread with tokio runtime context (Handle::current() must work).
pub fn run_install(
    runner: Arc<dyn CommandRunner>,
    config: &GlobalConfig,
    download_progress_tx: Option<Arc<tokio::sync::watch::Sender<DownloadProgress>>>,
) -> Result<()> {
    let cancel = CancellationToken::new();
    let rt = tokio::runtime::Handle::current();
    let mountpoint = PathBuf::from("/mnt");
    let pool_name = config
        .pool_name
        .as_deref()
        .ok_or_else(|| eyre!("pool name not set"))?;
    let prefix = &config.dataset_prefix;

    // Phase 0 checks (internet, UEFI, ZFS) are done on the welcome screen.

    tracing::info!("Phase 1: Disk preparation");
    tracing::info!(target: "metrics", event = "phase_start", num = 1u32, name = "Disk preparation");
    let parts = archinstall_zfs_core::prepare::prepare_disk(&*runner, config)?;
    let efi_partition = parts.efi;
    let zfs_partition = parts.zfs;
    let swap_partition = parts.swap;

    tracing::info!("Phase 2: ZFS pool and datasets");
    tracing::info!(target: "metrics", event = "phase_start", num = 2u32, name = "ZFS pool and datasets");
    rt.block_on(archinstall_zfs_core::prepare::prepare_zfs(
        &*runner,
        config,
        zfs_partition.as_deref(),
        &mountpoint,
    ))?;

    tracing::info!("Phase 3: Mounting EFI partition");
    tracing::info!(target: "metrics", event = "phase_start", num = 3u32, name = "Mounting EFI partition");
    archinstall_zfs_core::disk::partition::mount_efi(&*runner, &efi_partition, &mountpoint)?;

    tracing::info!("Phase 4-12: Running installer pipeline");
    let mut installer = archinstall_zfs_core::installer::Installer::new(
        runner.clone(),
        config.clone(),
        &mountpoint,
        cancel.clone(),
        download_progress_tx,
    );
    if let Some(swap) = swap_partition {
        installer.set_swap_partition(swap);
    }
    installer.perform_installation()?;

    tracing::info!("Phase 13: Setting up ZFSBootMenu");
    tracing::info!(target: "metrics", event = "phase_start", num = 13u32, name = "Setting up ZFSBootMenu");
    let zswap_on = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );
    rt.block_on(archinstall_zfs_core::bootmenu::set_zbm_properties(
        pool_name,
        prefix,
        config.init_system,
        zswap_on,
        config.set_bootfs,
    ))?;
    rt.block_on(archinstall_zfs_core::bootmenu::install_and_generate_zbm(
        runner.clone(),
        &mountpoint,
        config.init_system,
        &cancel,
        archinstall_zfs_core::system::async_download::DownloadConfig {
            concurrency: config.parallel_downloads as usize,
            ..Default::default()
        },
    ))?;
    archinstall_zfs_core::bootmenu::create_efi_entries(&*runner, &efi_partition)?;

    tracing::info!("Phase 14: Cleanup");
    tracing::info!(target: "metrics", event = "phase_start", num = 14u32, name = "Cleanup");
    nix::unistd::sync();
    let root_ds_full = format!("{pool_name}/{prefix}/root");
    archinstall_zfs_core::disk::partition::umount_efi(&*runner, &mountpoint)?;

    rt.block_on(
        archinstall_zfs_core::zfs_cleanup::cleanup_pool_after_install(pool_name, &root_ds_full),
    )?;

    tracing::info!("Installation complete!");
    Ok(())
}
