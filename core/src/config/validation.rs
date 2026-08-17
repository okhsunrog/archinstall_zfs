use std::fmt;
use std::path::PathBuf;

use super::types::{
    GlobalConfig, InstallationMode, SwapMode, ZFS_PASSPHRASE_MIN_LENGTH, ZfsEncryptionMode,
};

/// Something about the configuration that stops the installation.
///
/// A value rather than a sentence: the sentence is one rendering of it, and
/// an interface that wants to point at the offending field needs the parts,
/// not the prose. Rendering lives in the `Display` implementation below, so
/// every interface words a given problem the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    InstallationModeNotSelected,
    PoolNameMissing,
    PoolNameInvalid(String),
    DatasetPrefixInvalid(String),
    /// A device path the installer will not accept, and the setting it was
    /// given for.
    DevicePathUnsupported {
        setting: &'static str,
        path: PathBuf,
    },
    DiskRequired,
    EfiPartitionRequired(InstallationMode),
    ZfsPartitionRequired,
    /// Full-disk mode carves the swap partition itself and needs its size.
    SwapSizeRequired,
    /// The other modes need an existing partition to use.
    SwapPartitionRequired(InstallationMode),
    EncryptionPasswordMissing,
    EncryptionPasswordTooShort {
        minimum: usize,
    },
    HostnameInvalid(String),
    UnknownKernel(String),
    UsernameInvalid(String),
}

/// Valid Linux hostname: 1-63 chars, alphanumeric + hyphens, no leading/trailing hyphen.
fn is_valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Valid Linux username: 1-32 chars, starts with letter or underscore,
/// rest is alphanumeric + underscore + hyphen.
pub fn is_valid_username(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstallationModeNotSelected => write!(f, "Installation mode not selected"),
            Self::PoolNameMissing => write!(f, "Pool name is required"),
            Self::PoolNameInvalid(name) => write!(
                f,
                "Pool name '{name}' is invalid: must be alphanumeric, underscores, or hyphens"
            ),
            Self::DatasetPrefixInvalid(prefix) => write!(
                f,
                "Dataset prefix '{prefix}' is invalid: must be alphanumeric, underscores, or hyphens"
            ),
            Self::DevicePathUnsupported { setting, path } => write!(
                f,
                "{setting} must be a /dev/disk/by-id/, /dev/disk/by-path/, or supported /dev node \
                 path, got: {}",
                path.display()
            ),
            Self::DiskRequired => write!(f, "Full disk mode requires a disk selection (disk)"),
            Self::EfiPartitionRequired(mode) => {
                write!(f, "{} mode requires an EFI partition (efi_partition)", mode)
            }
            Self::ZfsPartitionRequired => {
                write!(f, "New pool mode requires a ZFS partition (zfs_partition)")
            }
            Self::SwapSizeRequired => write!(
                f,
                "Swap partition mode requires swap_partition_size in full disk mode"
            ),
            Self::SwapPartitionRequired(mode) => write!(
                f,
                "Swap partition mode requires swap_partition in {} mode",
                mode.to_string().to_lowercase()
            ),
            Self::EncryptionPasswordMissing => {
                write!(f, "Encryption enabled but no password provided")
            }
            Self::EncryptionPasswordTooShort { minimum } => {
                write!(
                    f,
                    "Encryption password must be at least {minimum} characters"
                )
            }
            Self::HostnameInvalid(name) => write!(
                f,
                "Hostname '{name}' is invalid: must be 1-63 chars, alphanumeric and hyphens, no \
                 leading/trailing hyphen"
            ),
            Self::UnknownKernel(name) => {
                let known: Vec<&str> = crate::kernel::AVAILABLE_KERNELS
                    .iter()
                    .map(|k| k.name)
                    .collect();
                write!(
                    f,
                    "Unknown kernel '{name}'. Available: {}",
                    known.join(", ")
                )
            }
            Self::UsernameInvalid(name) => write!(
                f,
                "Username '{name}' is invalid: must be 1-32 chars, start with lowercase letter or \
                 underscore, contain only lowercase, digits, underscore, hyphen"
            ),
        }
    }
}

impl GlobalConfig {
    /// Everything about this configuration that stops the installation.
    /// Empty means it can proceed.
    pub fn validate_for_install(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Without a mode there is nothing to check the rest against.
        let Some(mode) = self.installation_mode else {
            return vec![ValidationError::InstallationModeNotSelected];
        };

        if self.pool_name.is_none() {
            errors.push(ValidationError::PoolNameMissing);
        }
        errors.extend(self.validate_pool_name());
        errors.extend(self.validate_dataset_prefix());
        errors.extend(self.validate_device_paths());

        let wants_swap_partition = matches!(
            self.swap_mode,
            SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
        );

        match mode {
            InstallationMode::FullDisk => {
                if self.disk.is_none() {
                    errors.push(ValidationError::DiskRequired);
                }
                // Full-disk mode creates the partition, so it needs a size
                // rather than an existing one.
                if wants_swap_partition && self.swap_partition_size.is_none() {
                    errors.push(ValidationError::SwapSizeRequired);
                }
            }
            InstallationMode::NewPool | InstallationMode::ExistingPool => {
                if self.efi_partition.is_none() {
                    errors.push(ValidationError::EfiPartitionRequired(mode));
                }
                if mode == InstallationMode::NewPool && self.zfs_partition.is_none() {
                    errors.push(ValidationError::ZfsPartitionRequired);
                }
                if wants_swap_partition && self.swap_partition.is_none() {
                    errors.push(ValidationError::SwapPartitionRequired(mode));
                }
            }
        }

        if self.zfs_encryption_mode != ZfsEncryptionMode::None {
            match &self.zfs_encryption_password {
                None => errors.push(ValidationError::EncryptionPasswordMissing),
                Some(password) if password.len() < ZFS_PASSPHRASE_MIN_LENGTH => {
                    errors.push(ValidationError::EncryptionPasswordTooShort {
                        minimum: ZFS_PASSPHRASE_MIN_LENGTH,
                    });
                }
                Some(_) => {}
            }
        }

        if let Some(hostname) = &self.hostname
            && !is_valid_hostname(hostname)
        {
            errors.push(ValidationError::HostnameInvalid(hostname.clone()));
        }

        for kernel in self.kernels.iter().flatten() {
            if crate::kernel::get_kernel_info(kernel).is_none() {
                errors.push(ValidationError::UnknownKernel(kernel.clone()));
            }
        }

        for user in self.users.iter().flatten() {
            if !is_valid_username(&user.username) {
                errors.push(ValidationError::UsernameInvalid(user.username.clone()));
            }
        }

        errors
    }
}

#[cfg(test)]
mod tests {
    use super::ValidationError;
    use std::path::PathBuf;

    use super::*;
    use crate::config::types::{
        CompressionAlgo, GlobalConfig, InstallationMode, SwapMode, UserConfig,
    };

    fn valid_full_disk_config() -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::FullDisk),
            disk: Some(PathBuf::from("/dev/disk/by-id/virtio-archzfs-test-disk")),
            pool_name: Some("testpool".to_string()),
            ..Default::default()
        }
    }

    fn valid_new_pool_config() -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::NewPool),
            efi_partition: Some(PathBuf::from(
                "/dev/disk/by-id/virtio-archzfs-test-disk-part1",
            )),
            zfs_partition: Some(PathBuf::from(
                "/dev/disk/by-id/virtio-archzfs-test-disk-part2",
            )),
            pool_name: Some("testpool".to_string()),
            ..Default::default()
        }
    }

    fn valid_existing_pool_config() -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::ExistingPool),
            efi_partition: Some(PathBuf::from(
                "/dev/disk/by-id/virtio-archzfs-test-disk-part1",
            )),
            pool_name: Some("testpool".to_string()),
            ..Default::default()
        }
    }

    /// Every problem renders as something a user can act on, and the ones
    /// carrying a value mention it.
    #[test]
    fn errors_render_with_their_details() {
        assert_eq!(
            ValidationError::PoolNameInvalid("bad name".into()).to_string(),
            "Pool name 'bad name' is invalid: must be alphanumeric, underscores, or hyphens"
        );
        assert_eq!(
            ValidationError::EncryptionPasswordTooShort { minimum: 8 }.to_string(),
            "Encryption password must be at least 8 characters"
        );
        assert!(
            ValidationError::EfiPartitionRequired(InstallationMode::ExistingPool)
                .to_string()
                .starts_with("Existing Pool mode"),
            "the mode belongs in the message"
        );
        assert!(
            ValidationError::UnknownKernel("linux-custom".into())
                .to_string()
                .contains("linux-lts"),
            "an unknown kernel should list the known ones"
        );
    }

    #[test]
    fn test_valid_full_disk() {
        let cfg = valid_full_disk_config();
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_valid_new_pool() {
        let cfg = valid_new_pool_config();
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_valid_existing_pool() {
        let cfg = valid_existing_pool_config();
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_no_installation_mode() {
        let cfg = GlobalConfig::default();
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::InstallationModeNotSelected))
        );
    }

    #[test]
    fn test_full_disk_missing_disk() {
        let mut cfg = valid_full_disk_config();
        cfg.disk = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::DiskRequired))
        );
    }

    #[test]
    fn test_new_pool_missing_partitions() {
        let mut cfg = valid_new_pool_config();
        cfg.efi_partition = None;
        cfg.zfs_partition = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::EfiPartitionRequired(_)))
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::ZfsPartitionRequired))
        );
    }

    #[test]
    fn test_existing_pool_missing_efi() {
        let mut cfg = valid_existing_pool_config();
        cfg.efi_partition = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::EfiPartitionRequired(_)))
        );
    }

    #[test]
    fn test_missing_pool_name() {
        let mut cfg = valid_full_disk_config();
        cfg.pool_name = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::PoolNameMissing))
        );
    }

    #[test]
    fn test_encryption_no_password() {
        let mut cfg = valid_full_disk_config();
        cfg.zfs_encryption_mode = ZfsEncryptionMode::Pool;
        cfg.zfs_encryption_password = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::EncryptionPasswordMissing))
        );
    }

    #[test]
    fn test_encryption_short_password() {
        let mut cfg = valid_full_disk_config();
        cfg.zfs_encryption_mode = ZfsEncryptionMode::Dataset;
        cfg.zfs_encryption_password = Some("short".to_string());
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::EncryptionPasswordTooShort { .. }))
        );
    }

    #[test]
    fn test_encryption_valid_password() {
        let mut cfg = valid_full_disk_config();
        cfg.zfs_encryption_mode = ZfsEncryptionMode::Pool;
        cfg.zfs_encryption_password = Some("longpassword123".to_string());
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_full_disk_swap_partition_needs_size() {
        let mut cfg = valid_full_disk_config();
        cfg.swap_mode = SwapMode::ZswapPartition;
        cfg.swap_partition_size = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::SwapSizeRequired))
        );
    }

    #[test]
    fn test_full_disk_swap_partition_with_size() {
        let mut cfg = valid_full_disk_config();
        cfg.swap_mode = SwapMode::ZswapPartition;
        cfg.swap_partition_size = Some("8G".to_string());
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_new_pool_swap_needs_partition() {
        let mut cfg = valid_new_pool_config();
        cfg.swap_mode = SwapMode::ZswapPartitionEncrypted;
        cfg.swap_partition = None;
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::SwapPartitionRequired(_)))
        );
    }

    #[test]
    fn test_zram_requires_nothing_extra() {
        let mut cfg = valid_full_disk_config();
        cfg.swap_mode = SwapMode::Zram;
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_valid_by_path_disk_path() {
        let mut cfg = valid_full_disk_config();
        cfg.disk = Some(PathBuf::from("/dev/disk/by-path/pci-0000:00:04.0"));
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_valid_virtio_devnode_disk_path() {
        let mut cfg = valid_full_disk_config();
        cfg.disk = Some(PathBuf::from("/dev/vda"));
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_invalid_device_path() {
        let mut cfg = valid_full_disk_config();
        cfg.disk = Some(PathBuf::from("/tmp/not-a-disk"));
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::DevicePathUnsupported { .. }))
        );
    }

    #[test]
    fn test_valid_hostname() {
        let mut cfg = valid_full_disk_config();
        cfg.hostname = Some("my-host".to_string());
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_invalid_hostname_leading_hyphen() {
        let mut cfg = valid_full_disk_config();
        cfg.hostname = Some("-badhost".to_string());
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::HostnameInvalid(_)))
        );
    }

    #[test]
    fn test_invalid_hostname_special_chars() {
        let mut cfg = valid_full_disk_config();
        cfg.hostname = Some("host.name".to_string());
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::HostnameInvalid(_)))
        );
    }

    #[test]
    fn test_invalid_hostname_too_long() {
        let mut cfg = valid_full_disk_config();
        cfg.hostname = Some("a".repeat(64));
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::HostnameInvalid(_)))
        );
    }

    #[test]
    fn test_unknown_kernel() {
        let mut cfg = valid_full_disk_config();
        cfg.kernels = Some(vec!["linux-custom".to_string()]);
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::UnknownKernel(_)))
        );
    }

    #[test]
    fn test_valid_kernel() {
        let mut cfg = valid_full_disk_config();
        cfg.kernels = Some(vec!["linux-lts".to_string()]);
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_valid_username() {
        let mut cfg = valid_full_disk_config();
        cfg.users = Some(vec![UserConfig {
            username: "john".to_string(),
            password: None,
            sudo: false,
            shell: None,
            groups: None,
            ssh_authorized_keys: Vec::new(),
            autologin: false,
        }]);
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_invalid_username_uppercase() {
        let mut cfg = valid_full_disk_config();
        cfg.users = Some(vec![UserConfig {
            username: "John".to_string(),
            password: None,
            sudo: false,
            shell: None,
            groups: None,
            ssh_authorized_keys: Vec::new(),
            autologin: false,
        }]);
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::UsernameInvalid(_)))
        );
    }

    #[test]
    fn test_invalid_username_starts_with_digit() {
        let mut cfg = valid_full_disk_config();
        cfg.users = Some(vec![UserConfig {
            username: "1user".to_string(),
            password: None,
            sudo: false,
            shell: None,
            groups: None,
            ssh_authorized_keys: Vec::new(),
            autologin: false,
        }]);
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::UsernameInvalid(_)))
        );
    }

    #[test]
    fn test_invalid_username_spaces() {
        let mut cfg = valid_full_disk_config();
        cfg.users = Some(vec![UserConfig {
            username: "my user".to_string(),
            password: None,
            sudo: false,
            shell: None,
            groups: None,
            ssh_authorized_keys: Vec::new(),
            autologin: false,
        }]);
        let errors = cfg.validate_for_install();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::UsernameInvalid(_)))
        );
    }

    #[test]
    fn test_serde_roundtrip_full_config() {
        let cfg = GlobalConfig {
            installation_mode: Some(InstallationMode::FullDisk),
            disk: Some(PathBuf::from("/dev/disk/by-id/virtio-archzfs-test-disk")),
            pool_name: Some("mypool".to_string()),
            dataset_prefix: "arch0".to_string(),
            compression: CompressionAlgo::Zstd5,
            swap_mode: SwapMode::Zram,
            zfs_encryption_mode: ZfsEncryptionMode::Pool,
            zfs_encryption_password: Some("mysecretpw".to_string()),
            hostname: Some("workstation".to_string()),
            kernels: Some(vec!["linux-lts".to_string()]),
            ..Default::default()
        };

        let json = serde_json::to_string(&cfg).unwrap();
        let back: GlobalConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.compression, CompressionAlgo::Zstd5);
        assert_eq!(back.swap_mode, SwapMode::Zram);
        assert_eq!(back.pool_name.as_deref(), Some("mypool"));
    }

    #[test]
    fn test_serde_accepts_legacy_device_field_names() {
        let json = r#"{
            "installation_mode": "new_pool",
            "disk_by_id": "/dev/disk/by-id/legacy-disk",
            "efi_partition_by_id": "/dev/disk/by-id/legacy-disk-part1",
            "zfs_partition_by_id": "/dev/disk/by-id/legacy-disk-part2",
            "swap_partition_by_id": "/dev/disk/by-id/legacy-disk-part3",
            "pool_name": "testpool"
        }"#;

        let cfg: GlobalConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            cfg.disk.as_deref(),
            Some(std::path::Path::new("/dev/disk/by-id/legacy-disk"))
        );
        assert_eq!(
            cfg.efi_partition.as_deref(),
            Some(std::path::Path::new("/dev/disk/by-id/legacy-disk-part1"))
        );
        assert_eq!(
            cfg.zfs_partition.as_deref(),
            Some(std::path::Path::new("/dev/disk/by-id/legacy-disk-part2"))
        );
        assert_eq!(
            cfg.swap_partition.as_deref(),
            Some(std::path::Path::new("/dev/disk/by-id/legacy-disk-part3"))
        );
    }
}
