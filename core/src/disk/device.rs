use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{Result, WrapErr};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePathKind {
    ById,
    ByPath,
    DevNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePath {
    pub path: PathBuf,
    pub kind: DevicePathKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    pub devnode: PathBuf,
    pub aliases: Vec<DevicePath>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size_bytes: Option<u64>,
    pub transport: Option<String>,
    pub rotational: Option<bool>,
    pub removable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPartition {
    pub devnode: PathBuf,
    pub aliases: Vec<DevicePath>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceChoice {
    pub path: PathBuf,
    pub label: String,
}

impl BlockDevice {
    pub fn preferred_path(&self) -> DevicePath {
        preferred_path_for(&self.aliases, &self.devnode)
    }

    pub fn selection_label(&self) -> String {
        let mut parts = vec![self.devnode.display().to_string()];

        if let Some(model) = self.model.as_deref() {
            parts.push(model.to_string());
        }
        if let Some(size) = self.size_bytes {
            parts.push(format_size(size));
        }
        if let Some(transport) = self.transport.as_deref() {
            parts.push(transport.to_string());
        }
        if self.removable {
            parts.push("removable".to_string());
        }

        let preferred = self.preferred_path();
        if preferred.path != self.devnode {
            parts.push(format!("using {}", preferred.path.display()));
        }

        parts.join(" | ")
    }
}

impl BlockPartition {
    pub fn preferred_path(&self) -> DevicePath {
        preferred_path_for(&self.aliases, &self.devnode)
    }

    pub fn selection_label(&self) -> String {
        let mut parts = vec![self.devnode.display().to_string()];

        if let Some(size) = self.size_bytes {
            parts.push(format_size(size));
        }

        let preferred = self.preferred_path();
        if preferred.path != self.devnode {
            parts.push(format!("using {}", preferred.path.display()));
        }

        parts.join(" | ")
    }
}

pub fn disk_choices() -> Result<Vec<DeviceChoice>> {
    Ok(list_block_devices()?
        .into_iter()
        .map(|device| DeviceChoice {
            path: device.preferred_path().path,
            label: device.selection_label(),
        })
        .collect())
}

pub fn partition_choices() -> Result<Vec<DeviceChoice>> {
    Ok(list_block_partitions()?
        .into_iter()
        .map(|partition| DeviceChoice {
            path: partition.preferred_path().path,
            label: partition.selection_label(),
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct LsblkOutput {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Debug, Deserialize)]
struct LsblkDevice {
    path: Option<PathBuf>,
    #[serde(rename = "type")]
    device_type: Option<String>,
    size: Option<Value>,
    model: Option<String>,
    serial: Option<String>,
    tran: Option<String>,
    rota: Option<Value>,
    rm: Option<Value>,
    #[serde(default)]
    children: Vec<LsblkDevice>,
}

pub fn list_block_devices() -> Result<Vec<BlockDevice>> {
    let (parsed, aliases) = inspect_block_devices()?;

    let mut devices = Vec::new();
    for node in parsed.blockdevices {
        collect_lsblk_disks(node, &aliases, &mut devices);
    }

    devices.sort_by(|a, b| a.devnode.cmp(&b.devnode));
    Ok(devices)
}

pub fn list_block_partitions() -> Result<Vec<BlockPartition>> {
    let (parsed, aliases) = inspect_block_devices()?;

    let mut partitions = Vec::new();
    for node in parsed.blockdevices {
        collect_lsblk_partitions(node, &aliases, &mut partitions);
    }

    partitions.sort_by(|a, b| a.devnode.cmp(&b.devnode));
    Ok(partitions)
}

fn inspect_block_devices() -> Result<(LsblkOutput, HashMap<PathBuf, Vec<DevicePath>>)> {
    let output = Command::new("lsblk")
        .args([
            "--json",
            "--bytes",
            "--output",
            "PATH,TYPE,SIZE,MODEL,SERIAL,TRAN,ROTA,RM",
        ])
        .output()
        .wrap_err("failed to run lsblk")?;

    if !output.status.success() {
        return Err(color_eyre::eyre::eyre!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let parsed: LsblkOutput =
        serde_json::from_slice(&output.stdout).wrap_err("failed to parse lsblk JSON")?;
    let aliases = collect_device_aliases()?;
    Ok((parsed, aliases))
}

fn collect_lsblk_disks(
    node: LsblkDevice,
    aliases: &HashMap<PathBuf, Vec<DevicePath>>,
    devices: &mut Vec<BlockDevice>,
) {
    if node.device_type.as_deref() == Some("disk")
        && let Some(devnode) = node.path.clone()
        && is_installable_devnode(&devnode)
    {
        let canonical = fs::canonicalize(&devnode).unwrap_or_else(|_| devnode.clone());
        let mut device_aliases = aliases.get(&canonical).cloned().unwrap_or_default();
        device_aliases.sort_by_key(alias_preference_key);

        devices.push(BlockDevice {
            devnode,
            aliases: device_aliases,
            model: clean_string(node.model),
            serial: clean_string(node.serial),
            size_bytes: node.size.as_ref().and_then(value_as_u64),
            transport: clean_string(node.tran),
            rotational: node.rota.as_ref().and_then(value_as_bool),
            removable: node.rm.as_ref().and_then(value_as_bool).unwrap_or(false),
        });
    }

    for child in node.children {
        collect_lsblk_disks(child, aliases, devices);
    }
}

fn collect_lsblk_partitions(
    node: LsblkDevice,
    aliases: &HashMap<PathBuf, Vec<DevicePath>>,
    partitions: &mut Vec<BlockPartition>,
) {
    if node.device_type.as_deref() == Some("part")
        && let Some(devnode) = node.path.clone()
    {
        let canonical = fs::canonicalize(&devnode).unwrap_or_else(|_| devnode.clone());
        let mut partition_aliases = aliases.get(&canonical).cloned().unwrap_or_default();
        partition_aliases.sort_by_key(alias_preference_key);

        partitions.push(BlockPartition {
            devnode,
            aliases: partition_aliases,
            size_bytes: node.size.as_ref().and_then(value_as_u64),
        });
    }

    for child in node.children {
        collect_lsblk_partitions(child, aliases, partitions);
    }
}

fn is_installable_devnode(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    !(name.starts_with("loop") || name.starts_with("zram") || name.starts_with("sr"))
}

fn collect_device_aliases() -> Result<HashMap<PathBuf, Vec<DevicePath>>> {
    let mut aliases: HashMap<PathBuf, Vec<DevicePath>> = HashMap::new();
    collect_alias_dir(
        Path::new("/dev/disk/by-id"),
        DevicePathKind::ById,
        &mut aliases,
    )?;
    collect_alias_dir(
        Path::new("/dev/disk/by-path"),
        DevicePathKind::ByPath,
        &mut aliases,
    )?;
    Ok(aliases)
}

fn collect_alias_dir(
    dir: &Path,
    kind: DevicePathKind,
    aliases: &mut HashMap<PathBuf, Vec<DevicePath>>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).wrap_err_with(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let Ok(target) = fs::canonicalize(&path) else {
            continue;
        };

        aliases
            .entry(target)
            .or_default()
            .push(DevicePath { path, kind });
    }

    Ok(())
}

fn alias_preference_key(alias: &DevicePath) -> (u8, String) {
    let name = alias
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    let rank = match alias.kind {
        DevicePathKind::ById => by_id_rank(name),
        DevicePathKind::ByPath => 20,
        DevicePathKind::DevNode => 30,
    };

    (rank, alias.path.display().to_string())
}

fn preferred_path_for(aliases: &[DevicePath], devnode: &Path) -> DevicePath {
    aliases
        .iter()
        .filter(|alias| alias.kind == DevicePathKind::ById)
        .min_by_key(|alias| alias_preference_key(alias))
        .or_else(|| {
            aliases
                .iter()
                .filter(|alias| alias.kind == DevicePathKind::ByPath)
                .min_by_key(|alias| alias_preference_key(alias))
        })
        .cloned()
        .unwrap_or_else(|| DevicePath {
            path: devnode.to_path_buf(),
            kind: DevicePathKind::DevNode,
        })
}

fn by_id_rank(name: &str) -> u8 {
    if name.starts_with("wwn-") {
        0
    } else if name.starts_with("nvme-eui.") || name.starts_with("nvme-uuid.") {
        1
    } else if name.starts_with("ata-") || name.starts_with("nvme-") {
        2
    } else if name.starts_with("scsi-") {
        3
    } else if name.starts_with("virtio-") {
        4
    } else {
        5
    }
}

fn clean_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn value_as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(n) => n.as_u64().map(|value| value != 0),
        Value::String(s) => match s.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_by_id_over_by_path_and_devnode() {
        let device = BlockDevice {
            devnode: PathBuf::from("/dev/vda"),
            aliases: vec![
                DevicePath {
                    path: PathBuf::from("/dev/disk/by-path/pci-0000:00:04.0"),
                    kind: DevicePathKind::ByPath,
                },
                DevicePath {
                    path: PathBuf::from("/dev/disk/by-id/virtio-test-disk"),
                    kind: DevicePathKind::ById,
                },
            ],
            model: None,
            serial: None,
            size_bytes: None,
            transport: None,
            rotational: None,
            removable: false,
        };

        assert_eq!(
            device.preferred_path().path,
            PathBuf::from("/dev/disk/by-id/virtio-test-disk")
        );
    }

    #[test]
    fn falls_back_to_by_path_before_devnode() {
        let device = BlockDevice {
            devnode: PathBuf::from("/dev/vda"),
            aliases: vec![DevicePath {
                path: PathBuf::from("/dev/disk/by-path/pci-0000:00:04.0"),
                kind: DevicePathKind::ByPath,
            }],
            model: None,
            serial: None,
            size_bytes: None,
            transport: None,
            rotational: None,
            removable: false,
        };

        assert_eq!(
            device.preferred_path().path,
            PathBuf::from("/dev/disk/by-path/pci-0000:00:04.0")
        );
    }

    #[test]
    fn falls_back_to_devnode_without_aliases() {
        let device = BlockDevice {
            devnode: PathBuf::from("/dev/vda"),
            aliases: Vec::new(),
            model: None,
            serial: None,
            size_bytes: None,
            transport: None,
            rotational: None,
            removable: false,
        };

        assert_eq!(device.preferred_path().path, PathBuf::from("/dev/vda"));
    }

    #[test]
    fn selection_label_includes_identity_and_preferred_path() {
        let device = BlockDevice {
            devnode: PathBuf::from("/dev/vda"),
            aliases: vec![DevicePath {
                path: PathBuf::from("/dev/disk/by-path/pci-0000:00:04.0"),
                kind: DevicePathKind::ByPath,
            }],
            model: Some("VirtIO Block Device".to_string()),
            serial: None,
            size_bytes: Some(64 * 1024 * 1024 * 1024),
            transport: Some("virtio".to_string()),
            rotational: Some(false),
            removable: false,
        };

        assert_eq!(
            device.selection_label(),
            "/dev/vda | VirtIO Block Device | 64 GiB | virtio | using /dev/disk/by-path/pci-0000:00:04.0"
        );
    }

    #[test]
    fn partition_selection_prefers_persistent_aliases() {
        let partition = BlockPartition {
            devnode: PathBuf::from("/dev/vda1"),
            aliases: vec![
                DevicePath {
                    path: PathBuf::from("/dev/disk/by-path/pci-0000:00:04.0-part1"),
                    kind: DevicePathKind::ByPath,
                },
                DevicePath {
                    path: PathBuf::from("/dev/disk/by-id/virtio-test-disk-part1"),
                    kind: DevicePathKind::ById,
                },
            ],
            size_bytes: Some(512 * 1024 * 1024),
        };

        assert_eq!(
            partition.preferred_path().path,
            PathBuf::from("/dev/disk/by-id/virtio-test-disk-part1")
        );
        assert_eq!(
            partition.selection_label(),
            "/dev/vda1 | 512 MiB | using /dev/disk/by-id/virtio-test-disk-part1"
        );
    }
}
