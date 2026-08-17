use color_eyre::eyre::{Context, Result};

use crate::config::types::ZfsModuleMode;
use crate::distro::Distribution;

pub mod scanner;

#[derive(Debug, Clone)]
pub struct KernelInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub precompiled_package: Option<&'static str>,
    pub headers_package: &'static str,
}

/// Look up one of a distribution's kernels by package name.
pub fn get_kernel_info(distro: &'static Distribution, name: &str) -> Option<&'static KernelInfo> {
    distro.kernels.iter().find(|k| k.name == name)
}

pub fn get_zfs_packages(
    distro: &'static Distribution,
    kernel: &str,
    mode: ZfsModuleMode,
) -> Vec<String> {
    let mut packages = vec!["zfs-utils".to_string()];
    packages.extend(zfs_module_packages(distro, kernel, mode));
    packages
}

/// The packages that provide a ZFS module for `kernel`, without `zfs-utils`
/// (which is shared by every kernel and installed once).
///
/// Empty for an unknown kernel: there is nothing sensible to install for a
/// name the installer does not recognise.
///
/// The precompiled packages are version-locked to their kernel — `zfs-linux`
/// depends on `linux=7.1.8.arch1-3`, not on any `linux` — so one can be
/// unavailable while another is fine. They do not conflict with each other, so
/// several kernels can each have their own. `zfs-dkms` is the exception: it
/// conflicts with every precompiled package, which is why DKMS is a
/// system-wide choice rather than a per-kernel fallback.
pub fn zfs_module_packages(
    distro: &'static Distribution,
    kernel: &str,
    mode: ZfsModuleMode,
) -> Vec<String> {
    let Some(info) = get_kernel_info(distro, kernel) else {
        return Vec::new();
    };

    match mode {
        ZfsModuleMode::Precompiled => match info.precompiled_package {
            Some(pkg) => vec![pkg.to_string()],
            None => vec!["zfs-dkms".to_string(), info.headers_package.to_string()],
        },
        ZfsModuleMode::Dkms => {
            vec!["zfs-dkms".to_string(), info.headers_package.to_string()]
        }
    }
}

pub fn supports_precompiled(distro: &'static Distribution, kernel: &str) -> bool {
    get_kernel_info(distro, kernel)
        .and_then(|k| k.precompiled_package)
        .is_some()
}

/// Query a package version from the local pacman sync database using libalpm.
/// Returns None if the package is not found in any configured repo.
pub async fn query_package_version(package: &str) -> Result<Option<String>> {
    let versions = query_packages(&[package]).await?;
    Ok(versions.get(package).cloned())
}

/// Initialize an alpm handle from the system pacman.conf.
pub(crate) fn init_alpm() -> Result<alpm::Alpm> {
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

    // Sync databases so package queries return current data
    handle
        .syncdbs_mut()
        .update(false)
        .map_err(|e| color_eyre::eyre::eyre!("failed to sync databases: {e}"))?;

    Ok(handle)
}

/// Query multiple packages at once, returning a map of name -> version.
/// For ZFS packages (zfs-*), falls back to downloading archzfs.db directly
/// if the package isn't found in locally configured repos.
pub async fn query_packages(
    packages: &[&str],
) -> Result<std::collections::HashMap<String, String>> {
    // Phase 1: query local alpm database (sync — alpm is !Send)
    let packages_owned: Vec<String> = packages.iter().map(|s| s.to_string()).collect();
    let (mut result, missing_zfs) = tokio::task::spawn_blocking(move || -> Result<_> {
        let handle = init_alpm()?;
        let mut result = std::collections::HashMap::new();
        let mut missing_zfs = Vec::new();

        for pkg_name in &packages_owned {
            let mut found = false;
            for db in handle.syncdbs() {
                if let Ok(pkg) = db.pkg(pkg_name.as_bytes()) {
                    result.insert(pkg_name.to_string(), pkg.version().to_string());
                    found = true;
                    break;
                }
            }
            if !found && pkg_name.starts_with("zfs-") {
                missing_zfs.push(pkg_name.clone());
            }
        }
        Ok((result, missing_zfs))
    })
    .await??;

    // Phase 2: async HTTP fallback for missing ZFS packages
    if !missing_zfs.is_empty()
        && let Some(archzfs_versions) = fetch_archzfs_db_versions().await
    {
        for pkg_name in &missing_zfs {
            if let Some(ver) = archzfs_versions.get(pkg_name.as_str()) {
                tracing::debug!(
                    package = pkg_name,
                    version = ver,
                    "found ZFS package version from archzfs.db fallback"
                );
                result.insert(pkg_name.to_string(), ver.clone());
            }
        }
    }

    Ok(result)
}

/// HTTP client for the small metadata fetches the compatibility scan makes.
///
/// Both a connect and an overall timeout: these run on the welcome screen and
/// ahead of the install, where a captive portal or a black-holed route would
/// otherwise leave the scan waiting forever. The payloads are small, so a
/// bounded total is safe here in a way it would not be for package downloads.
pub(crate) fn metadata_http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("archinstall-zfs-rs")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

/// Download and parse the archzfs package database to get ZFS package versions.
/// This works even when archzfs repo isn't configured locally (e.g., before
/// add_archzfs_repo is called, or in CI environments).
async fn fetch_archzfs_db_versions() -> Option<std::collections::HashMap<String, String>> {
    let url = "https://github.com/archzfs/archzfs/releases/download/experimental/archzfs.db";
    tracing::debug!("downloading archzfs.db from {url}");

    let resp = metadata_http_client().ok()?.get(url).send().await.ok()?;
    let data = resp.bytes().await.ok()?;

    // archzfs.db is an XZ-compressed tar archive
    let mut decompressed = Vec::new();
    lzma_rs::xz_decompress(&mut std::io::Cursor::new(&data), &mut decompressed).ok()?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(decompressed));

    let mut versions = std::collections::HashMap::new();
    for entry in archive.entries().ok()? {
        let mut entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = match entry.path() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        // Look for desc files: "zfs-linux-lts-2.3.3_6.12.41.1-1/desc"
        if !path.ends_with("/desc") {
            continue;
        }
        let mut content = String::new();
        if std::io::Read::read_to_string(&mut entry, &mut content).is_err() {
            continue;
        }
        // Parse %NAME% and %VERSION% from desc format
        let mut name = None;
        let mut version = None;
        let mut section = "";
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('%') && trimmed.ends_with('%') {
                section = trimmed;
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            match section {
                "%NAME%" => name = Some(trimmed.to_string()),
                "%VERSION%" => version = Some(trimmed.to_string()),
                _ => {}
            }
        }
        if let (Some(n), Some(v)) = (name, version) {
            versions.insert(n, v);
        }
    }

    tracing::debug!(count = versions.len(), "parsed archzfs.db");
    Some(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_kernel_info() {
        let info = get_kernel_info(crate::distro::default(), "linux-lts").unwrap();
        assert_eq!(info.display_name, "Linux LTS");
        assert_eq!(info.precompiled_package, Some("zfs-linux-lts"));
    }

    #[test]
    fn test_unknown_kernel() {
        assert!(get_kernel_info(crate::distro::default(), "linux-custom").is_none());
    }

    #[test]
    fn test_get_zfs_packages_precompiled() {
        let pkgs = get_zfs_packages(
            crate::distro::default(),
            "linux-lts",
            ZfsModuleMode::Precompiled,
        );
        assert!(pkgs.contains(&"zfs-utils".to_string()));
        assert!(pkgs.contains(&"zfs-linux-lts".to_string()));
        assert!(!pkgs.contains(&"zfs-dkms".to_string()));
    }

    #[test]
    fn test_get_zfs_packages_dkms() {
        let pkgs = get_zfs_packages(crate::distro::default(), "linux", ZfsModuleMode::Dkms);
        assert!(pkgs.contains(&"zfs-utils".to_string()));
        assert!(pkgs.contains(&"zfs-dkms".to_string()));
        assert!(pkgs.contains(&"linux-headers".to_string()));
    }

    #[test]
    fn test_supports_precompiled() {
        assert!(supports_precompiled(crate::distro::default(), "linux-lts"));
        assert!(supports_precompiled(crate::distro::default(), "linux"));
        assert!(!supports_precompiled(
            crate::distro::default(),
            "linux-custom"
        ));
    }

    // Integration test: only runs on Arch with synced pacman DB
    #[tokio::test]
    async fn test_query_package_version_on_arch() {
        if !std::path::Path::new("/var/lib/pacman/sync").exists() {
            return; // Skip on non-Arch
        }
        let ver = query_package_version("linux-lts").await;
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
