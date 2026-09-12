//! What is on each disk, in the terms the Disk step asks the user to decide
//! in: which operating systems are there, where the pools and free space
//! are, and which installation mode that suggests.
//!
//! Built from the same `lsblk` inventory the pickers use, so it needs no
//! extra probing and no root-only tools; the alongside survey still does
//! the precise resize checks once that mode is chosen.

use std::path::{Path, PathBuf};

use crate::config::types::InstallationMode;

use super::device::{DeviceChoice, format_size};

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;
/// Smaller gaps are alignment padding, not usable space.
const MIN_GAP: u64 = 64 * MIB;
/// Room that makes an alongside install worth suggesting.
const ALONGSIDE_FREE: u64 = 20 * GIB;
/// A system partition this big can usually give up enough space.
const ALONGSIDE_SHRINKABLE: u64 = 60 * GIB;

const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
const MSR_TYPE: &str = "e3c9e316-0b5c-4db8-817d-f92df00215ae";
const WINRE_TYPE: &str = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";
const SWAP_TYPE: &str = "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f";

/// Colours of the disk map; the numbers are what the Slint side switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Other = 0,
    Zfs = 1,
    Efi = 2,
    Windows = 3,
    Free = 4,
    Swap = 5,
    Linux = 6,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub offset: f32,
    pub fraction: f32,
    pub kind: SegmentKind,
    pub label: String,
    pub short_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub mode: InstallationMode,
    /// Button text, such as "Install alongside Windows".
    pub title: String,
    /// One sentence on why this fits and what it keeps.
    pub reason: String,
    /// For an existing-pool suggestion, the pool.
    pub pool: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskSummary {
    /// The path the configuration refers to the disk by.
    pub path: PathBuf,
    pub devnode: PathBuf,
    pub title: String,
    pub subtitle: String,
    pub installer_medium: bool,
    pub segments: Vec<Segment>,
    /// What was found, one line: "Windows on sda3 · EFI 100 MiB · 8 GiB free".
    pub findings: String,
    /// ZFS pools found on the disk, by label.
    pub pools: Vec<String>,
    pub suggestion: Option<Suggestion>,
}

/// One line per installation mode saying what it would do on this machine,
/// and whether it applies at all.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeHint {
    pub mode: InstallationMode,
    pub hint: String,
    pub available: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiskOverview {
    pub disks: Vec<DiskSummary>,
    pub hints: Vec<ModeHint>,
    /// Index into `disks` of the suggestion to lead with.
    pub recommended: Option<usize>,
}

struct Facts<'a> {
    esp: Option<&'a DeviceChoice>,
    msr_or_winre: bool,
    ntfs: Vec<&'a DeviceChoice>,
    linux: Vec<&'a DeviceChoice>,
    pools: Vec<(String, &'a DeviceChoice)>,
    swap: Vec<&'a DeviceChoice>,
    free_bytes: u64,
}

impl Facts<'_> {
    fn windows(&self) -> Option<&DeviceChoice> {
        if !self.msr_or_winre {
            return None;
        }
        self.ntfs
            .iter()
            .copied()
            .max_by_key(|p| p.size_bytes.unwrap_or(0))
    }
    fn largest_system(&self) -> Option<&DeviceChoice> {
        self.windows().or_else(|| {
            self.linux
                .iter()
                .copied()
                .max_by_key(|p| p.size_bytes.unwrap_or(0))
        })
    }
}

pub fn build(disks: &[DeviceChoice], partitions: &[DeviceChoice]) -> DiskOverview {
    let summaries: Vec<DiskSummary> = disks
        .iter()
        .map(|disk| {
            let parts: Vec<&DeviceChoice> = partitions
                .iter()
                .filter(|p| {
                    p.parent_path.as_deref() == Some(disk.path.as_path())
                        || p.parent_path
                            .as_deref()
                            .is_some_and(|parent| same_disk(parent, &disk.path, disk))
                })
                .collect();
            summarize(disk, &parts)
        })
        .collect();
    let recommended = ["alongside", "existing", "erase"].iter().find_map(|rank| {
        summaries.iter().position(|d| {
            d.suggestion
                .as_ref()
                .is_some_and(|s| suggestion_rank(s) == *rank)
        })
    });
    let hints = mode_hints(&summaries);
    DiskOverview {
        disks: summaries,
        hints,
        recommended,
    }
}

fn suggestion_rank(s: &Suggestion) -> &'static str {
    match s.mode {
        InstallationMode::Alongside => "alongside",
        InstallationMode::ExistingPool | InstallationMode::NewPool => "existing",
        InstallationMode::FullDisk => "erase",
    }
}

/// Partitions carry the parent's devnode; disks are listed by their
/// preferred (by-id) path, so compare by devnode when the paths differ.
fn same_disk(parent: &Path, disk_path: &Path, disk: &DeviceChoice) -> bool {
    parent == disk_path
        || parent.file_name() == disk.label.rsplit('/').next().map(std::ffi::OsStr::new)
}

fn gather<'a>(disk: &DeviceChoice, parts: &[&'a DeviceChoice]) -> Facts<'a> {
    let mut facts = Facts {
        esp: None,
        msr_or_winre: false,
        ntfs: Vec::new(),
        linux: Vec::new(),
        pools: Vec::new(),
        swap: Vec::new(),
        free_bytes: 0,
    };
    for part in parts {
        let fs = part.usage.filesystem.to_ascii_lowercase();
        let kind = part.usage.partition_type.to_ascii_lowercase();
        if kind == ESP_TYPE || (fs == "vfat" && part.usage.label.eq_ignore_ascii_case("EFI")) {
            if facts.esp.is_none() {
                facts.esp = Some(part);
            }
        } else if kind == MSR_TYPE || kind == WINRE_TYPE {
            facts.msr_or_winre = true;
        } else if fs == "ntfs" {
            facts.ntfs.push(part);
        } else if fs == "zfs_member" {
            facts.pools.push((part.usage.label.clone(), part));
        } else if fs == "swap" || kind == SWAP_TYPE {
            facts.swap.push(part);
        } else if matches!(
            fs.as_str(),
            "ext4" | "ext3" | "ext2" | "xfs" | "btrfs" | "f2fs"
        ) {
            facts.linux.push(part);
        }
    }
    facts.free_bytes = gaps(disk, parts).iter().map(|(_, size)| size).sum();
    facts
}

/// Unallocated extents as (start, size), ignoring alignment padding and the
/// space GPT reserves at both ends.
fn gaps(disk: &DeviceChoice, parts: &[&DeviceChoice]) -> Vec<(u64, u64)> {
    let Some(total) = disk.size_bytes else {
        return Vec::new();
    };
    let mut extents: Vec<(u64, u64)> = parts
        .iter()
        .filter_map(|p| Some((p.start_bytes?, p.size_bytes?)))
        .collect();
    if extents.is_empty() {
        return if parts.is_empty() && total > MIN_GAP {
            vec![(MIB, total.saturating_sub(2 * MIB))]
        } else {
            Vec::new()
        };
    }
    extents.sort_unstable();
    let mut result = Vec::new();
    let mut cursor = MIB;
    for (start, size) in extents {
        if start > cursor + MIN_GAP {
            result.push((cursor, start - cursor));
        }
        cursor = cursor.max(start + size);
    }
    let end = total.saturating_sub(MIB);
    if end > cursor + MIN_GAP {
        result.push((cursor, end - cursor));
    }
    result
}

fn summarize(disk: &DeviceChoice, parts: &[&DeviceChoice]) -> DiskSummary {
    let facts = gather(disk, parts);
    let installer_medium = disk.usage.in_use
        && (disk.removable
            || parts.iter().any(|p| {
                p.usage
                    .mountpoints
                    .iter()
                    .any(|m| m.starts_with("/run/archiso"))
            }));
    let name = short_name(&disk.label);
    let title = if disk.model.is_empty() {
        name.clone()
    } else {
        format!("{} · {name}", disk.model)
    };
    let mut subtitle = vec![disk.size.clone()];
    if !disk.transport.is_empty() {
        subtitle.push(disk.transport.to_ascii_uppercase());
    }
    if !disk.media.is_empty() {
        subtitle.push(disk.media.clone());
    }
    let subtitle = subtitle
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");

    let mut findings = Vec::new();
    if installer_medium {
        // Not a target: what is on it does not bear on the choice.
        return DiskSummary {
            path: disk.path.clone(),
            devnode: PathBuf::from(&disk.label),
            title,
            subtitle,
            installer_medium,
            segments: segments(disk, parts, &facts),
            findings: "Installer medium, in use".into(),
            pools: Vec::new(),
            suggestion: None,
        };
    }
    if let Some(win) = facts.windows() {
        findings.push(format!(
            "Windows on {} ({})",
            short_name(&win.label),
            win.size
        ));
    }
    // A disk carved into many data partitions gets a count, not a list.
    let data: Vec<&&DeviceChoice> = facts
        .ntfs
        .iter()
        .filter(|part| facts.windows().is_none_or(|w| w.path != part.path))
        .collect();
    for part in data.iter().take(3) {
        findings.push(format!(
            "NTFS data on {} ({})",
            short_name(&part.label),
            part.size
        ));
    }
    if data.len() > 3 {
        findings.push(format!("{} more NTFS partitions", data.len() - 3));
    }
    for part in facts.linux.iter().take(3) {
        findings.push(format!(
            "Linux {} on {} ({})",
            part.usage.filesystem,
            short_name(&part.label),
            part.size
        ));
    }
    if facts.linux.len() > 3 {
        findings.push(format!("{} more Linux partitions", facts.linux.len() - 3));
    }
    for (pool, part) in &facts.pools {
        findings.push(format!(
            "ZFS pool {pool} on {} ({})",
            short_name(&part.label),
            part.size
        ));
    }
    if let Some(esp) = facts.esp {
        findings.push(format!("EFI {}", esp.size));
    }
    for part in &facts.swap {
        findings.push(format!("swap {}", part.size));
    }
    if facts.free_bytes >= MIN_GAP {
        findings.push(format!("{} free", format_size(facts.free_bytes)));
    }
    if findings.is_empty() {
        findings.push(if parts.is_empty() {
            "No partitions".to_string()
        } else {
            "No operating system found".to_string()
        });
    }

    let suggestion = suggest(&facts, parts);
    DiskSummary {
        path: disk.path.clone(),
        devnode: PathBuf::from(&disk.label),
        title,
        subtitle,
        installer_medium,
        segments: segments(disk, parts, &facts),
        findings: findings.join(" · "),
        pools: facts.pools.iter().map(|(pool, _)| pool.clone()).collect(),
        suggestion,
    }
}

fn suggest(facts: &Facts, parts: &[&DeviceChoice]) -> Option<Suggestion> {
    if let Some(system) = facts.largest_system() {
        let os = if facts.windows().is_some() {
            "Windows"
        } else {
            "Linux"
        };
        let room = facts.free_bytes >= ALONGSIDE_FREE
            || system.size_bytes.unwrap_or(0) >= ALONGSIDE_SHRINKABLE;
        if room && facts.esp.is_some() {
            let how = if facts.free_bytes >= ALONGSIDE_FREE {
                format!(
                    "{} of free space is used; nothing is resized.",
                    format_size(facts.free_bytes)
                )
            } else {
                format!("{os} keeps its files; its partition is shrunk to make room.")
            };
            return Some(Suggestion {
                mode: InstallationMode::Alongside,
                title: format!("Install alongside {os}"),
                reason: how,
                pool: None,
            });
        }
        return None;
    }
    if let Some((pool, _)) = facts.pools.first()
        && facts.esp.is_some()
    {
        return Some(Suggestion {
            mode: InstallationMode::ExistingPool,
            title: format!("Use pool {pool}"),
            reason: "Its datasets stay; the installation becomes a new boot environment.".into(),
            pool: Some(pool.clone()),
        });
    }
    if facts.ntfs.is_empty() {
        return Some(Suggestion {
            mode: InstallationMode::FullDisk,
            title: "Erase this disk".into(),
            reason: if parts.is_empty() {
                "The disk is empty.".into()
            } else {
                "No operating system was found on it.".into()
            },
            pool: None,
        });
    }
    None
}

fn segments(disk: &DeviceChoice, parts: &[&DeviceChoice], facts: &Facts) -> Vec<Segment> {
    let Some(total) = disk.size_bytes.filter(|t| *t > 0) else {
        return Vec::new();
    };
    let mut regions: Vec<(u64, u64, SegmentKind, String)> = parts
        .iter()
        .filter_map(|p| {
            let start = p.start_bytes?;
            let size = p.size_bytes?;
            let fs = p.usage.filesystem.to_ascii_lowercase();
            let kind = if facts.esp.is_some_and(|e| e.path == p.path) {
                SegmentKind::Efi
            } else if fs == "zfs_member" {
                SegmentKind::Zfs
            } else if facts.ntfs.iter().any(|n| n.path == p.path) {
                SegmentKind::Windows
            } else if facts.swap.iter().any(|s| s.path == p.path) {
                SegmentKind::Swap
            } else if facts.linux.iter().any(|l| l.path == p.path) {
                SegmentKind::Linux
            } else {
                SegmentKind::Other
            };
            let name = if !p.usage.label.is_empty() {
                p.usage.label.clone()
            } else if !fs.is_empty() {
                fs.clone()
            } else {
                short_name(&p.label)
            };
            Some((start, size, kind, name))
        })
        .collect();
    for (start, size) in gaps(disk, parts) {
        regions.push((start, size, SegmentKind::Free, "Free".into()));
    }
    regions.sort_by_key(|r| r.0);
    regions
        .into_iter()
        .map(|(start, size, kind, name)| Segment {
            offset: start as f32 / total as f32,
            fraction: size as f32 / total as f32,
            kind,
            short_label: format_size(size),
            label: format!("{name} · {}", format_size(size)),
        })
        .collect()
}

fn mode_hints(disks: &[DiskSummary]) -> Vec<ModeHint> {
    let usable: Vec<&DiskSummary> = disks.iter().filter(|d| !d.installer_medium).collect();
    let erasable: Vec<&DiskSummary> = usable
        .iter()
        .copied()
        .filter(|d| {
            d.suggestion
                .as_ref()
                .is_some_and(|s| s.mode == InstallationMode::FullDisk)
        })
        .collect();
    let with_pool: Vec<(&DiskSummary, &str)> = usable
        .iter()
        .copied()
        .flat_map(|d| d.pools.iter().map(move |pool| (d, pool.as_str())))
        .collect();
    let alongside: Vec<&DiskSummary> = usable
        .iter()
        .copied()
        .filter(|d| {
            d.suggestion
                .as_ref()
                .is_some_and(|s| s.mode == InstallationMode::Alongside)
        })
        .collect();
    let names = |list: &[&DiskSummary]| {
        list.iter()
            .map(|d| short_name(&d.devnode.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    vec![
        ModeHint {
            mode: InstallationMode::FullDisk,
            hint: if usable.is_empty() {
                "No disk to install to was found".into()
            } else if erasable.is_empty() {
                "Every disk holds a system or data".into()
            } else {
                format!("Nothing to keep on {}", names(&erasable))
            },
            available: !usable.is_empty(),
        },
        ModeHint {
            mode: InstallationMode::NewPool,
            hint: "Pick an EFI and a ZFS partition yourself".into(),
            available: !usable.is_empty(),
        },
        ModeHint {
            mode: InstallationMode::ExistingPool,
            hint: match with_pool.as_slice() {
                [] => "No ZFS pool was found".into(),
                pools => pools
                    .iter()
                    .map(|(d, pool)| {
                        format!("{pool} on {}", short_name(&d.devnode.to_string_lossy()))
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            },
            available: !with_pool.is_empty(),
        },
        ModeHint {
            mode: InstallationMode::Alongside,
            hint: if alongside.is_empty() {
                "No system with room beside it was found".into()
            } else {
                alongside
                    .iter()
                    .map(|d| {
                        format!(
                            "{} on {}",
                            d.suggestion
                                .as_ref()
                                .map(|s| s
                                    .title
                                    .trim_start_matches("Install alongside ")
                                    .to_string())
                                .unwrap_or_default(),
                            short_name(&d.devnode.to_string_lossy())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            available: !usable.is_empty(),
        },
    ]
}

fn short_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::device::DeviceUsage;

    fn disk(node: &str, size: u64, removable: bool, in_use: bool) -> DeviceChoice {
        DeviceChoice {
            path: format!("/dev/disk/by-id/x-{node}").into(),
            label: format!("/dev/{node}"),
            model: "Model".into(),
            size: format_size(size),
            transport: "sata".into(),
            removable,
            usage: DeviceUsage {
                in_use,
                ..Default::default()
            },
            size_bytes: Some(size),
            ..blank()
        }
    }

    fn part(
        disk: &str,
        number: u32,
        start: u64,
        size: u64,
        fs: &str,
        label: &str,
        kind: &str,
    ) -> DeviceChoice {
        DeviceChoice {
            path: format!("/dev/disk/by-id/x-{disk}-part{number}").into(),
            label: format!("/dev/{disk}{number}"),
            size: format_size(size),
            usage: DeviceUsage {
                filesystem: fs.into(),
                label: label.into(),
                partition_type: kind.into(),
                ..Default::default()
            },
            parent_path: Some(format!("/dev/{disk}").into()),
            start_bytes: Some(start),
            size_bytes: Some(size),
            ..blank()
        }
    }

    fn blank() -> DeviceChoice {
        DeviceChoice {
            path: PathBuf::new(),
            label: String::new(),
            icon: String::new(),
            model: String::new(),
            serial: String::new(),
            size: String::new(),
            transport: String::new(),
            media: String::new(),
            removable: false,
            persistent_path: String::new(),
            persistent_kind: String::new(),
            group_label: String::new(),
            group_model: String::new(),
            group_serial: String::new(),
            group_size: String::new(),
            group_transport: String::new(),
            group_media: String::new(),
            group_removable: false,
            usage: DeviceUsage::default(),
            parent_path: None,
            start_bytes: None,
            size_bytes: None,
        }
    }

    #[test]
    fn a_windows_disk_with_room_suggests_alongside() {
        let disks = vec![disk("sda", 240 * GIB, false, false)];
        let parts = vec![
            part("sda", 1, MIB, 100 * MIB, "vfat", "", ESP_TYPE),
            part("sda", 2, 101 * MIB, 16 * MIB, "", "", MSR_TYPE),
            part("sda", 3, 117 * MIB, 130 * GIB, "ntfs", "", ""),
            part(
                "sda",
                4,
                117 * MIB + 130 * GIB,
                546 * MIB,
                "ntfs",
                "",
                WINRE_TYPE,
            ),
        ];
        let overview = build(&disks, &parts);
        let sda = &overview.disks[0];
        assert!(
            sda.findings.starts_with("Windows on sda3"),
            "{}",
            sda.findings
        );
        assert!(sda.findings.ends_with("free"), "{}", sda.findings);
        let suggestion = sda.suggestion.as_ref().unwrap();
        assert_eq!(suggestion.mode, InstallationMode::Alongside);
        assert_eq!(suggestion.title, "Install alongside Windows");
        assert_eq!(overview.recommended, Some(0));
        let kinds: Vec<SegmentKind> = sda.segments.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SegmentKind::Efi,
                SegmentKind::Other,
                SegmentKind::Windows,
                SegmentKind::Other,
                SegmentKind::Free
            ]
        );
    }

    #[test]
    fn a_pool_with_an_esp_suggests_the_existing_pool() {
        let disks = vec![disk("nvme0n1", 500 * GIB, false, false)];
        let parts = vec![
            part("nvme0n1", 1, MIB, GIB, "vfat", "EFI", ESP_TYPE),
            part(
                "nvme0n1",
                2,
                GIB + MIB,
                499 * GIB - 2 * MIB,
                "zfs_member",
                "zroot",
                "",
            ),
        ];
        let overview = build(&disks, &parts);
        let suggestion = overview.disks[0].suggestion.as_ref().unwrap();
        assert_eq!(suggestion.mode, InstallationMode::ExistingPool);
        assert_eq!(suggestion.pool.as_deref(), Some("zroot"));
        assert_eq!(overview.hints[2].hint, "zroot on nvme0n1");
        assert!(overview.hints[2].available);
    }

    #[test]
    fn an_empty_disk_suggests_erasing_and_the_installer_medium_nothing() {
        let disks = vec![
            disk("sdb", 115 * GIB, true, true),
            disk("sdc", 1000 * GIB, false, false),
        ];
        let overview = build(&disks, &[]);
        assert!(overview.disks[0].installer_medium);
        assert!(overview.disks[0].suggestion.is_none());
        assert_eq!(overview.disks[0].findings, "Installer medium, in use");
        let sdc = &overview.disks[1];
        assert_eq!(
            sdc.suggestion.as_ref().unwrap().mode,
            InstallationMode::FullDisk
        );
        assert_eq!(sdc.findings, "1000 GiB free");
        assert_eq!(sdc.segments.len(), 1);
        assert_eq!(sdc.segments[0].kind, SegmentKind::Free);
        assert_eq!(overview.recommended, Some(1));
        assert_eq!(overview.hints[0].hint, "Nothing to keep on sdc");
        assert!(!overview.hints[2].available);
    }

    #[test]
    fn a_full_linux_disk_gets_no_suggestion() {
        let disks = vec![disk("sda", 40 * GIB, false, false)];
        let parts = vec![
            part("sda", 1, MIB, 512 * MIB, "vfat", "", ESP_TYPE),
            part("sda", 2, 513 * MIB, 39 * GIB, "ext4", "", ""),
        ];
        let overview = build(&disks, &parts);
        assert!(overview.disks[0].suggestion.is_none());
        assert!(overview.disks[0].findings.starts_with("Linux ext4 on sda2"));
        assert_eq!(overview.recommended, None);
        assert_eq!(
            overview.hints[3].hint,
            "No system with room beside it was found"
        );
    }
}
