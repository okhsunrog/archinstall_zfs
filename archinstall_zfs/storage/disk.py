from pathlib import Path
from typing import List

from archinstall import debug, info, error
from archinstall.lib.general import SysCommand
from archinstall.lib.disk import device_handler
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup, EditMenu
from archinstall.tui.menu_item import MenuItem
import parted


class DiskManager:
    def get_available_disks(self) -> List[MenuItem]:
        debug("Scanning for available disks")
        disks = []
        for dev in device_handler.devices:
            by_id_path = Path("/dev/disk/by-id").glob(f"*{dev.device_info.path.name}")
            disk_path = next(by_id_path, dev.device_info.path)
            disks.append(MenuItem(str(disk_path), str(disk_path)))
            debug(f"Found disk: {disk_path}")
        info(f"Found {len(disks)} available disks")
        return disks

    def select_disk(self) -> str:
        debug("Displaying disk selection menu")
        disk_menu = SelectMenu(
            MenuItemGroup(self.get_available_disks()),
            header="Select disk for installation\nWARNING: All data will be erased!",
        )
        selected = disk_menu.run().item().value
        info(f"Selected disk: {selected}")
        return selected

    def get_partitions(self, disk: str) -> List[MenuItem]:
        debug(f"Scanning partitions on disk: {disk}")
        device = parted.getDevice(disk)
        disk_obj = parted.Disk(
            device
        )  # Create separate variable for parted.Disk object
        partitions = [
            MenuItem(f"{p.path} ({p.getLength()}MB)", p.path)
            for p in disk_obj.partitions  # Use disk_obj instead of disk string
        ]
        info(f"Found {len(partitions)} partitions")
        return partitions

    def select_partition(self, disk: str) -> str:
        debug(f"Displaying partition selection menu for disk: {disk}")
        partition_menu = SelectMenu(
            MenuItemGroup(self.get_partitions(disk)),
            header="Select partition for ZFS pool",
        )
        selected = partition_menu.run().item().value
        info(f"Selected partition: {selected}")
        return selected

    def get_dataset_prefix(self) -> str:
        debug("Requesting dataset prefix")
        prefix_menu = EditMenu(
            "Dataset Prefix",
            header="Enter prefix for ZFS datasets (e.g., sys, main)",
            default_text="sys",
        )
        prefix = prefix_menu.input().text()
        info(f"Using dataset prefix: {prefix}")
        return prefix

    def get_disk_by_id(self, disk_path: str) -> str:
        """Convert /dev/sdX path to /dev/disk/by-id path"""
        debug(f"Getting by-id path for disk: {disk_path}")

        disk_name = Path(disk_path).name
        by_id_path = Path("/dev/disk/by-id")

        for path in by_id_path.iterdir():
            if path.is_symlink() and path.readlink().name == disk_name:
                if not path.name.split('-')[-1].startswith('part'):
                    debug(f"Found by-id path: {path}")
                    return str(path)

        error(f"No by-id path found for disk: {disk_path}")
        raise RuntimeError(f"Could not find /dev/disk/by-id path for {disk_path}")

    def prepare_disk(self, drive: str) -> str:
        """Prepare disk and return ZFS partition path using by-id"""
        info(f"Preparing disk: {drive}")

        # Get the by-id path first
        drive_by_id = self.get_disk_by_id(drive)
        debug(f"Using disk by-id path: {drive_by_id}")

        debug("Clearing disk signatures")
        try:
            debug(f"Zeroing first 34 sectors of {drive_by_id}")
            SysCommand(
                f"dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={drive_by_id}"
            )
            sectors = int(SysCommand(f"blockdev --getsz {drive_by_id}").decode().strip())
            seek_position = sectors - 34
            debug(f"Zeroing last 34 sectors of {drive_by_id} at position {seek_position}")
            SysCommand(
                f"dd if=/dev/zero of={drive_by_id} bs=512 count=34 seek={seek_position}"
            )
        except Exception as e:
            error(f"Failed to clear disk signatures: {str(e)}")
            raise

        debug("Creating partition table")
        try:
            debug(f"Zapping existing partitions on {drive_by_id}")
            SysCommand(f"sgdisk -Z {drive_by_id}")
            debug(f"Creating fresh GPT on {drive_by_id}")
            SysCommand(f"sgdisk -o {drive_by_id}")

            debug(f"Creating EFI partition (500MB) on {drive_by_id}")
            SysCommand(f"sgdisk -n 1:0:+500M -t 1:ef00 {drive_by_id}")

            debug(f"Creating ZFS partition (rest of disk) on {drive_by_id}")
            SysCommand(f"sgdisk -n 2:0:0 -t 2:bf00 {drive_by_id}")

            debug("Updating kernel partition table")
            SysCommand(f"partprobe {drive_by_id}")
            debug("Waiting for udev to settle")
            SysCommand("udevadm settle")

            debug("Formatting EFI partition")
            efi_part_path = f"{drive_by_id}-part1"
            debug(f"Using EFI partition path: {efi_part_path}")
            SysCommand(f"mkfs.fat -I -F32 {efi_part_path}")
            debug(f"Successfully formatted EFI partition at {efi_part_path}")
        except Exception as e:
            error(f"Failed to create partitions: {str(e)}")
            raise

        # Return path to ZFS partition using by-id
        zfs_part_path = f"{drive_by_id}-part2"
        info(f"Created ZFS partition at: {zfs_part_path}")
        return zfs_part_path
