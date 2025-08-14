"""
ZFS package management with intelligent fallback logic.

This module provides the ZFSPackageManager class that handles installation
of ZFS packages with proper fallback behavior that maintains kernel consistency.
"""

from __future__ import annotations

import re
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from archinstall import debug, info, warn
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand

from ..menu.models import ZFSModuleMode
from .registry import KernelRegistry
from .variants import KernelVariant


@dataclass
class InstallationResult:
    """Result of ZFS package installation."""

    kernel_variant: KernelVariant
    requested_mode: ZFSModuleMode
    actual_mode: ZFSModuleMode | None = None
    success: bool = False
    fallback_occurred: bool = False
    installed_packages: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)

    def add_error(self, error_msg: str) -> None:
        """Add an error message to the result."""
        self.errors.append(error_msg)
        debug(f"Installation error: {error_msg}")

    def get_summary(self) -> str:
        """Get human-readable installation summary."""
        if not self.success:
            error_summary = "; ".join(self.errors) if self.errors else "Unknown error"
            return f"Installation failed for {self.kernel_variant.name}: {error_summary}"

        mode_text = self.actual_mode.value if self.actual_mode else "unknown"
        fallback_text = " (after fallback)" if self.fallback_occurred else ""
        packages_text = f" - Packages: {', '.join(self.installed_packages)}"

        return f"Successfully installed ZFS {mode_text} for {self.kernel_variant.name}{fallback_text}{packages_text}"


class ZFSPackageSearcher:
    """Handles searching for ZFS packages in repositories."""

    def __init__(self) -> None:
        self.search_urls = ["https://github.com/archzfs/archzfs/releases/download/experimental/"]

    def search_zfs_package(self, package_name: str, version: str) -> tuple[str, str] | None:
        """Search for a specific ZFS package version.

        Args:
            package_name: Name of the ZFS package (e.g., "zfs-linux-lts")
            version: Kernel version to match

        Returns:
            Tuple of (url, package_filename) if found, None otherwise
        """
        pattern = f'{package_name}-[0-9][^"]*{version}[^"]*x86_64[^"]*'

        for url in self.search_urls:
            info(f"Searching {package_name} on {url}")
            try:
                response = SysCommand(f"curl -s {url}").decode()
                matches = re.findall(pattern, response)
                if matches:
                    package = matches[-1]  # Use latest match
                    return url, package
            except Exception as e:
                warn(f"Failed to search package at {url}: {e}")

        return None

    def extract_pkginfo(self, package_path: Path) -> str:
        """Extract zfs-utils version from package info.

        Args:
            package_path: Path to the package file

        Returns:
            The zfs-utils version string

        Raises:
            ValueError: If version cannot be extracted
        """
        try:
            pkginfo = SysCommand(f"bsdtar -qxO -f {package_path} .PKGINFO").decode()
            match = re.search(r"depend = zfs-utils=(.*)", pkginfo)
            if match:
                return match.group(1)
            raise ValueError("Could not find zfs-utils dependency in package info")
        except Exception as e:
            raise ValueError(f"Could not extract zfs-utils version from package info: {e}") from e


class ZFSPackageManager:
    """Manages ZFS package installation with proper fallback logic.

    This class handles the installation of ZFS packages, with intelligent
    fallback that maintains kernel consistency and provides detailed error reporting.
    """

    def __init__(self, kernel_registry: KernelRegistry) -> None:
        """Initialize the package manager.

        Args:
            kernel_registry: The kernel registry to use for variant lookup
        """
        self.kernel_registry = kernel_registry
        self.package_searcher = ZFSPackageSearcher()

    def install_zfs_packages(self, kernel_name: str, preferred_mode: ZFSModuleMode, installation: Any = None) -> InstallationResult:
        """Install ZFS packages with intelligent fallback.

        Key improvements:
        1. Maintains kernel consistency during fallback
        2. Supports precompiled for all kernel variants
        3. Proper error handling and reporting

        Args:
            kernel_name: Name of the kernel variant
            preferred_mode: Preferred ZFS module mode
            installation: Installation context (None for host system)

        Returns:
            InstallationResult with details of the installation attempt
        """
        variant = self.kernel_registry.get_variant(kernel_name)
        if not variant:
            result = InstallationResult(
                kernel_variant=KernelVariant(
                    name=kernel_name,
                    display_name=kernel_name,
                    kernel_package=kernel_name,
                    headers_package=f"{kernel_name}-headers",
                    zfs_precompiled_package=None,
                    supports_precompiled=False,
                ),
                requested_mode=preferred_mode,
            )
            result.add_error(f"Unsupported kernel: {kernel_name}")
            return result

        result = InstallationResult(kernel_variant=variant, requested_mode=preferred_mode)

        if preferred_mode == ZFSModuleMode.PRECOMPILED and variant.supports_precompiled:
            # Try precompiled installation
            if self._try_precompiled_install(variant, installation, result):
                result.actual_mode = ZFSModuleMode.PRECOMPILED
                result.success = True
                return result

            # Fallback to DKMS with SAME kernel variant
            info(f"Precompiled ZFS for {variant.name} failed, falling back to DKMS with {variant.name}")
            result.fallback_occurred = True

        # Install DKMS with the correct kernel variant
        if self._install_dkms(variant, installation, result):
            result.actual_mode = ZFSModuleMode.DKMS
            result.success = True

        return result

    def _try_precompiled_install(self, variant: KernelVariant, installation: Any, result: InstallationResult) -> bool:
        """Try to install precompiled ZFS packages.

        Args:
            variant: The kernel variant
            installation: Installation context (None for host system)
            result: Result object to update

        Returns:
            True if installation succeeded, False otherwise
        """
        try:
            if not variant.supports_precompiled or not variant.zfs_precompiled_package:
                result.add_error(f"Kernel variant {variant.name} does not support precompiled ZFS")
                return False

            packages = variant.get_precompiled_packages()

            # For host system installation, try to find matching packages
            if installation is None:
                return self._install_precompiled_host(variant, packages, result)
            # For target system installation, use pacman directly
            return self._install_packages(packages, installation, result)

        except Exception as e:
            result.add_error(f"Precompiled installation failed: {e}")
            return False

    def _install_precompiled_host(self, variant: KernelVariant, _packages: list[str], result: InstallationResult) -> bool:
        """Install precompiled packages on host system with package search.

        Args:
            variant: The kernel variant
            _packages: List of packages to install (unused, kept for compatibility)
            result: Result object to update

        Returns:
            True if installation succeeded, False otherwise
        """
        try:
            # Get running kernel version for package search
            kernel_version = SysCommand("uname -r").decode().strip()
            kernel_version_fixed = kernel_version.replace("-", ".")

            # Search for the precompiled ZFS package
            zfs_package = variant.zfs_precompiled_package
            if not zfs_package:
                result.add_error("No precompiled package specified")
                return False

            package_info = self.package_searcher.search_zfs_package(zfs_package, kernel_version_fixed)
            if not package_info:
                result.add_error(f"Precompiled package {zfs_package} not found for kernel {kernel_version}")
                return False

            url, package_filename = package_info
            package_url = f"{url}{package_filename}"

            info(f"Found {zfs_package} package: {package_filename}")

            with tempfile.TemporaryDirectory() as tmpdir:
                package_path = Path(tmpdir) / package_filename

                # Download the package
                SysCommand(f"curl -s -o {package_path} {package_url}")

                # Extract zfs-utils version requirement
                zfs_utils_version = self.package_searcher.extract_pkginfo(package_path)

                # Search for matching zfs-utils package
                utils_info = self.package_searcher.search_zfs_package("zfs-utils", zfs_utils_version)
                if not utils_info:
                    result.add_error(f"Compatible zfs-utils package not found for version {zfs_utils_version}")
                    return False

                utils_url = f"{utils_info[0]}{utils_info[1]}"

                # Install both packages
                info(f"Installing zfs-utils and {zfs_package}")
                SysCommand(f"pacman -U {utils_url} --noconfirm", peek_output=True)
                SysCommand(f"pacman -U {package_url} --noconfirm", peek_output=True)

                result.installed_packages.extend(["zfs-utils", zfs_package])
                return True

        except Exception as e:
            result.add_error(f"Host precompiled installation failed: {e}")
            return False

    def _install_dkms(self, variant: KernelVariant, installation: Any, result: InstallationResult) -> bool:
        """Install DKMS ZFS with proper kernel headers.

        Args:
            variant: The kernel variant
            installation: Installation context (None for host system)
            result: Result object to update

        Returns:
            True if installation succeeded, False otherwise
        """
        try:
            packages = variant.get_dkms_packages()

            # Deduplicate while preserving order
            packages = list(dict.fromkeys(packages))

            return self._install_packages(packages, installation, result)

        except Exception as e:
            result.add_error(f"DKMS installation failed: {e}")
            return False

    def _install_packages(self, packages: list[str], installation: Any, result: InstallationResult) -> bool:
        """Install packages using the appropriate method.

        Args:
            packages: List of package names to install
            installation: Installation context (None for host system)
            result: Result object to update

        Returns:
            True if installation succeeded, False otherwise
        """
        try:
            if installation is None:
                # Host system installation
                cmd = f"pacman -S {' '.join(packages)} --noconfirm"
                SysCommand(cmd, peek_output=True)
            else:
                # Target system installation
                installation.add_additional_packages(packages)

            result.installed_packages.extend(packages)
            return True

        except Exception as e:
            result.add_error(f"Package installation failed: {e}")
            return False

    def verify_packages_available(self, packages: list[str]) -> bool:
        """Verify that packages are available in repositories.

        Args:
            packages: List of package names to check

        Returns:
            True if all packages are available, False otherwise
        """
        for package in packages:
            try:
                SysCommand(f"pacman -Si {package}", peek_output=True)
            except SysCallError:
                debug(f"Package not available: {package}")
                return False
            except Exception:
                # If pacman is not available, assume packages are not available
                return False
        return True
