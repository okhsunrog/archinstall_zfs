from pathlib import Path

from pathlib import Path
from typing import List


def modify_zfs_cache_mountpoints(content: str, mountpoint: Path) -> str:
    """Modify ZFS cache mountpoints by replacing the temporary mountpoint prefix

    Args:
        content: Tab-separated ZFS cache content
        mountpoint: Temporary mountpoint to replace (e.g. /mnt)

    Returns:
        Modified cache content with correct final mountpoints
    """

    def process_mountpoint(path: str, prefix: str) -> str:
        if path == prefix:
            return '/'
        if path.startswith(prefix + '/'):
            return '/' + path[len(prefix) + 1:]
        return path

    mount_prefix = str(mountpoint).rstrip('/')
    lines = content.splitlines()
    modified_lines: List[str] = []

    for line in lines:
        fields = line.split('\t')
        if len(fields) > 1:
            fields[1] = process_mountpoint(fields[1], mount_prefix)
        modified_lines.append('\t'.join(fields))

    return '\n'.join(modified_lines)