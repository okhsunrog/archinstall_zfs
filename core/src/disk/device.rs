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
    pub parent_devnode: Option<PathBuf>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size_bytes: Option<u64>,
    pub parent_size_bytes: Option<u64>,
    pub transport: Option<String>,
    pub rotational: Option<bool>,
    pub removable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceChoice {
    pub path: PathBuf,
    pub label: String,
    pub icon: String,
    pub model: String,
    pub serial: String,
    pub size: String,
    pub transport: String,
    pub media: String,
    pub removable: bool,
    pub persistent_path: String,
    pub persistent_kind: String,
    pub group_label: String,
    pub group_model: String,
    pub group_serial: String,
    pub group_size: String,
    pub group_transport: String,
    pub group_media: String,
    pub group_removable: bool,
}

impl DeviceChoice {
    pub fn detail_summary(&self) -> String {
        let mut parts = Vec::new();

        if !self.model.is_empty() {
            parts.push(self.model.clone());
        }
        if !self.serial.is_empty() {
            parts.push(format!("SN {}", self.serial));
        }
        if !self.size.is_empty() {
            parts.push(self.size.clone());
        }
        if !self.media.is_empty() {
            parts.push(self.media.clone());
        }
        if !self.transport.is_empty() {
            parts.push(self.transport.clone());
        }
        if self.removable {
            parts.push("removable".to_string());
        }
        if !self.persistent_path.is_empty() {
            parts.push(format!("using {}", self.persistent_path));
        }

        parts.join(" | ")
    }

    pub fn display_label(&self) -> String {
        let detail_summary = self.detail_summary();
        if detail_summary.is_empty() {
            self.label.clone()
        } else {
            format!("{} | {}", self.label, detail_summary)
        }
    }
}

impl From<BlockDevice> for DeviceChoice {
    fn from(device: BlockDevice) -> Self {
        Self {
            path: device.preferred_path().path,
            label: device.selection_title(),
            icon: device.selection_icon().to_string(),
            model: device.selection_model(),
            serial: device.selection_serial(),
            size: device.selection_size(),
            transport: device.selection_transport(),
            media: device.selection_media(),
            removable: device.removable,
            persistent_path: device.selection_persistent_path(),
            persistent_kind: device.selection_persistent_kind(),
            group_label: String::new(),
            group_model: String::new(),
            group_serial: String::new(),
            group_size: String::new(),
            group_transport: String::new(),
            group_media: String::new(),
            group_removable: false,
        }
    }
}

impl From<BlockPartition> for DeviceChoice {
    fn from(partition: BlockPartition) -> Self {
        Self {
            path: partition.preferred_path().path,
            label: partition.selection_title(),
            icon: partition.selection_icon().to_string(),
            model: partition.selection_model(),
            serial: partition.selection_serial(),
            size: partition.selection_size(),
            transport: partition.selection_transport(),
            media: partition.selection_media(),
            removable: partition.removable,
            persistent_path: partition.selection_persistent_path(),
            persistent_kind: partition.selection_persistent_kind(),
            group_label: partition.selection_group_label(),
            group_model: partition.selection_group_model(),
            group_serial: partition.selection_group_serial(),
            group_size: partition.selection_group_size(),
            group_transport: partition.selection_group_transport(),
            group_media: partition.selection_group_media(),
            group_removable: partition.removable,
        }
    }
}

impl BlockDevice {
    pub fn preferred_path(&self) -> DevicePath {
        preferred_path_for(&self.aliases, &self.devnode)
    }

    pub fn selection_title(&self) -> String {
        self.devnode.display().to_string()
    }

    pub fn selection_model(&self) -> String {
        self.model.clone().unwrap_or_default()
    }

    pub fn selection_serial(&self) -> String {
        self.serial.clone().unwrap_or_default()
    }

    pub fn selection_size(&self) -> String {
        self.size_bytes.map(format_size).unwrap_or_default()
    }

    pub fn selection_transport(&self) -> String {
        self.transport.clone().unwrap_or_default()
    }

    pub fn selection_media(&self) -> String {
        media_label(self.rotational)
    }

    pub fn selection_icon(&self) -> &'static str {
        if self.removable
            || self
                .transport
                .as_deref()
                .is_some_and(|transport| transport.eq_ignore_ascii_case("usb"))
        {
            "usb"
        } else {
            "hard-drive"
        }
    }

    pub fn selection_persistent_path(&self) -> String {
        persistent_path_label(&self.preferred_path(), &self.devnode)
    }

    pub fn selection_persistent_kind(&self) -> String {
        persistent_kind_label(&self.preferred_path(), &self.devnode)
    }
}

impl BlockPartition {
    pub fn preferred_path(&self) -> DevicePath {
        preferred_path_for(&self.aliases, &self.devnode)
    }

    pub fn selection_title(&self) -> String {
        self.devnode.display().to_string()
    }

    pub fn selection_group_label(&self) -> String {
        self.parent_devnode
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    }

    pub fn selection_group_model(&self) -> String {
        self.selection_model()
    }

    pub fn selection_group_serial(&self) -> String {
        self.selection_serial()
    }

    pub fn selection_group_size(&self) -> String {
        self.parent_size_bytes.map(format_size).unwrap_or_default()
    }

    pub fn selection_group_transport(&self) -> String {
        self.selection_transport()
    }

    pub fn selection_group_media(&self) -> String {
        self.selection_media()
    }

    pub fn selection_model(&self) -> String {
        self.model.clone().unwrap_or_default()
    }

    pub fn selection_serial(&self) -> String {
        self.serial.clone().unwrap_or_default()
    }

    pub fn selection_size(&self) -> String {
        self.size_bytes.map(format_size).unwrap_or_default()
    }

    pub fn selection_transport(&self) -> String {
        self.transport.clone().unwrap_or_default()
    }

    pub fn selection_media(&self) -> String {
        media_label(self.rotational)
    }

    pub fn selection_icon(&self) -> &'static str {
        "hard-drive"
    }

    pub fn selection_persistent_path(&self) -> String {
        persistent_path_label(&self.preferred_path(), &self.devnode)
    }

    pub fn selection_persistent_kind(&self) -> String {
        persistent_kind_label(&self.preferred_path(), &self.devnode)
    }
}

pub fn disk_choices() -> Result<Vec<DeviceChoice>> {
    Ok(list_block_devices()?
        .into_iter()
        .map(DeviceChoice::from)
        .collect())
}

pub fn partition_choices() -> Result<Vec<DeviceChoice>> {
    Ok(list_block_partitions()?
        .into_iter()
        .map(DeviceChoice::from)
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

#[derive(Debug, Clone, Default)]
struct ParentDiskDetails {
    devnode: PathBuf,
    model: Option<String>,
    serial: Option<String>,
    size_bytes: Option<u64>,
    transport: Option<String>,
    rotational: Option<bool>,
    removable: bool,
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

    let parent_details_by_devnode = collect_parent_disk_details(&parsed.blockdevices);
    let mut partitions = Vec::new();
    for node in parsed.blockdevices {
        collect_lsblk_partitions(
            node,
            &aliases,
            &parent_details_by_devnode,
            &mut partitions,
            None,
        );
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
    parent_details_by_devnode: &HashMap<PathBuf, ParentDiskDetails>,
    partitions: &mut Vec<BlockPartition>,
    parent_details: Option<ParentDiskDetails>,
) {
    let current_details = if node.device_type.as_deref() == Some("disk") {
        node.path.clone().map(|devnode| ParentDiskDetails {
            devnode,
            model: clean_string(node.model.clone()),
            serial: clean_string(node.serial.clone()),
            size_bytes: node.size.as_ref().and_then(value_as_u64),
            transport: clean_string(node.tran.clone()),
            rotational: node.rota.as_ref().and_then(value_as_bool),
            removable: node.rm.as_ref().and_then(value_as_bool).unwrap_or(false),
        })
    } else {
        parent_details.clone()
    };

    if node.device_type.as_deref() == Some("part")
        && let Some(devnode) = node.path.clone()
    {
        let canonical = fs::canonicalize(&devnode).unwrap_or_else(|_| devnode.clone());
        let mut partition_aliases = aliases.get(&canonical).cloned().unwrap_or_default();
        partition_aliases.sort_by_key(alias_preference_key);
        let flat_parent_details = parent_devnode_for_partition(&devnode)
            .and_then(|parent| parent_details_by_devnode.get(&parent));
        let details = current_details.as_ref().or(flat_parent_details);

        partitions.push(BlockPartition {
            devnode,
            aliases: partition_aliases,
            parent_devnode: details.map(|details| details.devnode.clone()),
            model: details.and_then(|details| details.model.clone()),
            serial: details.and_then(|details| details.serial.clone()),
            size_bytes: node.size.as_ref().and_then(value_as_u64),
            parent_size_bytes: details.and_then(|details| details.size_bytes),
            transport: details.and_then(|details| details.transport.clone()),
            rotational: details.and_then(|details| details.rotational),
            removable: details.is_some_and(|details| details.removable),
        });
    }

    for child in node.children {
        collect_lsblk_partitions(
            child,
            aliases,
            parent_details_by_devnode,
            partitions,
            current_details.clone(),
        );
    }
}

fn collect_parent_disk_details(nodes: &[LsblkDevice]) -> HashMap<PathBuf, ParentDiskDetails> {
    let mut details = HashMap::new();
    for node in nodes {
        collect_parent_disk_details_inner(node, &mut details);
    }
    details
}

fn collect_parent_disk_details_inner(
    node: &LsblkDevice,
    details: &mut HashMap<PathBuf, ParentDiskDetails>,
) {
    if node.device_type.as_deref() == Some("disk")
        && let Some(devnode) = node.path.clone()
    {
        details.insert(
            devnode.clone(),
            ParentDiskDetails {
                devnode,
                model: clean_string(node.model.clone()),
                serial: clean_string(node.serial.clone()),
                size_bytes: node.size.as_ref().and_then(value_as_u64),
                transport: clean_string(node.tran.clone()),
                rotational: node.rota.as_ref().and_then(value_as_bool),
                removable: node.rm.as_ref().and_then(value_as_bool).unwrap_or(false),
            },
        );
    }

    for child in &node.children {
        collect_parent_disk_details_inner(child, details);
    }
}

fn is_installable_devnode(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    !(name.starts_with("loop") || name.starts_with("zram") || name.starts_with("sr"))
}

fn parent_devnode_for_partition(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let parent_name = strip_partition_suffix(name);
    if parent_name == name || parent_name.is_empty() {
        return None;
    }

    Some(match path.parent() {
        Some(parent) => parent.join(parent_name),
        None => PathBuf::from(parent_name),
    })
}

fn strip_partition_suffix(name: &str) -> &str {
    if let Some(pos) = name.rfind('p') {
        let after = &name[pos + 1..];
        if !after.is_empty() && after.bytes().all(|b| b.is_ascii_digit()) {
            return &name[..pos];
        }
    }

    name.trim_end_matches(|c: char| c.is_ascii_digit())
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

fn persistent_path_label(preferred: &DevicePath, devnode: &Path) -> String {
    if preferred.path == devnode {
        String::new()
    } else {
        preferred.path.display().to_string()
    }
}

fn persistent_kind_label(preferred: &DevicePath, devnode: &Path) -> String {
    if preferred.path == devnode {
        return String::new();
    }

    match preferred.kind {
        DevicePathKind::ById => "by-id",
        DevicePathKind::ByPath => "by-path",
        DevicePathKind::DevNode => "",
    }
    .to_string()
}

fn media_label(rotational: Option<bool>) -> String {
    match rotational {
        Some(true) => "HDD".to_string(),
        Some(false) => "SSD".to_string(),
        None => String::new(),
    }
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
    fn device_choice_includes_identity_and_preferred_path() {
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

        assert_eq!(device.selection_title(), "/dev/vda");
        assert_eq!(device.selection_icon(), "hard-drive");
        assert_eq!(device.selection_model(), "VirtIO Block Device");
        assert_eq!(device.selection_serial(), "");
        assert_eq!(device.selection_size(), "64 GiB");
        assert_eq!(device.selection_transport(), "virtio");
        assert_eq!(device.selection_media(), "SSD");
        assert_eq!(
            device.selection_persistent_path(),
            "/dev/disk/by-path/pci-0000:00:04.0"
        );
        assert_eq!(device.selection_persistent_kind(), "by-path");

        let choice = DeviceChoice::from(device);
        assert_eq!(
            choice.detail_summary(),
            "VirtIO Block Device | 64 GiB | SSD | virtio | using /dev/disk/by-path/pci-0000:00:04.0"
        );
        assert_eq!(
            choice.display_label(),
            "/dev/vda | VirtIO Block Device | 64 GiB | SSD | virtio | using /dev/disk/by-path/pci-0000:00:04.0"
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
            parent_devnode: Some(PathBuf::from("/dev/vda")),
            model: Some("VirtIO Block Device".to_string()),
            serial: Some("test-serial".to_string()),
            size_bytes: Some(512 * 1024 * 1024),
            parent_size_bytes: Some(64 * 1024 * 1024 * 1024),
            transport: Some("virtio".to_string()),
            rotational: Some(false),
            removable: false,
        };

        assert_eq!(
            partition.preferred_path().path,
            PathBuf::from("/dev/disk/by-id/virtio-test-disk-part1")
        );
        assert_eq!(partition.selection_title(), "/dev/vda1");
        assert_eq!(partition.selection_group_label(), "/dev/vda");
        assert_eq!(partition.selection_group_model(), "VirtIO Block Device");
        assert_eq!(partition.selection_group_serial(), "test-serial");
        assert_eq!(partition.selection_group_size(), "64 GiB");
        assert_eq!(partition.selection_group_transport(), "virtio");
        assert_eq!(partition.selection_group_media(), "SSD");
        assert_eq!(partition.selection_icon(), "hard-drive");
        assert_eq!(partition.selection_model(), "VirtIO Block Device");
        assert_eq!(partition.selection_serial(), "test-serial");
        assert_eq!(partition.selection_size(), "512 MiB");
        assert_eq!(partition.selection_transport(), "virtio");
        assert_eq!(partition.selection_media(), "SSD");
        assert_eq!(
            partition.selection_persistent_path(),
            "/dev/disk/by-id/virtio-test-disk-part1"
        );
        assert_eq!(partition.selection_persistent_kind(), "by-id");

        let choice = DeviceChoice::from(partition);
        assert_eq!(
            choice.detail_summary(),
            "VirtIO Block Device | SN test-serial | 512 MiB | SSD | virtio | using /dev/disk/by-id/virtio-test-disk-part1"
        );
        assert_eq!(
            choice.display_label(),
            "/dev/vda1 | VirtIO Block Device | SN test-serial | 512 MiB | SSD | virtio | using /dev/disk/by-id/virtio-test-disk-part1"
        );
        assert_eq!(choice.group_label, "/dev/vda");
        assert_eq!(choice.group_size, "64 GiB");
    }

    #[test]
    fn removable_or_usb_disks_use_usb_icon() {
        let device = BlockDevice {
            devnode: PathBuf::from("/dev/sdb"),
            aliases: Vec::new(),
            model: Some("USB Flash Drive".to_string()),
            serial: None,
            size_bytes: Some(16 * 1024 * 1024 * 1024),
            transport: Some("usb".to_string()),
            rotational: Some(false),
            removable: false,
        };

        assert_eq!(device.selection_icon(), "usb");
    }

    #[test]
    fn partition_collection_inherits_parent_disk_details() {
        let root = LsblkDevice {
            path: Some(PathBuf::from("/dev/sda")),
            device_type: Some("disk".to_string()),
            size: None,
            model: Some("Samsung SSD".to_string()),
            serial: Some("S123".to_string()),
            tran: Some("sata".to_string()),
            rota: Some(Value::Number(0.into())),
            rm: Some(Value::Number(1.into())),
            children: vec![LsblkDevice {
                path: Some(PathBuf::from("/dev/sda1")),
                device_type: Some("part".to_string()),
                size: Some(Value::Number((1024 * 1024 * 1024).into())),
                model: None,
                serial: None,
                tran: None,
                rota: None,
                rm: None,
                children: Vec::new(),
            }],
        };

        let aliases = HashMap::new();
        let parent_details_by_devnode = collect_parent_disk_details(std::slice::from_ref(&root));
        let mut partitions = Vec::new();
        collect_lsblk_partitions(
            root,
            &aliases,
            &parent_details_by_devnode,
            &mut partitions,
            None,
        );

        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].selection_model(), "Samsung SSD");
        assert_eq!(partitions[0].selection_serial(), "S123");
        assert_eq!(partitions[0].selection_group_label(), "/dev/sda");
        assert_eq!(partitions[0].selection_group_size(), "");
        assert_eq!(partitions[0].selection_transport(), "sata");
        assert_eq!(partitions[0].selection_media(), "SSD");
        assert!(partitions[0].removable);
    }

    #[test]
    fn flat_partition_collection_looks_up_parent_disk_details() {
        let disk = LsblkDevice {
            path: Some(PathBuf::from("/dev/nvme0n1")),
            device_type: Some("disk".to_string()),
            size: None,
            model: Some("Samsung SSD".to_string()),
            serial: Some("S123".to_string()),
            tran: Some("nvme".to_string()),
            rota: Some(Value::Bool(false)),
            rm: Some(Value::Bool(false)),
            children: Vec::new(),
        };
        let partition = LsblkDevice {
            path: Some(PathBuf::from("/dev/nvme0n1p1")),
            device_type: Some("part".to_string()),
            size: Some(Value::Number((1024 * 1024 * 1024).into())),
            model: None,
            serial: None,
            tran: Some("nvme".to_string()),
            rota: None,
            rm: None,
            children: Vec::new(),
        };

        let aliases = HashMap::new();
        let parent_details_by_devnode = collect_parent_disk_details(std::slice::from_ref(&disk));
        let mut partitions = Vec::new();
        collect_lsblk_partitions(
            partition,
            &aliases,
            &parent_details_by_devnode,
            &mut partitions,
            None,
        );

        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].selection_model(), "Samsung SSD");
        assert_eq!(partitions[0].selection_serial(), "S123");
        assert_eq!(partitions[0].selection_group_label(), "/dev/nvme0n1");
        assert_eq!(partitions[0].selection_group_size(), "");
        assert_eq!(partitions[0].selection_transport(), "nvme");
        assert_eq!(partitions[0].selection_media(), "SSD");
        assert!(!partitions[0].removable);
    }

    #[test]
    fn parent_devnode_for_partition_handles_common_names() {
        assert_eq!(
            parent_devnode_for_partition(Path::new("/dev/sda1")),
            Some(PathBuf::from("/dev/sda"))
        );
        assert_eq!(
            parent_devnode_for_partition(Path::new("/dev/vda12")),
            Some(PathBuf::from("/dev/vda"))
        );
        assert_eq!(
            parent_devnode_for_partition(Path::new("/dev/nvme0n1p1")),
            Some(PathBuf::from("/dev/nvme0n1"))
        );
        assert_eq!(
            parent_devnode_for_partition(Path::new("/dev/mmcblk0p2")),
            Some(PathBuf::from("/dev/mmcblk0"))
        );
    }
}
