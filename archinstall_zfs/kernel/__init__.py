"""
Enhanced kernel management system for ZFS installations.

This module provides a unified approach to managing kernel variants and their
associated ZFS packages, with proper fallback logic and extensibility.
"""

from .registry import KernelRegistry, get_kernel_registry
from .variants import KernelVariant
from .package_manager import ZFSPackageManager, InstallationResult
from .fallback import FallbackStrategy, EnhancedZFSInstaller

__all__ = [
    "KernelRegistry",
    "get_kernel_registry", 
    "KernelVariant",
    "ZFSPackageManager",
    "InstallationResult",
    "FallbackStrategy",
    "EnhancedZFSInstaller",
]