from pathlib import Path
import socket
from archinstall.tui.curses_menu import Tui
from archinstall.lib.general import SysCommand
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup
from archinstall.tui.menu_item import MenuItem
from storage.disk import DiskManager
from storage.zfs import ZFSManager

def check_internet():
    try:
        socket.create_connection(("archlinux.org", 80))
        return True
    except OSError:
        return False

def check_efi():
    efi_path = Path("/sys/firmware/efi/efivars")
    return efi_path.exists() and any(efi_path.iterdir())

def get_installation_mode():
    modes = [
        MenuItem("Full disk - Format and create new ZFS pool", "full_disk"),
        MenuItem("Partition - Create new ZFS pool on existing partition", "new_pool"),
        MenuItem("Existing pool - Install alongside existing ZFS system", "existing_pool")
    ]
    
    menu = SelectMenu(
        MenuItemGroup(modes),
        header="Select Installation Mode\n\nWarning: Make sure you have backups!"
    )
    
    return menu.run().item().value

def main():
    if not check_internet():
        print("Error: No internet connection detected")
        return False
        
    if not check_efi():
        print("Error: System not booted in UEFI mode")
        return False

    disk_manager = DiskManager()
    zfs_manager = ZFSManager()

    with Tui():
        mode = get_installation_mode()
        
        if mode == "full_disk":
            # Full disk installation flow
            selected_disk = disk_manager.select_disk()
            dataset_prefix = disk_manager.get_dataset_prefix()
            encryption_password = zfs_manager.get_encryption_password()
            
            # Prepare disk and create ZFS pool
            disk_manager.prepare_disk(selected_disk)
            zfs_manager.create_pool(
                partition="/dev/disk/by-partlabel/rootpart",
                prefix=dataset_prefix,
                encryption_password=encryption_password
            )
            
            # Import pool and mount for installation
            zfs_manager.import_pool(dataset_prefix, Path('/mnt'))

        elif mode == "new_pool":
            # TODO: Implement partition selection and pool creation
            pass
            
        elif mode == "existing_pool":
            # TODO: Implement existing pool handling
            pass

    return True

if __name__ == "__main__":
    main()

