use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{check_exit, CommandRunner};

pub fn load_zfs_module(runner: &dyn CommandRunner) -> Result<bool> {
    let output = runner.run("modprobe", &["zfs"])?;
    Ok(output.success())
}

pub fn check_zfs_module(runner: &dyn CommandRunner) -> Result<bool> {
    let output = runner.run("lsmod", &[])?;
    Ok(output.success() && output.stdout.contains("zfs"))
}

pub fn check_zfs_utils(runner: &dyn CommandRunner) -> Result<bool> {
    let zpool = runner.run("which", &["zpool"])?;
    let zfs = runner.run("which", &["zfs"])?;
    Ok(zpool.success() && zfs.success())
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

pub fn wait_for_reflector(runner: &dyn CommandRunner) -> Result<()> {
    // Check if reflector is running and wait for it
    let output = runner.run("systemctl", &["is-active", "--quiet", "reflector.service"])?;
    if output.success() {
        tracing::info!("waiting for reflector to finish...");
        let _ = runner.run("systemctl", &["start", "--no-block", "reflector.service"]);
        let wait_output = runner.run(
            "bash",
            &[
                "-c",
                "while systemctl is-active --quiet reflector.service; do sleep 1; done",
            ],
        )?;
    }

    // Stop reflector to prevent it from interfering
    let _ = runner.run("systemctl", &["stop", "reflector.service"]);
    let _ = runner.run("systemctl", &["stop", "reflector.timer"]);
    Ok(())
}

pub fn install_zfs_on_host(
    runner: &dyn CommandRunner,
    kernel: &str,
    precompiled: bool,
) -> Result<bool> {
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
    let mut args = vec!["--noconfirm", "--needed", "-S"];
    args.extend_from_slice(&pkg_refs);

    let output = runner.run("pacman", &args)?;
    Ok(output.success())
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

    #[test]
    fn test_install_zfs_precompiled() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        install_zfs_on_host(&runner, "linux-lts", true).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "pacman");
        let args_str = calls[0].args.join(" ");
        assert!(args_str.contains("zfs-utils"));
        assert!(args_str.contains("zfs-linux-lts"));
    }

    #[test]
    fn test_install_zfs_dkms() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        install_zfs_on_host(&runner, "linux", false).unwrap();

        let calls = runner.calls();
        let args_str = calls[0].args.join(" ");
        assert!(args_str.contains("zfs-dkms"));
        assert!(args_str.contains("zfs-utils"));
        assert!(args_str.contains("linux-headers"));
    }
}
