# Standard library imports
from pathlib import Path
import socket
from typing import Literal
import os

# Third-party imports
# import parted

# Local application imports
from archinstall import SysInfo, debug, info, error
from archinstall.tui.curses_menu import Tui, SelectMenu, MenuItemGroup
from archinstall.tui.menu_item import MenuItem
from archinstall.lib.storage import storage
from storage.disk import DiskManager
from storage.zfs import ZFSManager


InstallMode = Literal["full_disk", "new_pool", "existing_pool"]


def check_internet() -> bool:
    debug("Checking internet connection")
    try:
        socket.create_connection(("archlinux.org", 80))
        info("Internet connection available")
        return True
    except OSError as e:
        error(f"No internet connection: {str(e)}")
        return False

def get_installation_mode() -> InstallMode:
    debug("Displaying installation mode selection menu")
    modes = [
        MenuItem("Full disk - Format and create new ZFS pool", "full_disk"),
        MenuItem("Partition - Create new ZFS pool on existing partition", "new_pool"),
        MenuItem(
            "Existing pool - Install alongside existing ZFS system", "existing_pool"
        ),
    ]

    menu = SelectMenu(
        MenuItemGroup(modes),
        header="Select Installation Mode\n\nWarning: Make sure you have backups!",
    )

    selected = menu.run().item().value
    info(f"Selected installation mode: {selected}")
    return selected


def handle_full_disk_install(
    disk_manager: DiskManager, zfs_manager: ZFSManager, dataset_prefix: str
) -> bool:
    debug("Starting full disk installation")
    try:
        selected_disk = disk_manager.select_disk()
        info(f"Selected disk: {selected_disk}")

        debug("Preparing disk partitions")
        zfs_partition = disk_manager.prepare_dcisk(selected_disk)
        info(f"Created ZFS partition: {zfs_partition}")

        encryption_password = zfs_manager.get_encryption_password()
        debug("Creating ZFS pool")
        zfs_manager.create_pool(zfs_partition, dataset_prefix, encryption_password)

        debug("Importing and mounting pool")
        zfs_manager.import_pool(dataset_prefix, Path("/mnt"))
        return True
    except Exception as e:
        error(f"Full disk installation failed: {str(e)}")
        return False


def handle_new_pool_install(
    disk_manager: DiskManager, zfs_manager: ZFSManager, dataset_prefix: str
) -> bool:
    debug("Starting new pool installation")
    try:
        selected_disk = disk_manager.select_disk()
        selected_partition = disk_manager.select_partition(selected_disk)
        encryption_password = zfs_manager.get_encryption_password()

        debug("Creating ZFS pool on existing partition")
        zfs_manager.create_pool(selected_partition, dataset_prefix, encryption_password)

        debug("Importing and mounting pool")
        zfs_manager.import_pool(dataset_prefix, Path("/mnt"))
        return True
    except Exception as e:
        error(f"New pool installation failed: {str(e)}")
        return False


def handle_existing_pool_install(zfs_manager: ZFSManager, dataset_prefix: str) -> bool:
    debug("Starting existing pool installation")
    try:
        selected_pool = zfs_manager.select_pool()
        debug(f"Selected pool: {selected_pool}")

        debug("Importing existing pool")
        zfs_manager.import_pool(dataset_prefix, Path("/mnt"))

        debug("Creating datasets structure")
        zfs_manager.create_datasets(dataset_prefix)
        return True
    except Exception as e:
        error(f"Existing pool installation failed: {str(e)}")
        return False


def main() -> bool:
    storage['LOG_PATH'] = os.path.expanduser('~')
    storage['LOG_FILE'] = 'archinstall.log'
    storage['LOG_LEVEL'] = 'DEBUG'

    info("Starting ZFS installation")

    if not check_internet():
        error("Internet connection required")
        return False

    if not SysInfo.has_uefi():
        error("EFI boot mode required")
        return False

    disk_manager = DiskManager()
    zfs_manager = ZFSManager()

    try:
        with Tui():
            mode = get_installation_mode()
            dataset_prefix = disk_manager.get_dataset_prefix()
            info(f"Using dataset prefix: {dataset_prefix}")

            if mode == "full_disk":
                return handle_full_disk_install(
                    disk_manager, zfs_manager, dataset_prefix
                )
            elif mode == "new_pool":
                return handle_new_pool_install(
                    disk_manager, zfs_manager, dataset_prefix
                )
            elif mode == "existing_pool":
                return handle_existing_pool_install(zfs_manager, dataset_prefix)
    except Exception as e:
        error(f"Installation failed: {str(e)}")
        return False

    return True


if __name__ == "__main__":
    main()
