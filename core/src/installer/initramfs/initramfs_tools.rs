//! Debian's initramfs-tools with `zfs-initramfs`.
//!
//! The kernel packages install their own `/boot/vmlinuz-<version>` and call
//! `update-initramfs` from their maintainer scripts, so unlike dracut on Arch
//! there are no hooks to write: this only adds what the pool needs and builds
//! the images once ZFS is in place.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use color_eyre::eyre::{Context, Result, bail};

use crate::system::cmd::{CommandRunner, check_exit, chroot_checked, chroot_cmd};

/// Where the pool's passphrase lives in the target and in the image.
const KEY_FILE: &str = "/etc/zfs/zroot.key";

/// Copies the passphrase into the image, where `zfs-initramfs` loads it
/// from the `file://` keylocation.
const KEY_HOOK: &str = r#"#!/bin/sh
# Written by archinstall_zfs: the pool's passphrase, for zfs load-key.
PREREQ=""
prereqs() { echo "$PREREQ"; }
case "$1" in
prereqs) prereqs; exit 0 ;;
esac
. /usr/share/initramfs-tools/hook-functions
copy_file key /etc/zfs/zroot.key
"#;

/// An image holding the passphrase must not be readable by everyone.
const UMASK_CONF: &str =
    "# Written by archinstall_zfs: the image holds the pool's passphrase.\nUMASK=0077\n";

const KEY_HOOK_PATH: &str = "etc/initramfs-tools/hooks/azfs-zfs-key";
const UMASK_CONF_PATH: &str = "etc/initramfs-tools/conf.d/azfs-umask";

pub fn configure(target: &Path, encryption: bool) -> Result<()> {
    if !encryption {
        tracing::info!("configured initramfs-tools");
        return Ok(());
    }
    let hook = target.join(KEY_HOOK_PATH);
    let conf = target.join(UMASK_CONF_PATH);
    for path in [&hook, &conf] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&hook, KEY_HOOK).wrap_err("failed to write the initramfs key hook")?;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))?;
    fs::write(&conf, UMASK_CONF).wrap_err("failed to write the initramfs umask")?;
    tracing::info!("configured initramfs-tools with the pool key");
    Ok(())
}

/// Kernels in the target that can boot the pool: installed in `/boot` by
/// their package, with a ZFS module DKMS has built for them.
///
/// The module is looked for rather than assumed. DKMS builds on package
/// installation and does not fail the transaction when a build fails, so the
/// file is the only evidence that this kernel can import the pool.
fn kernels_with_zfs(target: &Path) -> Result<Vec<String>> {
    let modules_dir = target.join("usr/lib/modules");
    let entries = fs::read_dir(&modules_dir)
        .wrap_err_with(|| format!("failed to read {}", modules_dir.display()))?;

    let mut installed = Vec::new();
    let mut with_zfs = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let kver = entry.file_name().to_string_lossy().into_owned();
        if !target.join(format!("boot/vmlinuz-{kver}")).exists() {
            continue;
        }
        installed.push(kver.clone());
        if has_zfs_module(&entry.path()) {
            with_zfs.push(kver);
        } else {
            tracing::warn!(kver, "skipping initramfs: DKMS built no ZFS module for it");
        }
    }

    if installed.is_empty() {
        bail!("no installed kernel found under {}/boot", target.display());
    }
    if with_zfs.is_empty() {
        bail!("no installed kernel has a ZFS module, so none can boot this pool");
    }
    with_zfs.sort();
    Ok(with_zfs)
}

fn has_zfs_module(module_dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(module_dir.join("updates/dkms")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name == "zfs.ko" || name.starts_with("zfs.ko.")
    })
}

/// Build the image for every kernel that has a ZFS module, and check that it
/// carries what the pool needs.
///
/// The kernel's own installation already built one, before ZFS was there;
/// that image is replaced. A kernel without a module keeps whatever it has,
/// and is logged: ZFSBootMenu may still list it.
pub fn generate(runner: &dyn CommandRunner, target: &Path, encryption: bool) -> Result<()> {
    let kernels = kernels_with_zfs(target)?;
    for kver in &kernels {
        let image = format!("/boot/initrd.img-{kver}");
        let mode = if target.join(image.trim_start_matches('/')).exists() {
            "-u"
        } else {
            "-c"
        };
        tracing::info!(kver, "generating initramfs");
        chroot_checked(
            runner,
            target,
            "update-initramfs",
            &[mode, "-k", kver],
            &format!("update-initramfs for {kver}"),
        )?;
        verify_image(runner, target, &image, encryption)?;
    }
    tracing::info!(
        count = kernels.len(),
        "generated initramfs with initramfs-tools"
    );
    Ok(())
}

/// Refuse an image that could not import the pool.
fn verify_image(
    runner: &dyn CommandRunner,
    target: &Path,
    image: &str,
    encryption: bool,
) -> Result<()> {
    let output = chroot_cmd(runner, target, "lsinitramfs", &[image])?;
    check_exit(&output, &format!("list {image}"))?;
    // lsinitramfs lists paths relative to the image root.
    let paths: Vec<&str> = output
        .stdout
        .lines()
        .map(|line| line.trim().trim_start_matches('/'))
        .collect();
    let has_file = |path: &str| paths.contains(&path);
    // The command, not the `etc/zfs` directory the listing also shows.
    let has_command = paths
        .iter()
        .any(|p| *p == "sbin/zfs" || p.ends_with("/sbin/zfs"));
    let has_module = paths.iter().any(|p| {
        p.rsplit('/')
            .next()
            .is_some_and(|file| file == "zfs.ko" || file.starts_with("zfs.ko."))
    });
    if !has_module {
        bail!("{image} has no ZFS module");
    }
    if !has_command {
        bail!("{image} has no zfs command: zfs-initramfs is not installed");
    }
    if encryption && !has_file(KEY_FILE.trim_start_matches('/')) {
        bail!("{image} does not carry the pool key {KEY_FILE}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    fn add_kernel(target: &Path, kver: &str, zfs: bool) {
        let modules = target.join("usr/lib/modules").join(kver);
        fs::create_dir_all(&modules).unwrap();
        fs::create_dir_all(target.join("boot")).unwrap();
        fs::write(target.join(format!("boot/vmlinuz-{kver}")), "").unwrap();
        if zfs {
            fs::create_dir_all(modules.join("updates/dkms")).unwrap();
            fs::write(modules.join("updates/dkms/zfs.ko.xz"), "").unwrap();
        }
    }

    const IMAGE_LISTING: &str = "usr/lib/modules/6.12.107-amd64/updates/dkms/zfs.ko.xz\n\
                                 usr/sbin/zfs\nusr/sbin/zpool\n";

    #[test]
    fn an_unencrypted_pool_needs_no_configuration() {
        let dir = tempfile::tempdir().unwrap();
        configure(dir.path(), false).unwrap();
        assert!(!dir.path().join(KEY_HOOK_PATH).exists());
        assert!(!dir.path().join(UMASK_CONF_PATH).exists());
    }

    #[test]
    fn an_encrypted_pool_gets_its_key_into_a_private_image() {
        let dir = tempfile::tempdir().unwrap();
        configure(dir.path(), true).unwrap();

        let hook = dir.path().join(KEY_HOOK_PATH);
        assert!(
            fs::read_to_string(&hook)
                .unwrap()
                .contains("copy_file key /etc/zfs/zroot.key")
        );
        assert_eq!(
            fs::metadata(&hook).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(
            fs::read_to_string(dir.path().join(UMASK_CONF_PATH))
                .unwrap()
                .contains("UMASK=0077")
        );
    }

    #[test]
    fn only_kernels_with_a_built_module_get_an_image() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        add_kernel(dir.path(), "7.1.8+bpo-amd64", false);
        // Module directory of a removed kernel: no vmlinuz in /boot.
        fs::create_dir_all(
            dir.path()
                .join("usr/lib/modules/6.12.90-amd64/updates/dkms"),
        )
        .unwrap();

        assert_eq!(kernels_with_zfs(dir.path()).unwrap(), ["6.12.107-amd64"]);
    }

    #[test]
    fn no_module_anywhere_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", false);

        let error = kernels_with_zfs(dir.path()).unwrap_err().to_string();
        assert!(
            error.contains("no installed kernel has a ZFS module"),
            "{error}"
        );
    }

    #[test]
    fn an_existing_image_is_updated_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        fs::write(dir.path().join("boot/initrd.img-6.12.107-amd64"), "").unwrap();
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: IMAGE_LISTING.into(),
                ..Default::default()
            },
        ]);

        generate(&runner, dir.path(), false).unwrap();

        let calls = runner.calls();
        assert_eq!(
            calls[0].args[1..],
            ["update-initramfs", "-u", "-k", "6.12.107-amd64"]
        );
        assert_eq!(
            calls[1].args[1..],
            ["lsinitramfs", "/boot/initrd.img-6.12.107-amd64"]
        );
    }

    #[test]
    fn a_missing_image_is_created() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: IMAGE_LISTING.into(),
                ..Default::default()
            },
        ]);

        generate(&runner, dir.path(), false).unwrap();

        assert_eq!(runner.calls()[0].args[2], "-c");
    }

    #[test]
    fn an_image_without_the_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: IMAGE_LISTING.into(),
                ..Default::default()
            },
        ]);

        let error = generate(&runner, dir.path(), true).unwrap_err().to_string();
        assert!(error.contains("pool key"), "{error}");
    }

    #[test]
    fn an_image_with_the_key_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: format!("{IMAGE_LISTING}etc/zfs/zroot.key\n"),
                ..Default::default()
            },
        ]);

        generate(&runner, dir.path(), true).unwrap();
    }

    #[test]
    fn the_zfs_directory_is_not_the_zfs_command() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: "usr/lib/modules/6.12.107-amd64/updates/dkms/zfs.ko.xz\netc/zfs\n".into(),
                ..Default::default()
            },
        ]);

        let error = generate(&runner, dir.path(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no zfs command"), "{error}");
    }

    #[test]
    fn an_image_without_zfs_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        add_kernel(dir.path(), "6.12.107-amd64", true);
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: "usr/sbin/zfs\n".into(),
                ..Default::default()
            },
        ]);

        let error = generate(&runner, dir.path(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no ZFS module"), "{error}");
    }
}
