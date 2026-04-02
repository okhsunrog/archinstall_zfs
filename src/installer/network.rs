use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit};

pub fn copy_iso_network(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Copy systemd-networkd configs
    let src_networkd = Path::new("/etc/systemd/network");
    let dst_networkd = target.join("etc/systemd/network");
    if src_networkd.exists() {
        copy_dir_contents(src_networkd, &dst_networkd)
            .wrap_err("failed to copy systemd-networkd configs")?;
    }

    // Copy iwd configs (wifi)
    let src_iwd = Path::new("/var/lib/iwd");
    let dst_iwd = target.join("var/lib/iwd");
    if src_iwd.exists() {
        copy_dir_contents(src_iwd, &dst_iwd).wrap_err("failed to copy iwd configs")?;
    }

    // Create resolv.conf symlink
    let resolv = target.join("etc/resolv.conf");
    let _ = fs::remove_file(&resolv);
    std::os::unix::fs::symlink("/run/systemd/resolve/stub-resolv.conf", &resolv)?;

    // Enable services
    let services = ["systemd-networkd", "systemd-resolved"];
    for service in &services {
        let target_str = target.to_str().unwrap();
        let _ = runner.run("systemctl", &["--root", target_str, "enable", service]);
    }

    // Enable iwd if configs were copied
    if dst_iwd.exists() {
        let target_str = target.to_str().unwrap();
        let _ = runner.run("systemctl", &["--root", target_str, "enable", "iwd"]);
    }

    tracing::info!("copied ISO network configuration");
    Ok(())
}

pub fn install_network_manager(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    crate::system::pacman::pacstrap(runner, target, &["networkmanager"], None)?;

    let target_str = target.to_str().unwrap();
    let output = runner.run(
        "systemctl",
        &["--root", target_str, "enable", "NetworkManager"],
    )?;
    check_exit(&output, "enable NetworkManager")?;

    Ok(())
}

fn copy_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    let options = fs_extra::dir::CopyOptions::new()
        .content_only(true)
        .copy_inside(true);
    fs_extra::dir::copy(src, dst, &options)
        .map_err(|e| color_eyre::eyre::eyre!("copy_dir failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_dir_contents() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        // Create file and subdirectory
        fs::write(src.path().join("test.conf"), "content").unwrap();
        fs::create_dir_all(src.path().join("subdir")).unwrap();
        fs::write(src.path().join("subdir/nested.conf"), "nested").unwrap();

        let out = dst.path().join("out");
        copy_dir_contents(src.path(), &out).unwrap();

        assert!(out.join("test.conf").exists());
        assert!(out.join("subdir/nested.conf").exists());
        assert_eq!(
            fs::read_to_string(out.join("subdir/nested.conf")).unwrap(),
            "nested"
        );
    }
}
