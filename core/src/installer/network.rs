use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::CommandRunner;

/// Carry the live medium's networking into the target: systemd-networkd's
/// configuration, iwd's saved Wi-Fi profiles and a resolved-backed
/// `resolv.conf`.
///
/// Returns whether Wi-Fi profiles were copied, so the caller can install the
/// daemon that reads them: the base system does not carry iwd, and enabling
/// it without the package left the copied passphrases in an installed
/// system that could not use them.
pub fn copy_iso_network(runner: &dyn CommandRunner, target: &Path) -> Result<bool> {
    // Copy systemd-networkd configs
    let src_networkd = Path::new("/etc/systemd/network");
    let dst_networkd = target.join("etc/systemd/network");
    if src_networkd.exists() {
        copy_dir_contents(src_networkd, &dst_networkd)
            .wrap_err("failed to copy systemd-networkd configs")?;
        generalise_interface_names(&dst_networkd);
    }

    // Copy iwd configs (wifi)
    let src_iwd = Path::new("/var/lib/iwd");
    let dst_iwd = target.join("var/lib/iwd");
    let mut wifi = false;
    if src_iwd.exists() {
        copy_dir_contents(src_iwd, &dst_iwd).wrap_err("failed to copy iwd configs")?;
        wifi = dst_iwd.exists();
    }

    // Create resolv.conf symlink
    let resolv = target.join("etc/resolv.conf");
    let _ = fs::remove_file(&resolv);
    std::os::unix::fs::symlink("/run/systemd/resolve/stub-resolv.conf", &resolv)?;

    // Enable services. The helper already treats a non-zero exit as a
    // warning; a systemctl that cannot be run at all is ignored here as well.
    for service in ["systemd-networkd", "systemd-resolved"] {
        let _ = super::services::enable_service(runner, target, service);
    }

    tracing::info!(wifi, "copied ISO network configuration");
    Ok(wifi)
}

/// Match interfaces by kind rather than by name in the copied units.
///
/// The medium's configuration names the interfaces its own kernel found;
/// the installed kernel, with different modules and firmware, can name the
/// same card differently, and a `.network` that matches nothing leaves the
/// machine offline. `Name=enp0s31f6` therefore becomes `Name=en*`.
fn generalise_interface_names(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "network") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let updated = generalise_match_names(&content);
        if updated != content && fs::write(&path, &updated).is_ok() {
            tracing::info!(file = %path.display(), "matched interfaces by kind, not by name");
        }
    }
}

/// Rewrite the `Name=` values of a unit's `[Match]` section to the wildcard
/// for their kind. Names that are already patterns are left alone, as is
/// every other line.
fn generalise_match_names(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_match = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_match = trimmed.eq_ignore_ascii_case("[Match]");
        }
        let rewritten = in_match
            .then(|| trimmed.strip_prefix("Name="))
            .flatten()
            .map(|names| {
                let patterns: Vec<String> = names
                    .split_whitespace()
                    .map(|name| match name {
                        n if n.contains(['*', '?', '[']) => n.to_string(),
                        n => match n.get(..2) {
                            // en (ethernet), wl (wireless), ww (wwan) are
                            // the predictable prefixes systemd assigns.
                            Some(prefix @ ("en" | "wl" | "ww")) => format!("{prefix}*"),
                            _ => n.to_string(),
                        },
                    })
                    .collect();
                format!("Name={}", patterns.join(" "))
            });
        out.push_str(&rewritten.unwrap_or_else(|| line.to_string()));
        out.push('\n');
    }
    out
}

fn copy_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    let options = fs_extra::dir::CopyOptions::new()
        .content_only(true)
        .copy_inside(true);
    fs_extra::dir::copy(src, dst, &options)
        .map_err(|e| color_eyre::eyre::eyre!("copy_dir failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_dir_contents() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        // Create file and subdirectory
        fs::write(src.path().join("test.conf"), "content").unwrap();
        fs::create_dir_all(src.path().join("subdir")).unwrap();
        fs::write(src.path().join("subdir/nested.conf"), "nested").unwrap();

        let out = dst.path().join("out");
        copy_dir_contents(src.path(), &out).unwrap();

        assert!(out.join("test.conf").exists());
        assert!(out.join("subdir/nested.conf").exists());
        assert_eq!(
            fs::read_to_string(out.join("subdir/nested.conf")).unwrap(),
            "nested"
        );
    }
}

#[cfg(test)]
mod match_tests {
    use super::generalise_match_names;

    #[test]
    fn concrete_names_become_their_kind() {
        let unit = "[Match]\nName=enp0s31f6\n\n[Network]\nDHCP=yes\n";
        let out = generalise_match_names(unit);
        assert!(out.contains("Name=en*"), "{out}");
        assert!(out.contains("DHCP=yes"), "{out}");
    }

    #[test]
    fn wireless_and_several_names_are_handled() {
        let out = generalise_match_names("[Match]\nName=wlan0 enp3s0\n");
        assert_eq!(out, "[Match]\nName=wl* en*\n");
    }

    #[test]
    fn patterns_and_other_sections_are_left_alone() {
        let unit = "[Match]\nName=en*\n\n[Link]\nName=custom0\n";
        assert_eq!(generalise_match_names(unit), unit);
    }
}
