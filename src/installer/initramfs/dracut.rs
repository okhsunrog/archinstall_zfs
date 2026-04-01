use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{check_exit, chroot, CommandRunner};

const DRACUT_ZFS_CONF: &str = r#"hostonly=yes
hostonly_cmdline=no
compress=cat
omit_dracutmodules+=" network btrfs brltty plymouth "
"#;

const DRACUT_INSTALL_HOOK: &str = r#"[Trigger]
Type = Path
Operation = Install
Operation = Upgrade
Target = usr/lib/modules/*/vmlinuz
Target = usr/lib/kernel/install.d/*

[Action]
Description = Updating linux initramfs with dracut...
When = PostTransaction
Exec = /usr/local/bin/dracut-install.sh
Depends = dracut
NeedsTargets
"#;

const DRACUT_REMOVE_HOOK: &str = r#"[Trigger]
Type = Path
Operation = Remove
Target = usr/lib/modules/*/vmlinuz

[Action]
Description = Removing linux initramfs with dracut...
When = PreTransaction
Exec = /usr/local/bin/dracut-remove.sh
NeedsTargets
"#;

const DRACUT_INSTALL_SCRIPT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

while read -r line; do
    if [[ "$line" != */vmlinuz ]]; then
        continue
    fi
    kver="${line#/usr/lib/modules/}"
    kver="${kver%/vmlinuz}"
    dracut --force --hostonly --no-hostonly-cmdline "/boot/initramfs-${kver}.img" "$kver"
done
"#;

const DRACUT_REMOVE_SCRIPT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

while read -r line; do
    if [[ "$line" != */vmlinuz ]]; then
        continue
    fi
    kver="${line#/usr/lib/modules/}"
    kver="${kver%/vmlinuz}"
    rm -f "/boot/initramfs-${kver}.img"
done
"#;

pub fn configure(runner: &dyn CommandRunner, target: &Path, encryption: bool) -> Result<()> {
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

pub fn generate(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let output = chroot(
        runner,
        target,
        "dracut --force --hostonly --no-hostonly-cmdline /boot/initramfs-linux.img $(uname -r) 2>&1 || dracut --regenerate-all --force --hostonly --no-hostonly-cmdline",
    )?;
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
        assert!(conf.contains("hostonly=yes"));
        assert!(!conf.contains("zroot.key"));
    }

    #[test]
    fn test_configure_dracut_with_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![]);
        configure(&runner, dir.path(), true).unwrap();

        let conf = fs::read_to_string(dir.path().join("etc/dracut.conf.d/zfs.conf")).unwrap();
        assert!(conf.contains("zroot.key"));
    }
}
