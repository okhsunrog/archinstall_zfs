use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result, bail};

use crate::system::cmd::{CommandRunner, check_exit, chroot_cmd};

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
    })?;

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
    })?;

    // Set COMPRESSION
    new_content = set_conf_value(&new_content, "COMPRESSION", "cat");

    // Add key file to FILES if encryption enabled
    if encryption {
        new_content = patch_conf_array(&new_content, "FILES", |files| {
            let key = "/etc/zfs/zroot.key".to_string();
            if !files.contains(&key) {
                files.push(key);
            }
        })?;
    }

    fs::write(&conf_path, new_content).wrap_err("failed to write mkinitcpio.conf")?;
    tracing::info!("configured mkinitcpio");
    Ok(())
}

pub fn generate(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let output = chroot_cmd(runner, target, "mkinitcpio", &["-P"])?;
    check_exit(&output, "mkinitcpio -P")?;
    tracing::info!("generated initramfs with mkinitcpio");
    Ok(())
}

/// Rewrite a `KEY=(a b c)` array assignment in an mkinitcpio.conf.
///
/// Errors when the assignment cannot be parsed rather than falling back to an
/// empty array. Silently emptying `HOOKS` produces a `HOOKS=(zfs)` initramfs
/// that cannot mount root, and the installation would report success — a
/// failure here is recoverable, an unbootable system is not.
///
/// Where the same key is assigned more than once the last assignment is the
/// one the shell would use, so that is the one patched.
fn patch_conf_array(content: &str, key: &str, f: impl FnOnce(&mut Vec<String>)) -> Result<String> {
    let prefix = format!("{key}=(");
    let mut lines: Vec<String> = Vec::new();
    let mut target_line: Option<usize> = None;
    let mut values: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) {
            let Some(inner) = trimmed
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(')'))
            else {
                bail!(
                    "cannot patch {key} in mkinitcpio.conf: the assignment on line {} does not \
                     close on the same line. Rewriting it would drop the existing entries and \
                     leave an unbootable initramfs; put {key}=(...) on one line and retry.",
                    lines.len() + 1
                );
            };
            target_line = Some(lines.len());
            values = inner.split_whitespace().map(|s| s.to_string()).collect();
        }
        lines.push(line.to_string());
    }

    f(&mut values);
    let new_line = format!("{key}=({})", values.join(" "));
    match target_line {
        Some(index) => lines[index] = new_line,
        None => lines.push(new_line),
    }

    let mut result = lines.join("\n");
    result.push('\n');
    Ok(result)
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
            if !hooks.contains(&"zfs".to_string())
                && let Some(pos) = hooks.iter().position(|h| h == "filesystems")
            {
                hooks.insert(pos, "zfs".to_string());
            }
        })
        .unwrap();
        assert!(result.contains("zfs filesystems"));
    }

    #[test]
    fn multi_line_array_is_rejected_instead_of_emptied() {
        // A HOOKS array split across lines used to parse as empty, so the
        // patched config kept only the hook being added — an initramfs with no
        // base, udev or block hooks, i.e. a system that cannot boot.
        let input = "MODULES=()\nHOOKS=(base udev autodetect\n       block filesystems fsck)\n";

        let err = patch_conf_array(input, "HOOKS", |hooks| hooks.push("zfs".to_string()))
            .expect_err("a multi-line array must not be silently rewritten");

        let msg = err.to_string();
        assert!(msg.contains("HOOKS"), "error should name the key: {msg}");
        assert!(msg.contains("line 2"), "error should locate it: {msg}");
    }

    #[test]
    fn configure_refuses_a_conf_it_cannot_patch_safely() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("etc/mkinitcpio.conf");
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        let original = "MODULES=()\nHOOKS=(base udev\n  block filesystems)\n";
        fs::write(&conf_path, original).unwrap();

        assert!(configure(dir.path(), false).is_err());
        // The original config must be left intact for the user to fix.
        assert_eq!(fs::read_to_string(&conf_path).unwrap(), original);
    }

    #[test]
    fn repeated_assignment_patches_the_one_the_shell_would_use() {
        let input = "HOOKS=(base udev)\nHOOKS=(base udev block filesystems)\n";

        let result = patch_conf_array(input, "HOOKS", |hooks| {
            let pos = hooks.iter().position(|h| h == "filesystems").unwrap();
            hooks.insert(pos, "zfs".to_string());
        })
        .unwrap();

        assert_eq!(
            result,
            "HOOKS=(base udev)\nHOOKS=(base udev block zfs filesystems)\n"
        );
    }

    #[test]
    fn missing_array_is_appended() {
        let result = patch_conf_array("COMPRESSION=\"cat\"\n", "FILES", |files| {
            files.push("/etc/zfs/zroot.key".to_string())
        })
        .unwrap();

        assert_eq!(result, "COMPRESSION=\"cat\"\nFILES=(/etc/zfs/zroot.key)\n");
    }

    #[test]
    fn commented_out_assignment_is_left_alone() {
        let result = patch_conf_array("#MODULES=(vfat)\n", "MODULES", |modules| {
            modules.push("zfs".to_string())
        })
        .unwrap();

        assert_eq!(result, "#MODULES=(vfat)\nMODULES=(zfs)\n");
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
