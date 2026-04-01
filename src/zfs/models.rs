use std::collections::HashMap;

use serde::Deserialize;

// Common output version header present in all ZFS JSON output
#[derive(Debug, Deserialize)]
pub struct OutputVersion {
    pub command: String,
    pub vers_major: u32,
    pub vers_minor: u32,
}

// Source of a property value
#[derive(Debug, Deserialize)]
pub struct PropertySource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub data: String,
}

// A single property with value and source
#[derive(Debug, Deserialize)]
pub struct PropertyValue {
    pub value: String,
    pub source: PropertySource,
}

// --- zpool list -j ---

#[derive(Debug, Deserialize)]
pub struct ZpoolListOutput {
    pub output_version: OutputVersion,
    pub pools: HashMap<String, ZpoolListEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ZpoolListEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub state: String,
    pub pool_guid: String,
    pub txg: String,
    pub spa_version: String,
    pub zpl_version: String,
    pub properties: HashMap<String, PropertyValue>,
}

// --- zpool status -j ---

#[derive(Debug, Deserialize)]
pub struct ZpoolStatusOutput {
    pub output_version: OutputVersion,
    pub pools: HashMap<String, ZpoolStatusEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ZpoolStatusEntry {
    pub name: String,
    pub state: String,
    pub pool_guid: String,
    pub txg: String,
    pub spa_version: String,
    pub zpl_version: String,
    pub vdevs: HashMap<String, VdevEntry>,
    pub error_count: String,
}

#[derive(Debug, Deserialize)]
pub struct VdevEntry {
    pub name: String,
    pub vdev_type: String,
    pub guid: String,
    pub state: String,
    pub read_errors: String,
    pub write_errors: String,
    pub checksum_errors: String,
    #[serde(default)]
    pub vdevs: HashMap<String, VdevEntry>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub alloc_space: Option<String>,
    #[serde(default)]
    pub total_space: Option<String>,
    #[serde(default)]
    pub def_space: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub rep_dev_size: Option<String>,
    #[serde(default)]
    pub phys_space: Option<String>,
    #[serde(default)]
    pub slow_ios: Option<String>,
}

// --- zpool get -j ---

#[derive(Debug, Deserialize)]
pub struct ZpoolGetOutput {
    pub output_version: OutputVersion,
    pub pools: HashMap<String, ZpoolGetEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ZpoolGetEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub state: String,
    pub pool_guid: String,
    pub txg: String,
    pub spa_version: String,
    pub zpl_version: String,
    pub properties: HashMap<String, PropertyValue>,
}

// --- zfs list -j ---

#[derive(Debug, Deserialize)]
pub struct ZfsListOutput {
    pub output_version: OutputVersion,
    pub datasets: HashMap<String, ZfsListEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ZfsListEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub dataset_type: String,
    pub pool: String,
    pub createtxg: String,
    pub properties: HashMap<String, PropertyValue>,
}

// --- zfs get -j ---

#[derive(Debug, Deserialize)]
pub struct ZfsGetOutput {
    pub output_version: OutputVersion,
    pub datasets: HashMap<String, ZfsGetEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ZfsGetEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub dataset_type: String,
    pub pool: String,
    pub createtxg: String,
    pub properties: HashMap<String, PropertyValue>,
}

// --- zfs mount -j ---

#[derive(Debug, Deserialize)]
pub struct ZfsMountOutput {
    pub output_version: OutputVersion,
    pub datasets: HashMap<String, ZfsMountEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ZfsMountEntry {
    pub filesystem: String,
    pub mountpoint: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to load fixture {name}: {e}"))
    }

    #[test]
    fn test_deserialize_zpool_list() {
        let json = load_fixture("zpool_list.json");
        let output: ZpoolListOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(output.output_version.command, "zpool list");
        assert!(output.pools.contains_key("testpool"));
        let pool = &output.pools["testpool"];
        assert_eq!(pool.state, "ONLINE");
        assert_eq!(pool.name, "testpool");
        assert!(pool.properties.contains_key("size"));
        assert!(pool.properties.contains_key("health"));
    }

    #[test]
    fn test_deserialize_zpool_status() {
        let json = load_fixture("zpool_status.json");
        let output: ZpoolStatusOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(output.output_version.command, "zpool status");
        let pool = &output.pools["testpool"];
        assert_eq!(pool.state, "ONLINE");
        assert_eq!(pool.error_count, "0");
        // Root vdev
        let root_vdev = &pool.vdevs["testpool"];
        assert_eq!(root_vdev.vdev_type, "root");
        // Child vdev (the file)
        assert!(root_vdev.vdevs.contains_key("/tmp/test.img"));
    }

    #[test]
    fn test_deserialize_zpool_status_encrypted() {
        let json = load_fixture("zpool_status_encrypted.json");
        let output: ZpoolStatusOutput = serde_json::from_str(&json).unwrap();
        assert!(output.pools.contains_key("testpool_enc"));
        let pool = &output.pools["testpool_enc"];
        assert_eq!(pool.state, "ONLINE");
    }

    #[test]
    fn test_deserialize_zpool_get_all() {
        let json = load_fixture("zpool_get_all.json");
        let output: ZpoolGetOutput = serde_json::from_str(&json).unwrap();
        let pool = &output.pools["testpool"];
        assert!(pool.properties.contains_key("ashift"));
        assert!(pool.properties.contains_key("cachefile"));
        assert!(pool.properties.contains_key("autotrim"));
    }

    #[test]
    fn test_deserialize_zfs_list() {
        let json = load_fixture("zfs_list.json");
        let output: ZfsListOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(output.output_version.command, "zfs list");
        assert_eq!(output.datasets.len(), 3);
        assert!(output.datasets.contains_key("testpool"));
        assert!(output.datasets.contains_key("testpool/data"));
        assert!(output.datasets.contains_key("testpool/data/home"));

        let data = &output.datasets["testpool/data"];
        assert_eq!(data.dataset_type, "FILESYSTEM");
        assert_eq!(data.pool, "testpool");
        assert_eq!(data.properties["mountpoint"].value, "/mnt/test");
        assert_eq!(data.properties["mountpoint"].source.source_type, "LOCAL");
    }

    #[test]
    fn test_deserialize_zfs_list_all() {
        let json = load_fixture("zfs_list_all.json");
        let output: ZfsListOutput = serde_json::from_str(&json).unwrap();
        assert!(output.datasets.len() >= 3);
    }

    #[test]
    fn test_deserialize_zfs_get_all() {
        let json = load_fixture("zfs_get_all.json");
        let output: ZfsGetOutput = serde_json::from_str(&json).unwrap();
        let ds = &output.datasets["testpool"];
        assert!(ds.properties.contains_key("compression"));
        assert!(ds.properties.contains_key("mountpoint"));
    }

    #[test]
    fn test_deserialize_zfs_get_encryption_off() {
        let json = load_fixture("zfs_get_encryption_off.json");
        let output: ZfsGetOutput = serde_json::from_str(&json).unwrap();
        let ds = &output.datasets["testpool"];
        assert_eq!(ds.properties["encryption"].value, "off");
    }

    #[test]
    fn test_deserialize_zfs_get_encrypted() {
        let json = load_fixture("zfs_get_encrypted.json");
        let output: ZfsGetOutput = serde_json::from_str(&json).unwrap();
        let ds = &output.datasets["testpool_enc"];
        assert_eq!(ds.properties["encryption"].value, "aes-256-gcm");
        assert_eq!(ds.properties["keystatus"].value, "available");
        assert_eq!(ds.properties["keyformat"].value, "passphrase");
    }

    #[test]
    fn test_deserialize_zfs_mount() {
        let json = load_fixture("zfs_mount.json");
        let output: ZfsMountOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(output.output_version.command, "zfs mount");
        assert_eq!(output.datasets.len(), 3);
        assert_eq!(output.datasets["testpool"].mountpoint, "/testpool");
        assert_eq!(output.datasets["testpool/data"].mountpoint, "/mnt/test");
    }
}
