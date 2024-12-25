from pathlib import Path
from typing import List, Tuple

from archinstall import debug, info, error
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.lib.disk import device_handler
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup
from archinstall.tui.menu_item import MenuItem
import parted


class DiskManager:
    def __init__(self, selected_disk: str, efi_partition: str):
        self.selected_disk = selected_disk  # by-id path
        self.efi_partition = efi_partition  # by-id partition path

    def select_zfs_partition(self) -> str:
        debug(f"Displaying partition selection menu for ZFS")
        partition_menu = SelectMenu(
            MenuItemGroup(self._get_partitions()),
            header="Select partition for ZFS pool",
        )
        selected = partition_menu.run().item().value
        info(f"Selected ZFS partition: {selected}")
        return selected

    def _get_partitions(self) -> List[MenuItem]:
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

    def mount_efi_partition(self, mountpoint: Path) -> None:
        debug("Mounting EFI partition")
        efi_mount = mountpoint / "boot/efi"
        efi_mount.mkdir(parents=True, exist_ok=True)

        try:
            SysCommand(f"mount {self.efi_partition} {efi_mount}")
            info(f"Mounted EFI partition at {efi_mount}")
        except SysCallError as e:
            error(f"Failed to mount EFI partition: {str(e)}")
            raise


class DiskManagerBuilder:
    def __init__(self):
        self._selected_disk: str | None = None
        self._efi_partition: str | None = None

    def select_disk(self) -> 'DiskManagerBuilder':
        debug("Displaying disk selection menu")
        disk_menu = SelectMenu(
            MenuItemGroup(self._get_available_disks()),
            header="Select disk for installation",
        )
        self._selected_disk = disk_menu.run().item().value
        info(f"Selected disk: {self._selected_disk}")
        return self

    def select_efi_partition(self) -> 'DiskManagerBuilder':
        if not self._selected_disk:
            raise RuntimeError("No disk selected")

        debug("Displaying EFI partition selection menu")
        partition_menu = SelectMenu(
            MenuItemGroup(self._get_partitions()),
            header="Select EFI partition",
        )
        self._efi_partition = partition_menu.run().item().value
        info(f"Selected EFI partition: {self._efi_partition}")
        return self

    def build(self) -> DiskManager:
        if not self._selected_disk or not self._efi_partition:
            raise RuntimeError("Disk and EFI partition must be selected")
        return DiskManager(self._selected_disk, self._efi_partition)

    def destroying_build(self) -> Tuple[DiskManager, str]:
        if not self._selected_disk:
            raise RuntimeError("No disk selected")

        self._prepare_disk()
        self._efi_partition = f"{self._selected_disk}-part1"
        zfs_partition = f"{self._selected_disk}-part2"

        return DiskManager(self._selected_disk, self._efi_partition), zfs_partition

    def _get_available_disks(self) -> List[MenuItem]:
        debug("Scanning for available disks")
        disks = []
        for dev in device_handler.devices:
            by_id_path = Path("/dev/disk/by-id").glob(f"*{dev.device_info.path.name}")
            disk_path = next(by_id_path, dev.device_info.path)
            if disk_path.is_relative_to("/dev/disk/by-id"):
                disks.append(MenuItem(str(disk_path), str(disk_path)))
                debug(f"Found disk: {disk_path}")
        info(f"Found {len(disks)} available disks")
        return disks

    def _get_partitions(self) -> List[MenuItem]:
        if not self._selected_disk:
            raise RuntimeError("No disk selected")

        debug(f"Scanning partitions on disk: {self._selected_disk}")
        device = parted.getDevice(self._selected_disk)
        disk_obj = parted.Disk(device)
        partitions = []

        for p in disk_obj.partitions:
            part_name = Path(p.path).name
            by_id_part = Path(self._selected_disk + "-part" + part_name[-1])
            partitions.append(MenuItem(f"{by_id_part} ({p.getLength()}MB)", str(by_id_part)))

        info(f"Found {len(partitions)} partitions")
        return partitions

    def _prepare_disk(self) -> None:
        if not self._selected_disk:
            raise RuntimeError("No disk selected")

        info(f"Preparing disk: {self._selected_disk}")
        self._clear_disk_signatures()
        self._create_partitions()

    def _clear_disk_signatures(self) -> None:
        debug("Clearing disk signatures")
        try:
            debug(f"Zeroing first 34 sectors of {self._selected_disk}")
            SysCommand(f"dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={self._selected_disk}")
            sectors = int(SysCommand(f"blockdev --getsz {self._selected_disk}").decode().strip())
            seek_position = sectors - 34
            debug(f"Zeroing last 34 sectors at position {seek_position}")
            SysCommand(f"dd if=/dev/zero of={self._selected_disk} bs=512 count=34 seek={seek_position}")
        except Exception as e:
            error(f"Failed to clear disk signatures: {str(e)}")
            raise

    def _create_partitions(self) -> None:
        debug("Creating partition table")
        try:
            debug(f"Zapping existing partitions")
            SysCommand(f"sgdisk -Z {self._selected_disk}")
            debug(f"Creating fresh GPT")
            SysCommand(f"sgdisk -o {self._selected_disk}")

            debug(f"Creating EFI partition (500MB)")
            SysCommand(f"sgdisk -n 1:0:+500M -t 1:ef00 {self._selected_disk}")

            debug(f"Creating ZFS partition (rest of disk)")
            SysCommand(f"sgdisk -n 2:0:0 -t 2:bf00 {self._selected_disk}")

            debug("Updating kernel partition table")
            SysCommand(f"partprobe {self._selected_disk}")
            debug("Waiting for udev to settle")
            SysCommand("udevadm settle")

            debug("Formatting EFI partition")
            efi_part = f"{self._selected_disk}-part1"
            SysCommand(f"mkfs.fat -I -F32 {efi_part}")
            debug(f"Successfully formatted EFI partition")
        except Exception as e:
            error(f"Failed to create partitions: {str(e)}")
            raise
