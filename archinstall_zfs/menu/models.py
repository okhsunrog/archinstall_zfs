from __future__ import annotations

from enum import Enum

from pydantic import BaseModel, ConfigDict, field_validator


class InstallationMode(Enum):
    FULL_DISK = "full_disk"
    NEW_POOL = "new_pool"
    EXISTING_POOL = "existing_pool"


class InitSystem(Enum):
    DRACUT = "dracut"
    MKINITCPIO = "mkinitcpio"


class ZFSModuleMode(Enum):
    PRECOMPILED = "precompiled"
    DKMS = "dkms"


class ZFSEncryptionMode(Enum):
    NONE = "none"
    POOL = "pool"
    DATASET = "dataset"


class GlobalConfig(BaseModel):
    """Strongly typed configuration edited by the global menu.

    The model is used for validation and JSON I/O. Enums are stored by value
    for stable, human-readable JSON representation.
    """

    model_config = ConfigDict(use_enum_values=True)

    # Global flow
    installation_mode: InstallationMode | None = None
    disk_by_id: str | None = None
    efi_partition_by_id: str | None = None
    zfs_partition_by_id: str | None = None  # used for NEW_POOL
    pool_name: str | None = None

    # ZFS specifics
    dataset_prefix: str = "arch0"
    init_system: InitSystem = InitSystem.DRACUT
    zfs_module_mode: ZFSModuleMode = ZFSModuleMode.PRECOMPILED
    zfs_encryption_mode: ZFSEncryptionMode = ZFSEncryptionMode.NONE
    zfs_encryption_password: str | None = None

    @field_validator("pool_name")
    @classmethod
    def _validate_pool_name(cls, v: str | None) -> str | None:
        if v and not v.isalnum():
            raise ValueError("Pool name must be alphanumeric")
        return v

    @field_validator("dataset_prefix")
    @classmethod
    def _validate_dataset_prefix(cls, v: str) -> str:
        if not v.isalnum():
            raise ValueError("Dataset prefix must be alphanumeric")
        return v

    def validate_for_install(self) -> list[str]:
        """Return a list of validation errors; empty when valid."""
        errors: list[str] = []

        if self.zfs_encryption_mode is not ZFSEncryptionMode.NONE and not self.zfs_encryption_password:
            errors.append("ZFS encryption password is required when encryption is enabled")

        if not self.installation_mode:
            errors.append("Installation mode is required")
            return errors

        if self.installation_mode is InstallationMode.FULL_DISK:
            if not self.disk_by_id:
                errors.append("Target disk (/dev/disk/by-id) is required for full disk installation")
            if not self.pool_name:
                errors.append("ZFS pool name is required for full disk installation")

        if self.installation_mode is InstallationMode.NEW_POOL:
            if not self.efi_partition_by_id:
                errors.append("EFI partition (/dev/disk/by-id) is required for new pool installation")
            if not self.zfs_partition_by_id:
                errors.append("ZFS partition (/dev/disk/by-id) is required for new pool installation")
            if not self.pool_name:
                errors.append("ZFS pool name is required for new pool installation")

        if self.installation_mode is InstallationMode.EXISTING_POOL:
            if not self.efi_partition_by_id:
                errors.append("EFI partition (/dev/disk/by-id) is required for existing pool installation")
            if not self.pool_name:
                errors.append("ZFS pool name is required for existing pool installation")

        return errors

    def to_json(self) -> dict:
        return self.model_dump(mode="json", exclude_none=True)

    @classmethod
    def from_json(cls, data: dict) -> GlobalConfig:
        return cls.model_validate(data)
