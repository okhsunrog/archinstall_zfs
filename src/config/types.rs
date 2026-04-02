use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const ZFS_PASSPHRASE_MIN_LENGTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationMode {
    FullDisk,
    NewPool,
    ExistingPool,
}

impl std::fmt::Display for InstallationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FullDisk => write!(f, "Full Disk"),
            Self::NewPool => write!(f, "New Pool"),
            Self::ExistingPool => write!(f, "Existing Pool"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InitSystem {
    #[default]
    Dracut,
    Mkinitcpio,
}

impl std::fmt::Display for InitSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dracut => write!(f, "dracut"),
            Self::Mkinitcpio => write!(f, "mkinitcpio"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ZfsModuleMode {
    #[default]
    Precompiled,
    Dkms,
}

impl std::fmt::Display for ZfsModuleMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Precompiled => write!(f, "precompiled"),
            Self::Dkms => write!(f, "dkms"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ZfsEncryptionMode {
    #[default]
    None,
    Pool,
    Dataset,
}

impl std::fmt::Display for ZfsEncryptionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "No encryption"),
            Self::Pool => write!(f, "Encrypt entire pool"),
            Self::Dataset => write!(f, "Encrypt base dataset only"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompressionAlgo {
    Off,
    #[default]
    Lz4,
    Zstd,
    #[serde(rename = "zstd-5")]
    Zstd5,
    #[serde(rename = "zstd-10")]
    Zstd10,
}

impl std::fmt::Display for CompressionAlgo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Lz4 => write!(f, "lz4"),
            Self::Zstd => write!(f, "zstd"),
            Self::Zstd5 => write!(f, "zstd-5"),
            Self::Zstd10 => write!(f, "zstd-10"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SwapMode {
    #[default]
    None,
    Zram,
    ZswapPartition,
    ZswapPartitionEncrypted,
}

impl std::fmt::Display for SwapMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Zram => write!(f, "ZRAM"),
            Self::ZswapPartition => write!(f, "Swap partition"),
            Self::ZswapPartitionEncrypted => write!(f, "Swap partition (encrypted)"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    // Flow control
    pub installation_mode: Option<InstallationMode>,
    pub disk_by_id: Option<PathBuf>,
    pub efi_partition_by_id: Option<PathBuf>,
    pub zfs_partition_by_id: Option<PathBuf>,
    pub pool_name: Option<String>,

    // ZFS specifics
    #[serde(default = "default_dataset_prefix")]
    pub dataset_prefix: String,
    #[serde(default)]
    pub init_system: InitSystem,
    #[serde(default)]
    pub zfs_module_mode: ZfsModuleMode,
    #[serde(default)]
    pub zfs_encryption_mode: ZfsEncryptionMode,
    pub zfs_encryption_password: Option<String>,
    #[serde(default)]
    pub compression: CompressionAlgo,

    // Swap
    #[serde(default)]
    pub swap_mode: SwapMode,
    pub swap_partition_size: Option<String>,
    pub swap_partition_by_id: Option<PathBuf>,
    #[serde(default = "default_zram_size_expr")]
    pub zram_size_expr: Option<String>,

    // Boot
    #[serde(default = "default_set_bootfs")]
    pub set_bootfs: bool,

    // Optional features
    #[serde(default)]
    pub zrepl_enabled: bool,
    #[serde(default)]
    pub aur_packages: Vec<String>,

    // Arch config (from archinstall)
    pub hostname: Option<String>,
    pub locale: Option<String>,
    #[serde(default = "default_keyboard_layout")]
    pub keyboard_layout: String,
    pub timezone: Option<String>,
    #[serde(default = "default_ntp")]
    pub ntp: bool,
    pub root_password: Option<String>,
    pub users: Option<Vec<UserConfig>>,
    pub kernels: Option<Vec<String>>,
    pub profile: Option<String>,
    pub mirror_regions: Option<Vec<String>>,
    #[serde(default)]
    pub additional_packages: Vec<String>,
    #[serde(default)]
    pub network_copy_iso: bool,
    #[serde(default)]
    pub audio: Option<AudioServer>,
    #[serde(default)]
    pub bluetooth: bool,
    #[serde(default = "default_parallel_downloads")]
    pub parallel_downloads: u32,
    #[serde(default)]
    pub extra_services: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub username: String,
    pub password: Option<String>,
    pub sudo: bool,
    pub shell: Option<String>,
    pub groups: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioServer {
    Pipewire,
    Pulseaudio,
}

impl std::fmt::Display for AudioServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipewire => write!(f, "pipewire"),
            Self::Pulseaudio => write!(f, "pulseaudio"),
        }
    }
}

fn default_dataset_prefix() -> String {
    "arch0".to_string()
}

fn default_zram_size_expr() -> Option<String> {
    Some("min(ram / 2, 4096)".to_string())
}

fn default_keyboard_layout() -> String {
    "us".to_string()
}

fn default_ntp() -> bool {
    true
}

fn default_set_bootfs() -> bool {
    true
}

fn default_parallel_downloads() -> u32 {
    5
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            installation_mode: None,
            disk_by_id: None,
            efi_partition_by_id: None,
            zfs_partition_by_id: None,
            pool_name: None,
            dataset_prefix: default_dataset_prefix(),
            init_system: InitSystem::default(),
            zfs_module_mode: ZfsModuleMode::default(),
            zfs_encryption_mode: ZfsEncryptionMode::default(),
            zfs_encryption_password: None,
            compression: CompressionAlgo::default(),
            swap_mode: SwapMode::default(),
            swap_partition_size: None,
            swap_partition_by_id: None,
            zram_size_expr: default_zram_size_expr(),
            set_bootfs: true,
            zrepl_enabled: false,
            aur_packages: Vec::new(),
            hostname: None,
            locale: None,
            keyboard_layout: default_keyboard_layout(),
            timezone: None,
            ntp: true,
            root_password: None,
            users: None,
            kernels: None,
            profile: None,
            mirror_regions: None,
            additional_packages: Vec::new(),
            network_copy_iso: false,
            audio: None,
            bluetooth: false,
            parallel_downloads: default_parallel_downloads(),
            extra_services: Vec::new(),
        }
    }
}

impl GlobalConfig {
    pub fn encryption_enabled(&self) -> bool {
        self.zfs_encryption_mode != ZfsEncryptionMode::None
    }

    pub fn all_aur_packages(&self) -> Vec<&str> {
        let mut pkgs: Vec<&str> = self.aur_packages.iter().map(|s| s.as_str()).collect();
        if self.zrepl_enabled && !pkgs.contains(&"zrepl-bin") {
            pkgs.push("zrepl-bin");
        }
        pkgs
    }

    pub fn effective_kernels(&self) -> &[String] {
        match &self.kernels {
            Some(k) if !k.is_empty() => k,
            _ => {
                // Can't return a reference to a temporary, so callers that
                // need the default should use primary_kernel() instead.
                // This returns empty — base.rs handles it by also using primary_kernel.
                &[]
            }
        }
    }

    pub fn primary_kernel(&self) -> &str {
        self.effective_kernels()
            .first()
            .map(|s| s.as_str())
            .unwrap_or("linux-lts")
    }
}

fn is_valid_pool_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

fn is_valid_dataset_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

fn is_by_id_path(path: &std::path::Path) -> bool {
    path.starts_with("/dev/disk/by-id/")
}

/// Validates the config and returns a list of error messages.
/// Empty list means the config is valid for installation.
impl GlobalConfig {
    pub fn validate_pool_name(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(ref name) = self.pool_name {
            if !is_valid_pool_name(name) {
                errors.push(format!(
                    "Pool name '{name}' is invalid: must be alphanumeric, underscores, or hyphens"
                ));
            }
        }
        errors
    }

    pub fn validate_dataset_prefix(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if !is_valid_dataset_prefix(&self.dataset_prefix) {
            errors.push(format!(
                "Dataset prefix '{}' is invalid: must be alphanumeric, underscores, or hyphens",
                self.dataset_prefix
            ));
        }
        errors
    }

    pub fn validate_by_id_paths(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(ref p) = self.disk_by_id {
            if !is_by_id_path(p) {
                errors.push(format!(
                    "disk_by_id must be a /dev/disk/by-id/ path, got: {}",
                    p.display()
                ));
            }
        }
        if let Some(ref p) = self.efi_partition_by_id {
            if !is_by_id_path(p) {
                errors.push(format!(
                    "efi_partition_by_id must be a /dev/disk/by-id/ path, got: {}",
                    p.display()
                ));
            }
        }
        if let Some(ref p) = self.zfs_partition_by_id {
            if !is_by_id_path(p) {
                errors.push(format!(
                    "zfs_partition_by_id must be a /dev/disk/by-id/ path, got: {}",
                    p.display()
                ));
            }
        }
        if let Some(ref p) = self.swap_partition_by_id {
            if !is_by_id_path(p) {
                errors.push(format!(
                    "swap_partition_by_id must be a /dev/disk/by-id/ path, got: {}",
                    p.display()
                ));
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = GlobalConfig::default();
        assert_eq!(cfg.dataset_prefix, "arch0");
        assert_eq!(cfg.init_system, InitSystem::Dracut);
        assert_eq!(cfg.zfs_module_mode, ZfsModuleMode::Precompiled);
        assert_eq!(cfg.zfs_encryption_mode, ZfsEncryptionMode::None);
        assert_eq!(cfg.compression, CompressionAlgo::Lz4);
        assert_eq!(cfg.swap_mode, SwapMode::None);
        assert!(!cfg.encryption_enabled());
        assert_eq!(cfg.primary_kernel(), "linux-lts");
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = GlobalConfig {
            installation_mode: Some(InstallationMode::FullDisk),
            pool_name: Some("testpool".to_string()),
            zfs_encryption_mode: ZfsEncryptionMode::Pool,
            zfs_encryption_password: Some("secret123".to_string()),
            swap_mode: SwapMode::Zram,
            hostname: Some("archbox".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let deserialized: GlobalConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.installation_mode,
            Some(InstallationMode::FullDisk)
        );
        assert_eq!(deserialized.pool_name.as_deref(), Some("testpool"));
        assert_eq!(deserialized.zfs_encryption_mode, ZfsEncryptionMode::Pool);
        assert_eq!(
            deserialized.zfs_encryption_password.as_deref(),
            Some("secret123")
        );
        assert_eq!(deserialized.swap_mode, SwapMode::Zram);
        assert_eq!(deserialized.hostname.as_deref(), Some("archbox"));
        assert_eq!(deserialized.dataset_prefix, "arch0");
    }

    #[test]
    fn test_encryption_enabled() {
        let mut cfg = GlobalConfig::default();
        assert!(!cfg.encryption_enabled());

        cfg.zfs_encryption_mode = ZfsEncryptionMode::Pool;
        assert!(cfg.encryption_enabled());

        cfg.zfs_encryption_mode = ZfsEncryptionMode::Dataset;
        assert!(cfg.encryption_enabled());
    }

    #[test]
    fn test_all_aur_packages_with_zrepl() {
        let mut cfg = GlobalConfig::default();
        cfg.aur_packages = vec!["custom-pkg".to_string()];
        cfg.zrepl_enabled = true;

        let pkgs = cfg.all_aur_packages();
        assert!(pkgs.contains(&"custom-pkg"));
        assert!(pkgs.contains(&"zrepl-bin"));
    }

    #[test]
    fn test_valid_pool_name() {
        assert!(is_valid_pool_name("mypool"));
        assert!(is_valid_pool_name("my_pool-1"));
        assert!(!is_valid_pool_name(""));
        assert!(!is_valid_pool_name("my pool"));
        assert!(!is_valid_pool_name("pool/name"));
    }

    #[test]
    fn test_valid_dataset_prefix() {
        assert!(is_valid_dataset_prefix("arch0"));
        assert!(is_valid_dataset_prefix("my-prefix_1"));
        assert!(!is_valid_dataset_prefix(""));
        assert!(!is_valid_dataset_prefix("prefix/bad"));
    }

    #[test]
    fn test_all_enum_variants_serialize() {
        // Verify all enum variants round-trip through serde
        for mode in [
            InstallationMode::FullDisk,
            InstallationMode::NewPool,
            InstallationMode::ExistingPool,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: InstallationMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, back);
        }

        for algo in [
            CompressionAlgo::Off,
            CompressionAlgo::Lz4,
            CompressionAlgo::Zstd,
            CompressionAlgo::Zstd5,
            CompressionAlgo::Zstd10,
        ] {
            let json = serde_json::to_string(&algo).unwrap();
            let back: CompressionAlgo = serde_json::from_str(&json).unwrap();
            assert_eq!(algo, back);
        }

        for swap in [
            SwapMode::None,
            SwapMode::Zram,
            SwapMode::ZswapPartition,
            SwapMode::ZswapPartitionEncrypted,
        ] {
            let json = serde_json::to_string(&swap).unwrap();
            let back: SwapMode = serde_json::from_str(&json).unwrap();
            assert_eq!(swap, back);
        }
    }
}
