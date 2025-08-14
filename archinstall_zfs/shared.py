from __future__ import annotations

from enum import Enum


class ZFSModuleMode(Enum):
    """ZFS module installation mode."""

    PRECOMPILED = "precompiled"
    DKMS = "dkms"
