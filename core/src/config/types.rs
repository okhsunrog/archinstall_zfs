use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::profile::DisplayManager;
use crate::system::gpu::GfxDriver;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatAccess {
    /// Install seatd, enable seatd.service, add users to the `seat` group.
    Seatd,
    /// Rely on polkit (typically already present as a compositor dependency).
    Polkit,
}

impl std::fmt::Display for SeatAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Seatd => write!(f, "seatd"),
            Self::Polkit => write!(f, "polkit"),
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
    /// User's profile choice plus all profile-scoped sub-selections
    /// (optional packages, DM override, seat access). Atomic replace on
    /// profile switch, so stale settings cannot leak across.
    #[serde(default)]
    pub profile_selection: Option<ProfileSelection>,
    /// GPU driver (independent of profile — useful for headless installs too).
    pub gfx_driver: Option<GfxDriver>,
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
    #[serde(default)]
    pub post_install_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub username: String,
    pub password: Option<String>,
    pub sudo: bool,
    pub shell: Option<String>,
    pub groups: Option<Vec<String>>,
    #[serde(default)]
    pub ssh_authorized_keys: Vec<String>,
    #[serde(default)]
    pub autologin: bool,
}

/// User selection for a profile, plus all profile-scoped settings derived
/// from it. Replacing this struct atomically guarantees stale fields can't
/// leak between profile switches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSelection {
    /// Profile registry key (e.g. `"gnome"`).
    pub profile: String,
    /// Optional extras the user enabled. Subset of the profile's
    /// `optional_packages`. Sorted for stable diffs.
    #[serde(default)]
    pub optional_packages: BTreeSet<String>,
    /// User-chosen DM override. `None` means "use the profile's default DM".
    #[serde(default)]
    pub display_manager_override: Option<DisplayManager>,
    /// Seat access for Wayland compositors. Only meaningful when the
    /// underlying profile has `needs_seat_access = true`.
    #[serde(default)]
    pub seat_access: Option<SeatAccess>,
}

impl ProfileSelection {
    /// Create a fresh selection for `profile_name` with sensible defaults
    /// pulled from the registry. Returns `None` if the profile is unknown.
    pub fn new(profile_name: &str) -> Option<Self> {
        let p = crate::profile::get_profile(profile_name)?;
        Some(Self {
            profile: profile_name.to_string(),
            optional_packages: BTreeSet::new(),
            display_manager_override: None,
            // Pre-fill seatd as a sensible default for Wayland compositors
            // that need explicit seat access; user can override.
            seat_access: if p.needs_seat_access() {
                Some(SeatAccess::Seatd)
            } else {
                None
            },
        })
    }

    /// Resolve this selection's profile in the registry. Returns `None`
    /// when the profile name no longer exists (e.g. after a downgrade).
    pub fn profile_def(&self) -> Option<crate::profile::Profile> {
        crate::profile::get_profile(&self.profile)
    }

    /// Effective DM = explicit override, falling back to the profile default.
    pub fn effective_display_manager(&self) -> Option<DisplayManager> {
        self.display_manager_override
            .or_else(|| self.profile_def().and_then(|p| p.default_display_manager()))
    }

    /// Profile packages ∪ chosen optionals. Does *not* include the DM
    /// package — the installer enables/installs the DM separately so it can
    /// also handle overrides.
    pub fn resolved_packages(&self) -> Vec<String> {
        let Some(p) = self.profile_def() else {
            return Vec::new();
        };
        let mut out: Vec<String> = p.packages.iter().map(|s| s.to_string()).collect();
        for opt in &self.optional_packages {
            if !out.contains(opt) {
                out.push(opt.clone());
            }
        }
        out
    }
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
    10
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            installation_mode: None,
            disk_by_id: None,
            efi_partition_by_id: None,
            zfs_partition_by_id: None,
            pool_name: Some("zroot".to_string()),
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
            hostname: Some("archzfs".to_string()),
            locale: Some("en_US.UTF-8 UTF-8".to_string()),
            keyboard_layout: default_keyboard_layout(),
            timezone: None,
            ntp: true,
            root_password: None,
            users: None,
            kernels: None,
            profile_selection: None,
            gfx_driver: None,
            mirror_regions: None,
            additional_packages: Vec::new(),
            network_copy_iso: false,
            audio: None,
            bluetooth: false,
            parallel_downloads: default_parallel_downloads(),
            extra_services: Vec::new(),
            post_install_commands: Vec::new(),
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

    /// Return the list of kernels to install. Always contains at least one
    /// entry — defaults to `["linux-lts"]` when none are configured.
    pub fn effective_kernels(&self) -> Vec<&str> {
        match &self.kernels {
            Some(k) if !k.is_empty() => k.iter().map(|s| s.as_str()).collect(),
            _ => vec!["linux-lts"],
        }
    }

    pub fn primary_kernel(&self) -> &str {
        match &self.kernels {
            Some(k) if !k.is_empty() => k[0].as_str(),
            _ => "linux-lts",
        }
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
        if let Some(ref name) = self.pool_name
            && !is_valid_pool_name(name)
        {
            errors.push(format!(
                "Pool name '{name}' is invalid: must be alphanumeric, underscores, or hyphens"
            ));
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
        if let Some(ref p) = self.disk_by_id
            && !is_by_id_path(p)
        {
            errors.push(format!(
                "disk_by_id must be a /dev/disk/by-id/ path, got: {}",
                p.display()
            ));
        }
        if let Some(ref p) = self.efi_partition_by_id
            && !is_by_id_path(p)
        {
            errors.push(format!(
                "efi_partition_by_id must be a /dev/disk/by-id/ path, got: {}",
                p.display()
            ));
        }
        if let Some(ref p) = self.zfs_partition_by_id
            && !is_by_id_path(p)
        {
            errors.push(format!(
                "zfs_partition_by_id must be a /dev/disk/by-id/ path, got: {}",
                p.display()
            ));
        }
        if let Some(ref p) = self.swap_partition_by_id
            && !is_by_id_path(p)
        {
            errors.push(format!(
                "swap_partition_by_id must be a /dev/disk/by-id/ path, got: {}",
                p.display()
            ));
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
