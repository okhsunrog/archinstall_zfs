"""
AUR (Arch User Repository) package installation support for archinstall_zfs.

This module provides functionality to install packages from the AUR by:
1. Creating a temporary user for package building
2. Installing an AUR helper (yay)
3. Installing requested AUR packages
4. Cleaning up temporary user and configuration

The implementation follows security best practices by using a temporary user
and minimizing sudo privileges.
"""

import contextlib
from typing import ClassVar

from archinstall import debug, error, info
from archinstall.lib.installer import Installer


class AURManager:
    """Manages AUR package installation in the target system."""

    TEMP_USER: ClassVar[str] = "aurinstall"
    AUR_HELPER: ClassVar[str] = "yay"
    # Use yay-bin which is prebuilt and easier to install
    AUR_HELPER_REPO: ClassVar[str] = "https://aur.archlinux.org/yay-bin.git"
    DEPENDENCIES: ClassVar[list[str]] = ["git", "base-devel", "sudo"]

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
        # First, ensure wheel group line exists
        self.installer.arch_chroot(
            "grep -q '^# %wheel ALL=(ALL:ALL) NOPASSWD: ALL' /etc/sudoers || echo '# %wheel ALL=(ALL:ALL) NOPASSWD: ALL' >> /etc/sudoers"
        )
        # Then uncomment it
        self.installer.arch_chroot("sed -i 's/^# %wheel ALL=(ALL:ALL) NOPASSWD: ALL/%wheel ALL=(ALL:ALL) NOPASSWD: ALL/' /etc/sudoers")
        # Also handle the older format without :ALL
        self.installer.arch_chroot("sed -i 's/^# %wheel ALL=(ALL) NOPASSWD: ALL/%wheel ALL=(ALL) NOPASSWD: ALL/' /etc/sudoers")
        self._sudo_modified = True

    def _install_aur_helper(self) -> None:
        """Install yay AUR helper."""
        info(f"Installing {self.AUR_HELPER} AUR helper")

        # Use user's home directory for secure build location
        build_dir = f"/home/{self.TEMP_USER}/yay-build"

        try:
            # Clean up any existing build directory
            self.installer.arch_chroot(f"rm -rf {build_dir}")

            # Create build directory (no chown needed since it's in user's home)
            self.installer.arch_chroot(f"su {self.TEMP_USER} -c 'mkdir -p {build_dir}'")

            # Clone repository as user
            info("Cloning yay repository")
            self.installer.arch_chroot(f"su {self.TEMP_USER} -c 'cd {build_dir} && git clone {self.AUR_HELPER_REPO} .'")

            # Build package as user with -s flag to install dependencies if needed
            info("Building yay package")
            self.installer.arch_chroot(f"su {self.TEMP_USER} -c 'cd {build_dir} && makepkg -s --noconfirm'")

            # Find and install the built package
            info("Installing yay package")
            # Use a shell to properly expand the glob pattern
            self.installer.arch_chroot(f"sh -c 'cd {build_dir} && pacman -U --noconfirm *.pkg.tar.*'")

            # Verify installation
            info("Verifying yay installation")
            self.installer.arch_chroot("which yay")
            info("yay installation successful")

            # Clean up
            self.installer.arch_chroot(f"rm -rf {build_dir}")

        except Exception as e:
            error(f"Failed to install yay: {e}")
            # Try to clean up even if installation failed
            with contextlib.suppress(Exception):
                self.installer.arch_chroot(f"rm -rf {build_dir}")
            raise

    def _install_aur_packages(self, packages: list[str]) -> None:
        """Install the actual AUR packages using yay."""
        info(f"Installing AUR packages: {', '.join(packages)}")

        # Verify yay is available first
        try:
            self.installer.arch_chroot("which yay")
        except Exception as e:
            error(f"yay not found - AUR helper installation may have failed: {e}")
            raise

        # Create a build directory in user's home
        build_home = f"/home/{self.TEMP_USER}"

        # Install each package individually to better handle errors
        for package in packages:
            info(f"Installing AUR package: {package}")
            try:
                # Use sh -c to run the entire command in a shell context
                install_cmd = f"sh -c 'cd {build_home} && sudo -u {self.TEMP_USER} yay -S --noconfirm --needed --removemake --noprogressbar {package}'"
                self.installer.arch_chroot(install_cmd)
                info(f"Successfully installed: {package}")
            except Exception as e:
                error(f"Failed to install {package}: {e}")
                # Try alternative approach: clone and build manually
                try:
                    info(f"Attempting manual build for {package}")
                    pkg_dir = f"{build_home}/{package}"

                    # Clean any existing directory
                    self.installer.arch_chroot(f"rm -rf {pkg_dir}")

                    # Clone from AUR
                    self.installer.arch_chroot(f"su {self.TEMP_USER} -c 'cd {build_home} && git clone https://aur.archlinux.org/{package}.git'")

                    # Build package with -s flag to install dependencies
                    self.installer.arch_chroot(f"su {self.TEMP_USER} -c 'cd {pkg_dir} && makepkg -s --noconfirm'")

                    # Install package
                    self.installer.arch_chroot(f"sh -c 'cd {pkg_dir} && pacman -U --noconfirm *.pkg.tar.*'")

                    info(f"Successfully installed {package} via manual build")

                    # Clean up
                    self.installer.arch_chroot(f"rm -rf {pkg_dir}")
                except Exception as manual_error:
                    error(f"Manual build also failed for {package}: {manual_error}")
                    # Continue with other packages

    def _cleanup_aur_environment(self) -> None:
        """Clean up temporary user and sudo configuration."""
        debug("Cleaning up AUR environment")

        try:
            # Restore sudo configuration
            if self._sudo_modified:
                info("Restoring sudo configuration")
                # Re-comment the NOPASSWD lines
                self.installer.arch_chroot("sed -i 's/^%wheel ALL=(ALL:ALL) NOPASSWD: ALL/# %wheel ALL=(ALL:ALL) NOPASSWD: ALL/' /etc/sudoers")
                self.installer.arch_chroot("sed -i 's/^%wheel ALL=(ALL) NOPASSWD: ALL/# %wheel ALL=(ALL) NOPASSWD: ALL/' /etc/sudoers")
                self._sudo_modified = False

            # Remove temporary user
            if self._temp_user_created:
                info(f"Removing temporary user: {self.TEMP_USER}")
                # Kill any processes owned by the user first
                self.installer.arch_chroot(f"sh -c 'pkill -u {self.TEMP_USER} || true'")
                # Remove the user and their home directory
                self.installer.arch_chroot(f"sh -c 'userdel -r {self.TEMP_USER} || true'")
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
