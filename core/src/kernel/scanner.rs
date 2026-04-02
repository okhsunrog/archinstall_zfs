use std::collections::HashMap;

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

    // DKMS check: zfs-dkms must be available AND kernel must be in supported range
    let (dkms_ok, dkms_warn) = check_dkms_compat(&versions, kernel);

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

/// Validate a kernel/ZFS plan before installation.
/// Returns a list of warnings (empty = no issues).
pub fn validate_kernel_zfs_plan(
    kernel: &str,
    mode: crate::config::types::ZfsModuleMode,
) -> Vec<String> {
    let mut warnings = Vec::new();

    let info = match super::get_kernel_info(kernel) {
        Some(i) => i,
        None => {
            warnings.push(format!(
                "Unsupported kernel: {kernel}. Supported: {}",
                super::AVAILABLE_KERNELS
                    .iter()
                    .map(|k| k.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return warnings;
        }
    };

    if mode == crate::config::types::ZfsModuleMode::Precompiled
        && info.precompiled_package.is_none()
    {
        warnings.push(format!(
            "Precompiled ZFS not available for {kernel}, will use DKMS"
        ));
    }

    // Run the full compatibility scan
    let result = scan_kernel(kernel);
    match mode {
        crate::config::types::ZfsModuleMode::Precompiled => {
            if !result.precompiled_compatible {
                warnings.extend(result.precompiled_warnings);
            }
        }
        crate::config::types::ZfsModuleMode::Dkms => {
            if !result.dkms_compatible {
                warnings.extend(result.dkms_warnings);
            }
        }
    }

    warnings
}

// ── DKMS compatibility ──────────────────────────────

fn check_dkms_compat(versions: &HashMap<String, String>, kernel: &str) -> (bool, Vec<String>) {
    let dkms_ver = match versions.get("zfs-dkms") {
        Some(ver) => ver,
        None => return (false, vec!["zfs-dkms not found in repos".to_string()]),
    };

    let kernel_ver = match versions.get(kernel) {
        Some(ver) => ver,
        None => {
            return (
                false,
                vec![format!("Kernel {kernel} not found in repos")],
            );
        }
    };

    // Fetch kernel compatibility range from OpenZFS GitHub releases
    let base_zfs_ver = dkms_ver.split('-').next().unwrap_or(dkms_ver);
    match fetch_zfs_kernel_range(base_zfs_ver) {
        Some((min_ver, max_ver)) => {
            let kernel_base = kernel_ver.split('-').next().unwrap_or(kernel_ver);
            let kernel_parsed = parse_major_minor(kernel_base);
            let min_parsed = parse_major_minor(&min_ver);
            let max_parsed = parse_major_minor(&max_ver);

            if kernel_parsed >= min_parsed && kernel_parsed <= max_parsed {
                tracing::debug!(
                    kernel,
                    kernel_ver = kernel_base,
                    range = format!("{min_ver} - {max_ver}"),
                    "kernel is within ZFS DKMS supported range"
                );
                (true, vec![])
            } else {
                (
                    false,
                    vec![format!(
                        "Kernel {kernel} ({kernel_base}) is outside ZFS DKMS supported range ({min_ver} - {max_ver})"
                    )],
                )
            }
        }
        None => {
            // Can't fetch range — fall back to existence check (assume compatible)
            tracing::warn!(
                "Could not fetch ZFS kernel compatibility range from GitHub, assuming DKMS compatible"
            );
            (
                true,
                vec!["Could not verify DKMS kernel range (GitHub API unavailable)".to_string()],
            )
        }
    }
}

/// Fetch the supported kernel version range for a ZFS version from the
/// OpenZFS GitHub release notes.
/// Returns (min_kernel, max_kernel) or None if unavailable.
fn fetch_zfs_kernel_range(zfs_version: &str) -> Option<(String, String)> {
    let tag = format!("zfs-{zfs_version}");
    let url = format!("https://api.github.com/repos/openzfs/zfs/releases/tags/{tag}");

    tracing::debug!(url, "fetching ZFS kernel compatibility from GitHub");

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "archinstall-zfs-rs")
        .send()
        .ok()?;

    let data: serde_json::Value = resp.json().ok()?;
    let body = data.get("body")?.as_str()?;
    parse_kernel_range_from_release_notes(body)
}

/// Parse kernel compatibility range from OpenZFS release notes body.
/// Tries multiple patterns for robustness (same as Python version).
fn parse_kernel_range_from_release_notes(body: &str) -> Option<(String, String)> {
    let patterns = [
        // **Linux**: compatible with 6.1 - 6.15 kernels
        r"\*\*Linux\*\*:\s*compatible with\s+([\d.]+)\s*-\s*([\d.]+)\s*kernels",
        // Linux ... compatible with 6.1 - 6.15 kernels
        r"Linux.*?compatible with.*?([\d.]+)\s*-\s*([\d.]+)\s*kernels",
        // Kernel compatibility ... 6.1 - 6.15
        r"Kernel.*?compatibility.*?([\d.]+)\s*-\s*([\d.]+)",
        // Linux kernel 6.1 - 6.15
        r"Linux kernel.*?([\d.]+)\s*-\s*([\d.]+)",
    ];

    for pattern in &patterns {
        if let Some(caps) = regex::Regex::new(pattern).ok()?.captures(body) {
            let min = caps.get(1)?.as_str().to_string();
            let max = caps.get(2)?.as_str().to_string();
            tracing::debug!(min, max, "parsed ZFS kernel compatibility range");
            return Some((min, max));
        }
    }

    tracing::debug!("no kernel compatibility range found in release notes");
    None
}

/// Parse a version string into (major, minor) for range comparison.
/// Normalizes to major.minor only — patch versions don't matter for
/// DKMS range checking (6.15.9 is compatible with 6.15).
fn parse_major_minor(version: &str) -> (u32, u32) {
    // Strip non-numeric suffixes (e.g., "6.18.arch1" -> "6.18")
    let clean: String = version
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts: Vec<u32> = clean
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    (major, minor)
}

// ── Precompiled compatibility ───────────────────────

fn check_precompiled_compat(
    info: &super::KernelInfo,
    versions: &HashMap<String, String>,
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
    let after_underscore = match zfs_version.split('_').nth(1) {
        Some(s) => s,
        None => {
            return Err(format!(
                "Cannot parse ZFS version '{zfs_version}': no underscore separator"
            ));
        }
    };

    let zfs_kernel_base = strip_pkgrel(after_underscore);
    let kernel_base = strip_pkgrel(kernel_version);

    // Try exact match first
    if zfs_kernel_base == kernel_base {
        return Ok(());
    }

    // Try stripping trailing build suffix from ZFS version
    // e.g., "6.18.20.1" -> "6.18.20"
    if let Some(stripped) = strip_build_suffix(zfs_kernel_base) {
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
        Some((base, suffix))
            if suffix.len() == 1 && suffix.chars().all(|c| c.is_ascii_digit()) =>
        {
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
        assert!(validate_precompiled_version("2.4.1_6.18.20.1-1", "6.18.20-1").is_ok());
    }

    #[test]
    fn test_validate_precompiled_arch_suffix() {
        assert!(
            validate_precompiled_version("2.4.1_6.18.20.arch1.1-1", "6.18.20.arch1-1").is_ok()
        );
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
        assert_eq!(
            strip_build_suffix("6.18.20.arch1.1"),
            Some("6.18.20.arch1")
        );
        assert_eq!(strip_build_suffix("6.18.20"), None);
    }

    #[test]
    fn test_parse_major_minor() {
        assert_eq!(parse_major_minor("6.18.20"), (6, 18));
        assert_eq!(parse_major_minor("6.1"), (6, 1));
        assert_eq!(parse_major_minor("6.18.arch1"), (6, 18));
        assert_eq!(parse_major_minor("6"), (6, 0));
    }

    #[test]
    fn test_parse_kernel_range_from_release_notes() {
        let body = "## Changes\n**Linux**: compatible with 6.1 - 6.15 kernels\nSome other text";
        let result = parse_kernel_range_from_release_notes(body);
        assert_eq!(result, Some(("6.1".to_string(), "6.15".to_string())));
    }

    #[test]
    fn test_parse_kernel_range_alt_format() {
        let body = "Linux kernel 6.6 - 6.12 supported\nother stuff";
        let result = parse_kernel_range_from_release_notes(body);
        assert_eq!(result, Some(("6.6".to_string(), "6.12".to_string())));
    }

    #[test]
    fn test_parse_kernel_range_not_found() {
        let body = "No compatibility info here";
        assert!(parse_kernel_range_from_release_notes(body).is_none());
    }

    // Integration test: only on Arch with synced DB
    #[test]
    fn test_scan_kernel_on_arch() {
        if !std::path::Path::new("/var/lib/pacman/sync").exists() {
            return;
        }
        let result = scan_kernel("linux-lts");
        tracing::info!(?result, "scan result for linux-lts");
    }
}
