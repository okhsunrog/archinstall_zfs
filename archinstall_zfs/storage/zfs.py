from pathlib import Path
from typing import List, Dict, Tuple

from archinstall import info, error, debug
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.tui.curses_menu import EditMenu, SelectMenu, MenuItemGroup
from archinstall.tui.menu_item import MenuItem

DEFAULT_DATASETS = [
    ("root", {"mountpoint": "/"}),
    ("data/home", {"mountpoint": "/home"}),
    ("data/root", {"mountpoint": "/root"}),
    ("var", {"mountpoint": "/var"}),
    ("vm", {"mountpoint": "/vm"}),
]


class ZFSManager:
    def __init__(
            self,
            pool_name: str,
            dataset_prefix: str,
            mountpoint: Path,
            encryption: str | None,
            compression: str,
            datasets: List[Tuple[str, Dict[str, str]]]
    ):
        self.pool_name = pool_name
        self.dataset_prefix = dataset_prefix
        self.mountpoint = mountpoint
        self.encryption = encryption
        self.compression = compression
        self.datasets = datasets
        self.zfs_key_path = Path("/etc/zfs/zroot.key")
        self.pool_cache_path = Path("/etc/zfs/zpool.cache")
        self.hostid_path = Path("/etc/hostid")

    def create_datasets(self) -> None:
        base_dataset = f"{self.pool_name}/{self.dataset_prefix}"

        # Create and configure base dataset
        base_props = {
            "mountpoint": "none",
            "compression": self.compression
        }
        if self.encryption:
            base_props.update({
                "encryption": "aes-256-gcm",
                "keyformat": "passphrase",
                "keylocation": f"file://{self.zfs_key_path}"
            })

        props_str = " ".join(f"-o {k}={v}" for k, v in base_props.items())
        SysCommand(f"zfs create {props_str} {base_dataset}")

        # Create child datasets
        for dataset_path, props in self.datasets:
            full_path = f"{base_dataset}/{dataset_path}"
            props_str = " ".join(f"-o {k}={v}" for k, v in props.items())
            SysCommand(f"zfs create {props_str} {full_path}")
            debug(f"Created dataset: {full_path}")

    def export_pool(self) -> None:
        debug("Exporting ZFS pool")
        try:
            SysCommand("zfs umount -a")
            SysCommand(f"zpool export {self.pool_name}")
            info("Pool exported successfully")
        except SysCallError as e:
            error(f"Failed to export pool: {str(e)}")
            raise

    def import_pool(self) -> None:
        debug(f"Importing pool to mountpoint: {self.mountpoint}")
        try:
            SysCommand(f"zpool import -N -R {self.mountpoint} {self.pool_name}")
            if self.encryption:
                SysCommand(f"zfs load-key {self.pool_name}")
            SysCommand(f"zfs mount {self.pool_name}/{self.dataset_prefix}/root")
            SysCommand("zfs mount -a")

            debug("Setting pool cache file")
            SysCommand(f"zpool set cachefile={self.pool_cache_path} {self.pool_name}")

            debug("Copying ZFS configuration files")
            target_zfs = self.mountpoint / "etc/zfs"
            target_zfs.mkdir(parents=True, exist_ok=True)

            SysCommand(f"cp {self.pool_cache_path} {target_zfs}/")
            SysCommand(f"cp {self.hostid_path} {self.mountpoint}/etc/")
            if self.encryption:
                SysCommand(f"cp {self.zfs_key_path} {target_zfs}/")
            info("Pool imported and mounted successfully")
        except SysCallError as e:
            error(f"Failed to import/mount pool: {str(e)}")
            raise

    def verify_mounts(self) -> bool:
        # TODO: Implement a more robust check
        return False


class ZFSManagerBuilder:
    def __init__(self):
        self._pool_name: str | None = None
        self._dataset_prefix: str | None = None
        self._mountpoint: Path | None = None
        self._encryption: str | None = None
        self._compression: str = "lz4"
        self._datasets = DEFAULT_DATASETS

    def with_dataset_prefix(self, prefix: str) -> 'ZFSManagerBuilder':
        self._dataset_prefix = prefix
        return self

    def with_mountpoint(self, mountpoint: Path) -> 'ZFSManagerBuilder':
        self._mountpoint = mountpoint
        return self

    def with_compression(self, compression: str) -> 'ZFSManagerBuilder':
        self._compression = compression
        return self

    def with_datasets(self, datasets: List[Tuple[str, Dict[str, str]]]) -> 'ZFSManagerBuilder':
        self._datasets = datasets
        return self

    def select_pool_name(self) -> 'ZFSManagerBuilder':
        pool_menu = EditMenu(
            "Pool Name",
            header="Enter name for new ZFS pool",
            default_text="zroot"
        )
        self._pool_name = pool_menu.input().text()
        return self

    def select_existing_pool(self) -> 'ZFSManagerBuilder':
        debug("Displaying pool selection menu")
        pool_menu = SelectMenu(
            MenuItemGroup(self._get_available_pools()),
            header="Select existing ZFS pool"
        )
        self._pool_name = pool_menu.run().item().value
        return self

    def setup_encryption(self) -> 'ZFSManagerBuilder':
        debug("Requesting encryption password")
        while True:
            password_menu = EditMenu(
                "ZFS Encryption Password",
                header="Enter password for ZFS encryption",
                hide_input=True,
            )
            verify_menu = EditMenu(
                "Verify Password",
                header="Enter password again",
                hide_input=True
            )

            password = password_menu.input().text()
            verify = verify_menu.input().text()

            if password == verify and password:
                self._encryption = password
                debug("Encryption password verified")
                return self
            error("Password verification failed - retrying")

    def new_pool(self, partition: str) -> 'ZFSManagerBuilder':
        if not self._pool_name:
            raise RuntimeError("Pool name must be set before creating pool")

        try:
            debug("Generating host ID")
            SysCommand("zgenhostid")
        except SysCallError as e:
            if "File exists" not in str(e):
                raise

        pool_options = [
            "-o ashift=12",
            "-O acltype=posixacl",
            "-O relatime=on",
            "-O xattr=sa",
            "-o autotrim=on",
            "-O dnodesize=auto",
            "-O normalization=formD",
            "-O devices=off",
            "-m none",
        ]

        try:
            pool_cmd = f'zpool create -f {" ".join(pool_options)} {self._pool_name} {partition}'
            SysCommand(pool_cmd)
            info("ZFS pool created successfully")
        except SysCallError as e:
            error(f"Failed to create ZFS pool: {str(e)}")
            raise

        return self

    def build(self) -> ZFSManager:
        if not all([self._pool_name, self._dataset_prefix, self._mountpoint]):
            raise RuntimeError("Pool name, dataset prefix, and mountpoint must be set")

        if self._encryption:
            Path("/etc/zfs").mkdir(parents=True, exist_ok=True)
            Path("/etc/zfs/zroot.key").write_text(self._encryption, encoding="utf-8")
            Path("/etc/zfs/zroot.key").chmod(0o000)

        return ZFSManager(
            self._pool_name,
            self._dataset_prefix,
            self._mountpoint,
            self._encryption,
            self._compression,
            self._datasets
        )

    def _get_available_pools(self) -> List[MenuItem]:
        debug("Scanning for importable ZFS pools")
        try:
            output = SysCommand("zpool import").decode()
            pools = []
            for line in output.splitlines():
                if line.startswith("   pool:"):
                    pool_name = line.split(":")[1].strip()
                    pools.append(MenuItem(pool_name, pool_name))
                    debug(f"Found pool: {pool_name}")
            return pools
        except SysCallError as e:
            error(f"Failed to get pool list: {str(e)}")
            return []
