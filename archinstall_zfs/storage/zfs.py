import os
from pathlib import Path
from typing import List, Dict, Optional

from archinstall.tui import MenuItemGroup, SelectMenu, MenuItem, EditMenu
from pydantic import BaseModel, Field, field_validator
from archinstall import info, error, debug
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand


class DatasetConfig(BaseModel):
    name: str
    properties: Dict[str, str]


DEFAULT_DATASETS = [
    DatasetConfig(name="root", properties={"mountpoint": "/", "canmount": "noauto"}),
    DatasetConfig(name="data/home", properties={"mountpoint": "/home"}),
    DatasetConfig(name="data/root", properties={"mountpoint": "/root"}),
    DatasetConfig(name="var", properties={"mountpoint": "/var", "canmount": "off"}),
    DatasetConfig(name="var/lib", properties={"mountpoint": "/var/lib", "canmount": "off"}),
    DatasetConfig(name="var/lib/libvirt", properties={"mountpoint": "/var/lib/libvirt"}),
    DatasetConfig(name="var/lib/docker", properties={"mountpoint": "/var/lib/docker"}),
    DatasetConfig(name="vm", properties={"mountpoint": "/vm"})
]


# noinspection PyMethodParameters
class ZFSConfig(BaseModel):
    pool_name: str
    dataset_prefix: str
    mountpoint: Path
    encryption_password: Optional[str] = None
    compression: str = Field(default="lz4")
    # disabled because of PyCharm bug
    # noinspection PyDataclass
    datasets: List[DatasetConfig] = Field(default_factory=list)

    @field_validator('pool_name', check_fields=False)
    def validate_pool_name(cls, v: str) -> str:
        if not v.isalnum():
            raise ValueError('Pool name must be alphanumeric')
        return v

    @field_validator('dataset_prefix', check_fields=False)
    def validate_prefix(cls, v: str) -> str:
        if not v.isalnum():
            raise ValueError('Dataset prefix must be alphanumeric')
        return v


class ZFSPaths(BaseModel):
    zfs_key: Path = Field(default=Path("/etc/zfs/zroot.key"))
    pool_cache: Path = Field(default=Path("/etc/zfs/zpool.cache"))
    hostid: Path = Field(default=Path("/etc/hostid"))

    # noinspection PyMethodParameters
    @field_validator('zfs_key', 'pool_cache', 'hostid')
    def validate_path(cls, v: Path) -> Path:
        if not v.is_absolute():
            raise ValueError(f'Path {v} must be absolute')
        return v


class ZFSPool:
    """Handles ZFS pool operations"""
    DEFAULT_POOL_OPTIONS = [
        "-o ashift=12",
        "-O acltype=posixacl",
        "-O relatime=on",
        "-O xattr=sa",
        "-o autotrim=on",
        "-O dnodesize=auto",
        "-O normalization=formD",
        "-O devices=off",
        "-m none",
        "-R /mnt"
    ]

    def __init__(self, config: ZFSConfig):
        self.config = config
        self._validate_pool_device()

    def create(self, device: str) -> None:
        """Creates a new ZFS pool with specified options"""
        debug(f"Creating ZFS pool {self.config.pool_name} on {device}")
        cmd = f"zpool create -f {' '.join(self.DEFAULT_POOL_OPTIONS)} {self.config.pool_name} {device}"
        try:
            SysCommand(cmd)
            info(f"Created pool {self.config.pool_name}")
        except SysCallError as e:
            error(f"Failed to create pool: {str(e)}")
            raise

    def export(self) -> None:
        """Exports the ZFS pool"""
        debug(f"Exporting pool {self.config.pool_name}")
        try:
            os.sync()
            SysCommand("zfs umount -a")
            SysCommand(f"zpool export {self.config.pool_name}")
            info("Pool exported successfully")
        except SysCallError as e:
            error(f"Failed to export pool: {str(e)}")
            raise

    def import_pool(self, mountpoint: Path) -> None:
        """Imports the ZFS pool at specified mountpoint"""
        debug(f"Importing pool {self.config.pool_name} to {mountpoint}")
        try:
            SysCommand(f"zpool import -N -R {mountpoint} {self.config.pool_name}")
            if self.config.encryption_password:
                SysCommand(f"zfs load-key {self.config.pool_name}")
            info("Pool imported successfully")
        except SysCallError as e:
            error(f"Failed to import pool: {str(e)}")
            raise

    def _validate_pool_device(self) -> None:
        """Validates that pool device exists and is suitable for ZFS"""
        debug("Validating pool device")
        try:
            # Check if pool already exists
            output = SysCommand("zpool list").decode()
            if self.config.pool_name in output:
                raise ValueError(f"Pool {self.config.pool_name} already exists")

            # Additional validation can be added here:
            # - Check device permissions
            # - Verify device size
            # - Check device type

            debug("Pool device validation successful")
        except SysCallError as e:
            error(f"Pool device validation failed: {str(e)}")
            raise


class ZFSDatasetManager:
    """Handles ZFS dataset operations and properties"""

    def __init__(self, config: ZFSConfig, paths: ZFSPaths):
        self.config = config
        self.paths = paths
        self.base_dataset = f"{config.pool_name}/{config.dataset_prefix}"

    def create_base_dataset(self) -> None:
        """Creates and configures the base dataset with encryption if enabled"""
        props = {
            "mountpoint": "none",
            "compression": self.config.compression
        }

        if self.config.encryption_password:
            props.update({
                "encryption": "aes-256-gcm",
                "keyformat": "passphrase",
                "keylocation": f"file://{self.paths.zfs_key}"
            })

        props_str = " ".join(f"-o {k}={v}" for k, v in props.items())
        SysCommand(f"zfs create {props_str} {self.base_dataset}")
        debug(f"Created base dataset: {self.base_dataset}")

    # noinspection PyMethodMayBeStatic
    def _get_dataset_hierarchy(self, dataset_path: str) -> list[str]:
        """Get all parent datasets for a given dataset path"""
        parts = dataset_path.split('/')
        return ['/'.join(parts[:i + 1]) for i in range(len(parts))]

    def _ensure_parent_datasets(self, dataset_name: str) -> None:
        """Creates parent datasets if they don't exist"""
        hierarchy = self._get_dataset_hierarchy(dataset_name)
        for parent in hierarchy[:-1]:  # Exclude the dataset itself
            full_path = f"{self.base_dataset}/{parent}"
            try:
                SysCommand(f"zfs list {full_path}")
            except SysCallError:
                debug(f"Creating parent dataset: {full_path}")
                SysCommand(f"zfs create -o mountpoint=none {full_path}")

    def create_child_datasets(self) -> None:
        """Creates all datasets with proper hierarchy"""
        # Sort datasets by depth to ensure proper creation order
        sorted_datasets = sorted(self.config.datasets, key=lambda d: len(d.name.split('/')))

        for dataset in sorted_datasets:
            self._ensure_parent_datasets(dataset.name)
            full_path = f"{self.base_dataset}/{dataset.name}"
            props_str = " ".join(f"-o {k}={v}" for k, v in dataset.properties.items())
            debug(f"Creating dataset: {full_path}")
            SysCommand(f"zfs create {props_str} {full_path}")


class ZFSEncryption:
    """Handles ZFS encryption operations"""

    def __init__(self, password: Optional[str], key_path: Path):
        self.password = password
        self.key_path = key_path

    def setup(self) -> None:
        """Sets up encryption if enabled"""
        if not self.password:
            debug("Encryption disabled, skipping ZFS encryption setup")
            return

        debug("Setting up ZFS encryption")
        self.key_path.parent.mkdir(parents=True, exist_ok=True)
        self.key_path.write_text(self.password)
        self.key_path.chmod(0o000)
        debug("Encryption key stored securely")

    def get_dataset_properties(self) -> Dict[str, str]:
        """Returns encryption properties for dataset creation"""
        if not self.password:
            return {}

        return {
            "encryption": "aes-256-gcm",
            "keyformat": "passphrase",
            "keylocation": f"file://{self.key_path}"
        }

    @staticmethod
    def setup_encryption() -> Optional[str]:
        """Interactive encryption setup, returns password or None"""
        encryption_menu = SelectMenu(
            MenuItemGroup([
                MenuItem("Yes - Enable ZFS encryption", True),
                MenuItem("No - Skip encryption", False)
            ]),
            header="Do you want to enable ZFS encryption?"
        )

        if not encryption_menu.run().item().value:
            debug("Encryption disabled")
            return None

        return ZFSEncryption._get_password()

    @staticmethod
    def _get_password() -> str:
        while True:
            password = EditMenu(
                "ZFS Encryption Password",
                header="Enter password for ZFS encryption",
                hide_input=True,
            ).input().text()

            verify = EditMenu(
                "Verify Password",
                header="Enter password again",
                hide_input=True
            ).input().text()

            if password == verify and password:
                return password


class ZFSManagerBuilder:
    def __init__(self):
        self._pool_name: Optional[str] = None
        self._dataset_prefix: Optional[str] = None
        self._mountpoint: Optional[Path] = None
        self._encryption_handler: Optional[ZFSEncryption] = None
        self._compression: str = "lz4"
        self._datasets: List[DatasetConfig] = []
        self._device: Optional[str] = None
        self._is_new_pool: bool = True

    def select_pool_name(self) -> 'ZFSManagerBuilder':
        pool_menu = EditMenu(
            "Pool Name",
            header="Enter name for new ZFS pool",
            default_text="zroot"
        )
        self._pool_name = pool_menu.input().text()
        info(f"Selected pool name: {self._pool_name}")
        return self

    def new_pool(self, device: Path) -> 'ZFSManagerBuilder':
        self._device = str(device)  # Convert Path to str for ZFS commands
        self._is_new_pool = True
        return self

    def select_existing_pool(self) -> 'ZFSManagerBuilder':
        debug("Scanning for importable ZFS pools")
        try:
            output = SysCommand("zpool import").decode()
            pools = []
            for line in output.splitlines():
                if line.startswith("   pool:"):
                    pool_name = line.split(":")[1].strip()
                    pools.append(MenuItem(pool_name, pool_name))

            pool_menu = SelectMenu(
                MenuItemGroup(pools),
                header="Select existing ZFS pool"
            )
            self._pool_name = pool_menu.run().item().value
            self._is_new_pool = False
            return self
        except SysCallError as e:
            error(f"Failed to get pool list: {str(e)}")
            raise

    def with_pool_name(self, name: str) -> 'ZFSManagerBuilder':
        self._pool_name = name
        return self

    def with_dataset_prefix(self, prefix: str) -> 'ZFSManagerBuilder':
        self._dataset_prefix = prefix
        return self

    def with_mountpoint(self, path: Path) -> 'ZFSManagerBuilder':
        self._mountpoint = path
        return self

    def setup_encryption(self) -> 'ZFSManagerBuilder':
        password = ZFSEncryption.setup_encryption()
        if password:
            self._encryption_handler = ZFSEncryption(password, Path("/etc/zfs/zroot.key"))
        return self

    def build(self) -> 'ZFSManager':
        self._datasets = DEFAULT_DATASETS  # add configuration here later
        config = ZFSConfig(
            pool_name=self._pool_name,
            dataset_prefix=self._dataset_prefix,
            mountpoint=self._mountpoint,
            encryption_password=self._encryption_handler.password if self._encryption_handler else None,
            compression=self._compression,
            datasets=self._datasets
        )
        return ZFSManager(config, device=self._device)


class ZFSManager:
    def __init__(self, config: ZFSConfig, device: str | None = None):
        self.config = config
        self.device = device
        self.paths = ZFSPaths()
        self.pool = ZFSPool(config)
        self.datasets = ZFSDatasetManager(config, self.paths)
        self.encryption_handler = ZFSEncryption(config.encryption_password, self.paths.zfs_key)

    def mount_datasets(self) -> None:
        """Mount all datasets in the correct order"""
        debug("Mounting ZFS datasets")
        try:
            SysCommand(f"zfs mount {self.config.pool_name}/{self.config.dataset_prefix}/root")
            SysCommand("zfs mount -a")
            info("All datasets mounted successfully")
        except SysCallError as e:
            error(f"Failed to mount datasets: {str(e)}")

    def setup_cache_files(self, mountpoint: Path) -> None:
        """Set up ZFS cache files in the target system"""
        debug("Setting up ZFS cache files")
        try:
            # Set pool cache file
            SysCommand(f"zpool set cachefile={self.paths.pool_cache} {self.config.pool_name}")

            # Create target directories
            target_zfs = mountpoint / "etc/zfs"
            target_zfs.mkdir(parents=True, exist_ok=True)

            # Copy configuration files
            SysCommand(f"cp {self.paths.pool_cache} {target_zfs}/")
            SysCommand(f"cp {self.paths.hostid} {mountpoint}/etc/")

            # Copy encryption key if encryption is enabled
            if self.config.encryption_password:
                SysCommand(f"cp {self.paths.zfs_key} {target_zfs}/")

            info("ZFS cache files configured successfully")
        except SysCallError as e:
            error(f"Failed to setup cache files: {str(e)}")
            raise

    def prepare(self) -> None:
        """Main workflow for preparing ZFS setup"""
        try:
            debug("Generating host ID")
            SysCommand("zgenhostid")
        except SysCallError as e:
            if "File exists" not in str(e):
                raise
        self.encryption_handler.setup()
        if self.device:  # New pool setup
            self.pool.create(self.device)
            self.datasets.create_base_dataset()
            self.datasets.create_child_datasets()
            self.pool.export()

    def setup_for_installation(self, mountpoint: Path) -> None:
        """Configure ZFS for system installation"""
        self.pool.import_pool(mountpoint)
        self.mount_datasets()
        self.setup_cache_files(mountpoint)
