use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{CommandRunner, check_exit, chroot};

const TEMP_USER: &str = "aurinstall";

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

    for &pkg in packages {
        install_single_aur_package(runner, target, pkg)?;
    }

    cleanup_aur_environment(runner, target)?;

    Ok(())
}

fn setup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Install git and sudo via libalpm (base-devel already in base install)
    let target_conf = target.join("etc/pacman.conf");
    let mut ctx = crate::system::alpm_pacman::AlpmContext::for_target(target, &target_conf)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&["git", "sudo"])?;

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

fn install_single_aur_package(
    runner: &dyn CommandRunner,
    target: &Path,
    package: &str,
) -> Result<()> {
    // Clone PKGBUILD from AUR and build with makepkg.
    // --syncdeps installs repo dependencies automatically.
    // --noconfirm for non-interactive.
    let cmd = format!(
        "su - {TEMP_USER} -c 'cd /tmp && \
         git clone https://aur.archlinux.org/{package}.git && \
         cd {package} && \
         makepkg -si --noconfirm --needed'"
    );
    let output = chroot(runner, target, &cmd)?;
    check_exit(&output, &format!("AUR install {package}"))?;
    Ok(())
}

fn cleanup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let _ = std::fs::remove_file(target.join(format!("etc/sudoers.d/99_{TEMP_USER}")));
    let _ = chroot(runner, target, &format!("userdel -r {TEMP_USER}"));

    tracing::info!("cleaned up AUR environment");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::RecordingRunner;

    #[test]
    fn test_install_aur_packages_empty() {
        let runner = RecordingRunner::new(vec![]);
        install_aur_packages(&runner, Path::new("/mnt"), &[] as &[&str]).unwrap();
        assert!(runner.calls().is_empty());
    }
}
