use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{check_exit, chroot, CommandRunner};

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

/// Generate initramfs inside chroot.
/// Detects kernel version from /usr/lib/modules/ inside the target,
/// copies vmlinuz to /boot, and runs dracut.
pub fn generate(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    // Match Python: detect kver from installed modules, read pkgbase,
    // copy vmlinuz, then generate initramfs with dracut --force
    let cmd = concat!(
        "kver=$(ls -1 /usr/lib/modules | sort | tail -n1); ",
        "pkgbase=$(cat /usr/lib/modules/$kver/pkgbase 2>/dev/null || echo linux); ",
        "install -Dm0644 /usr/lib/modules/$kver/vmlinuz /boot/vmlinuz-$pkgbase; ",
        "dracut --force /boot/initramfs-$pkgbase.img --kver $kver",
    );
    let output = chroot(runner, target, cmd)?;
    check_exit(&output, "dracut generate initramfs")?;
    tracing::info!("generated initramfs with dracut");
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
        assert!(dir
            .path()
            .join("etc/pacman.d/hooks/90-dracut-install.hook")
            .exists());
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

    #[test]
    fn test_generate_dracut_command() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        let _ = generate(&runner, Path::new("/mnt"));

        let calls = runner.calls();
        assert_eq!(calls[0].program, "arch-chroot");
        let cmd = calls[0].args.join(" ");
        assert!(cmd.contains("ls -1 /usr/lib/modules"));
        assert!(cmd.contains("pkgbase"));
        assert!(cmd.contains("dracut --force"));
        assert!(cmd.contains("--kver"));
    }
}
