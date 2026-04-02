use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit, chroot};

use super::bootmenu::HOSTID_VALUE;

/// Custom ZED hook that filters zfs-list.cache to only include datasets from
/// the currently booted boot environment. This prevents systemd from mounting
/// datasets belonging to other BEs (e.g., /home from a neighboring BE).
/// Installed as immutable to prevent ZFS package updates from overwriting it.
const ZED_HISTORY_CACHER: &str = r#"#!/usr/bin/env python3
# ZED hook: history_event → boot environment aware zfs-list.cache updater
#
# Purpose:
# - On ZFS history events, regenerate /etc/zfs/zfs-list.cache/<pool> to include only:
#   - datasets belonging to the currently booted boot environment (BE), and
#   - shared datasets that are not part of any BE hierarchy.
# - This prevents mounts from other boot environments on the same pool, enabling
#   clean multi-OS/multi-BE setups and avoiding cross-environment mount issues.
# - Writes atomically with a lock and only updates the cache when content changes.
#
# Installed by the installer and marked immutable to avoid overwrites by zfs package updates.

import os
import sys
import subprocess
import fcntl

DEBUG = True

def log(message):
    if DEBUG:
        with open('/tmp/zed_debug.log', 'a') as log:
            log.write(f"{message}\n")

def get_current_root():
    """Find the current root ZFS dataset using multiple methods"""
    # Try /proc/mounts first
    try:
        with open('/proc/mounts', 'r') as f:
            for line in f:
                if ' / type zfs ' in line:
                    return line.split()[0]
    except:
        pass

    # Fallback to mount command
    try:
        result = subprocess.run(['mount'], capture_output=True, text=True)
        for line in result.stdout.split('\n'):
            if ' on / type zfs ' in line:
                return line.split()[0]
    except:
        pass

    # Second fallback to zfs mount
    try:
        result = subprocess.run(['zfs', 'mount'], capture_output=True, text=True)
        for line in result.stdout.split('\n'):
            if line.strip().endswith(' /'):
                return line.split()[0]
    except:
        pass

    return None

def get_dataset_props(pool):
    """Get all datasets and their properties"""
    props = [
        'name', 'mountpoint', 'canmount', 'atime', 'relatime', 'devices',
        'exec', 'readonly', 'setuid', 'nbmand', 'encroot', 'keylocation',
        'org.openzfs.systemd:requires', 'org.openzfs.systemd:requires-mounts-for',
        'org.openzfs.systemd:before', 'org.openzfs.systemd:after',
        'org.openzfs.systemd:wanted-by', 'org.openzfs.systemd:required-by',
        'org.openzfs.systemd:nofail', 'org.openzfs.systemd:ignore'
    ]
    cmd = ['zfs', 'list', '-H', '-t', 'filesystem', '-r', '-o', ','.join(props), pool]
    log(f"Running command: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    return [line.split('\t') for line in result.stdout.strip().split('\n')]

def find_boot_environments(datasets):
    """Identify boot environments by finding their root datasets"""
    boot_envs = set()
    for dataset in datasets:
        name, mountpoint = dataset[0], dataset[1]
        if mountpoint == '/':
            be = name.rsplit('/', 1)[0]
            boot_envs.add(be)
    return boot_envs

def is_part_of_be(dataset_name, boot_envs):
    """Check if dataset belongs to any boot environment"""
    return any(dataset_name.startswith(be) for be in boot_envs)

def filter_datasets(datasets, current_be, boot_envs):
    """Filter datasets to include current BE hierarchy and shared datasets"""
    filtered = []

    for dataset in datasets:
        name = dataset[0]
        if (name.startswith(current_be) or
            '/' not in name or  # pool itself
            not is_part_of_be(name, boot_envs)):  # shared dataset
            filtered.append(dataset)

    return filtered

def write_cache(datasets, cache_file, pool):
    """Write datasets to cache file, only update if content changed"""
    tmp_file = f"/var/run/zfs-list.cache@{pool}"
    log(f"Writing temporary cache file: {tmp_file}")

    with open(tmp_file, 'w') as f:
        for dataset in datasets:
            f.write('\t'.join(dataset) + '\n')

    try:
        with open(cache_file, 'r') as f:
            old_content = f.read()
        with open(tmp_file, 'r') as f:
            new_content = f.read()
        if old_content != new_content:
            log("Cache content changed, updating file")
            with open(cache_file, 'w') as f:
                f.write(new_content)
    except FileNotFoundError:
        log("No existing cache file, creating new one")
        with open(cache_file, 'w') as f:
            f.write(new_content)
    finally:
        os.remove(tmp_file)

def main():
    log("\n=== New ZED cache update started ===")

    if os.environ.get('ZEVENT_SUBCLASS') != 'history_event':
        log("Not a history event, exiting")
        sys.exit(0)

    pool = os.environ.get('ZEVENT_POOL')
    if not pool:
        log("No pool specified, exiting")
        sys.exit(0)
    log(f"Processing pool: {pool}")

    cache_file = f"/etc/zfs/zfs-list.cache/{pool}"
    if not os.access(cache_file, os.W_OK):
        log("Cache file not writable, exiting")
        sys.exit(0)

    lock_file = open(cache_file, 'a')
    try:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        log("Acquired file lock")

        current_root = get_current_root()
        if not current_root:
            log("Could not determine current root dataset, exiting")
            sys.exit(0)

        current_be = current_root.rsplit('/', 1)[0]
        log(f"Current boot environment: {current_be}")

        all_datasets = get_dataset_props(pool)
        log(f"Found {len(all_datasets)} total datasets")

        boot_envs = find_boot_environments(all_datasets)
        log(f"Identified boot environments: {boot_envs}")

        filtered_datasets = filter_datasets(all_datasets, current_be, boot_envs)
        log(f"Writing {len(filtered_datasets)} datasets to cache")

        write_cache(filtered_datasets, cache_file, pool)

    finally:
        fcntl.flock(lock_file, fcntl.LOCK_UN)
        lock_file.close()
        log("Released file lock")
        log("=== Cache update completed ===")

if __name__ == '__main__':
    main()
"#;

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
