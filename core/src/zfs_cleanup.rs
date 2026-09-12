//! End-of-install ZFS cleanup. Runs after the installer pipeline finishes
//! its non-ZFS work (pacstrap, chroot config, ZBM install) — unmounts
//! filesystems and exports only the operation-owned pool.
//!
//! Lives in core (rather than each UI crate) so the TUI and Slint installers
//! share the same logic and don't depend on zfskit directly.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail};

use crate::config::types::{GlobalConfig, InstallationMode};
use crate::system::cmd::{CommandRunner, check_exit};

/// Whether `pool_name` is currently imported. Used to avoid exporting a pool
/// that was already owned by the live environment before the installer ran.
pub async fn pool_is_imported(pool_name: &str) -> Result<bool> {
    let pools = zfskit::Zfs::new()
        .list_pools(&zfskit::pool::ListOptions::default())
        .await?;
    Ok(pools.iter().any(|pool| pool.name == pool_name))
}

/// Resources owned by one operation. The root mount is checked before recursive
/// unmount, and only an explicitly owned pool may be exported. Cleanup is retryable.
#[derive(Debug)]
pub struct OwnedMounts {
    pub root: std::path::PathBuf,
    pub root_dataset: String,
    pub pool_to_export: Option<String>,
}

impl OwnedMounts {
    pub fn cleanup(&mut self, runner: &dyn crate::system::cmd::CommandRunner) -> Result<()> {
        use crate::system::cmd::check_exit;
        use color_eyre::eyre::ensure;
        ensure!(
            self.root.is_absolute() && self.root != std::path::Path::new("/"),
            "Refusing cleanup of an unsafe root"
        );
        let root = self.root.to_string_lossy();
        let mount = runner.run(
            "findmnt",
            &[
                "--mountpoint",
                &root,
                "--noheadings",
                "--raw",
                "--output",
                "SOURCE",
            ],
        )?;
        match mount.exit_code {
            0 => {
                ensure!(
                    mount.stdout.trim() == self.root_dataset,
                    "Mount at {} is not owned by this operation: {}",
                    root,
                    mount.stdout.trim()
                );
                let output = runner.run("umount", &["--recursive", &root])?;
                check_exit(&output, "unmount operation's filesystems")?;
            }
            1 if mount.stdout.trim().is_empty() && mount.stderr.trim().is_empty() => {}
            _ => check_exit(&mount, "inspect operation's mount")?,
        }
        if let Some(pool) = &self.pool_to_export {
            let output = runner.run("zpool", &["export", pool])?;
            check_exit(&output, "export operation's pool")?;
            self.pool_to_export = None;
        }
        // The mountpoints stay behind as empty directories, and ZFS will not
        // mount the next attempt's root over them.
        crate::target_dir::prune_empty_dirs(&self.root);
        Ok(())
    }
}

/// Export a pool with the configured name that an earlier attempt left
/// imported, when it lives entirely on the devices this installation is
/// about to use: that is the earlier attempt's leftover, and creating the
/// pool again would be refused while it is imported. A same-named pool on
/// other devices is somebody's data; the installation is refused instead.
///
/// Returns whether a pool was exported.
pub fn release_leftover_pool(
    runner: &dyn CommandRunner,
    pool: &str,
    config: &GlobalConfig,
) -> Result<bool> {
    if config.installation_mode == Some(InstallationMode::ExistingPool) {
        return Ok(false);
    }
    let Some(devices) = imported_pool_devices(runner, pool)? else {
        return Ok(false);
    };
    let targets = target_devices(config);
    let on_target = !devices.is_empty()
        && devices.iter().all(|device| {
            targets
                .iter()
                .any(|target| same_or_partition_of(device, target))
        });
    let shown: Vec<String> = devices.iter().map(|d| d.display().to_string()).collect();
    if !on_target {
        bail!(
            "A pool named {pool} is already imported from {}, which this installation does not use. Choose another pool name or export that pool first.",
            shown.join(", ")
        );
    }
    tracing::warn!(pool, devices = ?shown, "exporting the pool an earlier attempt left imported");
    let output = runner.run("zpool", &["export", pool])?;
    if !output.success() {
        let forced = runner.run("zpool", &["export", "-f", pool])?;
        check_exit(&forced, "export the earlier attempt's pool")?;
    }
    Ok(true)
}

/// The devices of an imported pool, canonical paths; `None` when no pool of
/// that name is imported.
fn imported_pool_devices(runner: &dyn CommandRunner, pool: &str) -> Result<Option<Vec<PathBuf>>> {
    let output = runner.run("zpool", &["list", "-H", "-P", "-v", "-o", "name", pool])?;
    if !output.success() {
        return Ok(None);
    }
    Ok(Some(parse_pool_devices(&output.stdout)))
}

/// `zpool list -H -P -v` prints the pool, then one indented line per vdev;
/// leaf vdevs are absolute device paths, groups are names like `mirror-0`.
fn parse_pool_devices(listing: &str) -> Vec<PathBuf> {
    listing
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .filter(|token| token.starts_with('/'))
        .map(|token| {
            let path = PathBuf::from(token);
            std::fs::canonicalize(&path).unwrap_or(path)
        })
        .collect()
}

/// The devices this configuration writes to: a whole disk, or the partition
/// a new pool goes on.
fn target_devices(config: &GlobalConfig) -> Vec<PathBuf> {
    let raw: Vec<&Path> = match config.installation_mode {
        Some(InstallationMode::FullDisk) => config.disk.iter().map(PathBuf::as_path).collect(),
        Some(InstallationMode::Alongside) => config
            .alongside
            .iter()
            .map(|request| request.before.device.as_path())
            .collect(),
        Some(InstallationMode::NewPool) => {
            config.zfs_partition.iter().map(PathBuf::as_path).collect()
        }
        _ => Vec::new(),
    };
    raw.into_iter()
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
        .collect()
}

/// Whether `device` is `target` itself or a partition of it, judged by the
/// kernel's block device tree.
fn same_or_partition_of(device: &Path, target: &Path) -> bool {
    if device == target {
        return true;
    }
    let Some(name) = device.file_name() else {
        return false;
    };
    let Ok(link) = std::fs::read_link(Path::new("/sys/class/block").join(name)) else {
        return false;
    };
    // .../block/sda/sda6: the parent disk is the component before the last.
    let mut components = link.components().rev();
    components.next();
    components
        .next()
        .and_then(|parent| target.file_name().map(|t| parent.as_os_str() == t))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    fn ok(s: &str) -> CannedResponse {
        CannedResponse {
            stdout: s.into(),
            ..Default::default()
        }
    }
    fn fail(s: &str) -> CannedResponse {
        CannedResponse {
            stderr: s.into(),
            exit_code: 1,
            ..Default::default()
        }
    }

    fn mounts() -> OwnedMounts {
        OwnedMounts {
            root: "/run/azfs-test".into(),
            root_dataset: "pool/arch0/root".into(),
            pool_to_export: Some("pool".into()),
        }
    }

    #[test]
    fn busy_mount_preserves_export_for_retry() {
        let runner = RecordingRunner::new(vec![ok("pool/arch0/root\n"), fail("busy")]);
        let mut mounts = mounts();
        assert!(mounts.cleanup(&runner).is_err());
        assert_eq!(mounts.pool_to_export.as_deref(), Some("pool"));
        assert!(runner.calls().iter().all(|c| c.program != "zpool"));
    }

    #[test]
    fn foreign_root_is_never_unmounted() {
        let runner = RecordingRunner::new(vec![ok("other/root\n")]);
        assert!(mounts().cleanup(&runner).is_err());
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn no_global_or_forced_cleanup_and_success_is_retryable() {
        let runner = RecordingRunner::new(vec![
            ok("pool/arch0/root\n"),
            ok(""),
            ok(""),
            CannedResponse {
                stdout: "".into(),
                stderr: "".into(),
                exit_code: 1,
            },
        ]);
        let mut mounts = mounts();
        mounts.cleanup(&runner).unwrap();
        mounts.cleanup(&runner).unwrap();
        assert!(mounts.pool_to_export.is_none());
        assert_eq!(
            runner
                .calls()
                .iter()
                .filter(|c| c.program == "zpool")
                .count(),
            1
        );
        assert!(
            runner
                .calls()
                .iter()
                .all(|c| !c.args.iter().any(|a| a == "-a" || a == "-f"))
        );
    }
}

#[cfg(test)]
mod leftover_tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    fn ok(s: &str) -> CannedResponse {
        CannedResponse {
            stdout: s.into(),
            ..Default::default()
        }
    }

    fn new_pool_config(partition: &str) -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::NewPool),
            zfs_partition: Some(PathBuf::from(partition)),
            ..GlobalConfig::default()
        }
    }

    #[test]
    fn vdev_paths_are_read_from_the_listing() {
        let listing = "tank\n\tmirror-0\n\t\t/nonexistent/disk-a\n\t\t/nonexistent/disk-b\n";
        assert_eq!(
            parse_pool_devices(listing),
            vec![
                PathBuf::from("/nonexistent/disk-a"),
                PathBuf::from("/nonexistent/disk-b")
            ]
        );
    }

    #[test]
    fn a_pool_on_the_target_partition_is_exported() {
        let runner = RecordingRunner::new(vec![
            ok("zroot\n\t/nonexistent/part6\n"),
            CannedResponse::default(),
        ]);
        let exported =
            release_leftover_pool(&runner, "zroot", &new_pool_config("/nonexistent/part6"))
                .unwrap();
        assert!(exported);
        let last = runner.calls().pop().unwrap();
        assert_eq!(last.args, vec!["export", "zroot"]);
    }

    #[test]
    fn a_pool_elsewhere_stops_the_installation() {
        let runner = RecordingRunner::new(vec![ok("zroot\n\t/nonexistent/other\n")]);
        let error = release_leftover_pool(&runner, "zroot", &new_pool_config("/nonexistent/part6"))
            .unwrap_err();
        assert!(error.to_string().contains("/nonexistent/other"), "{error}");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn no_imported_pool_means_nothing_to_do() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            stderr: "cannot open 'zroot': no such pool".into(),
            exit_code: 1,
            ..Default::default()
        }]);
        assert!(
            !release_leftover_pool(&runner, "zroot", &new_pool_config("/nonexistent/part6"))
                .unwrap()
        );
    }

    #[test]
    fn an_existing_pool_installation_keeps_its_pool() {
        let runner = RecordingRunner::new(vec![]);
        let config = GlobalConfig {
            installation_mode: Some(InstallationMode::ExistingPool),
            ..GlobalConfig::default()
        };
        assert!(!release_leftover_pool(&runner, "zroot", &config).unwrap());
        assert!(runner.calls().is_empty());
    }
}
