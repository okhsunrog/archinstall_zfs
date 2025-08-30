"""
zrepl integration for archinstall_zfs.

This module provides zrepl configuration generation and package installation
for ZFS replication and snapshot management. Service enablement is handled
by the main installer using the standard archinstall pattern.
"""

from archinstall import info
from archinstall.lib.installer import Installer


def generate_zrepl_config(pool_name: str, dataset_prefix: str) -> str:
    """
    Generate a default zrepl configuration file with reasonable defaults.

    Args:
        pool_name: Name of the ZFS pool
        dataset_prefix: Dataset prefix (e.g., "arch0")

    Returns:
        zrepl configuration as a string
    """
    return f"""jobs:

# This job creates snapshots locally and prunes them aggressively.
- name: snapshot_{dataset_prefix}
  type: snap
  filesystems: {{
      "{pool_name}/{dataset_prefix}<": true
  }}
  snapshotting:
    type: periodic
    interval: 15m
    prefix: zrepl_
  pruning:
    keep:
    # This local grid is correct and uses valid suffixes.
    - type: grid
      grid: 4x15m(keep=all) | 24x1h | 3x1d
      regex: "^zrepl_.*"
    - type: regex
      negate: true
      regex: "^zrepl_.*"
"""


def install_zrepl_package(installation: Installer) -> bool:
    """
    Install zrepl package in the target system.

    Args:
        installation: Installer instance for target installation

    Returns:
        True if installation succeeded, False otherwise
    """
    try:
        info("Installing zrepl package")
        installation.arch_chroot("pacman -S --noconfirm zrepl")
        info("Successfully installed zrepl package")
        return True
    except Exception as e:
        info(f"Failed to install zrepl package: {e}")
        return False


def setup_zrepl_config(installation: Installer, pool_name: str, dataset_prefix: str) -> bool:
    """
    Create zrepl configuration file in the target system.

    Args:
        installation: Installer instance for target installation
        pool_name: Name of the ZFS pool
        dataset_prefix: Dataset prefix (e.g., "arch0")

    Returns:
        True if configuration was created successfully, False otherwise
    """
    try:
        info("Creating zrepl configuration")

        # Generate configuration
        config_content = generate_zrepl_config(pool_name, dataset_prefix)

        # Create config directory
        config_dir = installation.target / "etc" / "zrepl"
        config_dir.mkdir(parents=True, exist_ok=True)

        # Write configuration file
        config_file = config_dir / "zrepl.yml"
        config_file.write_text(config_content)

        # Set proper permissions
        installation.arch_chroot("chmod 644 /etc/zrepl/zrepl.yml")

        info("Successfully created zrepl configuration")
        return True
    except Exception as e:
        info(f"Failed to create zrepl configuration: {e}")
        return False


def setup_zrepl(installation: Installer, pool_name: str, dataset_prefix: str) -> bool:
    """
    Complete zrepl setup: install package and create config.

    Note: Service enablement is handled separately using the standard archinstall pattern.

    Args:
        installation: Installer instance for target installation
        pool_name: Name of the ZFS pool
        dataset_prefix: Dataset prefix (e.g., "arch0")

    Returns:
        True if all setup steps succeeded, False otherwise
    """
    success = True

    # Install package
    if not install_zrepl_package(installation):
        success = False

    # Create configuration
    if not setup_zrepl_config(installation, pool_name, dataset_prefix):
        success = False

    if success:
        info("zrepl setup completed successfully")
    else:
        info("zrepl setup completed with some errors")

    return success
