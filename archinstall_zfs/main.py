# Standard library imports
from pathlib import Path
import socket
from typing import Literal
import os

# Third-party imports
# import parted

# Local application imports
from archinstall import SysInfo, debug, info, error
from archinstall.tui.curses_menu import Tui, SelectMenu, MenuItemGroup, EditMenu
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


def prepare_installation(
        disk_manager: DiskManager,
        zfs_manager: ZFSManager,
        mode: InstallMode
) -> None:
    prefix_menu = EditMenu(
        "Dataset Prefix",
        header="Enter prefix for ZFS datasets (e.g., sys, main)",
        default_text="arch0",
    )
    zfs_manager.dataset_prefix = prefix_menu.input().text()

    if mode == "full_disk":
        disk_manager.select_disk()
        disk_manager.prepare_disk()
    elif mode == "new_pool":
        disk_manager.select_disk()
        disk_manager.select_partition("ZFS")
        disk_manager.select_partition("EFI")
    else:  # existing_pool
        zfs_manager.select_pool()
        disk_manager.select_disk()
        disk_manager.select_partition("EFI")


def perform_installation(
        disk_manager: DiskManager,
        zfs_manager: ZFSManager,
        mode: InstallMode,
) -> bool:
    try:
        if mode != "existing_pool":
            zfs_manager.get_encryption_password()
            zfs_manager.create_pool(disk_manager.zfs_partition)
            zfs_manager.create_datasets()
            zfs_manager.export_pool()

        zfs_manager.import_pool(Path("/mnt"))
        disk_manager.mount_efi_partition(Path("/mnt"))

        if not zfs_manager.verify_mounts():
            raise RuntimeError("Mount verification failed")

        return True
    except Exception as e:
        error(f"Installation failed: {str(e)}")
        return False


def main() -> bool:
    storage['LOG_PATH'] = Path(os.path.expanduser('~'))
    storage['LOG_FILE'] = Path('archinstall.log')
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
            prepare_installation(disk_manager, zfs_manager, mode)
            perform_installation(disk_manager, zfs_manager, mode)
    except Exception as e:
        error(f"Installation failed: {str(e)}")
