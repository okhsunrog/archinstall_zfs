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
        .filter(|line| {
            let trimmed = line.trim();
            // Keep empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return true;
            }
            // Filter out ZFS-managed mounts
            !trimmed.contains("zfs") && !trimmed.contains(pool_name)
        })
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
