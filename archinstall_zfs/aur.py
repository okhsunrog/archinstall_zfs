"""
AUR (Arch User Repository) package installation support for archinstall_zfs.

This module provides functionality to install packages from the AUR by:
1. Creating a temporary user for package building
2. Installing an AUR helper (paru)
3. Installing requested AUR packages
4. Cleaning up temporary user and configuration

The implementation follows security best practices by using a temporary user
and minimizing sudo privileges.
"""

from typing import ClassVar

from archinstall import debug, error, info
from archinstall.lib.installer import Installer


class AURManager:
    """Manages AUR package installation in the target system."""

    TEMP_USER: ClassVar[str] = "aurinstall"
    AUR_HELPER: ClassVar[str] = "yay"
    AUR_HELPER_REPO: ClassVar[str] = "https://aur.archlinux.org/yay-bin.git"
    DEPENDENCIES: ClassVar[list[str]] = ["git", "base-devel"]

    def __init__(self, installer: Installer):
        self.installer = installer
        self._temp_user_created = False
        self._sudo_modified = False

    def install_packages(self, packages: list[str]) -> bool:
        """
        Install a list of AUR packages.

        Args:
            packages: List of AUR package names to install

        Returns:
            True if all packages installed successfully, False otherwise
        """
        if not packages:
            info("No AUR packages to install")
            return True

        info(f"Installing AUR packages: {', '.join(packages)}")

        try:
            self._setup_aur_environment()
            self._install_aur_helper()
            self._install_aur_packages(packages)
            info("AUR packages installed successfully")
            return True
        except Exception as e:
            error(f"AUR installation failed: {e}")
            return False
        finally:
            self._cleanup_aur_environment()

    def _setup_aur_environment(self) -> None:
        """Set up environment for AUR package building."""
        debug("Setting up AUR environment")

        # Install dependencies
        info(f"Installing AUR dependencies: {', '.join(self.DEPENDENCIES)}")
        self.installer.arch_chroot(f"pacman -S --noconfirm --needed {' '.join(self.DEPENDENCIES)}")

        # Create temporary user
        info(f"Creating temporary user: {self.TEMP_USER}")
        self.installer.arch_chroot(f"useradd -m -G wheel {self.TEMP_USER}")
        self._temp_user_created = True

        # Enable passwordless sudo for wheel group (temporarily)
        info("Temporarily enabling passwordless sudo for package building")
        self.installer.arch_chroot("sed -i 's/^# %wheel ALL=(ALL) NOPASSWD: ALL/%wheel ALL=(ALL) NOPASSWD: ALL/' /etc/sudoers")
        self._sudo_modified = True

    def _install_aur_helper(self) -> None:
        """Install yay AUR helper."""
        info(f"Installing {self.AUR_HELPER} AUR helper")

        # Build yay package as regular user, then install as root
        # Use mktemp for secure temporary directory
        build_commands = [
            "BUILD_DIR=$(mktemp -d)",
            f"chown {self.TEMP_USER}:{self.TEMP_USER} $BUILD_DIR",
            f"su {self.TEMP_USER} -c 'cd $BUILD_DIR && git clone {self.AUR_HELPER_REPO} .'",
            f"su {self.TEMP_USER} -c 'cd $BUILD_DIR && makepkg --noconfirm'",
            "pacman -U --noconfirm $BUILD_DIR/*.pkg.tar.*",
            "rm -rf $BUILD_DIR",
        ]

        # Execute as a single command to preserve BUILD_DIR variable
        full_command = " && ".join(build_commands)
        self.installer.arch_chroot(full_command)

    def _install_aur_packages(self, packages: list[str]) -> None:
        """Install the actual AUR packages using yay."""
        info(f"Installing AUR packages: {', '.join(packages)}")

        # Install packages with yay
        install_cmd = f"su {self.TEMP_USER} -c '{self.AUR_HELPER} -Sy --noconfirm --needed {' '.join(packages)}'"
        self.installer.arch_chroot(install_cmd)

    def _cleanup_aur_environment(self) -> None:
        """Clean up temporary user and sudo configuration."""
        debug("Cleaning up AUR environment")

        try:
            # Restore sudo configuration
            if self._sudo_modified:
                info("Restoring sudo configuration")
                self.installer.arch_chroot("sed -i 's/^%wheel ALL=(ALL) NOPASSWD: ALL/# %wheel ALL=(ALL) NOPASSWD: ALL/' /etc/sudoers")
                self._sudo_modified = False

            # Remove temporary user
            if self._temp_user_created:
                info(f"Removing temporary user: {self.TEMP_USER}")
                # Use proper shell command for error handling
                self.installer.arch_chroot(f"userdel -r {self.TEMP_USER} || true")
                self._temp_user_created = False

        except Exception as e:
            error(f"Cleanup failed: {e}")
            # Continue anyway - this is cleanup, not critical functionality

    def is_package_available_in_aur(self, _package: str) -> bool:
        """
        Check if a package is available in the AUR.

        Note: This is a simple implementation that could be enhanced
        with actual AUR API queries in the future.

        Args:
            package: Package name to check

        Returns:
            True if package might be available (optimistic approach)
        """
        # For now, assume all packages might be available
        # In a more sophisticated implementation, we could query the AUR API
        return True
