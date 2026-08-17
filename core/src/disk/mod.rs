pub mod device;
pub mod partition;

/// Strip the partition suffix from a block device name, yielding the whole
/// disk it belongs to.
///
/// - `sda1` → `sda`, `sdb12` → `sdb` (trailing digits)
/// - `nvme0n1p2` → `nvme0n1`, and `nvme0n1` → `nvme0n1` (the namespace number
///   is part of the disk's name, not a partition)
/// - `mmcblk0p2` → `mmcblk0`
///
/// The `p<N>` rule applies only to names that use it. Treating any trailing
/// `p<digits>` as a partition suffix regardless of the prefix turns `sdp1`
/// into `sd` — and `sdp` is an ordinary name once a machine has sixteen SCSI
/// disks.
pub fn whole_disk_name(name: &str) -> &str {
    if name.starts_with("nvme") || name.starts_with("mmcblk") || name.starts_with("loop") {
        if let Some(pos) = name.rfind('p') {
            let after = &name[pos + 1..];
            if !after.is_empty() && after.bytes().all(|b| b.is_ascii_digit()) {
                return &name[..pos];
            }
        }
        return name;
    }
    name.trim_end_matches(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::whole_disk_name;

    #[test]
    fn strips_trailing_digit_partitions() {
        assert_eq!(whole_disk_name("sda1"), "sda");
        assert_eq!(whole_disk_name("sda"), "sda");
        assert_eq!(whole_disk_name("sdb12"), "sdb");
        assert_eq!(whole_disk_name("vda1"), "vda");
    }

    #[test]
    fn keeps_the_namespace_number_of_p_suffixed_devices() {
        assert_eq!(whole_disk_name("nvme0n1p1"), "nvme0n1");
        assert_eq!(whole_disk_name("nvme0n1p12"), "nvme0n1");
        assert_eq!(whole_disk_name("nvme0n1"), "nvme0n1");
        assert_eq!(whole_disk_name("mmcblk0p2"), "mmcblk0");
        assert_eq!(whole_disk_name("mmcblk0"), "mmcblk0");
    }

    #[test]
    fn a_p_in_a_scsi_name_is_not_a_partition_marker() {
        // The seventeenth SCSI disk. Splitting on the 'p' would report its
        // first partition as belonging to a disk called "sd".
        assert_eq!(whole_disk_name("sdp1"), "sdp");
        assert_eq!(whole_disk_name("sdp"), "sdp");
        assert_eq!(whole_disk_name("sdp15"), "sdp");
    }
}
