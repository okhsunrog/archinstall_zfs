"""
Kernel variant definitions and configuration.

This module defines the KernelVariant dataclass that encapsulates all
information needed to manage a specific kernel variant and its associated packages.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass
class KernelVariant:
    """Configuration for a specific kernel variant.

    This class encapsulates all the information needed to manage a kernel variant,
    including its packages, ZFS module support, and display information.
    """

    name: str  # e.g., "linux-lts"
    display_name: str  # e.g., "Linux LTS"
    kernel_package: str  # e.g., "linux-lts"
    headers_package: str  # e.g., "linux-lts-headers"
    zfs_precompiled_package: str | None  # e.g., "zfs-linux-lts" or None
    supports_precompiled: bool  # Whether precompiled ZFS is available
    is_default: bool = False  # Whether this is the default choice

    def __post_init__(self) -> None:
        """Validate the kernel variant configuration."""
        if not self.name:
            raise ValueError("Kernel variant name cannot be empty")

        if not self.display_name:
            raise ValueError("Kernel variant display name cannot be empty")

        if not self.kernel_package:
            raise ValueError("Kernel package name cannot be empty")

        if not self.headers_package:
            raise ValueError("Headers package name cannot be empty")

        # If supports_precompiled is True, zfs_precompiled_package must be set
        if self.supports_precompiled and not self.zfs_precompiled_package:
            raise ValueError(f"Kernel variant {self.name} claims to support precompiled ZFS but no precompiled package is specified")

    def get_dkms_packages(self) -> list[str]:
        """Get the list of packages needed for DKMS installation."""
        return ["zfs-utils", "zfs-dkms", self.headers_package]

    def get_precompiled_packages(self) -> list[str]:
        """Get the list of packages needed for precompiled installation."""
        if not self.supports_precompiled or not self.zfs_precompiled_package:
            raise ValueError(f"Kernel variant {self.name} does not support precompiled ZFS")

        return ["zfs-utils", self.zfs_precompiled_package]

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for serialization."""
        return {
            "name": self.name,
            "display_name": self.display_name,
            "kernel_package": self.kernel_package,
            "headers_package": self.headers_package,
            "zfs_precompiled_package": self.zfs_precompiled_package,
            "supports_precompiled": self.supports_precompiled,
            "is_default": self.is_default,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> KernelVariant:
        """Create from dictionary for deserialization."""
        return cls(**data)

    def __str__(self) -> str:
        """String representation for debugging."""
        precompiled_status = "✓" if self.supports_precompiled else "✗"
        default_status = " (default)" if self.is_default else ""
        return f"{self.display_name} [{self.name}] - Precompiled: {precompiled_status}{default_status}"
