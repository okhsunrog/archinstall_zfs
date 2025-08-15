"""
Kernel/ZFS compatibility validation for archinstall_zfs.

This module provides archinstall-specific wrappers around the core validation
logic in validation_core.py, adding archinstall logging integration.
"""

import sys
from pathlib import Path

# Add project root to path to import validation_core
project_root = Path(__file__).parent.parent
if str(project_root) not in sys.path:
    sys.path.insert(0, str(project_root))

# Import the core validation logic after path setup
import validation_core  # noqa: E402

_core_get_package_version = validation_core.get_package_version
_core_fetch_zfs_kernel_compatibility = validation_core.fetch_zfs_kernel_compatibility
_core_validate_kernel_zfs_compatibility = validation_core.validate_kernel_zfs_compatibility
_core_get_compatible_kernels = validation_core.get_compatible_kernels
_core_should_filter_kernel_options = validation_core.should_filter_kernel_options

try:
    from archinstall import debug, warn

    _HAS_ARCHINSTALL = True
except ImportError:
    # Fallback when archinstall is not available
    def debug(msg: str) -> None:
        print(f"DEBUG: {msg}", file=sys.stderr)

    def warn(msg: str) -> None:
        print(f"WARN: {msg}", file=sys.stderr)

    _HAS_ARCHINSTALL = False


def get_package_version(package_name: str) -> str | None:
    """Gets the version of a package from the pacman sync database."""
    result = _core_get_package_version(package_name)
    if result:
        debug(f"Found {package_name} version: {result}")
    else:
        debug(f"Failed to get version for {package_name}")
    return result


def fetch_zfs_kernel_compatibility(zfs_version: str) -> tuple[str, str] | None:
    """Fetches OpenZFS release data from the GitHub API and parses the kernel compatibility range."""
    result = _core_fetch_zfs_kernel_compatibility(zfs_version)
    if result:
        debug(f"Found compatible kernel range via API: {result[0]} - {result[1]}")
    else:
        debug(f"No kernel compatibility information found for ZFS {zfs_version}")
    return result


def validate_kernel_zfs_compatibility(kernel_name: str, zfs_mode: str) -> tuple[bool, list[str]]:
    """Validates compatibility between a kernel and ZFS module mode."""
    debug(f"Validating {kernel_name} + {zfs_mode} compatibility")
    is_compatible, warnings = _core_validate_kernel_zfs_compatibility(kernel_name, zfs_mode)

    # Log warnings using archinstall's warn function
    for warning in warnings:
        warn(warning)

    if is_compatible:
        debug(f"Kernel {kernel_name} is compatible with ZFS {zfs_mode}")
    else:
        warn(f"Kernel {kernel_name} is NOT compatible with ZFS {zfs_mode}")

    return is_compatible, warnings


def get_compatible_kernels(kernel_names: list[str]) -> tuple[list[str], list[str]]:
    """Get lists of compatible and incompatible kernels for ZFS DKMS."""
    debug(f"Checking compatibility for kernels: {kernel_names}")
    compatible, incompatible = _core_get_compatible_kernels(kernel_names)
    debug(f"Compatible kernels: {compatible}")
    debug(f"Incompatible kernels: {incompatible}")
    return compatible, incompatible


def should_filter_kernel_options() -> bool:
    """Determines whether kernel options should be filtered based on compatibility."""
    should_filter = _core_should_filter_kernel_options()
    debug(f"Kernel filtering {'enabled' if should_filter else 'disabled'}")
    return should_filter
