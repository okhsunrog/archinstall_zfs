#!/usr/bin/env python3
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


