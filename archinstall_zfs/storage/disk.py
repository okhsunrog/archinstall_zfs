from pathlib import Path
from typing import Optional, List, Tuple

from archinstall.lib.disk import device_handler
from archinstall.tui import MenuItem, SelectMenu, MenuItemGroup
from pydantic import BaseModel, Field, field_validator
from archinstall import debug, info, error
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
import parted


# noinspection PyMethodParameters
class DiskConfig(BaseModel):
    selected_disk: Path
    efi_partition: Optional[Path] = None
    zfs_partition: Optional[Path] = None

    @field_validator('selected_disk')
    def validate_disk_path(cls, v: Path) -> Path:
        if not v.exists():
            raise ValueError(f'Disk path {v} does not exist')
        if not v.is_absolute():
            raise ValueError(f'Disk path {v} must be absolute')
        return v

    @field_validator('efi_partition', 'zfs_partition')
    def validate_partition_path(cls, v: Optional[Path]) -> Optional[Path]:
        if v is None:
            return v
        if not v.exists():
            raise ValueError(f'Partition path {v} does not exist')
        if not v.is_absolute():
            raise ValueError(f'Partition path {v} must be absolute')
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
            error(f"Failed to clear disk signatures: {str(e)}")
            raise

    def create_partitions(self) -> None:
        """Creates fresh GPT and partitions for EFI and ZFS"""
        debug("Creating partition table")
        try:
            debug(f"Zapping existing partitions")
            SysCommand(f"sgdisk -Z {self.config.selected_disk}")
            debug(f"Creating fresh GPT")
            SysCommand(f"sgdisk -o {self.config.selected_disk}")

            debug(f"Creating EFI partition ({self.partition_config.efi_size})")
            SysCommand(
                f"sgdisk -n 1:0:+{self.partition_config.efi_size} -t 1:{self.partition_config.efi_partition_type} {self.config.selected_disk}")

            debug(f"Creating ZFS partition (rest of disk)")
            SysCommand(f"sgdisk -n 2:0:0 -t 2:{self.partition_config.zfs_partition_type} {self.config.selected_disk}")

            debug("Updating kernel partition table")
            SysCommand(f"partprobe {self.config.selected_disk}")
            debug("Waiting for udev to settle")
            SysCommand("udevadm settle")

            self._format_efi_partition()
        except SysCallError as e:
            error(f"Failed to create partitions: {str(e)}")
            raise

    def _format_efi_partition(self) -> None:
        """Formats the EFI partition with FAT32"""
        debug("Formatting EFI partition")
        efi_part = f"{self.config.selected_disk}-part1"
        SysCommand(f"mkfs.fat -I -F32 {efi_part}")
        debug(f"Successfully formatted EFI partition")

    def get_partitions(self) -> List[MenuItem]:
        """Returns list of partitions for selection menus"""
        debug(f"Scanning partitions on disk: {self.config.selected_disk}")
        device = parted.getDevice(str(self.config.selected_disk))
        disk_obj = parted.Disk(device)
        partitions = []

        for p in disk_obj.partitions:
            part_name = Path(p.path).name
            by_id_part = Path(str(self.config.selected_disk) + "-part" + part_name[-1])
            partitions.append(MenuItem(f"{by_id_part} ({p.getLength()}MB)", str(by_id_part)))

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
            error(f"Failed to mount EFI partition: {str(e)}")
            raise

    def select_zfs_partition(self) -> Path:
        debug(f"Displaying partition selection menu for ZFS")
        partition_menu = SelectMenu(
            MenuItemGroup(self.get_partitions()),
            header="Select partition for ZFS pool",
        )
        selected = Path(partition_menu.run().item().value)
        info(f"Selected ZFS partition: {selected}")
        return selected


class DiskManagerBuilder:
    def __init__(self):
        self._selected_disk: Optional[Path] = None
        self._efi_partition: Optional[Path] = None
        self._zfs_partition: Optional[Path] = None

    def select_disk(self) -> 'DiskManagerBuilder':
        debug("Displaying disk selection menu")
        disk_menu = SelectMenu(
            MenuItemGroup(self._get_available_disks()),
            header="Select disk for installation",
        )
        self._selected_disk = Path(disk_menu.run().item().value)
        info(f"Selected disk: {self._selected_disk}")
        return self

    def select_efi_partition(self) -> 'DiskManagerBuilder':
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

    def destroying_build(self) -> Tuple[DiskManager, Path]:
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
                zfs_partition=zfs_partition
            )
        ), zfs_partition

    def build(self) -> DiskManager:
        """Builds manager for existing partition scenarios"""
        if not self._selected_disk or not self._efi_partition:
            raise ValueError("Disk and EFI partition must be selected")

        return DiskManager(
            DiskConfig(
                selected_disk=self._selected_disk,
                efi_partition=self._efi_partition
            )
        )

    # noinspection PyMethodMayBeStatic
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
