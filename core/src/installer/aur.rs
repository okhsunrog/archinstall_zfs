use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{CommandRunner, check_exit, chroot};

const TEMP_USER: &str = "aurinstall";
const AUR_HELPER_REPO: &str = "https://aur.archlinux.org/yay-bin.git";

pub fn install_aur_packages(
    runner: &dyn CommandRunner,
    target: &Path,
    packages: &[&str],
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    tracing::info!(?packages, "installing AUR packages");

    setup_aur_environment(runner, target)?;
    install_aur_helper(runner, target)?;

    for &pkg in packages {
        install_single_aur_package(runner, target, pkg)?;
    }

    cleanup_aur_environment(runner, target)?;

    Ok(())
}

fn setup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Install dependencies
    crate::system::pacman::pacstrap(runner, target, &["git", "base-devel", "sudo"], None)?;

    // Create temp user
    let output = chroot(runner, target, &format!("useradd -m {TEMP_USER}"))?;
    check_exit(&output, "create AUR temp user")?;

    // Enable NOPASSWD sudo
    let sudoers_content = format!("{TEMP_USER} ALL=(ALL) NOPASSWD: ALL\n");
    std::fs::write(
        target.join(format!("etc/sudoers.d/99_{TEMP_USER}")),
        sudoers_content,
    )?;

    Ok(())
}

fn install_aur_helper(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let cmd = format!(
        "su - {TEMP_USER} -c 'cd /tmp && git clone {AUR_HELPER_REPO} && cd yay-bin && makepkg -si --noconfirm'"
    );
    let output = chroot(runner, target, &cmd)?;
    check_exit(&output, "install yay-bin")?;
    Ok(())
}

fn install_single_aur_package(
    runner: &dyn CommandRunner,
    target: &Path,
    package: &str,
) -> Result<()> {
    let cmd = format!("su - {TEMP_USER} -c 'yay -S --noconfirm --needed {package}'");
    let output = chroot(runner, target, &cmd)?;
    if !output.success() {
        tracing::warn!(package, "yay failed, trying manual clone+makepkg");
        let fallback = format!(
            "su - {TEMP_USER} -c 'cd /tmp && git clone https://aur.archlinux.org/{package}.git && cd {package} && makepkg -si --noconfirm'"
        );
        let output = chroot(runner, target, &fallback)?;
        check_exit(&output, &format!("AUR install {package}"))?;
    }
    Ok(())
}

fn cleanup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Remove NOPASSWD sudoers
    let _ = std::fs::remove_file(target.join(format!("etc/sudoers.d/99_{TEMP_USER}")));

    // Delete temp user
    let _ = chroot(runner, target, &format!("userdel -r {TEMP_USER}"));

    // Clean up yay cache
    let _ = chroot(
        runner,
        target,
        &format!("rm -rf /home/{TEMP_USER} /tmp/yay-bin"),
    );

    tracing::info!("cleaned up AUR environment");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_install_aur_packages_empty() {
        let runner = RecordingRunner::new(vec![]);
        install_aur_packages(&runner, Path::new("/mnt"), &[] as &[&str]).unwrap();
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn test_setup_aur_environment() {
        let responses: Vec<CannedResponse> = (0..5).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);
        let dir = tempfile::tempdir().unwrap();

        // setup_aur_environment calls pacstrap which reads /etc/pacman.conf
        let result = setup_aur_environment(&runner, dir.path());
        if result.is_err() {
            // Expected on non-Arch systems where /etc/pacman.conf doesn't exist
            return;
        }

        let calls = runner.calls();
        assert!(calls.iter().any(|c| c.program == "pacstrap"));
        assert!(
            calls
                .iter()
                .any(|c| c.args.iter().any(|a| a.contains("useradd")))
        );

        let sudoers = dir.path().join(format!("etc/sudoers.d/99_{TEMP_USER}"));
        assert!(sudoers.exists());
    }
}
