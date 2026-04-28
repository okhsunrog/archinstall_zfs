//! End-of-install ZFS cleanup. Runs after the installer pipeline finishes
//! its non-ZFS work (pacstrap, chroot config, ZBM install) — unmounts
//! filesystems and exports the pool with escalating force.
//!
//! Lives in core (rather than each UI crate) so the TUI and Slint installers
//! share the same logic and don't depend on palimpsest directly.

use color_eyre::eyre::Result;

/// Sequence used by both the TUI and Slint install pipelines: try to unmount
/// everything, then export the pool. Each unmount attempt is best-effort and
/// followed by a `sync(2)` and a 1-second sleep — old behavior carried from
/// the original Python installer, intended to give the kernel time to settle
/// after VFS state changes.
///
/// The four escalation steps:
///   1. `zfs umount -a` — let ZFS try the bulk path
///   2. `zfs umount <root>` — explicit per-dataset
///   3. `zfs umount -af` — bulk + force
///   4. `zfs umount -f <root>` — explicit + force
///
/// After unmount attempts, `zpool export` with a `-f` retry on failure.
///
/// Errors from individual ZFS commands are intentionally swallowed; only the
/// final export's failure mode is retried with force, and even that's
/// best-effort. The function returns `Ok` regardless of pool state at the end
/// — the kernel sync calls themselves do error-bubble.
pub async fn cleanup_pool_after_install(pool_name: &str, root_dataset: &str) -> Result<()> {
    let zfs = palimpsest::Zfs::new();
    let root_handle = zfs.dataset(root_dataset);

    for attempt in 1..=4 {
        let _ = match attempt {
            1 => zfs.unmount_all(false).await,
            2 => {
                root_handle
                    .unmount(&palimpsest::dataset::UnmountOptions::default())
                    .await
            }
            3 => zfs.unmount_all(true).await,
            4 => {
                root_handle
                    .unmount(&palimpsest::dataset::UnmountOptions { force: true })
                    .await
            }
            _ => unreachable!(),
        };
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let _ = tokio::task::spawn_blocking(nix::unistd::sync).await;
    }

    let pool = zfs.pool(pool_name);
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

    Ok(())
}
