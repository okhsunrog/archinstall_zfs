import os
from pathlib import Path
from typing import Annotated

import parted  # type: ignore[import-untyped]
from archinstall import debug, error, info
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.tui import MenuItem, MenuItemGroup, SelectMenu
from pydantic import BaseModel, BeforeValidator, Field, field_validator


def validate_disk_path(path: Path | str) -> Path:
    """Ensure disk path is in by-id format"""
    path = Path(path)
    if not path.is_relative_to("/dev/disk/by-id"):
        raise ValueError(f"Disk path must be in /dev/disk/by-id format: {path}")
    return path


ByIdPath = Annotated[Path, BeforeValidator(validate_disk_path)]


class DiskConfig(BaseModel):
    selected_disk: ByIdPath
    efi_partition: ByIdPath | None = None

    # noinspection PyMethodParameters
    @field_validator("selected_disk", "efi_partition", check_fields=False)
    def validate_path_exists(cls, v: ByIdPath | None) -> ByIdPath | None:
        if v is not None and not v.exists():
            raise ValueError(f"Path does not exist: {v}")
        return v


class PartitionConfig(BaseModel):
    """Configuration for partition sizes and types"""

    efi_size: str = Field(default="500M")
    efi_filesystem: str = Field(default="fat32")
    efi_partition_type: str = Field(default="ef00")  # EFI System Partition
    zfs_partition_type: str = Field(default="bf00")  # Solaris/ZFS


class DiskManager:
    """Handles disk operations and partitioning"""

    def __init__(self, config: DiskConfig):
        self.config = config
        self.partition_config = PartitionConfig()

    def clear_disk_signatures(self) -> None:
        """Clears existing disk signatures to prevent conflicts"""
        debug("Clearing disk signatures")
        try:
            debug(f"Zeroing first 34 sectors of {self.config.selected_disk}")
            SysCommand(f"dd if=/dev/zero bs=512 count=34 status=progress oflag=sync of={self.config.selected_disk}")
            sectors = int(SysCommand(f"blockdev --getsz {self.config.selected_disk}").decode().strip())
            seek_position = sectors - 34
            debug(f"Zeroing last 34 sectors at position {seek_position}")
            SysCommand(f"dd if=/dev/zero of={self.config.selected_disk} bs=512 count=34 seek={seek_position}")
        except SysCallError as e:
            error(f"Failed to clear disk signatures: {e!s}")
            raise

    def create_partitions(self) -> None:
        """Creates fresh GPT and partitions for EFI and ZFS"""
        debug("Creating partition table")
        try:
            debug("Zapping existing partitions")
            SysCommand(f"sgdisk -Z {self.config.selected_disk}")
            debug("Creating fresh GPT")
            SysCommand(f"sgdisk -o {self.config.selected_disk}")

            debug(f"Creating EFI partition ({self.partition_config.efi_size})")
            SysCommand(f"sgdisk -n 1:0:+{self.partition_config.efi_size} -t 1:{self.partition_config.efi_partition_type} {self.config.selected_disk}")

            debug("Creating ZFS partition (rest of disk)")
            SysCommand(f"sgdisk -n 2:0:0 -t 2:{self.partition_config.zfs_partition_type} {self.config.selected_disk}")

            debug("Updating kernel partition table")
            SysCommand(f"partprobe {self.config.selected_disk}")
            debug("Waiting for udev to settle")
            SysCommand("udevadm settle")

            self._format_efi_partition()
        except SysCallError as e:
            error(f"Failed to create partitions: {e!s}")
            raise

    def _format_efi_partition(self) -> None:
        """Formats the EFI partition with FAT32"""
        debug("Formatting EFI partition")
        efi_part = f"{self.config.selected_disk}-part1"
        SysCommand(f"mkfs.fat -I -F32 {efi_part}")
        debug("Successfully formatted EFI partition")

    def get_partitions(self) -> list[MenuItem]:
        """Returns list of partitions for selection menus"""
        debug(f"Scanning partitions on disk: {self.config.selected_disk}")
        device = parted.getDevice(str(self.config.selected_disk))
        disk_obj = parted.Disk(device)
        partitions = []

        MB_TO_GB_THRESHOLD = 1024
        for p in disk_obj.partitions:
            size_mb = (p.getLength() * device.sectorSize) / (1024 * 1024)
            size_display = f"{size_mb:.1f}M" if size_mb < MB_TO_GB_THRESHOLD else f"{size_mb / MB_TO_GB_THRESHOLD:.1f}G"
            part_name = Path(p.path).name
            by_id_part = Path(str(self.config.selected_disk) + "-part" + part_name[-1])
            partitions.append(MenuItem(f"{by_id_part} ({size_display})", str(by_id_part)))

        info(f"Found {len(partitions)} partitions")
        return partitions

    def mount_efi_partition(self, mountpoint: Path) -> None:
        """Mounts EFI partition at the specified mountpoint"""
        debug("Mounting EFI partition")
        efi_mount = mountpoint / "boot/efi"
        efi_mount.mkdir(parents=True, exist_ok=True)

        try:
            SysCommand(f"mount {self.config.efi_partition} {efi_mount}")
            info(f"Mounted EFI partition at {efi_mount}")
        except SysCallError as e:
            error(f"Failed to mount EFI partition: {e!s}")
            raise

    def select_zfs_partition(self) -> ByIdPath:
        debug("Displaying partition selection menu for ZFS")
        partition_menu = SelectMenu(
            MenuItemGroup(self.get_partitions()),
            header="Select partition for ZFS pool",
        )
        selected = Path(partition_menu.run().item().value)
        info(f"Selected ZFS partition: {selected}")
        return selected

    @staticmethod
    def finish(mountpoint: Path) -> None:
        """Clean up EFI mounts"""
        os.sync()
        efi_path = mountpoint / "boot/efi"
        SysCommand(f"umount {efi_path}")
        info("EFI partitions unmounted")


class DiskManagerBuilder:
    def __init__(self) -> None:
        self._selected_disk: ByIdPath | None = None
        self._efi_partition: ByIdPath | None = None

    def select_efi_partition(self) -> "DiskManagerBuilder":
        if not self._selected_disk:
            raise ValueError("No disk selected")

        debug("Displaying EFI partition selection menu")
        disk_manager = DiskManager(DiskConfig(selected_disk=self._selected_disk))
        partition_menu = SelectMenu(
            MenuItemGroup(disk_manager.get_partitions()),
            header="Select EFI partition",
        )
        self._efi_partition = Path(partition_menu.run().item().value)
        info(f"Selected EFI partition: {self._efi_partition}")
        return self

    def destroying_build(self) -> tuple[DiskManager, ByIdPath]:
        """Builds manager for full disk installation"""
        if not self._selected_disk:
            raise ValueError("No disk selected")

        disk_manager = DiskManager(DiskConfig(selected_disk=self._selected_disk))
        disk_manager.clear_disk_signatures()
        disk_manager.create_partitions()

        self._efi_partition = Path(f"{self._selected_disk}-part1")
        zfs_partition = Path(f"{self._selected_disk}-part2")

        return DiskManager(
            DiskConfig(
                selected_disk=self._selected_disk,
                efi_partition=self._efi_partition,
            )
        ), zfs_partition

    def build(self) -> DiskManager:
        """Builds manager for existing partition scenarios"""
        if not self._selected_disk or not self._efi_partition:
            raise ValueError("Disk and EFI partition must be selected")

        return DiskManager(DiskConfig(selected_disk=self._selected_disk, efi_partition=self._efi_partition))

    @staticmethod
    def get_disk_by_id(disk_path: str) -> str:
        """Convert /dev/sdX path to /dev/disk/by-id path"""
        debug(f"Getting by-id path for disk: {disk_path}")

        disk_name = Path(disk_path).name
        by_id_path = Path("/dev/disk/by-id")

        for path in by_id_path.iterdir():
            if path.is_symlink() and path.readlink().name == disk_name and not path.name.split("-")[-1].startswith("part"):
                debug(f"Found by-id path: {path}")
                return str(path)

        error(f"No by-id path found for disk: {disk_path}")
        raise RuntimeError(f"Could not find /dev/disk/by-id path for {disk_path}")

    # noinspection PyMethodMayBeStatic
    def _get_available_disks(self) -> list[MenuItem]:
        debug("Scanning for available disks using parted")
        disks = []

        for device in parted.getAllDevices():
            if device.path.startswith("/dev/sd") or device.path.startswith("/dev/nvme"):
                size_gb = device.length * device.sectorSize / (1024**3)
                disks.append(MenuItem(f"{device.path} ({size_gb:.1f}GB)", device.path))
                debug(f"Found disk: {device.path}")

        info(f"Found {len(disks)} available disks")
        return disks

    def select_disk(self) -> "DiskManagerBuilder":
        debug("Displaying disk selection menu")
        disk_menu = SelectMenu(
            MenuItemGroup(self._get_available_disks()),
            header="Select disk for installation",
        )
        selected_path = disk_menu.run().item().value
        by_id_path = self.get_disk_by_id(selected_path)
        self._selected_disk = Path(by_id_path)
        info(f"Selected disk: {self._selected_disk}")
        return self
