use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result, bail};

use crate::system::cmd::{CommandRunner, check_exit, chroot_cmd};

const DRACUT_ZFS_CONF: &str = r#"hostonly="yes"
hostonly_cmdline="no"
fscks="no"
early_microcode="yes"
# ZFS datasets are already compressed, use uncompressed initramfs to avoid double compression
compress="cat"
omit_dracutmodules+=" network btrfs brltty plymouth "
"#;

const DRACUT_INSTALL_HOOK: &str = r#"[Trigger]
Type = Path
Operation = Install
Operation = Upgrade
Target = usr/lib/modules/*/pkgbase

[Action]
Description = Updating linux initcpios (with dracut!)...
When = PostTransaction
Exec = /usr/local/bin/dracut-install.sh
Depends = dracut
NeedsTargets
"#;

const DRACUT_REMOVE_HOOK: &str = r#"[Trigger]
Type = Path
Operation = Remove
Target = usr/lib/modules/*/pkgbase

[Action]
Description = Removing linux initcpios...
When = PreTransaction
Exec = /usr/local/bin/dracut-remove.sh
NeedsTargets
"#;

const DRACUT_INSTALL_SCRIPT: &str = r#"#!/usr/bin/env bash
args=('--force' '--no-hostonly-cmdline')
while read -r line; do
    if [[ "$line" == 'usr/lib/modules/'+([^/])'/pkgbase' ]]; then
        read -r pkgbase < "/${line}"
        kver="${line#'usr/lib/modules/'}"
        kver="${kver%'/pkgbase'}"
        install -Dm0644 "/${line%'/pkgbase'}/vmlinuz" "/boot/vmlinuz-${pkgbase}"
        dracut "${args[@]}" "/boot/initramfs-${pkgbase}.img" --kver "$kver"
    fi
done
"#;

const DRACUT_REMOVE_SCRIPT: &str = r#"#!/usr/bin/env bash
while read -r line; do
    if [[ "$line" == 'usr/lib/modules/'+([^/])'/pkgbase' ]]; then
        read -r pkgbase < "/${line}"
        rm -f "/boot/vmlinuz-${pkgbase}" "/boot/initramfs-${pkgbase}.img"
    fi
done
"#;

pub fn configure(_runner: &dyn CommandRunner, target: &Path, encryption: bool) -> Result<()> {
    // Write dracut.conf.d/zfs.conf
    let conf_dir = target.join("etc/dracut.conf.d");
    fs::create_dir_all(&conf_dir)?;

    let mut conf = DRACUT_ZFS_CONF.to_string();
    if encryption {
        conf.push_str("install_items+=\" /etc/zfs/zroot.key \"\n");
    }
    fs::write(conf_dir.join("zfs.conf"), conf).wrap_err("failed to write dracut config")?;

    // Write pacman hooks
    let hooks_dir = target.join("etc/pacman.d/hooks");
    fs::create_dir_all(&hooks_dir)?;
    fs::write(
        hooks_dir.join("90-dracut-install.hook"),
        DRACUT_INSTALL_HOOK,
    )?;
    fs::write(hooks_dir.join("60-dracut-remove.hook"), DRACUT_REMOVE_HOOK)?;

    // Write scripts
    let bin_dir = target.join("usr/local/bin");
    fs::create_dir_all(&bin_dir)?;

    let install_script = bin_dir.join("dracut-install.sh");
    fs::write(&install_script, DRACUT_INSTALL_SCRIPT)?;
    set_executable(&install_script)?;

    let remove_script = bin_dir.join("dracut-remove.sh");
    fs::write(&remove_script, DRACUT_REMOVE_SCRIPT)?;
    set_executable(&remove_script)?;

    tracing::info!("configured dracut");
    Ok(())
}

/// An installed kernel in the target: its module directory name (the version
/// dracut needs for `--kver`) and the package base its images are named after.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InstalledKernel {
    kver: String,
    pkgbase: String,
}

/// Enumerate the kernels installed in the target.
///
/// A module directory belongs to a kernel package exactly when it contains a
/// `pkgbase` file — that is the same test the pacman hooks use, and it skips
/// directories left behind by DKMS or by an incompletely removed package.
fn installed_kernels(target: &Path) -> Result<Vec<InstalledKernel>> {
    let modules_dir = target.join("usr/lib/modules");
    let entries = fs::read_dir(&modules_dir)
        .wrap_err_with(|| format!("failed to read {}", modules_dir.display()))?;

    let mut kernels = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Ok(pkgbase) = fs::read_to_string(entry.path().join("pkgbase")) else {
            continue;
        };
        kernels.push(InstalledKernel {
            kver: entry.file_name().to_string_lossy().into_owned(),
            pkgbase: pkgbase.trim().to_string(),
        });
    }

    // Deterministic order so the logs and any failure are reproducible.
    kernels.sort();
    Ok(kernels)
}

/// Install each kernel's vmlinuz into /boot and build its initramfs.
///
/// Every kernel in `with_zfs` is covered — the configuration accepts a list of
/// them, and one without an initramfs cannot boot. Kernels outside that list
/// are skipped: an initramfs built for a kernel with no ZFS module produces a
/// boot entry that drops to an emergency shell, so leaving /boot without it
/// keeps ZFSBootMenu from offering it at all.
///
/// The kernel versions come from the target's module directories rather than
/// from the configuration, because that is what is actually installed; the
/// package base recorded there is what matches `with_zfs`.
pub fn generate(runner: &dyn CommandRunner, target: &Path, with_zfs: &[&str]) -> Result<()> {
    let installed = installed_kernels(target)?;
    if installed.is_empty() {
        bail!(
            "no installed kernel found under {}/usr/lib/modules",
            target.display()
        );
    }

    let (kernels, skipped): (Vec<_>, Vec<_>) = installed
        .into_iter()
        .partition(|kernel| with_zfs.contains(&kernel.pkgbase.as_str()));

    for kernel in &skipped {
        tracing::warn!(
            kver = kernel.kver,
            pkgbase = kernel.pkgbase,
            "skipping initramfs: this kernel has no ZFS module"
        );
    }

    if kernels.is_empty() {
        bail!("no installed kernel has a ZFS module, so none can boot this pool");
    }

    for kernel in &kernels {
        let InstalledKernel { kver, pkgbase } = kernel;
        tracing::info!(kver, pkgbase, "generating initramfs");

        let vmlinuz_src = format!("/usr/lib/modules/{kver}/vmlinuz");
        let vmlinuz_dst = format!("/boot/vmlinuz-{pkgbase}");
        let output = chroot_cmd(
            runner,
            target,
            "install",
            &["-Dm0644", &vmlinuz_src, &vmlinuz_dst],
        )?;
        check_exit(&output, &format!("install vmlinuz for {pkgbase}"))?;

        let image = format!("/boot/initramfs-{pkgbase}.img");
        let output = chroot_cmd(
            runner,
            target,
            "dracut",
            &["--force", &image, "--kver", kver],
        )?;
        check_exit(
            &output,
            &format!("dracut generate initramfs for {pkgbase} ({kver})"),
        )?;
    }

    tracing::info!(count = kernels.len(), "generated initramfs with dracut");
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_configure_dracut_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![]);
        configure(&runner, dir.path(), false).unwrap();

        assert!(dir.path().join("etc/dracut.conf.d/zfs.conf").exists());
        assert!(
            dir.path()
                .join("etc/pacman.d/hooks/90-dracut-install.hook")
                .exists()
        );
        assert!(dir.path().join("usr/local/bin/dracut-install.sh").exists());

        let conf = fs::read_to_string(dir.path().join("etc/dracut.conf.d/zfs.conf")).unwrap();
        assert!(conf.contains("hostonly"));
        assert!(conf.contains("fscks"));
        assert!(conf.contains("early_microcode"));
        assert!(!conf.contains("zroot.key"));

        let hook = fs::read_to_string(dir.path().join("etc/pacman.d/hooks/90-dracut-install.hook"))
            .unwrap();
        assert!(hook.contains("pkgbase"));

        let script =
            fs::read_to_string(dir.path().join("usr/local/bin/dracut-install.sh")).unwrap();
        assert!(script.contains("vmlinuz"));
        assert!(script.contains("pkgbase"));
    }

    #[test]
    fn test_configure_dracut_with_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![]);
        configure(&runner, dir.path(), true).unwrap();

        let conf = fs::read_to_string(dir.path().join("etc/dracut.conf.d/zfs.conf")).unwrap();
        assert!(conf.contains("zroot.key"));
    }

    /// Create a module directory for an installed kernel package.
    fn add_kernel(target: &Path, kver: &str, pkgbase: &str) {
        let dir = target.join("usr/lib/modules").join(kver);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pkgbase"), format!("{pkgbase}\n")).unwrap();
    }

    #[test]
    fn every_installed_kernel_gets_an_initramfs() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.4-arch1-1", "linux");
        add_kernel(dir.path(), "6.6.63-1-lts", "linux-lts");
        // A leftover directory with no pkgbase is not an installed kernel.
        fs::create_dir_all(dir.path().join("usr/lib/modules/extramodules-lts")).unwrap();

        // install + dracut per kernel.
        let responses: Vec<CannedResponse> = (0..4).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);

        generate(&runner, dir.path(), &["linux", "linux-lts"]).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 4, "expected install + dracut for both kernels");
        for call in &calls {
            assert_eq!(call.program, "arch-chroot");
        }

        let joined: Vec<String> = calls.iter().map(|c| c.args.join(" ")).collect();
        assert!(joined.iter().any(|c| c.contains("/boot/vmlinuz-linux-lts")));
        assert!(
            joined
                .iter()
                .any(|c| c.contains("/boot/initramfs-linux-lts.img") && c.contains("6.6.63-1-lts"))
        );
        assert!(
            joined
                .iter()
                .any(|c| c.contains("/boot/initramfs-linux.img") && c.contains("6.12.4-arch1-1"))
        );
    }

    #[test]
    fn a_kernel_without_a_zfs_module_gets_no_initramfs() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.6.63-1-lts", "linux-lts");
        add_kernel(dir.path(), "7.1.8-arch1-3", "linux");

        // Only linux-lts has a module: archzfs had no build matching the
        // current `linux` version.
        let responses: Vec<CannedResponse> = (0..2).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);

        generate(&runner, dir.path(), &["linux-lts"]).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "only the kernel with a module is built");
        let joined = calls
            .iter()
            .map(|c| c.args.join(" "))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("initramfs-linux-lts.img"));
        assert!(
            !joined.contains("initramfs-linux.img"),
            "a kernel with no ZFS module must not get a boot entry: {joined}"
        );
        assert!(!joined.contains("vmlinuz-linux "));
    }

    #[test]
    fn no_kernel_with_a_module_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "7.1.8-arch1-3", "linux");
        let runner = RecordingRunner::new(vec![]);

        let err = generate(&runner, dir.path(), &["linux-lts"]).unwrap_err();

        assert!(
            err.to_string()
                .contains("no installed kernel has a ZFS module")
        );
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn kernel_version_is_not_chosen_by_string_order() {
        // A plain sort puts 6.9 after 6.12, so picking a single "latest" kernel
        // lexicographically would build the initramfs for the wrong one.
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.4-arch1-1", "linux");
        add_kernel(dir.path(), "6.9.10-arch1-1", "linux-older");

        let kernels = installed_kernels(dir.path()).unwrap();

        assert_eq!(kernels.len(), 2);
        let pkgbases: Vec<&str> = kernels.iter().map(|k| k.pkgbase.as_str()).collect();
        assert!(pkgbases.contains(&"linux"));
        assert!(pkgbases.contains(&"linux-older"));
    }

    #[test]
    fn missing_kernel_is_an_error_rather_than_a_silent_skip() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("usr/lib/modules")).unwrap();
        let runner = RecordingRunner::new(vec![]);

        let err = generate(&runner, dir.path(), &["linux-lts"]).unwrap_err();

        assert!(err.to_string().contains("no installed kernel"));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn dracut_failure_for_one_kernel_stops_the_install() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.6.63-1-lts", "linux-lts");

        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // install vmlinuz
            CannedResponse {
                exit_code: 1,
                stderr: "dracut: installkernel failed".into(),
                ..Default::default()
            },
        ]);

        let err = generate(&runner, dir.path(), &["linux-lts"]).unwrap_err();
        assert!(err.to_string().contains("linux-lts"));
    }
}
