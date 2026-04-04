use std::path::Path;

use color_eyre::eyre::Result;

use crate::config::types::ZfsModuleMode;
use crate::system::alpm_pacman::AlpmContext;
use crate::system::cmd::CommandRunner;

pub fn load_zfs_module(runner: &dyn CommandRunner) -> Result<bool> {
    let output = runner.run("modprobe", &["zfs"])?;
    Ok(output.success())
}

pub fn check_zfs_module(runner: &dyn CommandRunner) -> Result<bool> {
    let output = runner.run("lsmod", &[])?;
    let found = output.success() && output.stdout.contains("zfs");
    tracing::info!(found, "check_zfs_module");
    Ok(found)
}

pub fn check_zfs_utils(runner: &dyn CommandRunner) -> Result<bool> {
    // Use 'command -v' via bash since 'which' may be a shell builtin
    let zpool = runner.run("bash", &["-c", "command -v zpool"])?;
    let zfs = runner.run("bash", &["-c", "command -v zfs"])?;
    let found = zpool.success() && zfs.success();
    tracing::info!(found, "check_zfs_utils");
    Ok(found)
}

pub fn increase_cowspace(runner: &dyn CommandRunner) -> Result<()> {
    let output = runner.run(
        "mount",
        &["-o", "remount,size=50%", "/run/archiso/cowspace"],
    )?;
    if !output.success() {
        tracing::warn!("failed to increase cowspace (may not be on live ISO)");
    }
    Ok(())
}

/// Wait for reflector.service to finish (if running), then stop it permanently.
/// Matches Python ensure_reflector_finished_and_stopped().
pub fn ensure_reflector_finished_and_stopped(runner: &dyn CommandRunner) -> Result<()> {
    // Poll SubState until dead/failed/exited
    tracing::info!("checking reflector status...");
    for i in 0..300 {
        let output = runner.run(
            "systemctl",
            &[
                "show",
                "--no-pager",
                "-p",
                "SubState",
                "--value",
                "reflector.service",
            ],
        )?;
        let substate = output.stdout.trim().to_lowercase();
        match substate.as_str() {
            "dead" | "failed" | "exited" | "" => break,
            _ => {
                if i == 0 {
                    tracing::info!(substate, "waiting for reflector to finish...");
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }

    // Stop both units permanently
    let _ = runner.run("systemctl", &["stop", "reflector.service"]);
    let _ = runner.run("systemctl", &["stop", "reflector.timer"]);
    tracing::info!("reflector stopped");
    Ok(())
}

/// Check if the mirrorlist is stale and refresh it with reflector.
/// The testing ISO bakes in a mirrorlist at build time; if the ISO is old,
/// pacman downloads will be extremely slow or fail entirely.
pub fn refresh_mirrors_if_stale(runner: &dyn CommandRunner) -> Result<()> {
    let mirrorlist = std::path::Path::new("/etc/pacman.d/mirrorlist");
    if !mirrorlist.exists() {
        return Ok(());
    }

    // Check the age of the mirrorlist
    let stale = match std::fs::metadata(mirrorlist) {
        Ok(meta) => match meta.modified() {
            Ok(mtime) => {
                let age = std::time::SystemTime::now()
                    .duration_since(mtime)
                    .unwrap_or_default();
                // Consider stale if older than 24 hours
                age.as_secs() > 86400
            }
            Err(_) => true,
        },
        Err(_) => true,
    };

    if !stale {
        tracing::info!("mirrorlist is fresh, skipping reflector");
        return Ok(());
    }

    tracing::info!("mirrorlist is stale, refreshing with reflector...");
    let output = runner.run(
        "reflector",
        &[
            "--latest",
            "20",
            "--protocol",
            "https",
            "--sort",
            "rate",
            "--save",
            "/etc/pacman.d/mirrorlist",
        ],
    )?;
    if output.success() {
        tracing::info!("mirrors refreshed successfully");
    } else {
        tracing::warn!("reflector failed: {}", output.stderr.trim());
        // Not fatal — old mirrors may still work, just slowly
    }
    Ok(())
}

pub fn install_zfs_on_host(
    kernel: &str,
    precompiled: bool,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    let packages = if precompiled {
        let zfs_pkg = format!("zfs-{kernel}");
        vec!["zfs-utils".to_string(), zfs_pkg]
    } else {
        vec![
            "zfs-dkms".to_string(),
            "zfs-utils".to_string(),
            format!("{kernel}-headers"),
        ]
    };

    let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();

    let pacman_conf = Path::new("/etc/pacman.conf");
    let mut ctx = AlpmContext::for_host(pacman_conf)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&pkg_refs, cancel)?;
    Ok(())
}

/// Full ZFS initialization on the live host.
/// Matches Python initialize_zfs() from kmod_setup.py.
pub fn initialize_zfs(
    runner: &dyn CommandRunner,
    kernel: &str,
    mode: ZfsModuleMode,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    // 1. Wait for reflector and stop it
    ensure_reflector_finished_and_stopped(runner)?;

    // 2. Refresh mirrors if the mirrorlist is stale
    refresh_mirrors_if_stale(runner)?;

    // 3. Check if ZFS is already available
    let module_ok = check_zfs_module(runner).unwrap_or(false);
    let utils_ok = check_zfs_utils(runner).unwrap_or(false);
    if module_ok && utils_ok {
        tracing::info!("ZFS already available on host");
        return Ok(());
    }

    tracing::info!("preparing live system for ZFS support");

    // 4. Add archzfs repo on host
    crate::system::pacman::add_archzfs_repo(runner, None)?;

    // 5. Increase cowspace
    increase_cowspace(runner)?;

    // 6. Install ZFS packages (precompiled first, fallback to DKMS)
    let precompiled = mode == ZfsModuleMode::Precompiled;
    if let Err(e) = install_zfs_on_host(kernel, precompiled, cancel) {
        if precompiled {
            tracing::warn!("precompiled ZFS install failed ({e}), falling back to DKMS");
            install_zfs_on_host(kernel, false, cancel)?;
        } else {
            return Err(e);
        }
    }

    // 6. Load ZFS module
    let loaded = load_zfs_module(runner)?;
    if !loaded {
        color_eyre::eyre::bail!("failed to load ZFS kernel module");
    }

    tracing::info!("ZFS initialized on host");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_load_zfs_module() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        assert!(load_zfs_module(&runner).unwrap());

        let calls = runner.calls();
        assert_eq!(calls[0].program, "modprobe");
        assert_eq!(calls[0].args, vec!["zfs"]);
    }

    // Note: install_zfs_on_host now uses AlpmContext directly (libalpm),
    // so it can only be tested with a real pacman environment (QEMU).
    // The old RecordingRunner-based tests are removed.

    #[test]
    fn test_initialize_zfs_already_available() {
        let runner = RecordingRunner::new(vec![
            // ensure_reflector: systemctl show SubState
            CannedResponse {
                stdout: "dead\n".into(),
                ..Default::default()
            },
            // stop reflector.service
            CannedResponse::default(),
            // stop reflector.timer
            CannedResponse::default(),
            // refresh_mirrors_if_stale: reflector (may or may not be called depending on FS state)
            CannedResponse::default(),
            // check_zfs_module: lsmod (contains zfs)
            CannedResponse {
                stdout: "zfs  1234  0\n".into(),
                ..Default::default()
            },
            // check_zfs_utils: which zpool
            CannedResponse::default(),
            // check_zfs_utils: which zfs
            CannedResponse::default(),
            // extra padding in case FS state triggers additional calls
            CannedResponse::default(),
            CannedResponse::default(),
        ]);

        initialize_zfs(
            &runner,
            "linux-lts",
            ZfsModuleMode::Precompiled,
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();

        // Should return early without attempting package installation
        // since ZFS was already available
    }
}
