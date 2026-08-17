use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit};

pub fn generate_fstab(
    runner: &dyn CommandRunner,
    target: &Path,
    pool_name: &str,
    prefix: &str,
) -> Result<()> {
    let target_str = target.to_string_lossy();

    // Run genfstab
    let output = runner.run("genfstab", &["-U", &target_str])?;
    check_exit(&output, "genfstab")?;

    // Filter out ZFS lines and fix EFI mount options
    let fstab_content: String = output
        .stdout
        .lines()
        .filter(|line| !is_zfs_managed_mount(line, pool_name))
        .map(|line| {
            // For the EFI mount (/boot/efi vfat), set passno to 0 (no fsck
            // for vfat) and ensure nofail so a failed mount doesn't block boot
            if line.contains("/boot/efi") && line.contains("vfat") && !line.trim().starts_with('#')
            {
                let mut fixed = line.to_string();
                // Inject nofail into mount options
                if !fixed.contains("nofail") {
                    fixed = fixed.replacen("\trw,", "\trw,nofail,", 1);
                }
                // Replace passno 2 or 1 with 0 at end of line
                if fixed.ends_with("\t0\t2") || fixed.ends_with("\t0\t1") {
                    let len = fixed.len();
                    fixed.replace_range(len - 3.., "0\t0");
                }
                fixed
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Add root dataset explicitly
    let root_ds = format!("{pool_name}/{prefix}/root");
    let mut final_fstab = fstab_content;
    final_fstab.push_str(&format!(
        "\n# ZFS root dataset\n{root_ds}\t/\tzfs\tdefaults\t0\t0\n"
    ));

    // Write fstab
    let fstab_path = target.join("etc/fstab");
    if let Some(parent) = fstab_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&fstab_path, final_fstab).wrap_err("failed to write fstab")?;

    tracing::info!("generated fstab");
    Ok(())
}

/// Whether an fstab line describes a mount ZFS manages itself, which the
/// zfs-mount-generator handles and fstab must not duplicate.
///
/// Decided on the filesystem-type column, not on whether the line happens to
/// contain the text "zfs" anywhere: device paths legitimately carry the pool's
/// name (`/dev/disk/by-id/…-archzfs-…`, a pool named after the host), and
/// dropping the EFI mount because its by-id path contains "zfs" would leave
/// the system without /boot/efi.
fn is_zfs_managed_mount(line: &str, pool_name: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }

    let mut fields = trimmed.split_whitespace();
    let Some(device) = fields.next() else {
        return false;
    };
    let fs_type = fields.nth(1); // skip the mountpoint

    // A dataset of our pool is ZFS-managed whatever type column it was given.
    fs_type == Some("zfs") || device == pool_name || device.starts_with(&format!("{pool_name}/"))
}

pub fn add_swap_entry(target: &Path, device: &str) -> Result<()> {
    let fstab_path = target.join("etc/fstab");
    let mut content = if fstab_path.exists() {
        fs::read_to_string(&fstab_path)?
    } else {
        String::new()
    };

    content.push_str(&format!("\n# Swap\n{device}\tnone\tswap\tdefaults\t0\t0\n"));
    fs::write(&fstab_path, content)?;
    Ok(())
}

pub fn add_cryptswap_entry(target: &Path, device: &str) -> Result<()> {
    // Add crypttab entry
    let crypttab_path = target.join("etc/crypttab");
    if let Some(parent) = crypttab_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut crypttab = if crypttab_path.exists() {
        fs::read_to_string(&crypttab_path)?
    } else {
        String::new()
    };
    crypttab.push_str(&format!(
        "cryptswap\t{device}\t/dev/urandom\tswap,cipher=aes-xts-plain64,size=256\n"
    ));
    fs::write(&crypttab_path, crypttab)?;

    // Add fstab entry for the decrypted device
    add_swap_entry(target, "/dev/mapper/cryptswap")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_generate_fstab_filters_zfs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("etc")).unwrap();

        let genfstab_output = "# /etc/fstab
UUID=1234 /boot/efi vfat defaults 0 2
testpool/arch0/root / zfs defaults 0 0
testpool/arch0/data/home /home zfs defaults 0 0
";
        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: genfstab_output.into(),
            ..Default::default()
        }]);

        generate_fstab(&runner, dir.path(), "testpool", "arch0").unwrap();

        let fstab = fs::read_to_string(dir.path().join("etc/fstab")).unwrap();
        // Should keep EFI entry
        assert!(fstab.contains("UUID=1234"));
        // Should filter out ZFS lines from genfstab
        assert!(!fstab.contains("data/home"));
        // Should add explicit root dataset
        assert!(fstab.contains("testpool/arch0/root\t/\tzfs"));
    }

    #[test]
    fn efi_entry_survives_a_device_path_containing_the_pool_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("etc")).unwrap();

        // genfstab -U normally emits UUIDs, but it falls back to the device
        // path when a filesystem has no UUID — and that path routinely
        // contains both "zfs" and the pool name.
        let genfstab_output = "\
/dev/disk/by-id/virtio-archzfs-test-disk-part1\t/boot/efi\tvfat\trw,relatime\t0\t2
zroot/arch0/root\t/\tzfs\tdefaults\t0\t0
";
        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: genfstab_output.into(),
            ..Default::default()
        }]);

        generate_fstab(&runner, dir.path(), "zroot", "arch0").unwrap();

        let fstab = fs::read_to_string(dir.path().join("etc/fstab")).unwrap();
        assert!(
            fstab.contains("/boot/efi"),
            "the EFI mount must not be dropped: {fstab}"
        );
        // genfstab's ZFS line is filtered; the explicit root entry this
        // function appends is the only one left.
        let root_entries = fstab
            .lines()
            .filter(|line| line.starts_with("zroot/arch0/root"))
            .count();
        assert_eq!(root_entries, 1, "root dataset must appear once: {fstab}");
    }

    #[test]
    fn zfs_managed_mounts_are_recognised_by_column_not_substring() {
        // Real ZFS mounts, by type column and by dataset name.
        assert!(is_zfs_managed_mount(
            "zroot/arch0/data/home\t/home\tzfs\tdefaults\t0\t0",
            "zroot"
        ));
        assert!(is_zfs_managed_mount(
            "zroot\t/\tzfs\tdefaults\t0\t0",
            "zroot"
        ));

        // Not ZFS mounts, despite the substrings.
        assert!(!is_zfs_managed_mount(
            "/dev/disk/by-id/nvme-archzfs_ssd-part1\t/boot/efi\tvfat\trw\t0\t2",
            "zroot"
        ));
        assert!(!is_zfs_managed_mount(
            "UUID=1234\t/mnt/zfsbackup\text4\tdefaults\t0\t2",
            "zroot"
        ));
        // A pool named like a common word must not eat unrelated mounts.
        assert!(!is_zfs_managed_mount(
            "/dev/sda2\t/data\text4\tdefaults\t0\t2",
            "data"
        ));

        // Comments and blanks are kept.
        assert!(!is_zfs_managed_mount("# zroot/arch0/root", "zroot"));
        assert!(!is_zfs_managed_mount("", "zroot"));
    }

    #[test]
    fn test_add_swap_entry() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        fs::write(dir.path().join("etc/fstab"), "# existing\n").unwrap();

        add_swap_entry(dir.path(), "/dev/sda3").unwrap();

        let fstab = fs::read_to_string(dir.path().join("etc/fstab")).unwrap();
        assert!(fstab.contains("/dev/sda3"));
        assert!(fstab.contains("swap"));
    }

    #[test]
    fn test_add_cryptswap_entry() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        fs::write(dir.path().join("etc/fstab"), "").unwrap();

        add_cryptswap_entry(dir.path(), "/dev/disk/by-id/test-part3").unwrap();

        let crypttab = fs::read_to_string(dir.path().join("etc/crypttab")).unwrap();
        assert!(crypttab.contains("cryptswap"));
        assert!(crypttab.contains("/dev/disk/by-id/test-part3"));

        let fstab = fs::read_to_string(dir.path().join("etc/fstab")).unwrap();
        assert!(fstab.contains("/dev/mapper/cryptswap"));
    }
}
