"""
Simple kernel management for ZFS installation.

This module provides a straightforward approach to handling kernel variants
and their ZFS packages, replacing the over-engineered registry system.
"""

from dataclasses import dataclass

from archinstall import error, info
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.lib.installer import Installer

from archinstall_zfs.shared import ZFSModuleMode


@dataclass
class KernelInfo:
    """Simple kernel information - no complex abstractions needed."""

    name: str
    display_name: str
    precompiled_package: str | None
    headers_package: str


# Single source of truth - based on actual archzfs packages
AVAILABLE_KERNELS = {
    "linux-lts": KernelInfo(name="linux-lts", display_name="Linux LTS", precompiled_package="zfs-linux-lts", headers_package="linux-lts-headers"),
    "linux": KernelInfo(name="linux", display_name="Linux", precompiled_package="zfs-linux", headers_package="linux-headers"),
    "linux-zen": KernelInfo(name="linux-zen", display_name="Linux Zen", precompiled_package="zfs-linux-zen", headers_package="linux-zen-headers"),
    "linux-hardened": KernelInfo(
        name="linux-hardened", display_name="Linux Hardened", precompiled_package="zfs-linux-hardened", headers_package="linux-hardened-headers"
    ),
}


def get_supported_kernels() -> list[str]:
    """Get list of supported kernel names."""
    return list(AVAILABLE_KERNELS.keys())


def get_kernel_info(kernel_name: str) -> KernelInfo:
    """Get kernel information, raising error if unsupported."""
    if kernel_name not in AVAILABLE_KERNELS:
        raise ValueError(f"Unsupported kernel: {kernel_name}. Supported: {list(AVAILABLE_KERNELS.keys())}")
    return AVAILABLE_KERNELS[kernel_name]


def get_kernel_display_name(kernel_name: str) -> str:
    """Get human-readable kernel name."""
    return get_kernel_info(kernel_name).display_name


def supports_precompiled_zfs(kernel_name: str) -> bool:
    """Check if kernel has precompiled ZFS packages available."""
    kernel_info = get_kernel_info(kernel_name)
    return kernel_info.precompiled_package is not None


def get_zfs_packages_for_kernel(kernel_name: str, mode: ZFSModuleMode) -> list[str]:
    """Get the ZFS packages needed for a kernel and mode."""
    kernel_info = get_kernel_info(kernel_name)

    if mode == ZFSModuleMode.PRECOMPILED:
        if not kernel_info.precompiled_package:
            raise ValueError(f"Kernel {kernel_name} does not support precompiled ZFS")
        return ["zfs-utils", kernel_info.precompiled_package]
    # DKMS
    return ["zfs-utils", "zfs-dkms", kernel_info.headers_package]


def install_zfs_packages(kernel_name: str, mode: ZFSModuleMode, installation: Installer | None = None) -> bool:
    """
    Install ZFS packages for the specified kernel and mode.

    Args:
        kernel_name: Name of the kernel (e.g., "linux-lts")
        mode: ZFS module mode (precompiled or DKMS)
        installation: Installer instance for target installation, None for host

    Returns:
        True if installation succeeded, False otherwise
    """
    try:
        packages = get_zfs_packages_for_kernel(kernel_name, mode)
        package_list = " ".join(packages)

        info(f"Installing ZFS packages for {kernel_name} ({mode.value}): {package_list}")

        if installation:
            # Install to target system
            installation.arch_chroot(f"pacman -S --noconfirm {package_list}")
        else:
            # Install to host system
            SysCommand(f"pacman -S --noconfirm {package_list}")

        info(f"Successfully installed ZFS packages for {kernel_name}")
        return True

    except (SysCallError, ValueError) as e:
        error(f"Failed to install ZFS packages for {kernel_name}: {e}")
        return False


def install_zfs_with_fallback(kernel_name: str, preferred_mode: ZFSModuleMode, installation: Installer | None = None) -> tuple[bool, ZFSModuleMode]:
    """
    Install ZFS with automatic fallback from precompiled to DKMS.

    Args:
        kernel_name: Name of the kernel
        preferred_mode: Preferred installation mode
        installation: Installer instance for target, None for host

    Returns:
        Tuple of (success, actual_mode_used)
    """
    kernel_info = get_kernel_info(kernel_name)

    # Try preferred mode first
    if preferred_mode == ZFSModuleMode.PRECOMPILED:
        if kernel_info.precompiled_package:
            if install_zfs_packages(kernel_name, ZFSModuleMode.PRECOMPILED, installation):
                return True, ZFSModuleMode.PRECOMPILED

            info(f"Precompiled ZFS failed for {kernel_name}, falling back to DKMS...")
        else:
            info(f"No precompiled ZFS available for {kernel_name}, using DKMS...")

    # Fallback to DKMS (always available)
    if install_zfs_packages(kernel_name, ZFSModuleMode.DKMS, installation):
        return True, ZFSModuleMode.DKMS

    return False, preferred_mode


def validate_kernel_zfs_plan(kernel_name: str, mode: ZFSModuleMode) -> list[str]:
    """
    Validate that a kernel/ZFS combination is possible.

    Returns:
        List of validation warnings (empty if no issues)
    """
    warnings = []

    try:
        kernel_info = get_kernel_info(kernel_name)
    except ValueError as e:
        warnings.append(str(e))
        return warnings

    if mode == ZFSModuleMode.PRECOMPILED and not kernel_info.precompiled_package:
        warnings.append(f"Precompiled ZFS not available for {kernel_name}, will use DKMS")

    return warnings


def get_menu_options() -> tuple[list[tuple[str, str, ZFSModuleMode]], list[str]]:
    """
    Get menu options for kernel/ZFS combinations with compatibility filtering.

    Returns:
        Tuple of (available_options, filtered_kernels)
        - available_options: List of (display_text, kernel_name, mode) tuples for compatible kernels
        - filtered_kernels: List of kernel display names that were filtered out due to incompatibility
    """
    # Import here to avoid circular imports and enable proper mocking in tests
    from archinstall_zfs.validation import get_compatible_kernels, should_filter_kernel_options  # noqa: PLC0415

    options = []
    filtered_kernels = []

    # Get compatibility information if filtering is enabled
    if should_filter_kernel_options():
        available_kernel_names = list(AVAILABLE_KERNELS.keys())
        compatible_kernels, incompatible_kernels = get_compatible_kernels(available_kernel_names)

        # Track which kernels were filtered for display
        for incompatible_kernel in incompatible_kernels:
            if incompatible_kernel in AVAILABLE_KERNELS:
                filtered_kernels.append(AVAILABLE_KERNELS[incompatible_kernel].display_name)
    else:
        # No filtering - all kernels are considered compatible
        compatible_kernels = list(AVAILABLE_KERNELS.keys())
        incompatible_kernels = []

    for kernel_name, kernel_info in AVAILABLE_KERNELS.items():
        # Add precompiled option if available (precompiled is always compatible)
        if kernel_info.precompiled_package:
            display = f"{kernel_info.display_name} + precompiled ZFS"
            if kernel_name == "linux-lts":
                display += " (recommended)"
            options.append((display, kernel_name, ZFSModuleMode.PRECOMPILED))

        # Add DKMS option only if kernel is compatible (or filtering is disabled)
        if kernel_name in compatible_kernels:
            display = f"{kernel_info.display_name} + ZFS DKMS"
            options.append((display, kernel_name, ZFSModuleMode.DKMS))

    return options, filtered_kernels
