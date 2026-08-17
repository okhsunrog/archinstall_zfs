//! The installation pipeline.
//!
//! Both user interfaces drive the same function, and differ only in how they
//! render the progress it reports. Keeping one copy is what stops them
//! drifting: they previously carried separate transcriptions of these phases,
//! and the graphical one had silently lost the kernel compatibility check
//! along the way.
//!
//! Progress is reported two ways. Phase transitions and everything else go out
//! as tracing events, which each interface picks up with its own subscriber
//! layer; per-package download progress goes through `progress_tx`, because it
//! updates far too often to be worth formatting as text.
//!
//! Work is split by what it needs rather than by phase: subprocess calls and
//! libalpm (whose handle is `!Send`) run inside `spawn_blocking`, while ZFS
//! operations and HTTP are awaited directly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use color_eyre::eyre::{Result, bail, eyre};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::boot_environment::BootEnvironment;
use crate::config::types::{GlobalConfig, SwapMode};
use crate::system::async_download::{DownloadConfig, DownloadProgress};
use crate::system::cmd::CommandRunner;

/// Where the pool is mounted while the target system is assembled.
const MOUNTPOINT: &str = "/mnt";

type ProgressSender = Arc<watch::Sender<DownloadProgress>>;

/// Run the installation described by `config`.
///
/// Cancellation is checked between phases and inside the package downloader;
/// a cancelled run unwinds through the same cleanup as a failed one.
pub async fn run_install(
    runner: Arc<dyn CommandRunner>,
    config: GlobalConfig,
    cancel: CancellationToken,
    progress_tx: Option<ProgressSender>,
) -> Result<()> {
    let pool_name = config
        .pool_name
        .as_deref()
        .ok_or_else(|| eyre!("pool name not set"))?
        .to_string();
    let root_dataset = BootEnvironment::new(&pool_name, config.dataset_prefix.as_str()).root();
    let cleanup = Arc::new(CleanupState::default());

    let result = install(runner.clone(), config, cancel, progress_tx, cleanup.clone()).await;

    cleanup
        .run(&*runner, &pool_name, &root_dataset, result.is_ok())
        .await?;

    result?;
    tracing::info!("Installation complete!");
    Ok(())
}

/// What the pipeline got far enough to create, and therefore what has to be
/// undone afterwards.
struct CleanupState {
    /// The pipeline started creating or importing the pool, so it is this
    /// installation's to export.
    pool_setup_started: AtomicBool,
    efi_mounted: AtomicBool,
    /// The pool was already imported before the pipeline ran, so it belongs to
    /// the live environment and must be left alone. Defaults to true so a pool
    /// whose initial ownership could not be established is never exported.
    pool_preexisting: AtomicBool,
}

impl Default for CleanupState {
    fn default() -> Self {
        Self {
            pool_setup_started: AtomicBool::new(false),
            efi_mounted: AtomicBool::new(false),
            pool_preexisting: AtomicBool::new(true),
        }
    }
}

impl CleanupState {
    /// Unmount and export what this installation created.
    ///
    /// A cleanup failure is fatal to a successful installation — reporting
    /// success while the installer still holds the pool would leave the user
    /// to discover it at reboot — but only logged after a failed one, where it
    /// would otherwise mask the error that actually stopped the install.
    async fn run(
        &self,
        runner: &dyn CommandRunner,
        pool_name: &str,
        root_dataset: &str,
        install_succeeded: bool,
    ) -> Result<()> {
        if !self.pool_setup_started.load(Ordering::Acquire)
            || self.pool_preexisting.load(Ordering::Acquire)
        {
            return Ok(());
        }

        tracing::info!("Phase 14: Cleanup");
        tracing::info!(target: "metrics", event = "phase_start", num = 14u32, name = "Cleanup");

        if self.efi_mounted.load(Ordering::Acquire) {
            nix::unistd::sync();
            let _ = crate::disk::partition::umount_efi(runner, Path::new(MOUNTPOINT));
        }

        let result = crate::zfs_cleanup::cleanup_pool_after_install(pool_name, root_dataset).await;
        match result {
            Ok(()) => Ok(()),
            Err(error) if install_succeeded => Err(error),
            Err(error) => {
                tracing::error!(%error, "cleanup after failed or cancelled installation failed");
                Ok(())
            }
        }
    }
}

async fn install(
    runner: Arc<dyn CommandRunner>,
    config: GlobalConfig,
    cancel: CancellationToken,
    progress_tx: Option<ProgressSender>,
    cleanup: Arc<CleanupState>,
) -> Result<()> {
    let mountpoint = PathBuf::from(MOUNTPOINT);
    let pool_name = config
        .pool_name
        .as_deref()
        .ok_or_else(|| eyre!("pool name not set"))?
        .to_string();
    let be = BootEnvironment::new(&pool_name, config.dataset_prefix.as_str());
    let kernel = config.primary_kernel().to_string();
    let download_config = DownloadConfig {
        concurrency: config.parallel_downloads as usize,
        ..Default::default()
    };
    let config = Arc::new(config);

    ensure_not_cancelled(&cancel)?;

    // ── Phase 0: Pre-installation checks ───────────────────────
    tracing::info!("Phase 0: Pre-installation checks");

    if !crate::system::net::check_internet() {
        bail!("No internet connectivity. Connect to the network and retry.");
    }
    tracing::info!("Internet connectivity OK");

    if !crate::system::sysinfo::has_uefi() {
        bail!("UEFI boot required. This installer only supports UEFI systems.");
    }
    tracing::info!("UEFI boot detected");

    for warning in
        crate::kernel::scanner::validate_kernel_zfs_plan(&kernel, config.zfs_module_mode).await
    {
        tracing::warn!("kernel compatibility: {warning}");
    }

    // ZFS on the live system. Returns early when the module and tools are
    // already present, which is the usual case once a UI has done this while
    // the user was still filling in the wizard.
    {
        let runner = runner.clone();
        let kernel = kernel.clone();
        let zfs_mode = config.zfs_module_mode;
        let cancel = cancel.clone();
        let download_config = download_config.clone();
        tokio::task::spawn_blocking(move || {
            crate::zfs_setup::initialize_zfs(&*runner, &kernel, zfs_mode, &cancel, download_config)
        })
        .await??;
    }
    tracing::info!("ZFS initialized on host");

    cleanup.pool_preexisting.store(
        crate::zfs_cleanup::pool_is_imported(&pool_name)
            .await
            .unwrap_or(true),
        Ordering::Release,
    );
    ensure_not_cancelled(&cancel)?;

    // ── Phase 1: Disk preparation ──────────────────────────────
    tracing::info!("Phase 1: Disk preparation");
    tracing::info!(target: "metrics", event = "phase_start", num = 1u32, name = "Disk preparation");
    let (efi_partition, zfs_partition, swap_partition) = {
        let runner = runner.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || -> Result<_> {
            let parts = crate::prepare::prepare_disk(&*runner, &config)?;
            Ok((parts.efi, parts.zfs, parts.swap))
        })
        .await??
    };

    // ── Phase 2: ZFS pool + datasets + encryption ──────────────
    tracing::info!("Phase 2: ZFS pool and datasets");
    tracing::info!(target: "metrics", event = "phase_start", num = 2u32, name = "ZFS pool and datasets");
    ensure_not_cancelled(&cancel)?;
    cleanup.pool_setup_started.store(true, Ordering::Release);
    crate::prepare::prepare_zfs(&*runner, &config, zfs_partition.as_deref(), &mountpoint).await?;

    // ── Phase 3: EFI partition ─────────────────────────────────
    tracing::info!("Phase 3: Mounting EFI partition");
    tracing::info!(target: "metrics", event = "phase_start", num = 3u32, name = "Mounting EFI partition");
    ensure_not_cancelled(&cancel)?;
    {
        let runner = runner.clone();
        let efi = efi_partition.clone();
        let mountpoint = mountpoint.clone();
        tokio::task::spawn_blocking(move || {
            crate::disk::partition::mount_efi(&*runner, &efi, &mountpoint)
        })
        .await??;
    }
    cleanup.efi_mounted.store(true, Ordering::Release);

    // ── Phases 4-12: Installer pipeline ────────────────────────
    // One blocking task for all of it: AlpmContext holds a `!Send` handle and
    // is reused across the package phases, so it cannot cross an await point.
    tracing::info!("Phase 4-12: Running installer pipeline");
    let notices = {
        let runner = runner.clone();
        let config = config.clone();
        let mountpoint = mountpoint.clone();
        let cancel = cancel.clone();
        let progress_tx = progress_tx.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let mut installer = crate::installer::Installer::new(
                runner,
                (*config).clone(),
                &mountpoint,
                cancel,
                progress_tx,
            );
            if let Some(swap) = swap_partition {
                installer.set_swap_partition(swap);
            }
            installer.perform_installation()?;
            Ok(installer.notices().to_vec())
        })
        .await??
    };
    ensure_not_cancelled(&cancel)?;

    // TRIM strategy: post-install ZFS-side configuration (no Alpm involved).
    crate::zfs_trim::configure_zfs_trim(&*runner, &mountpoint, &pool_name, &config).await?;

    // ── Phase 13: ZFSBootMenu ──────────────────────────────────
    tracing::info!("Phase 13: Setting up ZFSBootMenu");
    tracing::info!(target: "metrics", event = "phase_start", num = 13u32, name = "Setting up ZFSBootMenu");
    ensure_not_cancelled(&cancel)?;

    let zswap_on = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );
    crate::bootmenu::set_zbm_properties(&be, config.init_system, zswap_on, config.set_bootfs)
        .await?;

    crate::bootmenu::install_and_generate_zbm(
        runner.clone(),
        &mountpoint,
        config.init_system,
        &cancel,
        download_config,
    )
    .await?;
    tracing::info!("ZFSBootMenu built and installed");

    {
        let runner = runner.clone();
        let efi = efi_partition.clone();
        tokio::task::spawn_blocking(move || crate::bootmenu::create_efi_entries(&*runner, &efi))
            .await??;
    }

    // Last, so they are the final thing in the log the user is looking at
    // rather than something scrolled away thousands of lines ago.
    if !notices.is_empty() {
        tracing::warn!("The installation finished with notices:");
        for notice in &notices {
            tracing::warn!("  • {notice}");
        }
    }

    Ok(())
}

fn ensure_not_cancelled(cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() {
        bail!("installation cancelled");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::RecordingRunner;

    #[tokio::test]
    async fn a_pool_that_was_never_touched_is_not_exported() {
        let cleanup = CleanupState::default();
        let runner = RecordingRunner::new(vec![]);

        // pool_setup_started is false: the run failed before phase 2.
        cleanup
            .run(&runner, "zroot", "zroot/arch0/root", false)
            .await
            .expect("nothing to clean up");

        assert!(
            runner.calls().is_empty(),
            "must not touch an untouched pool"
        );
    }

    #[tokio::test]
    async fn a_pool_owned_by_the_live_environment_is_left_imported() {
        let cleanup = CleanupState::default();
        cleanup.pool_setup_started.store(true, Ordering::Release);
        cleanup.efi_mounted.store(true, Ordering::Release);
        // Imported before the installer ran — exporting it would pull the pool
        // out from under whoever mounted it.
        cleanup.pool_preexisting.store(true, Ordering::Release);
        let runner = RecordingRunner::new(vec![]);

        cleanup
            .run(&runner, "zroot", "zroot/arch0/root", true)
            .await
            .expect("pre-existing pool is left alone");

        assert!(runner.calls().is_empty());
    }

    #[test]
    fn cleanup_defaults_to_treating_the_pool_as_not_ours() {
        let cleanup = CleanupState::default();
        assert!(cleanup.pool_preexisting.load(Ordering::Acquire));
        assert!(!cleanup.pool_setup_started.load(Ordering::Acquire));
        assert!(!cleanup.efi_mounted.load(Ordering::Acquire));
    }

    #[test]
    fn cancelled_token_stops_the_pipeline() {
        let cancel = CancellationToken::new();
        assert!(ensure_not_cancelled(&cancel).is_ok());
        cancel.cancel();
        assert!(ensure_not_cancelled(&cancel).is_err());
    }
}
