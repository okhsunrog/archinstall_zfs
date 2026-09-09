//! End-of-install ZFS cleanup. Runs after the installer pipeline finishes
//! its non-ZFS work (pacstrap, chroot config, ZBM install) — unmounts
//! filesystems and exports only the operation-owned pool.
//!
//! Lives in core (rather than each UI crate) so the TUI and Slint installers
//! share the same logic and don't depend on zfskit directly.

use color_eyre::eyre::Result;

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
        Ok(())
    }
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
