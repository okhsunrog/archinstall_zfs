//! Planning storage changes without moving existing partition starts.
//!
//! Sector geometry is kept as integers; UI percentages are presentation only.
//! A saved layout is an optimistic concurrency token, never authority to write.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, bail, ensure, eyre};
use serde::{Deserialize, Serialize};

use crate::system::cmd::{CommandRunner, check_exit};

mod efi;
mod execute;
pub use execute::execute;
mod probe;
pub use efi::{BootSpace, EfiChoice, EfiSpace, inspect_efi};
pub use probe::{Filesystem, inspect, minimum_size};

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
pub const MIN_LINUX_BYTES: u64 = 32 * GIB;
/// Size of the optional separate ESP; matches the full-disk layout.
pub const ESP_BYTES: u64 = 512 * MIB;
pub const EFI_TYPE: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
pub const BASIC_TYPE: &str = "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7";
pub const LINUX_TYPE: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    pub node: PathBuf,
    pub start: u64,
    pub size: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub attrs: String,
}

impl Partition {
    pub fn end(&self) -> Result<u64> {
        self.start
            .checked_add(self.size)
            .ok_or_else(|| eyre!("Partition geometry overflow"))
    }

    pub fn number(&self, disk: &Path) -> Result<u32> {
        let node = self
            .node
            .to_str()
            .ok_or_else(|| eyre!("Invalid partition path"))?;
        let disk = disk.to_str().ok_or_else(|| eyre!("Invalid disk path"))?;
        let suffix = node
            .strip_prefix(disk)
            .ok_or_else(|| eyre!("Partition belongs to another disk"))?;
        let number: u32 = suffix.strip_prefix('p').unwrap_or(suffix).parse()?;
        ensure!(
            number > 0 && number <= 128,
            "Unsupported GPT partition number"
        );
        Ok(number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub label: String,
    pub id: String,
    pub device: PathBuf,
    pub unit: String,
    pub firstlba: u64,
    pub lastlba: u64,
    pub sectorsize: u64,
    #[serde(default)]
    pub partitions: Vec<Partition>,
}

impl Layout {
    pub fn parse(json: &str) -> Result<Self> {
        // Diagnose MBR before decoding GPT-only fields.
        let v: serde_json::Value = serde_json::from_str(json)?;
        let table = &v["partitiontable"];
        ensure!(
            table["label"] == "gpt",
            "This disk does not use GPT. Installing alongside another system requires GPT; automatic MBR conversion is not supported. Existing data has not been changed."
        );
        let layout: Self = serde_json::from_value(table.clone())?;
        layout.validate()?;
        Ok(layout)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.label == "gpt" && self.unit == "sectors",
            "A GPT sector layout is required"
        );
        ensure!(
            matches!(self.sectorsize, 512 | 4096),
            "Unsupported logical sector size"
        );
        ensure!(
            !self.id.is_empty() && self.firstlba > 0 && self.lastlba >= self.firstlba,
            "Invalid GPT identity or usable range"
        );
        self.lastlba
            .checked_add(1)
            .and_then(|n| n.checked_mul(self.sectorsize))
            .ok_or_else(|| eyre!("Disk size overflow"))?;
        let mut sorted: Vec<_> = self.partitions.iter().collect();
        sorted.sort_by_key(|p| p.start);
        let mut end = self.firstlba;
        let mut numbers = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for p in sorted {
            ensure!(
                p.size > 0 && p.start >= end && p.end()? <= self.lastlba + 1,
                "Overlapping or out-of-range GPT partitions"
            );
            ensure!(
                !p.uuid.is_empty()
                    && ids.insert(p.uuid.to_uppercase())
                    && numbers.insert(p.number(&self.device)?),
                "Duplicate or missing GPT partition identity"
            );
            end = p.end()?;
        }
        Ok(())
    }

    pub fn read(runner: &dyn CommandRunner, disk: &Path) -> Result<Self> {
        let disk = std::fs::canonicalize(disk)
            .wrap_err_with(|| format!("Selected disk {} is unavailable", disk.display()))?;
        let out = runner.run("sfdisk", &["--json", &disk.to_string_lossy()])
            .wrap_err("Cannot inspect the partition table. Install util-linux (sfdisk) in the live system")?;
        check_exit(&out, "Read partition table")?;
        Self::parse(&out.stdout)
    }

    pub fn free_extents(&self) -> Result<Vec<(u64, u64)>> {
        self.validate()?;
        let mut sorted: Vec<_> = self.partitions.iter().collect();
        sorted.sort_by_key(|p| p.start);
        let mut start = self.firstlba;
        let mut result = Vec::new();
        for p in sorted {
            if p.start > start {
                result.push((start, p.start));
            }
            start = p.end()?;
        }
        if start <= self.lastlba {
            result.push((start, self.lastlba + 1));
        }
        Ok(result)
    }
}

/// An explicit region selected by the user. A partition shrink frees space at
/// its end; existing gaps are selectable independently and are never merged
/// across an intervening partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceSource {
    Shrink { partition: u32 },
    Unallocated { start: u64, end: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub before: Layout,
    pub source: SpaceSource,
    pub efi: EfiChoice,
    /// Includes the separate ESP when one was explicitly selected.
    pub allocation_bytes: u64,
    /// Optional new swap partition, included in allocation_bytes.
    #[serde(default)]
    pub swap_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub shrink: Option<(Partition, u64)>,
    pub efi_number: u32,
    pub zfs_number: u32,
    pub efi_start: Option<u64>,
    pub zfs_start: u64,
    pub end: u64,
    pub swap_number: Option<u32>,
    pub swap_start: Option<u64>,
}

impl Request {
    pub fn plan(&self) -> Result<Plan> {
        let l = &self.before;
        l.validate()?;
        let additional_efi = matches!(self.efi, EfiChoice::CreateSeparate { .. });
        let efi_bytes = if additional_efi { ESP_BYTES } else { 0 };
        let minimum = MIN_LINUX_BYTES
            .checked_add(efi_bytes)
            .and_then(|n| n.checked_add(self.swap_bytes))
            .ok_or_else(|| eyre!("Allocation overflow"))?;
        ensure!(
            self.swap_bytes.is_multiple_of(MIB),
            "Swap size must be aligned to MiB"
        );
        ensure!(
            self.allocation_bytes >= minimum,
            "Reserve at least 32 GiB for the ZFS pool, plus the selected swap and the separate EFI partition if selected"
        );
        let existing_efi = self.efi.existing_partition();
        ensure!(
            l.partitions
                .iter()
                .any(|p| p.number(&l.device).ok() == Some(existing_efi)
                    && p.kind.eq_ignore_ascii_case(EFI_TYPE)),
            "Select an existing EFI system partition on this disk"
        );
        ensure!(
            self.allocation_bytes.is_multiple_of(MIB),
            "Allocation must be aligned to MiB"
        );
        let alignment = MIB / l.sectorsize;
        let allocation = self.allocation_bytes / l.sectorsize;
        let (start, end, shrink) = match self.source {
            SpaceSource::Shrink { partition } => {
                ensure!(
                    partition != existing_efi,
                    "The existing EFI partition must not be shrunk"
                );
                let p = l
                    .partitions
                    .iter()
                    .find(|p| p.number(&l.device).ok() == Some(partition))
                    .ok_or_else(|| eyre!("Selected partition is missing"))?;
                let end = p.end()? / alignment * alignment;
                let start = end
                    .checked_sub(allocation)
                    .ok_or_else(|| eyre!("Not enough space"))?;
                ensure!(
                    start > p.start,
                    "Allocation would erase the selected filesystem"
                );
                (start, end, Some((p.clone(), start - p.start)))
            }
            SpaceSource::Unallocated { start, end } => {
                ensure!(
                    l.free_extents()?.contains(&(start, end)),
                    "Selected unallocated extent has changed"
                );
                let aligned = start.div_ceil(alignment) * alignment;
                let new_end = aligned
                    .checked_add(allocation)
                    .ok_or_else(|| eyre!("Allocation overflow"))?;
                ensure!(
                    new_end <= end,
                    "Allocation exceeds the selected free extent"
                );
                (aligned, new_end, None)
            }
        };
        let needed = 1 + usize::from(additional_efi) + usize::from(self.swap_bytes > 0);
        let unused: Vec<_> = (1..=128)
            .filter(|n| {
                !l.partitions
                    .iter()
                    .any(|p| p.number(&l.device).ok() == Some(*n))
            })
            .take(needed)
            .collect();
        ensure!(
            unused.len() == needed,
            "GPT has no room for the requested partition entries"
        );
        Ok(Plan {
            shrink,
            efi_number: if additional_efi {
                unused[0]
            } else {
                existing_efi
            },
            zfs_number: unused[needed - 1],
            efi_start: additional_efi.then_some(start),
            zfs_start: start + efi_bytes / l.sectorsize,
            end,
            swap_number: (self.swap_bytes > 0).then(|| unused[usize::from(additional_efi)]),
            swap_start: (self.swap_bytes > 0).then_some(end - self.swap_bytes / l.sectorsize),
        })
    }
}

/// Does not run any filesystem tools, mount filesystems, or repair metadata.
/// This is a capability check, not proof that a particular filesystem is safe.
pub fn required_tools(filesystem: &str) -> Result<&'static [(&'static str, &'static str)]> {
    match filesystem {
        "ntfs" => Ok(&[("ntfsresize", "ntfs-3g"), ("ntfs-3g.probe", "ntfs-3g")]),
        "ext4" => Ok(&[
            ("resize2fs", "e2fsprogs"),
            ("e2fsck", "e2fsprogs"),
            ("dumpe2fs", "e2fsprogs"),
        ]),
        "crypto_LUKS" | "BitLocker" => bail!(
            "Encrypted partitions cannot be resized by this installer. Shrink them from the existing system, then select unallocated space."
        ),
        "LVM2_member" | "linux_raid_member" => bail!(
            "Resizing LVM or RAID containers is not supported. Use the existing system's storage tools to make unallocated space."
        ),
        "btrfs" => bail!(
            "Automatic Btrfs shrinking is not supported yet. Make unallocated space first, then select it here."
        ),
        _ => bail!(
            "Shrinking {filesystem} is not supported. You can use unallocated space without resizing this partition."
        ),
    }
}

pub fn check_tools(tools: &[(&str, &str)]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let search = std::env::var_os("PATH").unwrap_or_default();
    let missing: Vec<_> = tools
        .iter()
        .filter(|(name, _)| {
            !std::env::split_paths(&search).any(|dir| {
                std::fs::metadata(dir.join(name))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
        })
        .map(|(name, package)| format!("{name} ({package})"))
        .collect();
    ensure!(
        missing.is_empty(),
        "This operation is unavailable: install {} in the live system, then refresh. Other installation modes remain available.",
        missing.join(", ")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(sectorsize: u64) -> Layout {
        Layout {
            label: "gpt".into(),
            id: "disk-guid".into(),
            device: "/dev/nvme0n1".into(),
            unit: "sectors".into(),
            firstlba: MIB / sectorsize,
            lastlba: 128 * GIB / sectorsize - 34,
            sectorsize,
            partitions: vec![Partition {
                node: "/dev/nvme0n1p3".into(),
                start: GIB / sectorsize,
                size: 100 * GIB / sectorsize,
                kind: BASIC_TYPE.into(),
                uuid: "partition-guid".into(),
                name: "Data".into(),
                attrs: "".into(),
            }],
        }
    }

    #[test]
    fn shrink_preserves_start_and_allocates_with_both_sector_sizes() {
        for sectorsize in [512, 4096] {
            let mut l = layout(sectorsize);
            l.partitions.push(Partition {
                node: "/dev/nvme0n1p1".into(),
                start: MIB / sectorsize,
                size: (GIB - MIB) / sectorsize,
                kind: EFI_TYPE.into(),
                uuid: "efi-guid".into(),
                name: "EFI".into(),
                attrs: "".into(),
            });
            let mut r = Request {
                before: l.clone(),
                source: SpaceSource::Shrink { partition: 3 },
                efi: EfiChoice::Reuse { partition: 1 },
                allocation_bytes: 32 * GIB,
                swap_bytes: 0,
            };
            let p = r.plan().unwrap();
            let (old, size) = p.shrink.unwrap();
            assert_eq!(old, l.partitions[0]);
            assert_eq!((old.start + size) * sectorsize, 69 * GIB);
            assert_eq!((p.end - p.zfs_start) * sectorsize, 32 * GIB);
            assert_eq!((p.efi_number, p.zfs_number), (1, 2));
            r.swap_bytes = 8 * GIB;
            assert!(
                r.plan().is_err(),
                "swap cannot consume the minimum ZFS capacity"
            );
            r.allocation_bytes = 40 * GIB;
            let p = r.plan().unwrap();
            assert_eq!((p.swap_start.unwrap() - p.zfs_start) * sectorsize, 32 * GIB);
            assert_eq!((p.end - p.swap_start.unwrap()) * sectorsize, 8 * GIB);
            assert_ne!(p.swap_number, Some(p.zfs_number));
            assert_ne!(p.swap_number, Some(p.efi_number));
            r.swap_bytes = u64::MAX;
            assert!(r.plan().is_err());
        }
    }

    #[test]
    fn rejects_mbr_even_without_gpt_fields() {
        assert!(
            Layout::parse(r#"{"partitiontable":{"label":"dos"}}"#)
                .unwrap_err()
                .to_string()
                .contains("MBR")
        );
    }

    #[test]
    fn rejects_overlap_overflow_and_allocation_through_neighbours() {
        let mut l = layout(512);
        l.partitions.push(l.partitions[0].clone());
        assert!(l.validate().is_err());
        l.partitions.pop();
        l.partitions[0].size = u64::MAX;
        assert!(l.validate().is_err());
        let l = layout(512);
        let r = Request {
            before: l,
            source: SpaceSource::Unallocated {
                start: 2048,
                end: 128 * GIB / 512,
            },
            efi: EfiChoice::Reuse { partition: 1 },
            allocation_bytes: 33 * GIB,
            swap_bytes: 0,
        };
        assert!(r.plan().is_err());
    }
}
