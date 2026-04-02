use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit, chroot};

pub fn configure(target: &Path, encryption: bool) -> Result<()> {
    let conf_path = target.join("etc/mkinitcpio.conf");
    if !conf_path.exists() {
        tracing::warn!("mkinitcpio.conf not found, skipping configuration");
        return Ok(());
    }

    let content = fs::read_to_string(&conf_path)?;
    let mut new_content = content.clone();

    // Ensure zfs is in MODULES
    new_content = patch_conf_array(&new_content, "MODULES", |modules| {
        if !modules.contains(&"zfs".to_string()) {
            modules.push("zfs".to_string());
        }
    });

    // The archzfs `zfs` hook is a legacy (udev-based) hook, not compatible
    // with systemd-based initramfs. Replace systemd/sd-vconsole with udev/keymap
    // if present, then insert zfs before filesystems.
    new_content = patch_conf_array(&new_content, "HOOKS", |hooks| {
        // Replace systemd hooks with udev equivalents
        if hooks.contains(&"systemd".to_string()) {
            hooks.retain(|h| h != "systemd" && h != "sd-vconsole");
            if !hooks.contains(&"udev".to_string()) {
                if let Some(pos) = hooks.iter().position(|h| h == "base") {
                    hooks.insert(pos + 1, "udev".to_string());
                } else {
                    hooks.insert(0, "udev".to_string());
                }
            }
            if !hooks.contains(&"keymap".to_string()) {
                if let Some(pos) = hooks.iter().position(|h| h == "keyboard") {
                    hooks.insert(pos + 1, "keymap".to_string());
                } else if let Some(pos) = hooks.iter().position(|h| h == "udev") {
                    hooks.insert(pos + 1, "keymap".to_string());
                }
            }
        }
        // Insert zfs before filesystems
        if !hooks.contains(&"zfs".to_string()) {
            if let Some(pos) = hooks.iter().position(|h| h == "filesystems") {
                hooks.insert(pos, "zfs".to_string());
            } else {
                hooks.push("zfs".to_string());
            }
        }
    });

    // Set COMPRESSION
    new_content = set_conf_value(&new_content, "COMPRESSION", "cat");

    // Add key file to FILES if encryption enabled
    if encryption {
        new_content = patch_conf_array(&new_content, "FILES", |files| {
            let key = "/etc/zfs/zroot.key".to_string();
            if !files.contains(&key) {
                files.push(key);
            }
        });
    }

    fs::write(&conf_path, new_content).wrap_err("failed to write mkinitcpio.conf")?;
    tracing::info!("configured mkinitcpio");
    Ok(())
}

pub fn generate(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let output = chroot(runner, target, "mkinitcpio -P")?;
    check_exit(&output, "mkinitcpio -P")?;
    tracing::info!("generated initramfs with mkinitcpio");
    Ok(())
}

fn patch_conf_array(content: &str, key: &str, f: impl FnOnce(&mut Vec<String>)) -> String {
    let prefix = format!("{key}=(");
    let mut result = String::new();
    let mut found = false;
    let mut pending_values: Option<Vec<String>> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) && !trimmed.starts_with('#') {
            found = true;
            let inner = trimmed
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("");
            let values: Vec<String> = inner.split_whitespace().map(|s| s.to_string()).collect();
            pending_values = Some(values);
            // placeholder, will be replaced after loop
            result.push_str(&format!("__PLACEHOLDER_{key}__\n"));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    let mut values = pending_values.unwrap_or_default();
    f(&mut values);
    let new_line = format!("{key}=({})", values.join(" "));

    if found {
        result = result.replace(&format!("__PLACEHOLDER_{key}__"), &new_line);
    } else {
        result.push_str(&new_line);
        result.push('\n');
    }

    result
}

fn set_conf_value(content: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut result = String::new();
    let mut found = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) || trimmed.starts_with(&format!("#{prefix}")) {
            found = true;
            result.push_str(&format!("{key}=\"{value}\"\n"));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    if !found {
        result.push_str(&format!("{key}=\"{value}\"\n"));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_conf_array_adds_zfs() {
        let input = "MODULES=()\nHOOKS=(base udev autodetect modconf block filesystems fsck)\n";
        let result = patch_conf_array(input, "HOOKS", |hooks| {
            if !hooks.contains(&"zfs".to_string()) {
                if let Some(pos) = hooks.iter().position(|h| h == "filesystems") {
                    hooks.insert(pos, "zfs".to_string());
                }
            }
        });
        assert!(result.contains("zfs filesystems"));
    }

    #[test]
    fn test_configure_mkinitcpio() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("etc/mkinitcpio.conf");
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        fs::write(
            &conf_path,
            "MODULES=()\nHOOKS=(base udev autodetect modconf block filesystems fsck)\n#COMPRESSION=\"zstd\"\n",
        )
        .unwrap();

        configure(dir.path(), true).unwrap();

        let content = fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("MODULES=(zfs)"));
        assert!(content.contains("zfs filesystems"));
        assert!(content.contains("COMPRESSION=\"cat\""));
        assert!(content.contains("/etc/zfs/zroot.key"));
    }

    #[test]
    fn test_set_conf_value() {
        let input = "#COMPRESSION=\"zstd\"\n";
        let result = set_conf_value(input, "COMPRESSION", "cat");
        assert!(result.contains("COMPRESSION=\"cat\""));
        assert!(!result.contains("#COMPRESSION"));
    }
}
