use std::path::Path;

use color_eyre::eyre::{Result, bail};

use crate::system::cmd::{CommandRunner, check_exit, chroot, chroot_cmd, shell_quote};

const TEMP_USER: &str = "aurinstall";

/// Validate that a package name contains only characters allowed by the AUR.
/// AUR package names: lowercase alphanumeric, @, ., _, +, -
fn validate_aur_package_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("AUR package name cannot be empty");
    }
    if !name.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '@' | '.' | '_' | '+' | '-')
    }) {
        bail!("AUR package name '{}' contains invalid characters", name);
    }
    Ok(())
}

pub fn install_aur_packages(
    runner: &dyn CommandRunner,
    target: &Path,
    packages: &[&str],
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    for &pkg in packages {
        validate_aur_package_name(pkg)?;
    }

    tracing::info!(?packages, "installing AUR packages");

    // Resolve the full AUR dependency tree (including AUR-to-AUR deps)
    let install_order = resolve_aur_deps(target, packages)?;

    if install_order.is_empty() {
        tracing::info!("all AUR packages already installed");
        return Ok(());
    }

    tracing::info!(?install_order, "resolved AUR install order");

    setup_aur_environment(runner, target)?;

    for pkg in &install_order {
        install_single_aur_package(runner, target, pkg)?;
    }

    cleanup_aur_environment(runner, target)?;

    Ok(())
}

/// Use raur + aur-depends to resolve the full AUR dependency tree,
/// returning package names in correct install order (deps before dependents).
fn resolve_aur_deps(target: &Path, packages: &[&str]) -> Result<Vec<String>> {
    let target_conf = target.join("etc/pacman.conf");
    let conf = pacmanconf::Config::from_file(target_conf.to_str().unwrap_or("/etc/pacman.conf"))
        .map_err(|e| color_eyre::eyre::eyre!("failed to parse pacman.conf: {e}"))?;

    let target_str = target.to_string_lossy();
    let db_path = format!("{}/var/lib/pacman", target_str);

    let mut alpm = alpm::Alpm::new(target_str.as_ref(), &db_path)
        .map_err(|e| color_eyre::eyre::eyre!("failed to init alpm: {e}"))?;

    alpm_utils::configure_alpm(&mut alpm, &conf)
        .map_err(|e| color_eyre::eyre::eyre!("failed to configure alpm: {e}"))?;

    let raur_handle = raur::Handle::new();
    let mut cache = raur::Cache::new();

    let resolver =
        aur_depends::Resolver::new(&alpm, &mut cache, &raur_handle, aur_depends::Flags::new());

    // resolve_targets is async — bridge with a small tokio runtime
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| color_eyre::eyre::eyre!("failed to create tokio runtime: {e}"))?;

    let targets: Vec<String> = packages.iter().map(|s| s.to_string()).collect();
    let actions = rt.block_on(resolver.resolve_targets(&targets))?;

    // Collect AUR packages in dependency order
    let mut order = Vec::new();
    for aur_pkg in actions.iter_aur_pkgs() {
        let name = aur_pkg.pkg.package_base.clone();
        order.push(name);
    }

    Ok(order)
}

fn setup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Install git and sudo via libalpm (base-devel already in base install)
    let target_conf = target.join("etc/pacman.conf");
    let mut ctx = crate::system::alpm_pacman::AlpmContext::for_target(target, &target_conf)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&["git", "sudo"])?;

    // Create temp user
    let output = chroot_cmd(runner, target, "useradd", &["-m", TEMP_USER])?;
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
    tracing::info!(package, "building AUR package");

    // Clone PKGBUILD from AUR and build with makepkg.
    // --syncdeps installs repo dependencies automatically.
    // --skippgpcheck avoids PGP key issues in automated installs.
    let quoted_pkg = shell_quote(package);
    let cmd = format!(
        "su - {TEMP_USER} -c 'cd /tmp && \
         git clone https://aur.archlinux.org/{quoted_pkg}.git && \
         cd {quoted_pkg} && \
         makepkg -si --noconfirm --needed --skippgpcheck'"
    );
    let output = chroot(runner, target, &cmd)?;
    check_exit(&output, &format!("AUR install {package}"))?;
    Ok(())
}

fn cleanup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let _ = std::fs::remove_file(target.join(format!("etc/sudoers.d/99_{TEMP_USER}")));
    let _ = chroot_cmd(runner, target, "userdel", &["-r", TEMP_USER]);

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

    #[test]
    fn test_validate_aur_package_name() {
        assert!(validate_aur_package_name("zfsbootmenu").is_ok());
        assert!(validate_aur_package_name("perl-boolean").is_ok());
        assert!(validate_aur_package_name("yay-bin").is_ok());
        assert!(validate_aur_package_name("").is_err());
        assert!(validate_aur_package_name("Bad Name").is_err());
        assert!(validate_aur_package_name("pkg;rm -rf /").is_err());
    }
}
