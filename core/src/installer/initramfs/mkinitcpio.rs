use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result, bail};

use crate::system::cmd::{CommandRunner, chroot_checked};
use crate::system::conf::{patch_conf_array, set_conf_value};

pub fn configure(target: &Path, encryption: bool) -> Result<()> {
    let conf_path = target.join("etc/mkinitcpio.conf");
    if !conf_path.exists() {
        tracing::warn!("mkinitcpio.conf not found, skipping configuration");
        return Ok(());
    }

    let content = fs::read_to_string(&conf_path)?;
    let mut new_content = content.clone();

    // Ensure zfs is in MODULES
    new_content = patch_conf_array(&new_content, "MODULES", |modules| {
        if !modules.contains(&"zfs".to_string()) {
            modules.push("zfs".to_string());
        }
    })?;

    // The archzfs `zfs` hook is a legacy (udev-based) hook, not compatible
    // with systemd-based initramfs. Replace systemd/sd-vconsole with udev/keymap
    // if present, then insert zfs before filesystems.
    new_content = patch_conf_array(&new_content, "HOOKS", |hooks| {
        // Replace systemd hooks with udev equivalents
        if hooks.contains(&"systemd".to_string()) {
            hooks.retain(|h| h != "systemd" && h != "sd-vconsole");
            if !hooks.contains(&"udev".to_string()) {
                if let Some(pos) = hooks.iter().position(|h| h == "base") {
                    hooks.insert(pos + 1, "udev".to_string());
                } else {
                    hooks.insert(0, "udev".to_string());
                }
            }
            if !hooks.contains(&"keymap".to_string()) {
                if let Some(pos) = hooks.iter().position(|h| h == "keyboard") {
                    hooks.insert(pos + 1, "keymap".to_string());
                } else if let Some(pos) = hooks.iter().position(|h| h == "udev") {
                    hooks.insert(pos + 1, "keymap".to_string());
                }
            }
        }
        // Insert zfs before filesystems
        if !hooks.contains(&"zfs".to_string()) {
            if let Some(pos) = hooks.iter().position(|h| h == "filesystems") {
                hooks.insert(pos, "zfs".to_string());
            } else {
                hooks.push("zfs".to_string());
            }
        }
    })?;

    // Set COMPRESSION
    new_content = set_conf_value(&new_content, "COMPRESSION", "cat");

    // Add key file to FILES if encryption enabled
    if encryption {
        new_content = patch_conf_array(&new_content, "FILES", |files| {
            let key = "/etc/zfs/zroot.key".to_string();
            if !files.contains(&key) {
                files.push(key);
            }
        })?;
    }

    fs::write(&conf_path, new_content).wrap_err("failed to write mkinitcpio.conf")?;
    tracing::info!("configured mkinitcpio");
    Ok(())
}

/// Build the initramfs for each kernel in `with_zfs`.
///
/// One preset at a time rather than `-P`: a kernel with no ZFS module would
/// get an image that cannot import the pool, and a boot entry that drops to an
/// emergency shell is worse than no entry at all. Preset names are the package
/// bases, which is what mkinitcpio installs into /etc/mkinitcpio.d.
pub fn generate(runner: &dyn CommandRunner, target: &Path, with_zfs: &[&str]) -> Result<()> {
    if with_zfs.is_empty() {
        bail!("no installed kernel has a ZFS module, so none can boot this pool");
    }

    for kernel in with_zfs {
        chroot_checked(
            runner,
            target,
            "mkinitcpio",
            &["-p", kernel],
            &format!("mkinitcpio -p {kernel}"),
        )?;
        tracing::info!(kernel, "generated initramfs with mkinitcpio");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kernel_with_a_module_gets_its_own_preset_run() {
        use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

        let responses: Vec<CannedResponse> = (0..2).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);

        generate(&runner, Path::new("/mnt"), &["linux-lts", "linux-zen"]).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        for call in &calls {
            assert_eq!(call.program, "arch-chroot");
            assert!(call.args.contains(&"-p".to_string()), "preset run expected");
        }
        assert!(calls[0].args.contains(&"linux-lts".to_string()));
        assert!(calls[1].args.contains(&"linux-zen".to_string()));
    }

    #[test]
    fn generating_for_no_kernel_at_all_is_an_error() {
        use crate::system::cmd::tests::RecordingRunner;

        let runner = RecordingRunner::new(vec![]);
        let err = generate(&runner, Path::new("/mnt"), &[]).unwrap_err();
        assert!(
            err.to_string()
                .contains("no installed kernel has a ZFS module")
        );
    }

    #[test]
    fn configure_refuses_a_conf_it_cannot_patch_safely() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("etc/mkinitcpio.conf");
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        let original = "MODULES=()\nHOOKS=(base udev\n  block filesystems)\n";
        fs::write(&conf_path, original).unwrap();

        assert!(configure(dir.path(), false).is_err());
        // The original config must be left intact for the user to fix.
        assert_eq!(fs::read_to_string(&conf_path).unwrap(), original);
    }

    #[test]
    fn test_configure_mkinitcpio() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("etc/mkinitcpio.conf");
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        fs::write(
            &conf_path,
            "MODULES=()\nHOOKS=(base udev autodetect modconf block filesystems fsck)\n#COMPRESSION=\"zstd\"\n",
        )
        .unwrap();

        configure(dir.path(), true).unwrap();

        let content = fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("MODULES=(zfs)"));
        assert!(content.contains("zfs filesystems"));
        assert!(content.contains("COMPRESSION=\"cat\""));
        assert!(content.contains("/etc/zfs/zroot.key"));
    }
}
