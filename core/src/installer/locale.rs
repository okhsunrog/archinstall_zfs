use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit, chroot};

pub fn set_hostname(target: &Path, hostname: &str) -> Result<()> {
    let path = target.join("etc/hostname");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{hostname}\n")).wrap_err("failed to write hostname")?;
    tracing::info!(hostname, "set hostname");
    Ok(())
}

pub fn set_locale(runner: &dyn CommandRunner, target: &Path, locale: &str) -> Result<()> {
    // Uncomment the locale in locale.gen
    let locale_gen = target.join("etc/locale.gen");
    if locale_gen.exists() {
        let content = fs::read_to_string(&locale_gen)?;
        let uncommented = content.replace(&format!("#{locale}"), locale);
        fs::write(&locale_gen, uncommented)?;
    }

    // Run locale-gen
    let output = chroot(runner, target, "locale-gen")?;
    check_exit(&output, "locale-gen")?;

    // Write locale.conf
    let locale_conf = target.join("etc/locale.conf");
    let lang = locale.split_whitespace().next().unwrap_or(locale);
    fs::write(&locale_conf, format!("LANG={lang}\n")).wrap_err("failed to write locale.conf")?;

    tracing::info!(locale, "set locale");
    Ok(())
}

pub fn set_keyboard(_runner: &dyn CommandRunner, target: &Path, layout: &str) -> Result<()> {
    let vconsole = target.join("etc/vconsole.conf");
    if let Some(parent) = vconsole.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&vconsole, format!("KEYMAP={layout}\n")).wrap_err("failed to write vconsole.conf")?;
    tracing::info!(layout, "set keyboard layout");
    Ok(())
}

const TIMEZONE_REGIONS: &[&str] = &[
    "Africa",
    "America",
    "Antarctica",
    "Arctic",
    "Asia",
    "Atlantic",
    "Australia",
    "Europe",
    "Indian",
    "Pacific",
];

/// List available timezone regions (e.g. Europe, America, Asia).
pub fn list_timezone_regions() -> Vec<&'static str> {
    TIMEZONE_REGIONS.to_vec()
}

/// List cities/zones within a region by reading /usr/share/zoneinfo/<region>.
pub fn list_timezone_cities(region: &str) -> Vec<String> {
    let dir = Path::new("/usr/share/zoneinfo").join(region);
    let mut cities = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip non-files (symlinks to directories like posix/)
            if !path.is_file() {
                // Could be a subdirectory (e.g. America/Argentina/Buenos_Aires)
                if path.is_dir()
                    && let Some(subdir) = path.file_name().and_then(|n| n.to_str())
                    && let Ok(sub_entries) = fs::read_dir(&path)
                {
                    for sub_entry in sub_entries.flatten() {
                        if sub_entry.path().is_file()
                            && let Some(name) = sub_entry.file_name().to_str()
                        {
                            cities.push(format!("{subdir}/{name}"));
                        }
                    }
                }
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                cities.push(name.to_string());
            }
        }
    }
    cities.sort();
    cities
}

/// List available locales by parsing /etc/locale.gen.
/// Returns only UTF-8 locales (most common), sorted.
pub fn list_locales() -> Vec<String> {
    let path = Path::new("/etc/locale.gen");
    let mut locales = Vec::new();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            // Lines look like: #en_US.UTF-8 UTF-8
            let entry = line.strip_prefix('#').unwrap_or(line).trim();
            if entry.is_empty() || entry.starts_with('#') {
                continue;
            }
            if entry.contains("UTF-8") {
                locales.push(entry.to_string());
            }
        }
    }
    locales.sort();
    locales.dedup();
    locales
}

pub fn set_timezone(target: &Path, timezone: &str) -> Result<()> {
    let localtime = target.join("etc/localtime");
    let zoneinfo = format!("/usr/share/zoneinfo/{timezone}");
    let target_zoneinfo = target.join(format!("usr/share/zoneinfo/{timezone}"));
    if !target_zoneinfo.exists() {
        tracing::warn!(
            timezone,
            "zoneinfo file not found on target, symlink may be broken"
        );
    }

    // Remove existing symlink
    let _ = fs::remove_file(&localtime);

    if let Some(parent) = localtime.parent() {
        fs::create_dir_all(parent)?;
    }

    // Create relative symlink
    std::os::unix::fs::symlink(&zoneinfo, &localtime)
        .wrap_err_with(|| format!("failed to symlink timezone: {timezone}"))?;

    tracing::info!(timezone, "set timezone");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_hostname() {
        let dir = tempfile::tempdir().unwrap();
        set_hostname(dir.path(), "archbox").unwrap();

        let content = fs::read_to_string(dir.path().join("etc/hostname")).unwrap();
        assert_eq!(content, "archbox\n");
    }

    #[test]
    fn test_set_keyboard() {
        let dir = tempfile::tempdir().unwrap();
        let runner = crate::system::cmd::tests::RecordingRunner::new(vec![]);
        set_keyboard(&runner, dir.path(), "de-latin1").unwrap();

        let content = fs::read_to_string(dir.path().join("etc/vconsole.conf")).unwrap();
        assert_eq!(content, "KEYMAP=de-latin1\n");
    }

    #[test]
    fn test_set_timezone() {
        let dir = tempfile::tempdir().unwrap();
        // Create etc dir
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        set_timezone(dir.path(), "Europe/Berlin").unwrap();

        let localtime = dir.path().join("etc/localtime");
        assert!(localtime.is_symlink());
        let target = fs::read_link(&localtime).unwrap();
        assert_eq!(
            target.to_str().unwrap(),
            "/usr/share/zoneinfo/Europe/Berlin"
        );
    }
}
