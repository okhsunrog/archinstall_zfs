import os
import time
from enum import Enum
from pathlib import Path
from typing import ClassVar

from archinstall import debug, error, info
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.tui import EditMenu, MenuItem, MenuItemGroup, SelectMenu
from pydantic import BaseModel, Field, field_validator

from archinstall_zfs.utils import modify_zfs_cache_mountpoints


class DatasetConfig(BaseModel):
    name: str
    properties: dict[str, str]


DEFAULT_DATASETS = [
    DatasetConfig(name="root", properties={"mountpoint": "/", "canmount": "noauto"}),
    DatasetConfig(name="data/home", properties={"mountpoint": "/home"}),
    DatasetConfig(name="data/root", properties={"mountpoint": "/root"}),
    DatasetConfig(name="vm", properties={"mountpoint": "/vm"}),
]

ZFS_SERVICES = ["zfs.target", "zfs-import.target", "zfs-volumes.target", "zfs-import-cache.service", "zfs-zed.service"]


class EncryptionMode(Enum):
    NONE = "No encryption"
    POOL = "Encrypt entire pool"
    DATASET = "Encrypt base dataset only"


# noinspection PyMethodParameters
class ZFSConfig(BaseModel):
    pool_name: str
    dataset_prefix: str
    mountpoint: Path
    compression: str = Field(default="lz4")
    # disabled because of PyCharm bug
    # noinspection PyDataclass
    datasets: list[DatasetConfig] = Field(default_factory=list)

    @field_validator("pool_name", check_fields=False)
    def validate_pool_name(cls, v: str) -> str:
        if not v.isalnum():
            raise ValueError("Pool name must be alphanumeric")
        return v

    @field_validator("dataset_prefix", check_fields=False)
    def validate_prefix(cls, v: str) -> str:
        if not v.isalnum():
            raise ValueError("Dataset prefix must be alphanumeric")
        return v


class ZFSPaths(BaseModel):
    base_zfs: Path = Field(default=Path("/etc/zfs"))
    cache_dir: Path = Field(default=Path("/etc/zfs/zfs-list.cache"))
    key_file: Path = Field(default=Path("/etc/zfs/zroot.key"))
    hostid: Path = Field(default=Path("/etc/hostid"))
    _pool_name: str | None = None

    @property
    def pool_name(self) -> str:
        if self._pool_name is None:
            raise ValueError("Pool name not set")
        return self._pool_name

    @pool_name.setter
    def pool_name(self, value: str) -> None:
        self._pool_name = value

    @property
    def cache_file(self) -> Path:
        return self.cache_dir / self.pool_name

    @classmethod
    def create_mounted(cls, base_paths: "ZFSPaths", mountpoint: Path) -> "ZFSPaths":
        new_paths = cls(
            base_zfs=mountpoint / str(base_paths.base_zfs).lstrip("/"),
            cache_dir=mountpoint / str(base_paths.cache_dir).lstrip("/"),
            key_file=mountpoint / str(base_paths.key_file).lstrip("/"),
            hostid=mountpoint / str(base_paths.hostid).lstrip("/"),
        )
        new_paths.pool_name = base_paths.pool_name
        return new_paths

    # noinspection PyMethodParameters
    @field_validator("*")
    def validate_absolute_path(cls, v: Path) -> Path:
        if not v.is_absolute():
            raise ValueError(f"Path {v} must be absolute")
        return v


class ZFSEncryption:
    def __init__(self, key_path: Path, is_new_pool: bool, pool_name: str):
        self.key_path: Path = key_path
        self.password: str | None = None
        self.mode: EncryptionMode | None = None

        if is_new_pool:
            self._setup_new_pool_encryption()
        else:
            self._setup_existing_pool_encryption(pool_name)

    def _setup_new_pool_encryption(self) -> None:
        encryption_menu = SelectMenu(
            MenuItemGroup(
                [
                    MenuItem(EncryptionMode.NONE.value, EncryptionMode.NONE),
                    MenuItem(EncryptionMode.POOL.value, EncryptionMode.POOL),
                    MenuItem(EncryptionMode.DATASET.value, EncryptionMode.DATASET),
                ]
            ),
            header="Select encryption mode",
        )

        self.mode = encryption_menu.run().item().value
        if self.mode != EncryptionMode.NONE:
            self.password = self._get_password()

    def _setup_existing_pool_encryption(self, pool_name: str) -> None:
        if self._is_pool_encrypted(pool_name):
            debug("Detected encrypted pool")
            self.password = self._get_password()
            self.mode = EncryptionMode.POOL
            return

        encryption_menu = SelectMenu(
            MenuItemGroup([MenuItem("Yes - Encrypt new base dataset", True), MenuItem("No - Skip encryption", False)]),
            header="Do you want to encrypt the new base dataset?",
        )

        if encryption_menu.run().item().value:
            self.password = self._get_password()
            self.mode = EncryptionMode.DATASET

    def setup(self) -> None:
        if not self.password:
            debug("Encryption disabled, skipping ZFS encryption setup")
            return

        debug("Setting up ZFS encryption")
        self.key_path.parent.mkdir(parents=True, exist_ok=True)
        self.key_path.write_text(self.password)
        self.key_path.chmod(0o000)
        debug("Encryption key stored securely")

    def _get_encryption_properties(self) -> dict[str, str]:
        if not self.password:
            return {}

        return {"encryption": "aes-256-gcm", "keyformat": "passphrase", "keylocation": f"file://{self.key_path}"}

    def get_pool_properties(self) -> dict[str, str]:
        return self._get_encryption_properties() if self.mode == EncryptionMode.POOL else {}

    def get_dataset_properties(self) -> dict[str, str]:
        return self._get_encryption_properties() if self.mode == EncryptionMode.DATASET else {}

    @staticmethod
    def _is_pool_encrypted(pool_name: str) -> bool:
        try:
            SysCommand(f"zpool import -fN {pool_name}")
            output = SysCommand(f"zfs get -H encryption {pool_name}").decode()
            SysCommand(f"zpool export {pool_name}")
            return "aes-256-gcm" in output
        except SysCallError:
            return False

    @staticmethod
    def _get_password() -> str:
        while True:
            password = (
                EditMenu(
                    "ZFS Encryption Password",
                    header="Enter password for ZFS encryption",
                    hide_input=True,
                )
                .input()
                .text()
            )

            verify = EditMenu("Verify Password", header="Enter password again", hide_input=True).input().text()

            if password == verify and password:
                return password


class ZFSPool:
    """Handles ZFS pool operations"""

    DEFAULT_POOL_OPTIONS: ClassVar[list[str]] = [
        "-o ashift=12",
        "-O acltype=posixacl",
        "-O relatime=on",
        "-O xattr=sa",
        "-o autotrim=on",
        "-O dnodesize=auto",
        "-O normalization=formD",
        "-O devices=off",
        "-m none",
        "-R /mnt",
    ]

    def __init__(self, config: ZFSConfig):
        self.config: ZFSConfig = config
        self.encryption_handler: ZFSEncryption | None = None
        self._validate_pool_device()

    def create(self, device: str, encryption_handler: ZFSEncryption) -> None:
        encryption_props = encryption_handler.get_pool_properties()
        """Creates a new ZFS pool with specified options"""
        debug(f"Creating ZFS pool {self.config.pool_name} on {device}")

        options = self.DEFAULT_POOL_OPTIONS.copy()
        if encryption_props:
            for key, value in encryption_props.items():
                options.append(f"-O {key}={value}")

        cmd = f"zpool create -f {' '.join(options)} {self.config.pool_name} {device}"
        try:
            SysCommand(cmd)
            # Set pool cache file to none, as it's deprecated
            SysCommand(f"zpool set cachefile=/etc/zfs/zpool.cache {self.config.pool_name}")
            info(f"Created pool {self.config.pool_name}")
        except SysCallError as e:
            error(f"Failed to create pool: {e!s}")
            raise

    def load_key(self) -> None:
        """Load encryption key for encrypted pools"""
        debug(f"Loading encryption key for pool {self.config.pool_name}")
        try:
            SysCommand(f"zfs load-key {self.config.pool_name}")
            info("Pool encryption key loaded successfully")
        except SysCallError as e:
            error(f"Failed to load pool encryption key: {e!s}")
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
            error(f"Failed to export pool: {e!s}")
            raise

    def import_pool(self, mountpoint: Path, encryption_handler: ZFSEncryption) -> None:
        """Imports the ZFS pool at specified mountpoint"""
        debug(f"Importing pool {self.config.pool_name} to {mountpoint}")
        try:
            SysCommand(f"zpool import -N -R {mountpoint} {self.config.pool_name}")
            info("Pool imported successfully")
            if encryption_handler.password:
                self.load_key()
        except SysCallError as e:
            error(f"Failed to import pool: {e!s}")
            raise

    def _validate_pool_device(self) -> None:
        """Validates that pool device exists and is suitable for ZFS"""
        debug("Validating pool device")
        try:
            output = SysCommand("zpool list").decode()
            if self.config.pool_name in output:
                raise ValueError(f"Pool {self.config.pool_name} already exists")
            debug("Pool device validation successful")
        except SysCallError as e:
            error(f"Pool device validation failed: {e!s}")
            raise


class ZFSDatasetManager:
    """Handles ZFS dataset operations and properties"""

    def __init__(self, config: ZFSConfig, paths: ZFSPaths):
        self.config = config
        self.paths = paths
        self.base_dataset = f"{config.pool_name}/{config.dataset_prefix}"

    def create_base_dataset(self, encryption_handler: ZFSEncryption) -> None:
        """Creates and configures the base dataset with optional encryption"""
        props = {"mountpoint": "none", "compression": self.config.compression}
        props.update(encryption_handler.get_dataset_properties())

        props_str = " ".join(f"-o {k}={v}" for k, v in props.items())
        SysCommand(f"zfs create {props_str} {self.base_dataset}")
        debug(f"Created base dataset: {self.base_dataset}")

    def validate_prefix(self) -> None:
        """Validate that dataset prefix doesn't exist on the pool"""
        debug(f"Checking if prefix {self.base_dataset} exists")
        try:
            SysCommand(f"zfs list {self.base_dataset}")
            # If command succeeds, dataset exists
            raise ValueError(f"Dataset prefix {self.base_dataset} already exists on pool {self.config.pool_name}")
        except SysCallError:
            # Command failed = dataset doesn't exist, which is what we want
            debug(f"Prefix {self.base_dataset} is available")

    # noinspection PyMethodMayBeStatic
    def _get_dataset_hierarchy(self, dataset_path: str) -> list[str]:
        """Get all parent datasets for a given dataset path"""
        parts = dataset_path.split("/")
        return ["/".join(parts[: i + 1]) for i in range(len(parts))]

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
        sorted_datasets = sorted(self.config.datasets, key=lambda d: len(d.name.split("/")))

        for dataset in sorted_datasets:
            self._ensure_parent_datasets(dataset.name)
            full_path = f"{self.base_dataset}/{dataset.name}"
            props_str = " ".join(f"-o {k}={v}" for k, v in dataset.properties.items())
            debug(f"Creating dataset: {full_path}")
            SysCommand(f"zfs create {props_str} {full_path}")


class ZFSManagerBuilder:
    def __init__(self):
        self._pool_name: str | None = None
        self._dataset_prefix: str | None = None
        self._mountpoint: Path | None = None
        self._encryption_password: str | None = None
        self._encryption_mode: EncryptionMode | None = None
        self._compression: str = "lz4"
        self._datasets: list[DatasetConfig] = []
        self._device: str | None = None
        self._is_new_pool: bool = True
        self._paths: ZFSPaths = ZFSPaths()
        self._encryption_handler: ZFSEncryption | None = None

    def new_pool(self, device: Path) -> "ZFSManagerBuilder":
        pool_menu = EditMenu("Pool Name", header="Enter name for new ZFS pool", default_text="zroot")
        pool_name = pool_menu.input().text()
        info(f"Selected pool name: {pool_name}")

        self._device = str(device)
        self._pool_name = pool_name
        self._paths.pool_name = pool_name
        self._is_new_pool = True
        return self

    def select_existing_pool(self) -> "ZFSManagerBuilder":
        debug("Scanning for importable ZFS pools")
        try:
            output = SysCommand("zpool import").decode()
            pools = []
            for line in output.splitlines():
                if line.strip().startswith("pool:"):
                    pool_name = line.split(":")[1].strip()
                    debug(f"Found pool: {pool_name}")
                    pools.append(MenuItem(pool_name, pool_name))

            if not pools:
                error("No importable ZFS pools found")
                raise ValueError("No importable ZFS pools found. Make sure pools exist and are exported.")

            pool_menu = SelectMenu(MenuItemGroup(pools), header="Select existing ZFS pool")
            self._pool_name = pool_menu.run().item().value
            self._paths.pool_name = self._pool_name
            self._is_new_pool = False
            return self
        except SysCallError as e:
            error(f"Failed to get pool list: {e!s}")
            raise

    def with_pool_name(self, name: str) -> "ZFSManagerBuilder":
        self._pool_name = name
        self._paths.pool_name = name
        return self

    def with_dataset_prefix(self, prefix: str) -> "ZFSManagerBuilder":
        self._dataset_prefix = prefix
        return self

    def with_mountpoint(self, path: Path) -> "ZFSManagerBuilder":
        self._mountpoint = path
        return self

    def build(self) -> "ZFSManager":
        if not self._pool_name:
            raise ValueError("Pool name must be set before building ZFS manager")
        self._datasets = DEFAULT_DATASETS
        self._encryption_handler = ZFSEncryption(self._paths.key_file, self._is_new_pool, self._pool_name)
        config = ZFSConfig(
            pool_name=self._pool_name, dataset_prefix=self._dataset_prefix, mountpoint=self._mountpoint, compression=self._compression, datasets=self._datasets
        )
        mounted_paths = ZFSPaths.create_mounted(self._paths, self._mountpoint)
        return ZFSManager(config, self._paths, mounted_paths, self._encryption_handler, device=self._device)


class ZFSManager:
    def __init__(self, config: ZFSConfig, paths: ZFSPaths, mounted_paths: ZFSPaths, encryption_handler: ZFSEncryption, device: str | None = None):
        self.config = config
        self.device = device
        self.paths = paths
        self.mounted_paths = mounted_paths
        self.pool = ZFSPool(config)
        self.datasets = ZFSDatasetManager(config, self.paths)
        self.encryption_handler = encryption_handler

    def mount_datasets(self) -> None:
        """Mount all datasets in the correct order"""
        debug("Mounting ZFS datasets")
        SysCommand(f"zfs mount {self.config.pool_name}/{self.config.dataset_prefix}/root")
        prefix_path = f"{self.config.pool_name}/{self.config.dataset_prefix}"
        SysCommand(f"zfs mount -R {prefix_path}")
        info("All datasets mounted successfully")

    @staticmethod
    def create_hostid() -> None:
        """Create a static hostid"""
        debug("Creating static hostid")
        try:
            SysCommand("zgenhostid -f 0x00bab10c")
            info("Created static hostid")
        except SysCallError as e:
            error(f"Failed to create hostid: {e!s}")
            raise

    def prepare_zfs_cache(self) -> None:
        """Prepare ZFS cache directory and files"""
        debug("Preparing ZFS cache")

        self.paths.base_zfs.mkdir(parents=True, exist_ok=True)
        self.paths.cache_dir.mkdir(parents=True, exist_ok=True)
        self.paths.cache_file.touch()

        SysCommand("systemctl enable --now zfs-zed.service")
        info("ZFS cache prepared")

    def copy_misc_files(self) -> None:
        """Set up ZFS cache files in the target system"""
        debug("Setting up ZFS misc files")
        try:
            # Create target directories
            self.mounted_paths.base_zfs.mkdir(parents=True, exist_ok=True)

            # Read and modify cache file content
            content = self.paths.cache_file.read_text()
            modified_content = modify_zfs_cache_mountpoints(content, self.config.mountpoint)

            # Write modified content to target
            self.mounted_paths.cache_dir.mkdir(parents=True, exist_ok=True)
            self.mounted_paths.cache_file.write_text(modified_content)

            # Copy hostid
            SysCommand(f"cp {self.paths.hostid} {self.mounted_paths.hostid}")

            # Copy zpool cache
            SysCommand("cp /etc/zfs/zpool.cache /mnt/etc/zfs/zpool.cache")

            info("ZFS misc files configured successfully")
        except SysCallError as e:
            error(f"Failed to copy ZFS misc files: {e!s}")
            raise

    def copy_enc_key(self) -> None:
        """Copy encryption key to target system"""
        debug("Copying encryption key")

        self.mounted_paths.base_zfs.mkdir(parents=True, exist_ok=True)
        SysCommand(f"cp {self.paths.key_file} {self.mounted_paths.key_file}")
        info("Encryption key copied successfully")

    def genfstab(self) -> None:
        """Generate fstab entries for ZFS installation"""
        debug("Generating fstab for ZFS")
        fstab_path = self.config.mountpoint / "etc" / "fstab"

        # Generate full fstab with UUIDs
        raw_fstab = SysCommand(f"/usr/bin/genfstab -t UUID {self.config.mountpoint}").decode()

        # Filter out pool-related entries and add root dataset
        filtered_lines = [line for line in raw_fstab.splitlines() if self.config.pool_name not in line]
        root_dataset = next(ds for ds in self.config.datasets if ds.properties.get("mountpoint") == "/")
        full_dataset_path = f"{self.datasets.base_dataset}/{root_dataset.name}"
        filtered_lines.append(f"{full_dataset_path} / zfs defaults 0 0")

        # Write final fstab
        fstab_path.write_text("\n".join(filtered_lines) + "\n")
        info("Generated fstab successfully")

    def setup_for_installation(self) -> None:
        """Configure ZFS for system installation"""
        self.create_hostid()
        self.prepare_zfs_cache()
        self.encryption_handler.setup()

        if self.device:  # New pool setup
            self.pool.create(self.device, self.encryption_handler)
            self.datasets.create_base_dataset(self.encryption_handler)
            self.datasets.create_child_datasets()
            self.pool.export()
            self.pool.import_pool(self.config.mountpoint, self.encryption_handler)
        else:  # Existing pool setup
            self.pool.import_pool(self.config.mountpoint, self.encryption_handler)
            self.datasets.validate_prefix()
            self.datasets.create_base_dataset(self.encryption_handler)
            self.datasets.create_child_datasets()

        self.mount_datasets()
        if self.encryption_handler.password:
            self.copy_enc_key()

    def finish(self) -> None:
        """Clean up ZFS mounts and export pool"""
        debug("Finishing ZFS setup")
        SysCommand(f'zfs set org.zfsbootmenu:commandline="spl.spl_hostid=$(hostid) zswap.enabled=0 rw" {self.datasets.base_dataset}')
        # SysCommand(f"zfs set org.zfsbootmenu:keysource=\"{root_dataset}\" {self.config.pool_name}")

        os.sync()

        root_dataset = next(ds for ds in self.config.datasets if ds.properties.get("mountpoint") == "/")
        full_dataset_path = f"{self.datasets.base_dataset}/{root_dataset.name}"
        debug(f"Root dataset: {full_dataset_path}")
        SysCommand(f"zpool set bootfs={full_dataset_path} {self.config.pool_name}")

        # Multiple unmount attempts with different strategies
        unmount_attempts = [
            lambda: SysCommand("zfs umount -a"),
            lambda: SysCommand(f"zfs unmount {full_dataset_path}"),
            lambda: SysCommand("zfs umount -af"),  # Force unmount if needed
            lambda: SysCommand(f"zfs unmount -f {full_dataset_path}"),
        ]

        for attempt in unmount_attempts:
            try:
                attempt()
                time.sleep(1)
                os.sync()
            except Exception as e:
                debug(f"Unmount attempt: {e}")
                continue
        SysCommand(f"zpool export {self.config.pool_name}")
        info("ZFS cleanup completed")

    def setup_bootloader(self, efi_partition: Path) -> None:
        """Set up ZFSBootMenu bootloader"""
        info("Setting up ZFSBootMenu bootloader")

        # Create ZBM directory
        zbm_path = self.config.mountpoint / "boot/efi/EFI/ZBM"
        zbm_path.mkdir(parents=True, exist_ok=True)

        # Check if ZBM is already installed
        if not (zbm_path / "VMLINUZ.EFI").exists():
            # Download main ZBM image
            SysCommand(f"curl -o {zbm_path}/VMLINUZ.EFI -L https://get.zfsbootmenu.org/efi")
            # Download recovery image
            SysCommand(f"curl -o {zbm_path}/RECOVERY.EFI -L https://get.zfsbootmenu.org/efi/recovery")

            # Check for existing bootloader entries
            existing_entries = SysCommand("efibootmgr -v").decode()

            # Add main entry if not exists
            if "ZFSBootMenu" not in existing_entries:
                SysCommand(f"efibootmgr -c -d {efi_partition} -L 'ZFSBootMenu' -l '\\EFI\\ZBM\\VMLINUZ.EFI'")

            # Add recovery entry if not exists
            if "ZFSBootMenu-Recovery" not in existing_entries:
                SysCommand(f"efibootmgr -c -d {efi_partition} -L 'ZFSBootMenu-Recovery' -l '\\EFI\\ZBM\\RECOVERY.EFI'")

            info("ZFSBootMenu installed successfully")
        else:
            debug("ZFSBootMenu already installed, skipping")
