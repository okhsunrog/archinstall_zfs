from __future__ import annotations

from enum import Enum

# ZFS passphrase minimum length requirement
ZFS_PASSPHRASE_MIN_LENGTH = 8


class ZFSModuleMode(Enum):
    """ZFS module installation mode."""

    PRECOMPILED = "precompiled"
    DKMS = "dkms"
