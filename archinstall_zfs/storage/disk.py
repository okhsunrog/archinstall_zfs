import parted
from pathlib import Path
from archinstall.lib.general import SysCommand
from archinstall.lib.disk import device_handler
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup, EditMenu
from archinstall.tui.menu_item import MenuItem

class DiskManager:
    def get_available_disks(self):
        # Use by-id paths for more reliable disk identification
        disks = []
        for dev in device_handler.devices:
            by_id_path = Path(f"/dev/disk/by-id").glob(f"*{dev.device_info.path.name}")
            disk_path = next(by_id_path, dev.device_info.path)
            disks.append(MenuItem(str(disk_path), str(disk_path)))
        return disks

    def select_disk(self):
        disk_menu = SelectMenu(
            MenuItemGroup(self.get_available_disks()),
            header="Select disk for installation\nWARNING: All data will be erased!"
        )
        return disk_menu.run().item().value

    def get_partitions(self, disk: str):
        device = parted.getDevice(disk)
        disk = parted.Disk(device)
        return [MenuItem(f"{p.path} ({p.getLength()}MB)", p.path) for p in disk.partitions]

    def select_partition(self, disk: str):
        partition_menu = SelectMenu(
            MenuItemGroup(self.get_partitions(disk)),
            header="Select partition for ZFS pool"
        )
        return partition_menu.run().item().value

    def prepare_disk(self, drive: str) -> str:
        """Prepare disk and return ZFS partition path"""
        # Clear disk signatures
        SysCommand(f'dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={drive}')
        sectors = int(SysCommand(f'blockdev --getsz {drive}').decode().strip())
        seek_position = sectors - 34
        SysCommand(f'dd if=/dev/zero of={drive} bs=512 count=34 seek={seek_position}')

        # Create GPT and partitions using sgdisk
        SysCommand(f'sgdisk -Z {drive}')
        SysCommand(f'sgdisk -o {drive}')
        
        # Create EFI partition (500MB)
        SysCommand(f'sgdisk -n 1:0:+500M -t 1:ef00 -c 1:EFI {drive}')
        
        # Create ZFS partition (rest of disk)
        SysCommand(f'sgdisk -n 2:0:0 -t 2:bf00 -c 2:rootpart {drive}')
        
        # Update kernel partition table
        SysCommand(f'partprobe {drive}')
        SysCommand('udevadm settle')
        
        # Format EFI partition
        SysCommand('mkfs.fat -I -F32 -n EFI /dev/disk/by-partlabel/EFI')

        # Return path to ZFS partition using by-id
        by_id_path = next(Path("/dev/disk/by-id").glob(f"*{Path(drive).name}-part2"))
        return str(by_id_path)
