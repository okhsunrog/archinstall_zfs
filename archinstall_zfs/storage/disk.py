from pathlib import Path
from typing import List

from archinstall import debug, info, error
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.lib.disk import device_handler
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup, EditMenu
from archinstall.tui.menu_item import MenuItem
import parted


class DiskManager:
    def __init__(self):
        self.selected_disk: str | None = None  # Will store by-id path
        self.efi_partition: str | None = None  # Will store by-id partition path
        self.zfs_partition: str | None = None  # Will store by-id partition path

    def get_available_disks(self) -> List[MenuItem]:
        debug("Scanning for available disks")
        disks = []
        for dev in device_handler.devices:
            by_id_path = Path("/dev/disk/by-id").glob(f"*{dev.device_info.path.name}")
            disk_path = next(by_id_path, dev.device_info.path)
            # Only add if we found a by-id path
            if disk_path.is_relative_to("/dev/disk/by-id"):
                disks.append(MenuItem(str(disk_path), str(disk_path)))
                debug(f"Found disk: {disk_path}")
        info(f"Found {len(disks)} available disks")
        return disks

    def select_disk(self) -> None:
        debug("Displaying disk selection menu")
        disk_menu = SelectMenu(
            MenuItemGroup(self.get_available_disks()),
            header="Select disk for installation\nWARNING: All data will be erased!",
        )
        self.selected_disk = disk_menu.run().item().value
        info(f"Selected disk: {self.selected_disk}")

    def get_partitions(self) -> List[MenuItem]:
        if not self.selected_disk:
            raise RuntimeError("No disk selected")

        debug(f"Scanning partitions on disk: {self.selected_disk}")
        device = parted.getDevice(self.selected_disk)
        disk_obj = parted.Disk(device)
        partitions = []

        for p in disk_obj.partitions:
            part_name = Path(p.path).name
            by_id_part = Path(self.selected_disk + "-part" + part_name[-1])
            partitions.append(MenuItem(f"{by_id_part} ({p.getLength()}MB)", str(by_id_part)))

        info(f"Found {len(partitions)} partitions")
        return partitions

    def select_partition(self, purpose: str) -> None:
        if not self.selected_disk:
            raise RuntimeError("No disk selected")

        debug(f"Displaying partition selection menu for {purpose}")
        partition_menu = SelectMenu(
            MenuItemGroup(self.get_partitions()),
            header=f"Select partition for {purpose}",
        )
        selected = partition_menu.run().item().value

        if purpose.lower() == "efi":
            self.efi_partition = selected
        elif purpose.lower() == "zfs":
            self.zfs_partition = selected

        info(f"Selected {purpose} partition: {selected}")

    def prepare_disk(self) -> None:
        if not self.selected_disk:
            raise RuntimeError("No disk selected")

        info(f"Preparing disk: {self.selected_disk}")
        debug("Clearing disk signatures")
        try:
            debug(f"Zeroing first 34 sectors of {self.selected_disk}")
            SysCommand(f"dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={self.selected_disk}")
            sectors = int(SysCommand(f"blockdev --getsz {self.selected_disk}").decode().strip())
            seek_position = sectors - 34
            debug(f"Zeroing last 34 sectors at position {seek_position}")
            SysCommand(f"dd if=/dev/zero of={self.selected_disk} bs=512 count=34 seek={seek_position}")
        except Exception as e:
            error(f"Failed to clear disk signatures: {str(e)}")
            raise

        debug("Creating partition table")
        try:
            debug(f"Zapping existing partitions")
            SysCommand(f"sgdisk -Z {self.selected_disk}")
            debug(f"Creating fresh GPT")
            SysCommand(f"sgdisk -o {self.selected_disk}")

            debug(f"Creating EFI partition (500MB)")
            SysCommand(f"sgdisk -n 1:0:+500M -t 1:ef00 {self.selected_disk}")

            debug(f"Creating ZFS partition (rest of disk)")
            SysCommand(f"sgdisk -n 2:0:0 -t 2:bf00 {self.selected_disk}")

            debug("Updating kernel partition table")
            SysCommand(f"partprobe {self.selected_disk}")
            debug("Waiting for udev to settle")
            SysCommand("udevadm settle")

            self.efi_partition = f"{self.selected_disk}-part1"
            self.zfs_partition = f"{self.selected_disk}-part2"

            debug("Formatting EFI partition")
            SysCommand(f"mkfs.fat -I -F32 {self.efi_partition}")
            debug(f"Successfully formatted EFI partition")
        except Exception as e:
            error(f"Failed to create partitions: {str(e)}")
            raise

        info(f"Disk preparation complete")

    def mount_efi_partition(self, mountpoint: Path) -> None:
        if not self.efi_partition:
            raise RuntimeError("No EFI partition selected")

        debug("Mounting EFI partition")
        efi_mount = mountpoint / "boot/efi"
        efi_mount.mkdir(parents=True, exist_ok=True)

        try:
            SysCommand(f"mount {self.efi_partition} {efi_mount}")
            info(f"Mounted EFI partition at {efi_mount}")
        except SysCallError as e:
            error(f"Failed to mount EFI partition: {str(e)}")
            raise
