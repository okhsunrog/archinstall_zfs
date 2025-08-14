"""
Enhanced kernel management system for ZFS installations.

This module provides a unified approach to managing kernel variants and their
associated ZFS packages, with proper fallback logic and extensibility.
"""

from .fallback import EnhancedZFSInstaller, FallbackStrategy
from .package_manager import InstallationResult, ZFSPackageManager
from .registry import KernelRegistry, get_kernel_registry
from .variants import KernelVariant

__all__ = [
    "EnhancedZFSInstaller",
    "FallbackStrategy",
    "InstallationResult",
    "KernelRegistry",
    "KernelVariant",
    "ZFSPackageManager",
    "get_kernel_registry",
]
