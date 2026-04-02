use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{CommandRunner, check_exit};

pub fn zap_disk(runner: &dyn CommandRunner, disk: &Path) -> Result<()> {
    let disk_str = disk.to_str().unwrap();

    // Clear first and last 34 sectors (GPT/MBR signatures)
    let output = runner.run(
        "dd",
        &[
            "if=/dev/zero",
            &format!("of={disk_str}"),
            "bs=512",
            "count=34",
            "conv=notrunc",
        ],
    )?;
    check_exit(&output, "dd zero first sectors")?;

    // Get disk size for zeroing the end
    let output = runner.run("blockdev", &["--getsz", disk_str])?;
    check_exit(&output, "blockdev --getsz")?;
    let sectors: u64 = output.stdout.trim().parse().unwrap_or(0);
    if sectors > 34 {
        let seek = sectors - 34;
        let _ = runner.run(
            "dd",
            &[
                "if=/dev/zero",
                &format!("of={disk_str}"),
                "bs=512",
                "count=34",
                &format!("seek={seek}"),
                "conv=notrunc",
            ],
        );
    }

    // Zap with sgdisk
    let output = runner.run("sgdisk", &["--zap-all", disk_str])?;
    check_exit(&output, "sgdisk --zap-all")?;

    Ok(())
}

pub struct PartitionLayout {
    pub efi_part_num: u32,
    pub zfs_part_num: u32,
    pub swap_part_num: Option<u32>,
}

pub fn create_partitions(
    runner: &dyn CommandRunner,
    disk: &Path,
    swap_size: Option<&str>,
) -> Result<PartitionLayout> {
    let disk_str = disk.to_str().unwrap();

    // Create GPT table
    let output = runner.run("sgdisk", &["-o", disk_str])?;
    check_exit(&output, "sgdisk create GPT")?;

    // Partition 1: EFI (500M)
    let output = runner.run(
        "sgdisk",
        &["-n", "1:0:+500M", "-t", "1:ef00", "-c", "1:EFI", disk_str],
    )?;
    check_exit(&output, "sgdisk create EFI partition")?;

    let layout = if let Some(swap_sz) = swap_size {
        // Partition 3: Swap (at end of disk)
        let swap_spec = format!("-{swap_sz}:0");
        let output = runner.run(
            "sgdisk",
            &[
                "-n",
                &format!("3:{swap_spec}"),
                "-t",
                "3:8200",
                "-c",
                "3:swap",
                disk_str,
            ],
        )?;
        check_exit(&output, "sgdisk create swap partition")?;

        // Partition 2: ZFS (remaining space)
        let output = runner.run(
            "sgdisk",
            &["-n", "2:0:0", "-t", "2:bf00", "-c", "2:ZFS", disk_str],
        )?;
        check_exit(&output, "sgdisk create ZFS partition")?;

        PartitionLayout {
            efi_part_num: 1,
            zfs_part_num: 2,
            swap_part_num: Some(3),
        }
    } else {
        // Partition 2: ZFS (rest of disk)
        let output = runner.run(
            "sgdisk",
            &["-n", "2:0:0", "-t", "2:bf00", "-c", "2:ZFS", disk_str],
        )?;
        check_exit(&output, "sgdisk create ZFS partition")?;

        PartitionLayout {
            efi_part_num: 1,
            zfs_part_num: 2,
            swap_part_num: None,
        }
    };

    // Inform kernel and udev about partition changes
    let _ = runner.run("partprobe", &[disk_str]);
    let _ = runner.run("udevadm", &["settle"]);

    // Wait for by-id symlinks to appear
    let efi_dev = format!("{disk_str}-part{}", layout.efi_part_num);
    for _ in 0..50 {
        if std::path::Path::new(&efi_dev).exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // Format EFI partition
    let output = runner.run("mkfs.fat", &["-I", "-F32", &efi_dev])?;
    check_exit(&output, "mkfs.fat EFI")?;

    Ok(layout)
}

/// Wait for /dev/disk/by-id partition symlinks to appear after partitioning.
pub fn wait_for_by_id_partitions(disk: &Path, layout: &PartitionLayout) -> Vec<std::path::PathBuf> {
    let disk_str = disk.to_str().unwrap();
    let mut parts = vec![
        format!("{disk_str}-part{}", layout.efi_part_num),
        format!("{disk_str}-part{}", layout.zfs_part_num),
    ];
    if let Some(swap) = layout.swap_part_num {
        parts.push(format!("{disk_str}-part{swap}"));
    }

    let mut result = Vec::new();
    for part in &parts {
        let path = std::path::PathBuf::from(part);
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        result.push(path);
    }
    result
}

pub fn mount_efi(
    runner: &dyn CommandRunner,
    efi_partition: &Path,
    mountpoint: &Path,
) -> Result<()> {
    let efi_str = efi_partition.to_str().unwrap();
    let mount_path = mountpoint.join("boot/efi");
    std::fs::create_dir_all(&mount_path)?;
    let mount_str = mount_path.to_str().unwrap();
    let output = runner.run("mount", &[efi_str, mount_str])?;
    check_exit(&output, "mount EFI")?;
    Ok(())
}

pub fn umount_efi(runner: &dyn CommandRunner, mountpoint: &Path) -> Result<()> {
    let mount_path = mountpoint.join("boot/efi");
    let mount_str = mount_path.to_str().unwrap();
    let _ = runner.run("umount", &[mount_str]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_zap_disk_command_sequence() {
        let responses = vec![
            CannedResponse::default(), // dd first sectors
            CannedResponse {
                stdout: "1000000\n".into(),
                ..Default::default()
            }, // blockdev --getsz
            CannedResponse::default(), // dd last sectors
            CannedResponse::default(), // sgdisk --zap-all
        ];
        let runner = RecordingRunner::new(responses);
        zap_disk(&runner, Path::new("/dev/disk/by-id/test-disk")).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "dd");
        assert_eq!(calls[1].program, "blockdev");
        assert_eq!(calls[2].program, "dd");
        assert_eq!(calls[3].program, "sgdisk");
        assert!(calls[3].args.contains(&"--zap-all".to_string()));
    }

    #[test]
    fn test_create_partitions_no_swap() {
        let responses = vec![
            CannedResponse::default(), // sgdisk -o
            CannedResponse::default(), // sgdisk EFI
            CannedResponse::default(), // sgdisk ZFS
            CannedResponse::default(), // mkfs.fat
        ];
        let runner = RecordingRunner::new(responses);
        let layout =
            create_partitions(&runner, Path::new("/dev/disk/by-id/test-disk"), None).unwrap();

        assert_eq!(layout.efi_part_num, 1);
        assert_eq!(layout.zfs_part_num, 2);
        assert!(layout.swap_part_num.is_none());
    }

    #[test]
    fn test_create_partitions_with_swap() {
        let responses = vec![
            CannedResponse::default(), // sgdisk -o
            CannedResponse::default(), // sgdisk EFI
            CannedResponse::default(), // sgdisk swap
            CannedResponse::default(), // sgdisk ZFS
            CannedResponse::default(), // mkfs.fat
        ];
        let runner = RecordingRunner::new(responses);
        let layout =
            create_partitions(&runner, Path::new("/dev/disk/by-id/test-disk"), Some("8G")).unwrap();

        assert_eq!(layout.efi_part_num, 1);
        assert_eq!(layout.zfs_part_num, 2);
        assert_eq!(layout.swap_part_num, Some(3));
    }
}
