use std::fs;
use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{CommandRunner, check_exit};

pub fn copy_iso_network(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Copy systemd-networkd configs
    let src_networkd = Path::new("/etc/systemd/network");
    let dst_networkd = target.join("etc/systemd/network");
    if src_networkd.exists() {
        copy_dir_contents(src_networkd, &dst_networkd)?;
    }

    // Copy iwd configs (wifi)
    let src_iwd = Path::new("/var/lib/iwd");
    let dst_iwd = target.join("var/lib/iwd");
    if src_iwd.exists() {
        copy_dir_contents(src_iwd, &dst_iwd)?;
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
    if src.is_dir() {
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                copy_dir_contents(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_dir_contents() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        fs::write(src.path().join("test.conf"), "content").unwrap();
        copy_dir_contents(src.path(), &dst.path().join("out")).unwrap();

        assert!(dst.path().join("out/test.conf").exists());
    }
}
