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
    archinstall_zfs_core::prepare::prepare_zfs(
        &*runner,
        config,
        zfs_partition.as_deref(),
        &mountpoint,
    )?;

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
    archinstall_zfs_core::zfs::bootmenu::set_zbm_properties(
        &*runner,
        pool_name,
        prefix,
        config.init_system,
        zswap_on,
        config.set_bootfs,
    )?;
    rt.block_on(
        archinstall_zfs_core::zfs::bootmenu::install_and_generate_zbm(
            runner.clone(),
            &mountpoint,
            config.init_system,
            &cancel,
            archinstall_zfs_core::system::async_download::DownloadConfig {
                concurrency: config.parallel_downloads as usize,
                ..Default::default()
            },
        ),
    )?;
    archinstall_zfs_core::zfs::bootmenu::create_efi_entries(&*runner, &efi_partition)?;

    tracing::info!("Phase 14: Cleanup");
    tracing::info!(target: "metrics", event = "phase_start", num = 14u32, name = "Cleanup");
    nix::unistd::sync();
    let root_ds = format!("{pool_name}/{prefix}/root");
    archinstall_zfs_core::disk::partition::umount_efi(&*runner, &mountpoint)?;

    for attempt in 1..=4 {
        let _result = match attempt {
            1 => runner.run("zfs", &["umount", "-a"]),
            2 => runner.run("zfs", &["unmount", &root_ds]),
            3 => runner.run("zfs", &["umount", "-af"]),
            4 => runner.run("zfs", &["unmount", "-f", &root_ds]),
            _ => unreachable!(),
        };
        std::thread::sleep(std::time::Duration::from_secs(1));
        nix::unistd::sync();
    }

    let output = runner.run("zpool", &["export", pool_name])?;
    if !output.success() {
        tracing::warn!("zpool export failed, trying force");
        let _ = runner.run("zpool", &["export", "-f", pool_name]);
    }

    tracing::info!("Installation complete!");
    Ok(())
}
