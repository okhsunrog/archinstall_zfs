use std::path::Path;

use color_eyre::eyre::Result;

use crate::config::types::ZfsModuleMode;
use crate::system::alpm_pacman::AlpmContext;
use crate::system::async_download::DownloadConfig;
use crate::system::cmd::CommandRunner;

pub const ZFS_SERVICES: &[&str] = &[
    "zfs.target",
    "zfs-import.target",
    "zfs-volumes.target",
    "zfs-import-scan.service",
    "zfs-zed.service",
];

/// Units the installed system must keep disabled.
///
/// * `zfs-mount.service` runs `zfs mount -a`, which mounts every `canmount=on`
///   dataset in the pool — including the other boot environments' `/home`,
///   `/root`, … — inside the running one. Mounting is done by
///   `zfs-mount-generator` from `zfs-list.cache`, which the ZED hook restricts
///   to the booted BE.
/// * `zfs-import-cache.service` needs `/etc/zfs/zpool.cache`; pools are created
///   with `cachefile=none` and imported by `zfs-import-scan.service` instead.
/// * `zfs-share.service` exports NFS/SMB shares nobody configured.
pub const ZFS_DISABLED_SERVICES: &[&str] = &[
    "zfs-mount.service",
    "zfs-share.service",
    "zfs-import-cache.service",
];

/// Where the preset policy is installed on the target. The `00-` prefix makes
/// it sort before the ZFS package's `50-zfs.preset`; presets are matched first
/// line wins across `/etc` and `/usr/lib`, so this file overrides it.
pub const ZFS_PRESET_PATH: &str = "etc/systemd/system-preset/00-zfs-mount-generator.preset";

/// The `systemd.preset(5)` policy matching [`ZFS_SERVICES`] and
/// [`ZFS_DISABLED_SERVICES`].
///
/// `systemctl enable` at install time decides the state once; presets decide
/// it again whenever systemd is asked to — `systemctl preset-all`, a package's
/// post-install `systemctl preset`, and every "first boot" (a missing, empty or
/// `uninitialized` `/etc/machine-id`). The stock `50-zfs.preset` enables
/// `zfs-mount.service` and disables `zfs-import-scan.service`, so any of those
/// events would silently undo the installer's choices. Pinning the policy here
/// keeps them in force.
pub fn zfs_preset_policy() -> String {
    let mut out = String::from(
        "# Written by archinstall_zfs. Mounting is handled by zfs-mount-generator from\n\
         # /etc/zfs/zfs-list.cache; zfs-mount.service would also mount the datasets of\n\
         # other boot environments sharing this pool. The pool has cachefile=none, so\n\
         # it is imported by scanning. Sorts before the package's 50-zfs.preset; the\n\
         # first matching line wins.\n",
    );
    for unit in ZFS_DISABLED_SERVICES {
        out.push_str("disable ");
        out.push_str(unit);
        out.push('\n');
    }
    for unit in ZFS_SERVICES {
        out.push_str("enable ");
        out.push_str(unit);
        out.push('\n');
    }
    out
}

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

pub fn install_zfs_on_host(
    kernel: &str,
    precompiled: bool,
    cancel: &tokio_util::sync::CancellationToken,
    download_config: DownloadConfig,
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
    let mut ctx = AlpmContext::for_host(pacman_conf, download_config)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&pkg_refs, cancel, None)?;
    Ok(())
}

/// Full ZFS initialization on the live host.
/// Matches Python initialize_zfs() from kmod_setup.py.
pub fn initialize_zfs(
    runner: &dyn CommandRunner,
    distro: &crate::distro::Distribution,
    kernel: &str,
    mode: ZfsModuleMode,
    cancel: &tokio_util::sync::CancellationToken,
    download_config: DownloadConfig,
) -> Result<()> {
    // 1. Wait for reflector and stop it, so it cannot overwrite the ranked
    //    mirrorlist behind the installer's back. Ranking itself happens in
    //    `system::mirrors`, called by the interfaces before this step.
    ensure_reflector_finished_and_stopped(runner)?;

    // 2. Check if ZFS is already available
    let module_ok = check_zfs_module(runner).unwrap_or(false);
    let utils_ok = check_zfs_utils(runner).unwrap_or(false);
    if module_ok && utils_ok {
        tracing::info!("ZFS already available on host");
        return Ok(());
    }

    tracing::info!("preparing live system for ZFS support");

    // 4. Add archzfs repo on host
    let isa = crate::system::sysinfo::detect_isa_level(runner);
    crate::system::pacman::add_repositories(runner, None, distro, isa)?;

    // 5. Increase cowspace
    increase_cowspace(runner)?;

    // 6. Install ZFS packages (precompiled first, fallback to DKMS)
    let precompiled = mode == ZfsModuleMode::Precompiled;
    if let Err(e) = install_zfs_on_host(kernel, precompiled, cancel, download_config.clone()) {
        if precompiled {
            tracing::warn!("precompiled ZFS install failed ({e}), falling back to DKMS");
            install_zfs_on_host(kernel, false, cancel, download_config)?;
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

    /// The preset is derived from the same lists the installer enables from,
    /// so it can only disagree with `systemctl enable` if a unit lands in both
    /// lists — presets are "first match wins", and that would decide silently.
    #[test]
    fn zfs_preset_policy_matches_the_service_lists() {
        for unit in ZFS_DISABLED_SERVICES {
            assert!(
                !ZFS_SERVICES.contains(unit),
                "{unit} is both enabled and disabled"
            );
        }

        let policy = zfs_preset_policy();
        let directives: Vec<&str> = policy.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            directives.len(),
            ZFS_SERVICES.len() + ZFS_DISABLED_SERVICES.len()
        );
        for unit in ZFS_DISABLED_SERVICES {
            assert!(directives.contains(&format!("disable {unit}").as_str()));
        }
        for unit in ZFS_SERVICES {
            assert!(directives.contains(&format!("enable {unit}").as_str()));
        }
    }

    /// These two are the whole point: the package's 50-zfs.preset says the
    /// opposite for both of them.
    #[test]
    fn zfs_preset_policy_overrides_the_package_defaults() {
        let policy = zfs_preset_policy();
        assert!(policy.contains("disable zfs-mount.service\n"));
        assert!(policy.contains("enable zfs-import-scan.service\n"));
        assert!(ZFS_PRESET_PATH.starts_with("etc/systemd/system-preset/00-"));
    }

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
            crate::distro::default(),
            "linux-lts",
            ZfsModuleMode::Precompiled,
            &tokio_util::sync::CancellationToken::new(),
            crate::system::async_download::DownloadConfig::default(),
        )
        .unwrap();

        // Should return early without attempting package installation
        // since ZFS was already available
    }
}
