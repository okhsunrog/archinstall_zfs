use super::types::{
    GlobalConfig, InstallationMode, SwapMode, ZFS_PASSPHRASE_MIN_LENGTH, ZfsEncryptionMode,
};

impl GlobalConfig {
    /// Validate configuration for installation.
    /// Returns a list of error messages. Empty means valid.
    pub fn validate_for_install(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Must have an installation mode
        let mode = match self.installation_mode {
            Some(m) => m,
            None => {
                errors.push("Installation mode not selected".to_string());
                return errors;
            }
        };

        // Pool name required for all modes
        if self.pool_name.is_none() {
            errors.push("Pool name is required".to_string());
        }
        errors.extend(self.validate_pool_name());
        errors.extend(self.validate_dataset_prefix());
        errors.extend(self.validate_by_id_paths());

        // Mode-dependent validation
        match mode {
            InstallationMode::FullDisk => {
                if self.disk_by_id.is_none() {
                    errors
                        .push("Full disk mode requires a disk selection (disk_by_id)".to_string());
                }
                if matches!(
                    self.swap_mode,
                    SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
                ) && self.swap_partition_size.is_none()
                {
                    errors.push(
                        "Swap partition mode requires swap_partition_size in full disk mode"
                            .to_string(),
                    );
                }
            }
            InstallationMode::NewPool => {
                if self.efi_partition_by_id.is_none() {
                    errors.push(
                        "New pool mode requires an EFI partition (efi_partition_by_id)".to_string(),
                    );
                }
                if self.zfs_partition_by_id.is_none() {
                    errors.push(
                        "New pool mode requires a ZFS partition (zfs_partition_by_id)".to_string(),
                    );
                }
                if matches!(
                    self.swap_mode,
                    SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
                ) && self.swap_partition_by_id.is_none()
                {
                    errors.push(
                        "Swap partition mode requires swap_partition_by_id in new pool mode"
                            .to_string(),
                    );
                }
            }
            InstallationMode::ExistingPool => {
                if self.efi_partition_by_id.is_none() {
                    errors.push(
                        "Existing pool mode requires an EFI partition (efi_partition_by_id)"
                            .to_string(),
                    );
                }
                if matches!(
                    self.swap_mode,
                    SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
                ) && self.swap_partition_by_id.is_none()
                {
                    errors.push(
                        "Swap partition mode requires swap_partition_by_id in existing pool mode"
                            .to_string(),
                    );
                }
            }
        }

        // Encryption validation
        if self.zfs_encryption_mode != ZfsEncryptionMode::None {
            match &self.zfs_encryption_password {
                None => {
                    errors.push("Encryption enabled but no password provided".to_string());
                }
                Some(pw) if pw.len() < ZFS_PASSPHRASE_MIN_LENGTH => {
                    errors.push(format!(
                        "Encryption password must be at least {ZFS_PASSPHRASE_MIN_LENGTH} characters"
                    ));
                }
                _ => {}
            }
        }

        errors
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::types::{CompressionAlgo, GlobalConfig, InstallationMode, SwapMode};

    fn valid_full_disk_config() -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::FullDisk),
            disk_by_id: Some(PathBuf::from("/dev/disk/by-id/virtio-archzfs-test-disk")),
            pool_name: Some("testpool".to_string()),
            ..Default::default()
        }
    }

    fn valid_new_pool_config() -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::NewPool),
            efi_partition_by_id: Some(PathBuf::from(
                "/dev/disk/by-id/virtio-archzfs-test-disk-part1",
            )),
            zfs_partition_by_id: Some(PathBuf::from(
                "/dev/disk/by-id/virtio-archzfs-test-disk-part2",
            )),
            pool_name: Some("testpool".to_string()),
            ..Default::default()
        }
    }

    fn valid_existing_pool_config() -> GlobalConfig {
        GlobalConfig {
            installation_mode: Some(InstallationMode::ExistingPool),
            efi_partition_by_id: Some(PathBuf::from(
                "/dev/disk/by-id/virtio-archzfs-test-disk-part1",
            )),
            pool_name: Some("testpool".to_string()),
            ..Default::default()
        }
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
        assert!(errors.iter().any(|e| e.contains("Installation mode")));
    }

    #[test]
    fn test_full_disk_missing_disk() {
        let mut cfg = valid_full_disk_config();
        cfg.disk_by_id = None;
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("disk selection")));
    }

    #[test]
    fn test_new_pool_missing_partitions() {
        let mut cfg = valid_new_pool_config();
        cfg.efi_partition_by_id = None;
        cfg.zfs_partition_by_id = None;
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("EFI partition")));
        assert!(errors.iter().any(|e| e.contains("ZFS partition")));
    }

    #[test]
    fn test_existing_pool_missing_efi() {
        let mut cfg = valid_existing_pool_config();
        cfg.efi_partition_by_id = None;
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("EFI partition")));
    }

    #[test]
    fn test_missing_pool_name() {
        let mut cfg = valid_full_disk_config();
        cfg.pool_name = None;
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("Pool name")));
    }

    #[test]
    fn test_encryption_no_password() {
        let mut cfg = valid_full_disk_config();
        cfg.zfs_encryption_mode = ZfsEncryptionMode::Pool;
        cfg.zfs_encryption_password = None;
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("no password")));
    }

    #[test]
    fn test_encryption_short_password() {
        let mut cfg = valid_full_disk_config();
        cfg.zfs_encryption_mode = ZfsEncryptionMode::Dataset;
        cfg.zfs_encryption_password = Some("short".to_string());
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("at least")));
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
        assert!(errors.iter().any(|e| e.contains("swap_partition_size")));
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
        cfg.swap_partition_by_id = None;
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("swap_partition_by_id")));
    }

    #[test]
    fn test_zram_requires_nothing_extra() {
        let mut cfg = valid_full_disk_config();
        cfg.swap_mode = SwapMode::Zram;
        let errors = cfg.validate_for_install();
        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_invalid_by_id_path() {
        let mut cfg = valid_full_disk_config();
        cfg.disk_by_id = Some(PathBuf::from("/dev/sda"));
        let errors = cfg.validate_for_install();
        assert!(errors.iter().any(|e| e.contains("/dev/disk/by-id/")));
    }

    #[test]
    fn test_serde_roundtrip_full_config() {
        let cfg = GlobalConfig {
            installation_mode: Some(InstallationMode::FullDisk),
            disk_by_id: Some(PathBuf::from("/dev/disk/by-id/virtio-archzfs-test-disk")),
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
}
