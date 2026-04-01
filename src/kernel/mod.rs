pub mod scanner;

use crate::config::types::ZfsModuleMode;

#[derive(Debug, Clone)]
pub struct KernelInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub precompiled_package: Option<&'static str>,
    pub headers_package: &'static str,
}

pub const AVAILABLE_KERNELS: &[KernelInfo] = &[
    KernelInfo {
        name: "linux-lts",
        display_name: "Linux LTS",
        precompiled_package: Some("zfs-linux-lts"),
        headers_package: "linux-lts-headers",
    },
    KernelInfo {
        name: "linux",
        display_name: "Linux",
        precompiled_package: Some("zfs-linux"),
        headers_package: "linux-headers",
    },
    KernelInfo {
        name: "linux-zen",
        display_name: "Linux Zen",
        precompiled_package: Some("zfs-linux-zen"),
        headers_package: "linux-zen-headers",
    },
    KernelInfo {
        name: "linux-hardened",
        display_name: "Linux Hardened",
        precompiled_package: Some("zfs-linux-hardened"),
        headers_package: "linux-hardened-headers",
    },
];

pub fn get_kernel_info(name: &str) -> Option<&'static KernelInfo> {
    AVAILABLE_KERNELS.iter().find(|k| k.name == name)
}

pub fn get_zfs_packages(kernel: &str, mode: ZfsModuleMode) -> Vec<String> {
    let mut packages = vec!["zfs-utils".to_string()];

    if let Some(info) = get_kernel_info(kernel) {
        match mode {
            ZfsModuleMode::Precompiled => {
                if let Some(pkg) = info.precompiled_package {
                    packages.push(pkg.to_string());
                } else {
                    // Fall back to DKMS
                    packages.push("zfs-dkms".to_string());
                    packages.push(info.headers_package.to_string());
                }
            }
            ZfsModuleMode::Dkms => {
                packages.push("zfs-dkms".to_string());
                packages.push(info.headers_package.to_string());
            }
        }
    }

    packages
}

pub fn supports_precompiled(kernel: &str) -> bool {
    get_kernel_info(kernel)
        .and_then(|k| k.precompiled_package)
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_kernel_info() {
        let info = get_kernel_info("linux-lts").unwrap();
        assert_eq!(info.display_name, "Linux LTS");
        assert_eq!(info.precompiled_package, Some("zfs-linux-lts"));
    }

    #[test]
    fn test_unknown_kernel() {
        assert!(get_kernel_info("linux-custom").is_none());
    }

    #[test]
    fn test_get_zfs_packages_precompiled() {
        let pkgs = get_zfs_packages("linux-lts", ZfsModuleMode::Precompiled);
        assert!(pkgs.contains(&"zfs-utils".to_string()));
        assert!(pkgs.contains(&"zfs-linux-lts".to_string()));
        assert!(!pkgs.contains(&"zfs-dkms".to_string()));
    }

    #[test]
    fn test_get_zfs_packages_dkms() {
        let pkgs = get_zfs_packages("linux", ZfsModuleMode::Dkms);
        assert!(pkgs.contains(&"zfs-utils".to_string()));
        assert!(pkgs.contains(&"zfs-dkms".to_string()));
        assert!(pkgs.contains(&"linux-headers".to_string()));
    }

    #[test]
    fn test_supports_precompiled() {
        assert!(supports_precompiled("linux-lts"));
        assert!(supports_precompiled("linux"));
        assert!(!supports_precompiled("linux-custom"));
    }
}
