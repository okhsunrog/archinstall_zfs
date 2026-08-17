#!/usr/bin/env python3
# ZED hook: history_event -> boot environment aware zfs-list.cache updater
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
import tempfile

# Set to True to trace every history event to /tmp/zed_debug.log. Off by
# default: this hook runs on every ZFS history event, so leaving it enabled
# grows an unrotated file in /tmp for the life of the system.
DEBUG = False

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
    except Exception:
        pass

    # Fallback to mount command
    try:
        result = subprocess.run(['mount'], capture_output=True, text=True)
        for line in result.stdout.split('\n'):
            if ' on / type zfs ' in line:
                return line.split()[0]
    except Exception:
        pass

    # Second fallback to zfs mount
    try:
        result = subprocess.run(['zfs', 'mount'], capture_output=True, text=True)
        for line in result.stdout.split('\n'):
            if line.strip().endswith(' /'):
                return line.split()[0]
    except Exception:
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
    return run_zfs_list(cmd)

def run_zfs_list(cmd):
    """Run a `zfs list` and return rows, or None if it produced nothing usable.

    Returning None rather than an empty list matters: the caller must leave the
    existing cache alone on failure. Overwriting it with nothing would strip
    every mount unit on the next boot.
    """
    try:
        result = subprocess.run(cmd, capture_output=True, text=True)
    except Exception as exc:
        log(f"Could not run zfs list: {exc}")
        return None
    if result.returncode != 0:
        log(f"zfs list failed (rc={result.returncode}): {result.stderr.strip()}")
        return None
    output = result.stdout.strip()
    if not output:
        log("zfs list returned no datasets")
        return None
    return [line.split('\t') for line in output.split('\n')]

def find_boot_environments(pool):
    """Identify boot environments by finding their root datasets.

    Queried separately from the cache rows on purpose: the cache column layout
    is dictated by zfs-mount-generator, so org.zfsbootmenu:active cannot simply
    be appended to it.

    ZFSBootMenu treats a filesystem as a boot environment when it has
    mountpoint=/ (unless org.zfsbootmenu:active=off hides it from the menu), or
    mountpoint=legacy together with org.zfsbootmenu:active=on. Datasets hidden
    from the menu still count here: hiding a boot environment does not make it
    safe to mount its /home underneath a different one.
    """
    rows = run_zfs_list([
        'zfs', 'list', '-H', '-t', 'filesystem', '-r',
        '-o', 'name,mountpoint,org.zfsbootmenu:active', pool,
    ])
    if rows is None:
        return None

    boot_envs = set()
    for row in rows:
        if len(row) < 3:
            continue
        name, mountpoint, active = row[0], row[1], row[2]
        if mountpoint == '/' or (mountpoint == 'legacy' and active == 'on'):
            boot_envs.add(name.rsplit('/', 1)[0])
    return boot_envs

def is_below(dataset_name, ancestor):
    """Dataset-path-aware prefix test.

    A plain str.startswith() would treat 'pool/arch10' as part of 'pool/arch1'
    and leak one boot environment's datasets into another's cache. Match only
    on a full dataset-name component boundary.
    """
    return dataset_name == ancestor or dataset_name.startswith(ancestor + '/')

def is_part_of_be(dataset_name, boot_envs):
    """Check if dataset belongs to any boot environment"""
    return any(is_below(dataset_name, be) for be in boot_envs)

def filter_datasets(datasets, current_be, boot_envs):
    """Filter datasets to include current BE hierarchy and shared datasets"""
    filtered = []

    for dataset in datasets:
        name = dataset[0]
        if (is_below(name, current_be) or
            '/' not in name or  # pool itself
            not is_part_of_be(name, boot_envs)):  # shared dataset
            filtered.append(dataset)

    return filtered

def write_cache(datasets, cache_file, pool):
    """Replace the cache file atomically, and only if the content changed."""
    new_content = ''.join('\t'.join(dataset) + '\n' for dataset in datasets)

    try:
        with open(cache_file, 'r') as f:
            if f.read() == new_content:
                log("Cache content unchanged, leaving file alone")
                return
    except FileNotFoundError:
        log("No existing cache file, creating new one")

    log("Cache content changed, updating file")
    # The temporary file must live in the cache directory: rename(2) is only
    # atomic within a single filesystem, and /run is not the same one as /etc.
    directory = os.path.dirname(cache_file) or '.'
    fd, tmp_file = tempfile.mkstemp(dir=directory, prefix='.' + pool + '.', suffix='.tmp')
    try:
        with os.fdopen(fd, 'w') as f:
            f.write(new_content)
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp_file, 0o644)
        os.replace(tmp_file, cache_file)
    except BaseException:
        # Never leave a partial cache behind for zfs-mount-generator to read.
        try:
            os.unlink(tmp_file)
        except FileNotFoundError:
            pass
        raise

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

    # Lock a dedicated file rather than the cache itself: the cache is now
    # replaced by rename(2), so a lock held on it would end up pinning an
    # unlinked inode and a concurrent zedlet could take the "same" lock.
    lock_file = open(f"/run/zfs-list.cache@{pool}.lock", 'w')
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
        if all_datasets is None:
            log("Could not enumerate datasets, leaving cache unchanged")
            sys.exit(0)
        log(f"Found {len(all_datasets)} total datasets")

        boot_envs = find_boot_environments(pool)
        if boot_envs is None:
            log("Could not identify boot environments, leaving cache unchanged")
            sys.exit(0)
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
