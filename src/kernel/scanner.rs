use color_eyre::eyre::{Context, Result};

/// Result of compatibility check for a single kernel.
#[derive(Debug, Clone)]
pub struct CompatibilityResult {
    pub kernel_name: String,
    pub kernel_version: Option<String>,
    pub dkms_compatible: bool,
    pub dkms_warnings: Vec<String>,
    pub precompiled_compatible: bool,
    pub precompiled_version: Option<String>,
    pub precompiled_warnings: Vec<String>,
}

/// Scan all known kernels for ZFS compatibility using libalpm.
pub fn scan_all_kernels() -> Vec<CompatibilityResult> {
    super::AVAILABLE_KERNELS
        .iter()
        .map(|k| scan_kernel(k.name))
        .collect()
}

/// Scan a single kernel for ZFS compatibility.
pub fn scan_kernel(kernel: &str) -> CompatibilityResult {
    let info = match super::get_kernel_info(kernel) {
        Some(i) => i,
        None => {
            return CompatibilityResult {
                kernel_name: kernel.to_string(),
                kernel_version: None,
                dkms_compatible: false,
                dkms_warnings: vec![format!("Unknown kernel: {kernel}")],
                precompiled_compatible: false,
                precompiled_version: None,
                precompiled_warnings: vec![format!("Unknown kernel: {kernel}")],
            };
        }
    };

    // Gather all packages we need to query
    let mut pkg_names: Vec<&str> = vec![kernel, "zfs-dkms", "zfs-utils"];
    if let Some(pre) = info.precompiled_package {
        pkg_names.push(pre);
    }

    // Query all at once via alpm
    let versions = match super::query_packages(&pkg_names) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(kernel, error = %e, "alpm query failed, assuming compatible");
            return CompatibilityResult {
                kernel_name: kernel.to_string(),
                kernel_version: None,
                dkms_compatible: true,
                dkms_warnings: vec![format!("Could not query packages: {e}")],
                precompiled_compatible: info.precompiled_package.is_some(),
                precompiled_version: None,
                precompiled_warnings: if info.precompiled_package.is_none() {
                    vec!["No precompiled package available".to_string()]
                } else {
                    vec![format!("Could not query packages: {e}")]
                },
            };
        }
    };

    let kernel_version = versions.get(kernel).cloned();

    // DKMS check: zfs-dkms must be available
    let (dkms_ok, dkms_warn) = check_dkms_compat(&versions);

    // Precompiled check: kernel version must match the version embedded in the ZFS package
    let (pre_ok, pre_ver, pre_warn) = check_precompiled_compat(info, &versions);

    CompatibilityResult {
        kernel_name: kernel.to_string(),
        kernel_version,
        dkms_compatible: dkms_ok,
        dkms_warnings: dkms_warn,
        precompiled_compatible: pre_ok,
        precompiled_version: pre_ver,
        precompiled_warnings: pre_warn,
    }
}

fn check_dkms_compat(versions: &std::collections::HashMap<String, String>) -> (bool, Vec<String>) {
    match versions.get("zfs-dkms") {
        Some(ver) => {
            tracing::debug!(version = ver, "zfs-dkms found");
            (true, vec![])
        }
        None => (false, vec!["zfs-dkms not found in repos".to_string()]),
    }
}

fn check_precompiled_compat(
    info: &super::KernelInfo,
    versions: &std::collections::HashMap<String, String>,
) -> (bool, Option<String>, Vec<String>) {
    let pre_pkg = match info.precompiled_package {
        Some(p) => p,
        None => {
            return (
                false,
                None,
                vec!["No precompiled package available".to_string()],
            );
        }
    };

    let pre_version = match versions.get(pre_pkg) {
        Some(v) => v.clone(),
        None => {
            return (false, None, vec![format!("{pre_pkg} not found in repos")]);
        }
    };

    let kernel_version = match versions.get(info.name) {
        Some(v) => v,
        None => {
            return (
                false,
                Some(pre_version),
                vec![format!("Kernel {} not found in repos", info.name)],
            );
        }
    };

    // Precompiled ZFS package version format: {zfs_ver}_{supported_kernel_ver}-{pkgrel}
    // e.g. "2.4.1_6.18.20.1-1" means it supports kernel version "6.18.20" with arch suffix
    let warnings = match validate_precompiled_version(&pre_version, kernel_version) {
        Ok(()) => vec![],
        Err(w) => vec![w],
    };

    let compatible = warnings.is_empty();
    (compatible, Some(pre_version), warnings)
}

/// Validate that a precompiled ZFS package version matches the kernel version.
/// ZFS precompiled version format: {zfs_version}_{kernel_version}-{pkgrel}
/// e.g. "2.4.1_6.18.20.1-1" supports kernel "6.18.20-1"
fn validate_precompiled_version(
    zfs_version: &str,
    kernel_version: &str,
) -> std::result::Result<(), String> {
    // Extract the kernel part from the ZFS version: everything after '_' and before the last '-'
    let after_underscore = match zfs_version.split('_').nth(1) {
        Some(s) => s,
        None => {
            return Err(format!(
                "Cannot parse ZFS version '{zfs_version}': no underscore separator"
            ));
        }
    };

    // The ZFS supported kernel part: "6.18.20.1-1"
    // The actual kernel version: "6.18.20-1"
    // We need to compare the base versions (ignoring the build suffix in ZFS version)

    let zfs_kernel_base = strip_pkgrel(after_underscore);
    let kernel_base = strip_pkgrel(kernel_version);

    // Try exact match first
    if zfs_kernel_base == kernel_base {
        return Ok(());
    }

    // Try stripping trailing build suffix from ZFS version
    // e.g., "6.18.20.1" -> "6.18.20"
    if let Some(stripped) = strip_build_suffix(&zfs_kernel_base) {
        if stripped == kernel_base {
            return Ok(());
        }
    }

    Err(format!(
        "Kernel version mismatch: ZFS built for '{zfs_kernel_base}', kernel is '{kernel_base}'"
    ))
}

/// Strip the package release suffix (-N) from a version string.
fn strip_pkgrel(version: &str) -> &str {
    match version.rsplit_once('-') {
        Some((base, _rel)) => base,
        None => version,
    }
}

/// Strip a trailing single-component build suffix.
/// "6.18.20.1" -> Some("6.18.20"), "6.18.20" -> None
fn strip_build_suffix(version: &str) -> Option<&str> {
    match version.rsplit_once('.') {
        Some((base, suffix)) if suffix.len() == 1 && suffix.chars().all(|c| c.is_ascii_digit()) => {
            Some(base)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_precompiled_exact_match() {
        assert!(validate_precompiled_version("2.4.1_6.18.20-1", "6.18.20-1").is_ok());
    }

    #[test]
    fn test_validate_precompiled_build_suffix() {
        // ZFS has build suffix .1, kernel doesn't
        assert!(validate_precompiled_version("2.4.1_6.18.20.1-1", "6.18.20-1").is_ok());
    }

    #[test]
    fn test_validate_precompiled_arch_suffix() {
        assert!(validate_precompiled_version("2.4.1_6.18.20.arch1.1-1", "6.18.20.arch1-1").is_ok());
    }

    #[test]
    fn test_validate_precompiled_mismatch() {
        let result = validate_precompiled_version("2.4.1_6.17.0-1", "6.18.20-1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[test]
    fn test_strip_pkgrel() {
        assert_eq!(strip_pkgrel("6.18.20-1"), "6.18.20");
        assert_eq!(strip_pkgrel("6.18.20.arch1-2"), "6.18.20.arch1");
        assert_eq!(strip_pkgrel("6.18.20"), "6.18.20");
    }

    #[test]
    fn test_strip_build_suffix() {
        assert_eq!(strip_build_suffix("6.18.20.1"), Some("6.18.20"));
        assert_eq!(strip_build_suffix("6.18.20.arch1.1"), Some("6.18.20.arch1"));
        assert_eq!(strip_build_suffix("6.18.20"), None); // "20" is too long for a build suffix
    }

    // Integration test: only on Arch with synced DB
    #[test]
    fn test_scan_kernel_on_arch() {
        if !std::path::Path::new("/var/lib/pacman/sync").exists() {
            return;
        }
        let result = scan_kernel("linux-lts");
        // Should at least not crash
        tracing::info!(?result, "scan result for linux-lts");
    }
}
