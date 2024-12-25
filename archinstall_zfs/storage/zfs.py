from pathlib import Path
from typing import List, Dict, Optional, Tuple

from archinstall import info, error, debug
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.tui.curses_menu import EditMenu, SelectMenu, MenuItemGroup
from archinstall.tui.menu_item import MenuItem


class ZFSManager:
    def __init__(self) -> None:
        self.zfs_key_path = Path("/etc/zfs/zroot.key")
        self.pool_cache_path = Path("/etc/zfs/zpool.cache")
        self.hostid_path = Path("/etc/hostid")
        self.pool_name: str = "zroot"
        self.encryption_password: str | None = None
        self.dataset_prefix: str | None = None
        self.mountpoint: Path | None = None

    def get_encryption_password(self) -> None:
        debug("Requesting encryption password")
        while True:
            password_menu = EditMenu(
                "ZFS Encryption Password",
                header="Enter password for ZFS encryption",
                hide_input=True,
            )
            verify_menu = EditMenu(
                "Verify Password", header="Enter password again", hide_input=True
            )

            password = password_menu.input().text()
            verify = verify_menu.input().text()

            if password == verify and password:
                self.encryption_password = password
                debug("Encryption password verified")
                return
            error("Password verification failed - retrying")

    def setup_encryption(self) -> None:
        if not self.encryption_password:
            raise RuntimeError("No encryption password set")

        debug("Setting up encryption key file")
        self.zfs_key_path.parent.mkdir(parents=True, exist_ok=True)
        self.zfs_key_path.write_text(self.encryption_password, encoding="utf-8")
        self.zfs_key_path.chmod(0o000)
        info("Encryption key file created")

    def create_pool(self, partition: str) -> None:
        if not self.encryption_password:
            raise RuntimeError("No encryption password set")

        debug(f"Creating ZFS pool on partition: {partition}")
        self.setup_encryption()

        try:
            debug("Generating host ID")
            SysCommand("zgenhostid")
        except SysCallError as e:
            if "File exists" not in str(e):
                error(f"Failed to generate hostid: {str(e)}")
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
            "-O compression=lz4",
            "-O encryption=aes-256-gcm",
            "-O keyformat=passphrase",
            f"-O keylocation=file://{self.zfs_key_path}",
            "-m none",
        ]

        try:
            pool_cmd = f'zpool create -f {" ".join(pool_options)} {self.pool_name} {partition}'
            debug("Executing pool creation command")
            SysCommand(pool_cmd)
            info("ZFS pool created successfully")
        except SysCallError as e:
            error(f"Failed to create ZFS pool: {str(e)}")
            raise

    def create_datasets(self) -> None:
        if not self.dataset_prefix:
            raise RuntimeError("No dataset prefix set")

        debug(f"Creating dataset structure with prefix: {self.dataset_prefix}")
        datasets: List[Tuple[str, Optional[Dict[str, str]]]] = [
            (f"{self.pool_name}/data_{self.dataset_prefix}", {"mountpoint": "none"}),
            (f"{self.pool_name}/ROOT_{self.dataset_prefix}", {"mountpoint": "none"}),
            (f"{self.pool_name}/ROOT_{self.dataset_prefix}/default", {"mountpoint": "/", "canmount": "noauto"}),
            (f"{self.pool_name}/data_{self.dataset_prefix}/home", {"mountpoint": "/home"}),
            (f"{self.pool_name}/data_{self.dataset_prefix}/root", {"mountpoint": "/root"}),
            (f"{self.pool_name}/var_{self.dataset_prefix}", {"mountpoint": "/var", "canmount": "off"}),
            (f"{self.pool_name}/var_{self.dataset_prefix}/lib", {"mountpoint": "/var/lib", "canmount": "off"}),
            (f"{self.pool_name}/var_{self.dataset_prefix}/lib/libvirt", None),
            (f"{self.pool_name}/var_{self.dataset_prefix}/lib/docker", None),
            (f"{self.pool_name}/vm_{self.dataset_prefix}", {"mountpoint": "/vm"}),
        ]

        for dataset, props in datasets:
            try:
                if props:
                    props_str = " ".join(f"-o {k}={v}" for k, v in props.items())
                    SysCommand(f"zfs create {props_str} {dataset}")
                else:
                    SysCommand(f"zfs create {dataset}")
                debug(f"Created dataset: {dataset}")
            except SysCallError as e:
                error(f"Failed to create dataset {dataset}: {str(e)}")
                raise

        debug("Setting bootfs property")
        SysCommand(f"zpool set bootfs={self.pool_name}/ROOT_{self.dataset_prefix}/default {self.pool_name}")
        info("Dataset structure created successfully")

    def import_pool(self, mountpoint: Path) -> None:
        if not self.dataset_prefix:
            raise RuntimeError("No dataset prefix set")

        self.mountpoint = mountpoint
        debug(f"Importing pool to mountpoint: {mountpoint}")
        try:
            SysCommand(f"zpool import -N -R {mountpoint} {self.pool_name}")
            SysCommand(f"zfs load-key {self.pool_name}")
            SysCommand(f"zfs mount {self.pool_name}/ROOT_{self.dataset_prefix}/default")
            SysCommand("zfs mount -a")

            debug("Setting pool cache file")
            SysCommand(f"zpool set cachefile={self.pool_cache_path} {self.pool_name}")

            debug("Copying ZFS configuration files")
            target_zfs = mountpoint / "etc/zfs"
            target_zfs.mkdir(parents=True, exist_ok=True)

            SysCommand(f"cp {self.pool_cache_path} {target_zfs}/")
            SysCommand(f"cp {self.hostid_path} {mountpoint}/etc/")
            SysCommand(f"cp {self.zfs_key_path} {target_zfs}/")
            info("Pool imported and mounted successfully")
        except SysCallError as e:
            error(f"Failed to import/mount pool: {str(e)}")
            raise

    def verify_mounts(self) -> bool:
        if not self.mountpoint or not self.dataset_prefix:
            raise RuntimeError("Mountpoint or dataset prefix not set")

        debug("Verifying dataset mounts")
        required_mounts = [
            self.mountpoint,
            self.mountpoint / "home",
            self.mountpoint / "root",
            self.mountpoint / "var/lib/docker",
            self.mountpoint / "var/lib/libvirt",
            self.mountpoint / "vm",
            self.mountpoint / "boot/efi"
        ]

        for mount in required_mounts:
            if not mount.is_mount():
                error(f"Required mount point not mounted: {mount}")
                return False
        return True

    def export_pool(self) -> None:
        debug("Exporting ZFS pool")
        try:
            SysCommand("zfs umount -a")
            SysCommand(f"zpool export {self.pool_name}")
            info("Pool exported successfully")
        except SysCallError as e:
            error(f"Failed to export pool: {str(e)}")
            raise

    def get_available_pools(self) -> List[MenuItem]:
        debug("Scanning for importable ZFS pools")
        try:
            output = SysCommand("zpool import").decode()
            pools = []
            for line in output.splitlines():
                if line.startswith("   pool:"):
                    pool_name = line.split(":")[1].strip()
                    pools.append(MenuItem(pool_name, pool_name))
                    debug(f"Found pool: {pool_name}")
            info(f"Found {len(pools)} importable pools")
            return pools
        except SysCallError as e:
            error(f"Failed to get pool list: {str(e)}")
            return []

    def select_pool(self) -> None:
        debug("Displaying pool selection menu")
        pool_menu = SelectMenu(
            MenuItemGroup(self.get_available_pools()),
            header="Select existing ZFS pool"
        )
        self.pool_name = pool_menu.run().item().value
        info(f"Selected pool: {self.pool_name}")

