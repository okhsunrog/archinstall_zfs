//! Line-based edits to shell-style and pacman configuration files.
//!
//! These files are small and regular enough that a line at a time is safer
//! than a parser: nothing outside the line being changed is reformatted, so
//! the user's comments and the package's examples survive an installation.

use std::path::Path;

use color_eyre::eyre::{Result, bail};

/// Rewrite a `KEY=(a b c)` array assignment in an mkinitcpio.conf.
///
/// Errors when the assignment cannot be parsed rather than falling back to an
/// empty array. Silently emptying `HOOKS` produces a `HOOKS=(zfs)` initramfs
/// that cannot mount root, and the installation would report success — a
/// failure here is recoverable, an unbootable system is not.
///
/// Where the same key is assigned more than once the last assignment is the
/// one the shell would use, so that is the one patched.
pub(crate) fn patch_conf_array(
    content: &str,
    key: &str,
    f: impl FnOnce(&mut Vec<String>),
) -> Result<String> {
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

/// Set `KEY="value"` in a shell-style configuration file.
pub(crate) fn set_conf_value(content: &str, key: &str, value: &str) -> String {
    set_conf_line(content, key, &format!("{key}=\"{value}\""))
}

/// Replace every active `KEY=` assignment with `line`. When the key is only
/// present as a commented example (stock files list several), activate the
/// first one and leave the other examples as they are. Append when absent.
pub(crate) fn set_conf_line(content: &str, key: &str, line: &str) -> String {
    let prefix = format!("{key}=");
    let commented = format!("#{prefix}");
    let has_active = content.lines().any(|l| l.trim().starts_with(&prefix));
    let mut result = String::new();
    let mut found = false;

    for l in content.lines() {
        let trimmed = l.trim();
        let replace = if has_active {
            trimmed.starts_with(&prefix)
        } else {
            !found && trimmed.starts_with(&commented)
        };
        if replace {
            found = true;
            result.push_str(line);
        } else {
            result.push_str(l);
        }
        result.push('\n');
    }

    if !found {
        result.push_str(line);
        result.push('\n');
    }

    result
}

/// Set a key in pacman.conf's `[options]`, adding it when absent.
///
/// Both keys this is used for appear only in that section, so a line-based
/// replacement is enough and does not need a full parser.
pub(crate) fn set_option(content: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut replaced = false;
    let mut result: Vec<String> = content
        .lines()
        .map(|existing| {
            let trimmed = existing.trim_start().trim_start_matches('#');
            if trimmed.starts_with(&format!("{key} ")) || trimmed.starts_with(&format!("{key}=")) {
                replaced = true;
                line.clone()
            } else {
                existing.to_string()
            }
        })
        .collect();

    if !replaced {
        // Straight after [options], which every pacman.conf opens with.
        let at = result
            .iter()
            .position(|l| l.trim() == "[options]")
            .map(|i| i + 1)
            .unwrap_or(0);
        result.insert(at, line);
    }

    let mut out = result.join("\n");
    out.push('\n');
    out
}

/// Set `ParallelDownloads` in pacman.conf: the medium's own when `target` is
/// `None`, the installed system's when given.
///
/// Unlike [`set_option`], a missing key is appended at the end of the file
/// rather than placed under `[options]`.
pub(crate) fn set_parallel_downloads(target: Option<&Path>, count: u32) -> Result<()> {
    let pacman_conf = match target {
        Some(t) => t.join("etc/pacman.conf"),
        None => std::path::PathBuf::from("/etc/pacman.conf"),
    };

    let content = std::fs::read_to_string(&pacman_conf)?;
    let new_line = format!("ParallelDownloads = {count}");

    let new_content = if content.contains("ParallelDownloads") {
        content
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("ParallelDownloads")
                    || line.trim_start().starts_with("#ParallelDownloads")
                {
                    new_line.as_str()
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        format!("{content}\n{new_line}\n")
    };

    std::fs::write(&pacman_conf, new_content)?;
    Ok(())
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
    fn test_set_conf_value() {
        let input = "#COMPRESSION=\"zstd\"\n";
        let result = set_conf_value(input, "COMPRESSION", "cat");
        assert!(result.contains("COMPRESSION=\"cat\""));
        assert!(!result.contains("#COMPRESSION"));
    }

    #[test]
    fn test_set_conf_line_activates_one_example_and_replaces_active_values() {
        let examples = "#COMPRESSION=\"zstd\"\n#COMPRESSION=\"xz\"\n#COMPRESSION_OPTIONS=()\n";
        assert_eq!(
            set_conf_line(examples, "COMPRESSION", "COMPRESSION=\"xz\""),
            "COMPRESSION=\"xz\"\n#COMPRESSION=\"xz\"\n#COMPRESSION_OPTIONS=()\n"
        );
        assert_eq!(
            set_conf_line(
                "#COMPRESSION=\"zstd\"\nCOMPRESSION=\"lz4\"\n",
                "COMPRESSION",
                "COMPRESSION=\"xz\""
            ),
            "#COMPRESSION=\"zstd\"\nCOMPRESSION=\"xz\"\n"
        );
        assert_eq!(
            set_conf_line(
                "HOOKS=(base)\n",
                "COMPRESSION_OPTIONS",
                "COMPRESSION_OPTIONS=(-9)"
            ),
            "HOOKS=(base)\nCOMPRESSION_OPTIONS=(-9)\n"
        );
    }

    #[test]
    fn an_option_is_replaced_in_place() {
        let conf = "[options]\nArchitecture = x86_64\nParallelDownloads = 5\n\n[core]\n";

        let result = set_option(conf, "Architecture", "auto");

        assert!(result.contains("Architecture = auto"));
        assert!(!result.contains("Architecture = x86_64"));
        assert!(result.contains("ParallelDownloads = 5"), "others untouched");
    }

    #[test]
    fn a_commented_option_is_taken_over() {
        let conf = "[options]\n#Architecture = auto\n\n[core]\n";

        let result = set_option(conf, "Architecture", "auto");

        assert_eq!(result.matches("Architecture").count(), 1);
        assert!(!result.contains('#'));
    }

    #[test]
    fn a_missing_option_is_added_under_options() {
        let conf = "[options]\nHoldPkg = pacman\n\n[core]\nSigLevel = Required\n";

        let result = set_option(conf, "Architecture", "auto");

        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "[options]");
        assert_eq!(
            lines[1], "Architecture = auto",
            "must land inside [options]"
        );
    }

    #[test]
    fn test_set_parallel_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("etc/pacman.conf");
        std::fs::create_dir_all(conf_path.parent().unwrap()).unwrap();
        std::fs::write(&conf_path, "#ParallelDownloads = 5\n").unwrap();

        set_parallel_downloads(Some(dir.path()), 10).unwrap();

        let content = std::fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("ParallelDownloads = 10"));
        assert!(!content.contains("#ParallelDownloads"));
    }
}
