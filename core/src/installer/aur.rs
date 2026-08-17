use std::path::Path;
use std::sync::Arc;

use color_eyre::eyre::{Context, Result, bail};

use crate::system::cmd::{CommandRunner, check_exit, chroot, chroot_cmd, shell_quote};
use crate::system::fs::write_file_with_mode;

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

pub async fn install_aur_packages(
    runner: Arc<dyn CommandRunner>,
    target: &Path,
    packages: &[&str],
    cancel: &tokio_util::sync::CancellationToken,
    download_config: crate::system::async_download::DownloadConfig,
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    for &pkg in packages {
        validate_aur_package_name(pkg)?;
    }

    tracing::info!(?packages, "installing AUR packages");

    // Resolve AUR dependency tree — uses alpm (!Send) internally via block_on
    let target_owned = target.to_path_buf();
    let pkgs: Vec<String> = packages.iter().map(|s| s.to_string()).collect();
    let install_order = tokio::task::spawn_blocking(move || {
        let pkg_refs: Vec<&str> = pkgs.iter().map(|s| s.as_str()).collect();
        resolve_aur_deps(&target_owned, &pkg_refs)
    })
    .await??;

    if install_order.is_empty() {
        tracing::info!("all AUR packages already installed");
        return Ok(());
    }

    tracing::info!(?install_order, "resolved AUR install order");

    // Sync operations: setup environment, build packages, cleanup
    let r = runner;
    let t = target.to_path_buf();
    let c = cancel.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        // Everything between setup and cleanup runs inside the closure so the
        // teardown below is reached on *every* exit path. A build that fails
        // partway through must not leave the `aurinstall` account and its
        // NOPASSWD sudoers drop-in behind on the installed system.
        let build_result = (|| -> Result<()> {
            setup_aur_environment(&*r, &t, &c, download_config)?;
            for pkg in &install_order {
                install_single_aur_package(&*r, &t, pkg)?;
            }
            Ok(())
        })();

        let cleanup_result = cleanup_aur_environment(&*r, &t);
        // The build error is the more useful one to report; a cleanup failure
        // only surfaces when the build itself succeeded.
        build_result.and(cleanup_result)
    })
    .await?
}

/// Use raur + aur-depends to resolve the full AUR dependency tree,
/// returning package names in correct install order (deps before dependents).
///
/// This function is sync because `alpm::Alpm` is `!Send` — the resolver holds
/// a reference to it, so we must `block_on` from the same thread.
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

    // resolve_targets is async — bridge via block_on. This is called from
    // spawn_blocking, but block_on works because we captured the tokio Handle
    // before entering the blocking context. The resolver borrows alpm (&Alpm)
    // which is !Send, so we cannot spawn it as a tokio task — block_on on the
    // current thread is the correct approach here.
    let rt = tokio::runtime::Handle::current();

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

fn setup_aur_environment(
    runner: &dyn CommandRunner,
    target: &Path,
    cancel: &tokio_util::sync::CancellationToken,
    download_config: crate::system::async_download::DownloadConfig,
) -> Result<()> {
    // Install git and sudo via libalpm (base-devel already in base install)
    let target_conf = target.join("etc/pacman.conf");
    let mut ctx =
        crate::system::alpm_pacman::AlpmContext::for_target(target, &target_conf, download_config)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&["git", "sudo"], cancel, None)?;

    // Create temp user
    let output = chroot_cmd(runner, target, "useradd", &["-m", TEMP_USER])?;
    check_exit(&output, "create AUR temp user")?;

    // Enable NOPASSWD sudo — removed again by cleanup_aur_environment.
    let sudoers_dir = target.join("etc/sudoers.d");
    std::fs::create_dir_all(&sudoers_dir)?;
    write_file_with_mode(
        &sudoers_dir.join(format!("99_{TEMP_USER}")),
        format!("{TEMP_USER} ALL=(ALL) NOPASSWD: ALL\n").as_bytes(),
        0o440,
        "AUR sudoers drop-in",
    )?;

    Ok(())
}

fn install_single_aur_package(
    runner: &dyn CommandRunner,
    target: &Path,
    package: &str,
) -> Result<()> {
    tracing::info!(package, "building AUR package");

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

/// Remove the temporary build account and its sudo rule.
///
/// The sudoers drop-in is the security-relevant half: failing to delete it
/// would ship a passwordless-sudo rule on the installed system, so that
/// failure is reported rather than swallowed. A `userdel` failure leaves only
/// a locked, password-less account with no sudo rights, which is worth a
/// warning but not worth failing an otherwise complete installation.
fn cleanup_aur_environment(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let sudoers = target.join(format!("etc/sudoers.d/99_{TEMP_USER}"));
    match std::fs::remove_file(&sudoers) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e).wrap_err(format!(
                "failed to remove {} — the target would keep a passwordless sudo rule for {TEMP_USER}",
                sudoers.display()
            ));
        }
    }

    match chroot_cmd(runner, target, "userdel", &["-r", TEMP_USER]) {
        Ok(output) if output.success() => {}
        Ok(output) => tracing::warn!(
            user = TEMP_USER,
            exit_code = output.exit_code,
            stderr = %output.stderr.trim(),
            "failed to delete AUR build user (sudo rule was removed)"
        ),
        Err(error) => tracing::warn!(
            user = TEMP_USER,
            %error,
            "could not run userdel for AUR build user (sudo rule was removed)"
        ),
    }

    tracing::info!("cleaned up AUR environment");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::RecordingRunner;

    #[tokio::test]
    async fn test_install_aur_packages_empty() {
        let runner: Arc<dyn CommandRunner> = Arc::new(RecordingRunner::new(vec![]));
        install_aur_packages(
            runner.clone(),
            Path::new("/mnt"),
            &[] as &[&str],
            &tokio_util::sync::CancellationToken::new(),
            crate::system::async_download::DownloadConfig::default(),
        )
        .await
        .unwrap();
        // Can't check calls on Arc easily, but the test verifies no panic
    }

    #[test]
    fn cleanup_removes_the_sudo_rule_and_reports_when_it_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let sudoers_dir = dir.path().join("etc/sudoers.d");
        std::fs::create_dir_all(&sudoers_dir).unwrap();
        let sudoers = sudoers_dir.join(format!("99_{TEMP_USER}"));
        std::fs::write(&sudoers, "aurinstall ALL=(ALL) NOPASSWD: ALL\n").unwrap();

        let runner = RecordingRunner::new(vec![]);
        cleanup_aur_environment(&runner, dir.path()).unwrap();
        assert!(!sudoers.exists(), "sudo rule must not survive cleanup");

        // Already gone is success, not an error: cleanup runs on paths where
        // setup never got as far as writing the drop-in.
        cleanup_aur_environment(&runner, dir.path()).unwrap();
    }

    #[test]
    fn failed_build_still_tears_down_the_sudo_rule() {
        let dir = tempfile::tempdir().unwrap();
        let sudoers_dir = dir.path().join("etc/sudoers.d");
        std::fs::create_dir_all(&sudoers_dir).unwrap();
        let sudoers = sudoers_dir.join(format!("99_{TEMP_USER}"));
        std::fs::write(&sudoers, "aurinstall ALL=(ALL) NOPASSWD: ALL\n").unwrap();

        // A build failure, mirrored from install_single_aur_package's error path.
        let build_result: Result<()> = Err(color_eyre::eyre::eyre!("AUR install failed"));
        let runner = RecordingRunner::new(vec![]);
        let cleanup_result = cleanup_aur_environment(&runner, dir.path());
        let combined = build_result.and(cleanup_result);

        assert!(
            !sudoers.exists(),
            "sudo rule must not survive a failed build"
        );
        assert!(
            combined.unwrap_err().to_string().contains("AUR install"),
            "the build error must be the one reported"
        );
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
