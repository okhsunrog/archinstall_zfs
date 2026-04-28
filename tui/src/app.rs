use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, bail, eyre};
use tokio_util::sync::CancellationToken;

use archinstall_zfs_core::config::types::{GlobalConfig, SwapMode};
use archinstall_zfs_core::system::cmd::{CommandRunner, RealRunner};

use crate::Cli;

pub async fn run(
    cli: Cli,
    ui_log_rx: tokio::sync::mpsc::UnboundedReceiver<(String, i32)>,
) -> Result<()> {
    let config = if let Some(ref path) = cli.config {
        tracing::info!(path = %path.display(), "loading config from file");
        GlobalConfig::load_from_file(path)?
    } else {
        GlobalConfig::default()
    };

    if cli.silent {
        if cli.config.is_none() {
            bail!("--silent requires --config");
        }
        let errors = config.validate_for_install();
        if !errors.is_empty() {
            bail!("Config validation failed:\n  {}", errors.join("\n  "));
        }
        tracing::info!("silent mode: config valid, starting installation");
        let runner: Arc<dyn CommandRunner> = Arc::new(RealRunner);
        let cancel = CancellationToken::new();
        return run_install(runner, config, cancel, None).await;
    }

    // Interactive TUI mode
    crate::tui::run_tui(config, cli.dry_run, ui_log_rx).await
}

/// Full installation pipeline — async orchestrator.
///
/// Sync operations (CommandRunner subprocess calls, Alpm FFI) run inside
/// `spawn_blocking`. Async operations (HTTP, AUR resolver, package downloads)
/// are awaited directly.
pub async fn run_install(
    runner: Arc<dyn CommandRunner>,
    config: GlobalConfig,
    cancel: CancellationToken,
    download_progress_tx: Option<
        Arc<
            tokio::sync::watch::Sender<
                archinstall_zfs_core::system::async_download::DownloadProgress,
            >,
        >,
    >,
) -> Result<()> {
    let mountpoint = PathBuf::from("/mnt");
    let pool_name = config
        .pool_name
        .as_deref()
        .ok_or_else(|| eyre!("pool name not set"))?
        .to_string();
    let prefix = config.dataset_prefix.clone();
    let kernel = config.primary_kernel().to_string();
    let config = Arc::new(config);

    // ── Phase 0: Pre-installation checks ───────────────────────
    tracing::info!("Phase 0: Pre-installation checks");

    if !archinstall_zfs_core::system::net::check_internet() {
        bail!("No internet connectivity. Connect to the network and retry.");
    }
    tracing::info!("Internet connectivity OK");

    if !archinstall_zfs_core::system::sysinfo::has_uefi() {
        bail!("UEFI boot required. This installer only supports UEFI systems.");
    }
    tracing::info!("UEFI boot detected");

    // Async: HTTP calls to validate kernel/ZFS compatibility
    let warnings = archinstall_zfs_core::kernel::scanner::validate_kernel_zfs_plan(
        &kernel,
        config.zfs_module_mode,
    )
    .await;
    for w in &warnings {
        tracing::warn!("kernel compatibility: {w}");
    }

    // Sync: initialize ZFS on host (runner + alpm)
    {
        let r = runner.clone();
        let k = kernel.clone();
        let zfs_mode = config.zfs_module_mode;
        let c = cancel.clone();
        let dl_config = archinstall_zfs_core::system::async_download::DownloadConfig {
            concurrency: config.parallel_downloads as usize,
            ..Default::default()
        };
        tokio::task::spawn_blocking(move || {
            archinstall_zfs_core::zfs::kmod::initialize_zfs(&*r, &k, zfs_mode, &c, dl_config)
        })
        .await??;
    }
    tracing::info!("ZFS initialized on host");

    // ── Phase 1: Disk preparation (sync) ──────────────────────
    tracing::info!("Phase 1: Disk preparation");
    let (efi_partition, zfs_partition, swap_partition) = {
        let r = runner.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || -> Result<_> {
            let parts = archinstall_zfs_core::prepare::prepare_disk(&*r, &config)?;
            Ok((parts.efi, parts.zfs, parts.swap))
        })
        .await??
    };

    // ── Phase 2: ZFS pool + datasets + encryption (sync) ──────
    tracing::info!("Phase 2: ZFS pool and datasets");
    {
        let r = runner.clone();
        let zfs_partition = zfs_partition.clone();
        let efi_partition = efi_partition.clone();
        let mountpoint = mountpoint.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            archinstall_zfs_core::prepare::prepare_zfs(
                &*r,
                &config,
                zfs_partition.as_deref(),
                &mountpoint,
            )?;

            // ── Phase 3: Mount EFI ─────────────────────────────────────
            tracing::info!("Phase 3: Mounting EFI partition");
            archinstall_zfs_core::disk::partition::mount_efi(&*r, &efi_partition, &mountpoint)?;

            Ok(())
        })
        .await??;
    }

    // ── Phases 4-12: Installer pipeline (sync — AlpmContext is !Send) ──
    tracing::info!("Phase 4-12: Running installer pipeline");
    {
        let r = runner.clone();
        let config = config.clone();
        let mountpoint = mountpoint.clone();
        let cancel = cancel.clone();
        let swap = swap_partition;
        let download_tx = download_progress_tx.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut installer = archinstall_zfs_core::installer::Installer::new(
                r,
                (*config).clone(),
                &mountpoint,
                cancel,
                download_tx,
            );
            if let Some(swap) = swap {
                installer.set_swap_partition(swap);
            }
            installer.perform_installation()
        })
        .await??;
    }

    // ── Phase 13: ZFSBootMenu ──────────────────────────────────
    tracing::info!("Phase 13: Setting up ZFSBootMenu");

    let zswap_on = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );
    {
        let r = runner.clone();
        let pool_name = pool_name.clone();
        let prefix = prefix.clone();
        let init_system = config.init_system;
        let set_bootfs = config.set_bootfs;
        tokio::task::spawn_blocking(move || {
            archinstall_zfs_core::zfs::bootmenu::set_zbm_properties(
                &*r,
                &pool_name,
                &prefix,
                init_system,
                zswap_on,
                set_bootfs,
            )
        })
        .await??;
    }

    archinstall_zfs_core::zfs::bootmenu::install_and_generate_zbm(
        runner.clone(),
        &mountpoint,
        config.init_system,
        &cancel,
        archinstall_zfs_core::system::async_download::DownloadConfig {
            concurrency: config.parallel_downloads as usize,
            ..Default::default()
        },
    )
    .await?;
    tracing::info!("ZFSBootMenu built and installed");

    {
        let r = runner.clone();
        let efi = efi_partition.clone();
        tokio::task::spawn_blocking(move || {
            archinstall_zfs_core::zfs::bootmenu::create_efi_entries(&*r, &efi)
        })
        .await??;
    }

    // ── Phase 14: Cleanup ──────────────────────────────────────
    tracing::info!("Phase 14: Cleanup");
    let zfs = palimpsest::Zfs::new();
    let root_ds_full = format!("{pool_name}/{prefix}/root");

    // sync(2) + umount_efi are sync but quick. Wrap in spawn_blocking so we
    // don't block the tokio worker on the FFI/subprocess.
    {
        let r = runner.clone();
        let mp = mountpoint.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            nix::unistd::sync();
            archinstall_zfs_core::disk::partition::umount_efi(&*r, &mp)?;
            Ok(())
        })
        .await??;
    }

    // Unmount filesystems with escalating force. Each attempt is best-effort;
    // palimpsest's unmount is idempotent on "not currently mounted" so the
    // second pass after a successful first pass returns Ok without noise.
    let root_ds = zfs.dataset(&root_ds_full);
    for attempt in 1..=4 {
        let _ = match attempt {
            1 => zfs.unmount_all(false).await,
            2 => {
                root_ds
                    .unmount(&palimpsest::dataset::UnmountOptions::default())
                    .await
            }
            3 => zfs.unmount_all(true).await,
            4 => {
                root_ds
                    .unmount(&palimpsest::dataset::UnmountOptions { force: true })
                    .await
            }
            _ => unreachable!(),
        };
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let _ = tokio::task::spawn_blocking(nix::unistd::sync).await;
    }

    let pool = zfs.pool(pool_name.as_str());
    if pool
        .export(&palimpsest::pool::ExportOptions::default())
        .await
        .is_err()
    {
        tracing::warn!("zpool export failed, trying force");
        let _ = pool
            .export(&palimpsest::pool::ExportOptions { force: true })
            .await;
    }

    tracing::info!("Installation complete!");
    Ok(())
}
