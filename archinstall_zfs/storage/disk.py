import parted
from pathlib import Path
from archinstall.lib.general import SysCommand
from archinstall.lib.disk import device_handler
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup, EditMenu
from archinstall.tui.menu_item import MenuItem

class DiskManager:
    def get_available_disks(self):
        return [MenuItem(str(dev.device_info.path), str(dev.device_info.path)) 
                for dev in device_handler.devices]

    def select_disk(self):
        disk_menu = SelectMenu(
            MenuItemGroup(self.get_available_disks()),
            header="Select disk for installation\nWARNING: All data will be erased!"
        )
        return disk_menu.run().item().value

    def get_dataset_prefix(self):
        prefix_menu = EditMenu(
            "Dataset Prefix",
            header="Enter prefix for ZFS datasets (e.g., sys, main)",
            default_text="sys"
        )
        return prefix_menu.input().text()

    def prepare_disk(self, drive: str):
        # Clear disk signatures
        SysCommand(f'dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={drive}')
        sectors = int(SysCommand(f'blockdev --getsz {drive}').decode().strip())
        seek_position = sectors - 34
        SysCommand(f'dd if=/dev/zero of={drive} bs=512 count=34 seek={seek_position}')

        # Create GPT and partitions using sgdisk
        SysCommand(f'sgdisk -Z {drive}')  # Zap all existing partitions
        SysCommand(f'sgdisk -o {drive}')  # Create fresh GPT
        
        # Create EFI partition (500MB)
        SysCommand(f'sgdisk -n 1:0:+500M -t 1:ef00 -c 1:EFI {drive}')
        
        # Create ZFS partition (rest of disk)
        SysCommand(f'sgdisk -n 2:0:0 -t 2:bf00 -c 2:rootpart {drive}')
        
        # Update kernel partition table
        SysCommand(f'partprobe {drive}')
        SysCommand('udevadm settle')
        
        # Format EFI partition
        SysCommand('mkfs.fat -I -F32 -n EFI /dev/disk/by-partlabel/EFI')

