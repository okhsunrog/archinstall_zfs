use color_eyre::eyre::{Context, Result};

use crate::config::types::ZfsModuleMode;

pub mod scanner;

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

/// Query a package version from the local pacman sync database using libalpm.
/// Returns None if the package is not found in any configured repo.
pub fn query_package_version(package: &str) -> Result<Option<String>> {
    let versions = query_packages(&[package])?;
    Ok(versions.get(package).cloned())
}

/// Initialize an alpm handle from the system pacman.conf.
fn init_alpm() -> Result<alpm::Alpm> {
    let pacman_conf = pacmanconf::Config::from_file("/etc/pacman.conf")
        .wrap_err("failed to parse pacman.conf")?;

    let db_path = &pacman_conf.db_path;
    let root = &pacman_conf.root_dir;

    let mut handle = alpm::Alpm::new(root.as_str(), db_path.as_str())
        .wrap_err("failed to initialize libalpm")?;

    for repo in &pacman_conf.repos {
        let db = handle
            .register_syncdb_mut(repo.name.as_str(), alpm::SigLevel::NONE)
            .wrap_err_with(|| format!("failed to register repo: {}", repo.name))?;
        for server in &repo.servers {
            let _ = db.add_server(server.as_str());
        }
    }

    Ok(handle)
}

/// Query multiple packages at once, returning a map of name -> version.
pub fn query_packages(packages: &[&str]) -> Result<std::collections::HashMap<String, String>> {
    let handle = init_alpm()?;

    let mut result = std::collections::HashMap::new();
    for &pkg_name in packages {
        for db in handle.syncdbs() {
            if let Ok(pkg) = db.pkg(pkg_name.as_bytes()) {
                result.insert(pkg_name.to_string(), pkg.version().to_string());
                break;
            }
        }
    }

    Ok(result)
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

    // Integration test: only runs on Arch with synced pacman DB
    #[test]
    fn test_query_package_version_on_arch() {
        if !std::path::Path::new("/var/lib/pacman/sync").exists() {
            return; // Skip on non-Arch
        }
        let ver = query_package_version("linux-lts");
        match ver {
            Ok(Some(v)) => {
                assert!(!v.is_empty());
                tracing::info!(version = v, "linux-lts version from alpm");
            }
            Ok(None) => {
                // DB might not be synced
            }
            Err(_) => {
                // libalpm not available
            }
        }
    }
}
