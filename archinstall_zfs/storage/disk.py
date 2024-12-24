from pathlib import Path
from typing import List

from archinstall.lib.output import info, error, debug
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

    def prepare_disk(self, drive: str) -> str:
        """Prepare disk and return ZFS partition path using by-id"""
        info(f"Preparing disk: {drive}")

        debug("Clearing disk signatures")
        try:
            SysCommand(
                f"dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={drive}"
            )
            sectors = int(SysCommand(f"blockdev --getsz {drive}").decode().strip())
            seek_position = sectors - 34
            SysCommand(
                f"dd if=/dev/zero of={drive} bs=512 count=34 seek={seek_position}"
            )
        except Exception as e:
            error(f"Failed to clear disk signatures: {str(e)}")
            raise

        debug("Creating partition table")
        try:
            SysCommand(f"sgdisk -Z {drive}")  # Zap all existing partitions
            SysCommand(f"sgdisk -o {drive}")  # Create fresh GPT

            # Create EFI partition (500MB)
            SysCommand(f"sgdisk -n 1:0:+500M -t 1:ef00 -c 1:EFI {drive}")

            # Create ZFS partition (rest of disk)
            SysCommand(f"sgdisk -n 2:0:0 -t 2:bf00 -c 2:rootpart {drive}")

            debug("Updating kernel partition table")
            SysCommand(f"partprobe {drive}")
            SysCommand("udevadm settle")

            debug("Formatting EFI partition")
            SysCommand("mkfs.fat -I -F32 -n EFI /dev/disk/by-partlabel/EFI")
        except Exception as e:
            error(f"Failed to create partitions: {str(e)}")
            raise

        # Return path to ZFS partition using by-id
        debug("Resolving ZFS partition by-id path")
        try:
            by_id_path = next(
                Path("/dev/disk/by-id").glob(f"*{Path(drive).name}-part2")
            )
            info(f"Created ZFS partition at: {by_id_path}")
            return str(by_id_path)
        except StopIteration:
            error("Failed to find ZFS partition by-id path")
            raise RuntimeError("ZFS partition not found in /dev/disk/by-id")
