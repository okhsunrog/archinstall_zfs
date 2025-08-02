"""
ZFS-specific installer class that properly extends archinstall's Installer.

This module provides a clean way to handle ZFS-specific base packages without
manipulating private attributes of the archinstall.Installer class.
"""

from pathlib import Path
from typing import Optional, List

from archinstall.lib.installer import Installer
from archinstall.lib.models.device import DiskLayoutConfiguration


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
        base_packages: Optional[List[str]] = None,
        kernels: Optional[List[str]] = None,
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
            base_packages = [
                'base',
                'base-devel',
                'linux-firmware',
                'linux-firmware-marvell',
                'sof-firmware',
                'dracut'
            ]
        
        # Default to linux-lts for ZFS compatibility
        if kernels is None:
            kernels = ['linux-lts']
        
        # Call parent constructor with our custom packages
        super().__init__(
            target=target,
            disk_config=disk_config,
            base_packages=base_packages,
            kernels=kernels
        )