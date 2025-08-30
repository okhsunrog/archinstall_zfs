"""
ZFS-specific installer class that properly extends archinstall's Installer.

This module provides a clean way to handle ZFS-specific base packages without
manipulating private attributes of the archinstall.Installer class.
"""

from pathlib import Path

from archinstall.lib.installer import Installer
from archinstall.lib.models.device import DiskLayoutConfiguration

from archinstall_zfs.aur import AURManager
from archinstall_zfs.initramfs.base import InitramfsHandler


class ZFSInstaller(Installer):
    """
    Custom installer for ZFS installations that properly handles base packages.

    This class extends archinstall.Installer to provide ZFS-specific functionality
    while maintaining compatibility with the archinstall API.
    """

    def __init__(
        self,
        target: Path,
        disk_config: DiskLayoutConfiguration,
        initramfs_handler: InitramfsHandler,
        base_packages: list[str] | None = None,
        kernels: list[str] | None = None,
    ):
        """
        Initialize ZFS installer with custom base packages.

        Args:
            target: Installation target directory
            disk_config: Disk layout configuration
            base_packages: Custom base packages for ZFS installation
            kernels: Kernel packages to install
        """
        # Define ZFS-specific base packages if not provided
        if base_packages is None:
            base_packages = ["base", "base-devel", "linux-firmware", "linux-firmware-marvell", "sof-firmware"]

        # Merge initramfs packages provided by handler
        base_packages.extend(pkg for pkg in initramfs_handler.install_packages() if pkg not in base_packages)

        # Default to linux-lts for ZFS compatibility
        if kernels is None:
            kernels = ["linux-lts"]

        # Call parent constructor with our custom packages
        super().__init__(target=target, disk_config=disk_config, base_packages=base_packages, kernels=kernels)

        # Store handler and perform its configuration inside target
        self.initramfs_handler: InitramfsHandler = initramfs_handler
        self.initramfs_handler.configure()
        self.initramfs_handler.setup_hooks()

    # Delegate mkinitcpio step to our initramfs handler
    def mkinitcpio(self, _: list[str]) -> bool:
        try:
            return all(self.initramfs_handler.generate_initramfs(kernel) for kernel in self.kernels)
        except Exception:
            return False

    def regenerate_initramfs(self) -> bool:
        """Regenerate initramfs for all kernels via the active initramfs handler."""
        try:
            return all(self.initramfs_handler.generate_initramfs(kernel) for kernel in self.kernels)
        except Exception:
            return False

    def install_aur_packages(self, packages: list[str]) -> bool:
        """
        Install AUR packages using the AUR manager.

        Args:
            packages: List of AUR package names to install

        Returns:
            True if all packages installed successfully, False otherwise
        """
        if not packages:
            return True

        aur_manager = AURManager(self)
        return aur_manager.install_packages(packages)
