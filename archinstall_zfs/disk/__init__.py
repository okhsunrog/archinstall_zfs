import os
import re
import time
from pathlib import Path
from typing import Annotated

from archinstall import debug, error, info
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
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
    swap_partition: ByIdPath | None = None

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
    swap_partition_type: str = Field(default="8200")  # Linux swap
    swap_size: str | None = None  # if set, reserve tail for swap (e.g. "16G")


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
        """Creates fresh GPT and partitions for EFI, ZFS and optionally SWAP (tail)."""
        debug("Creating partition table")
        try:
            debug("Zapping existing partitions")
            SysCommand(f"sgdisk -Z {self.config.selected_disk}")
            debug("Creating fresh GPT")
            SysCommand(f"sgdisk -o {self.config.selected_disk}")

            debug(f"Creating EFI partition ({self.partition_config.efi_size})")
            SysCommand(f"sgdisk -n 1:0:+{self.partition_config.efi_size} -t 1:{self.partition_config.efi_partition_type} {self.config.selected_disk}")

            if self.partition_config.swap_size:
                debug(f"Creating ZFS partition (to tail -{self.partition_config.swap_size})")
                SysCommand(f"sgdisk -n 2:0:-{self.partition_config.swap_size} -t 2:{self.partition_config.zfs_partition_type} {self.config.selected_disk}")
                debug("Creating SWAP partition (tail)")
                SysCommand(f"sgdisk -n 3:0:0 -t 3:{self.partition_config.swap_partition_type} {self.config.selected_disk}")
            else:
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
        self._swap_size: str | None = None

    def destroying_build(self) -> tuple[DiskManager, ByIdPath]:
        """Builds manager for full disk installation"""
        if not self._selected_disk:
            raise ValueError("No disk selected")

        disk_manager = DiskManager(DiskConfig(selected_disk=self._selected_disk))
        # Configure optional swap tail if requested
        if self._swap_size:
            disk_manager.partition_config.swap_size = self._swap_size
        disk_manager.clear_disk_signatures()
        disk_manager.create_partitions()

        self._efi_partition = Path(f"{self._selected_disk}-part1")
        zfs_partition = Path(f"{self._selected_disk}-part2")
        swap_partition: Path | None = Path(f"{self._selected_disk}-part3") if self._swap_size else None

        # Wait for by-id symlinks to appear after partitioning
        self._wait_for_partition_symlink(self._efi_partition)
        self._wait_for_partition_symlink(zfs_partition)
        if swap_partition is not None:
            self._wait_for_partition_symlink(swap_partition)

        return DiskManager(
            DiskConfig(
                selected_disk=self._selected_disk,
                efi_partition=self._efi_partition,
                swap_partition=swap_partition,
            )
        ), zfs_partition

    def build(self) -> DiskManager:
        """Builds manager for existing partition scenarios"""
        # For existing/new pool flows, we must have an EFI partition.
        if not self._efi_partition:
            raise ValueError("EFI partition must be selected")

        # If a disk wasn't explicitly selected (e.g. Existing Pool mode),
        # derive it from the selected EFI partition's by-id name.
        if not self._selected_disk:
            part_name = self._efi_partition.name
            match = re.match(r"(.+)-part\d+$", part_name)
            if not match:
                raise ValueError(f"EFI partition is not a by-id partition path: {self._efi_partition}")
            base_name = match.group(1)
            derived = self._efi_partition.parent / base_name
            # Validate and normalize the derived disk path
            self._selected_disk = validate_disk_path(derived)
            if not self._selected_disk.exists():
                raise ValueError(f"Derived disk path does not exist: {self._selected_disk}")

        # Ensure EFI partition belongs to the selected disk
        if not str(self._efi_partition).startswith(f"{self._selected_disk}-part"):
            raise ValueError("Provided EFI partition does not belong to selected disk")

        return DiskManager(DiskConfig(selected_disk=self._selected_disk, efi_partition=self._efi_partition))

    # --- Internal helpers ---
    @staticmethod
    def _wait_for_partition_symlink(path: Path, timeout_seconds: float = 10.0, poll_interval: float = 0.2) -> None:
        """Wait until a by-id partition symlink exists, or raise on timeout."""
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if path.exists():
                return
            time.sleep(poll_interval)
        raise TimeoutError(f"Partition symlink did not appear: {path}")

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

    def with_selected_disk(self, by_id_disk: Path) -> "DiskManagerBuilder":
        """Set the selected disk non-interactively (expects /dev/disk/by-id path)."""
        self._selected_disk = validate_disk_path(by_id_disk)
        debug(f"Selected disk: {self._selected_disk}")
        return self

    def with_efi_partition(self, by_id_partition: Path) -> "DiskManagerBuilder":
        """Set the EFI partition non-interactively (expects /dev/disk/by-id path)."""
        self._efi_partition = validate_disk_path(by_id_partition)
        debug(f"Selected EFI partition: {self._efi_partition}")
        return self

    def with_swap_size(self, size: str | None) -> "DiskManagerBuilder":
        """Optionally set a swap size for full-disk installs (e.g. "16G")."""
        if size:
            self._swap_size = size
            debug(f"Requested swap tail size: {self._swap_size}")
        return self
