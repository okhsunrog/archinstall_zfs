use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit, chroot};

use super::bootmenu::HOSTID_VALUE;

/// Custom ZED hook that filters zfs-list.cache to only include datasets from
/// the currently booted boot environment. Prevents cross-BE mount issues.
/// Installed as immutable to prevent ZFS package updates from overwriting it.
const ZED_HISTORY_CACHER: &str = include_str!("../../../assets/history_event-zfs-list-cacher.sh");

pub fn create_hostid(runner: &dyn CommandRunner) -> Result<()> {
    let output = runner.run("zgenhostid", &["-f", HOSTID_VALUE])?;
    check_exit(&output, "zgenhostid")?;
    Ok(())
}

pub fn prepare_zfs_cache(target: &Path, pool_name: &str) -> Result<()> {
    let cache_dir = target.join("etc/zfs/zfs-list.cache");
    fs::create_dir_all(&cache_dir)
        .wrap_err_with(|| format!("failed to create cache dir: {}", cache_dir.display()))?;

    let cache_file = cache_dir.join(pool_name);
    if !cache_file.exists() {
        fs::write(&cache_file, "")
            .wrap_err_with(|| format!("failed to create cache file: {}", cache_file.display()))?;
    }
    Ok(())
}

pub fn copy_hostid(target: &Path) -> Result<()> {
    let src = Path::new("/etc/hostid");
    let dst = target.join("etc/hostid");
    if src.exists() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, &dst).wrap_err("failed to copy hostid to target")?;
    }
    Ok(())
}

pub fn copy_zfs_cache(target: &Path, pool_name: &str, mountpoint: &Path) -> Result<()> {
    let src_cache = Path::new("/etc/zfs/zfs-list.cache").join(pool_name);
    let dst_cache = target.join("etc/zfs/zfs-list.cache").join(pool_name);
    if src_cache.exists() {
        if let Some(parent) = dst_cache.parent() {
            fs::create_dir_all(parent)?;
        }
        // Read cache content and rewrite mountpoints: strip the temporary
        // mountpoint prefix (e.g. /mnt) so paths are correct on the target.
        let content = fs::read_to_string(&src_cache).wrap_err("failed to read ZFS cache file")?;
        let modified = rewrite_cache_mountpoints(&content, mountpoint);
        fs::write(&dst_cache, modified).wrap_err("failed to write ZFS cache to target")?;
    }
    Ok(())
}

/// Rewrite mountpoints in the zfs-list.cache file by stripping the temporary
/// mountpoint prefix. The cache is tab-separated; the second field is the
/// mountpoint. For example, `/mnt/home` becomes `/home`.
fn rewrite_cache_mountpoints(content: &str, mountpoint: &Path) -> String {
    let prefix = mountpoint.to_str().unwrap_or("/mnt").trim_end_matches('/');
    let mut result = Vec::new();
    for line in content.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() > 1 {
            let path = fields[1];
            let rewritten = if path == prefix {
                "/".to_string()
            } else if let Some(rest) = path.strip_prefix(&format!("{prefix}/")) {
                format!("/{rest}")
            } else {
                path.to_string()
            };
            // Rebuild the line with the rewritten mountpoint
            let owned_fields: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    if i == 1 {
                        rewritten.clone()
                    } else {
                        f.to_string()
                    }
                })
                .collect();
            result.push(owned_fields.join("\t"));
        } else {
            result.push(line.to_string());
        }
    }
    result.join("\n")
}

/// Install the custom boot-environment-aware ZED cache hook on the target.
/// This replaces the default zfs-list.cache updater with one that filters
/// datasets to only include the currently booted BE, preventing cross-BE
/// mount issues. The file is marked immutable to survive ZFS package updates.
pub fn install_zed_cache_hook(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let zed_dir = target.join("etc/zfs/zed.d");
    fs::create_dir_all(&zed_dir)?;

    let hook_path = zed_dir.join("history_event-zfs-list-cacher.sh");

    // Remove immutable flag if file already exists (e.g., from ZFS package)
    let _ = chroot(
        runner,
        target,
        "chattr -i /etc/zfs/zed.d/history_event-zfs-list-cacher.sh",
    );

    // Remove existing file
    if hook_path.exists() {
        fs::remove_file(&hook_path).wrap_err("failed to remove existing ZED hook")?;
    }

    // Write our custom hook
    fs::write(&hook_path, ZED_HISTORY_CACHER).wrap_err("failed to write ZED cache hook")?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
    }

    // Mark immutable so ZFS package updates don't overwrite it
    let output = chroot(
        runner,
        target,
        "chattr +i /etc/zfs/zed.d/history_event-zfs-list-cacher.sh",
    )?;
    if !output.success() {
        tracing::warn!("failed to set immutable flag on ZED hook (non-fatal)");
    }

    tracing::info!("installed custom ZED boot-environment-aware cache hook");
    Ok(())
}

pub fn copy_misc_files(
    runner: &dyn CommandRunner,
    target: &Path,
    pool_name: &str,
    mountpoint: &Path,
) -> Result<()> {
    copy_hostid(target)?;
    copy_zfs_cache(target, pool_name, mountpoint)?;
    install_zed_cache_hook(runner, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_create_hostid() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        create_hostid(&runner).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "zgenhostid");
        assert!(calls[0].args.contains(&HOSTID_VALUE.to_string()));
    }

    #[test]
    fn test_prepare_zfs_cache() {
        let dir = tempfile::tempdir().unwrap();
        prepare_zfs_cache(dir.path(), "testpool").unwrap();

        let cache_file = dir.path().join("etc/zfs/zfs-list.cache/testpool");
        assert!(cache_file.exists());
    }

    #[test]
    fn test_rewrite_cache_mountpoints() {
        let content = "zroot/arch0/root\t/mnt\ton\ton\ton\ton\toff\ton\toff\t-\t-\n\
                        zroot/arch0/data/home\t/mnt/home\ton\ton\ton\ton\toff\ton\toff\t-\t-\n\
                        zroot/arch0/data/root\t/mnt/root\ton\ton\ton\ton\toff\ton\toff\t-\t-\n\
                        zroot/arch0/vm\t/mnt/vm\ton\ton\ton\ton\toff\ton\toff\t-\t-";
        let result = rewrite_cache_mountpoints(content, Path::new("/mnt"));
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 4);
        // /mnt → /
        assert!(lines[0].contains("\t/\t"));
        // /mnt/home → /home
        assert!(lines[1].contains("\t/home\t"));
        // /mnt/root → /root
        assert!(lines[2].contains("\t/root\t"));
        // /mnt/vm → /vm
        assert!(lines[3].contains("\t/vm\t"));
    }

    #[test]
    fn test_rewrite_cache_mountpoints_preserves_other_fields() {
        let content = "zroot\tnone\ton\toff";
        let result = rewrite_cache_mountpoints(content, Path::new("/mnt"));
        // "none" doesn't start with /mnt, so it stays unchanged
        assert_eq!(result, "zroot\tnone\ton\toff");
    }

    #[test]
    fn test_install_zed_cache_hook() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // chattr -i (remove old immutable)
            CannedResponse::default(), // chattr +i (set immutable)
        ]);

        install_zed_cache_hook(&runner, dir.path()).unwrap();

        let hook_path = dir
            .path()
            .join("etc/zfs/zed.d/history_event-zfs-list-cacher.sh");
        assert!(hook_path.exists());

        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("boot environment aware"));
        assert!(content.contains("get_current_root"));
        assert!(content.contains("filter_datasets"));

        // Verify chattr +i was called
        let calls = runner.calls();
        let chattr_call = calls
            .iter()
            .find(|c| c.args.iter().any(|a| a.contains("chattr +i")))
            .expect("should call chattr +i");
        assert_eq!(chattr_call.program, "arch-chroot");
    }
}
