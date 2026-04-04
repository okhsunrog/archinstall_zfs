pub fn cpu_vendor() -> CpuVendor {
    let info = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::nothing()),
    );
    let vendor = info
        .cpus()
        .first()
        .map(|c| c.vendor_id().to_lowercase())
        .unwrap_or_default();

    if vendor.contains("intel") {
        CpuVendor::Intel
    } else if vendor.contains("amd") || vendor.contains("authenticamd") {
        CpuVendor::Amd
    } else {
        CpuVendor::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

impl CpuVendor {
    pub fn microcode_package(&self) -> Option<&'static str> {
        match self {
            Self::Intel => Some("intel-ucode"),
            Self::Amd => Some("amd-ucode"),
            Self::Unknown => None,
        }
    }
}

pub fn has_uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

/// What kind of storage a device is, used to pick the right TRIM strategy.
///
/// ZFS has two TRIM mechanisms:
///   - `autotrim=on`: continuous background TRIM issued as blocks are freed.
///     Fast and fine on NVMe (deep command queues absorb it with no penalty).
///     Harmful on SATA SSDs: TRIM commands block I/O on the SATA bus, causing
///     latency spikes under load on consumer drives.
///   - `zpool trim <pool>` (periodic): one-shot, runs offline or via the
///     shipped `zfs-trim-weekly@<pool>.timer` unit. Correct for SATA SSDs.
///
/// `fstrim`/`fstrim.timer` does NOT work with ZFS — it operates at the
/// filesystem level and has no awareness of ZFS's internal block allocator.
/// Enabling it on a ZFS-only system is a harmless no-op, but provides zero
/// benefit and should not be used.
///
/// References:
///   - https://openzfs.github.io/openzfs-docs/man/master/8/zpool-trim.8.html
///   - https://cr0x.net/en/zfs-autotrim-ssd-pools/
///   - https://discourse.practicalzfs.com/t/trim-on-ssd-pools/4441
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    /// NVMe SSD — use `autotrim=on` (pool property).
    Nvme,
    /// SATA/SAS SSD — use periodic `zpool trim` via `zfs-trim-weekly@<pool>.timer`.
    SataSsd,
    /// Spinning hard disk — TRIM not applicable.
    Hdd,
}

/// Detect the storage type for a device path (may be a partition or disk,
/// raw `/dev/…` node, or a `/dev/disk/by-id/…` symlink).
pub fn detect_storage_type(dev_path: &std::path::Path) -> StorageType {
    let real = std::fs::canonicalize(dev_path).unwrap_or_else(|_| dev_path.to_path_buf());
    let dev_name = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    let base = strip_partition_suffix(dev_name);

    // NVMe device nodes are always named nvme<ctrl>n<ns>[p<part>].
    if base.starts_with("nvme") {
        return StorageType::Nvme;
    }

    let rotational_path = format!("/sys/class/block/{base}/queue/rotational");
    let is_ssd = std::fs::read_to_string(rotational_path)
        .map(|s| s.trim() == "0")
        .unwrap_or(false);

    if is_ssd {
        StorageType::SataSsd
    } else {
        StorageType::Hdd
    }
}

/// Strip the partition suffix from a block device name.
///
/// - NVMe:  `nvme0n1p2` → `nvme0n1`  (partition suffix is `p<N>`)
/// - NVMe:  `nvme0n1`   → `nvme0n1`  (no partition, unchanged)
/// - SCSI:  `sda1`      → `sda`      (partition suffix is trailing digits)
fn strip_partition_suffix(name: &str) -> &str {
    if name.starts_with("nvme") {
        // NVMe partition suffix is 'p' followed by one or more digits.
        // Only strip if we actually find that pattern to avoid eating the
        // namespace number (e.g. the '1' in nvme0n1 is NOT a partition).
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
    use super::*;

    #[test]
    fn test_cpu_vendor_microcode() {
        assert_eq!(CpuVendor::Intel.microcode_package(), Some("intel-ucode"));
        assert_eq!(CpuVendor::Amd.microcode_package(), Some("amd-ucode"));
        assert_eq!(CpuVendor::Unknown.microcode_package(), None);
    }

    #[test]
    fn test_strip_partition_suffix() {
        assert_eq!(strip_partition_suffix("sda1"), "sda");
        assert_eq!(strip_partition_suffix("sda"), "sda");
        assert_eq!(strip_partition_suffix("sdb12"), "sdb");
        assert_eq!(strip_partition_suffix("nvme0n1p1"), "nvme0n1");
        assert_eq!(strip_partition_suffix("nvme0n1p12"), "nvme0n1");
        assert_eq!(strip_partition_suffix("nvme0n1"), "nvme0n1");
        assert_eq!(strip_partition_suffix("vda1"), "vda");
    }
}
