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

    def select_pool(self) -> str:
        debug("Displaying pool selection menu")
        pool_menu = SelectMenu(
            MenuItemGroup(self.get_available_pools()), header="Select existing ZFS pool"
        )
        selected = pool_menu.run().item().value
        info(f"Selected pool: {selected}")
        return selected

    def get_encryption_password(self) -> str:
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
                debug("Encryption password verified")
                return password
            error("Password verification failed - retrying")

    def setup_encryption(self, password: str) -> None:
        debug("Setting up encryption key file")
        self.zfs_key_path.parent.mkdir(parents=True, exist_ok=True)
        self.zfs_key_path.write_text(password, encoding="utf-8")
        self.zfs_key_path.chmod(0o000)
        info("Encryption key file created")

    def create_pool(self, partition: str, prefix: str, encryption_password: str) -> str:
        debug(f"Creating ZFS pool on partition: {partition}")
        self.setup_encryption(encryption_password)

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
            pool_cmd = f'zpool create -f {" ".join(pool_options)} zroot {partition}'
            debug("Executing pool creation command")
            SysCommand(pool_cmd)
            info("ZFS pool created successfully")

            self.create_datasets(prefix)
            debug("Exporting new pool")
            SysCommand("zpool export zroot")
            return "zroot"
        except SysCallError as e:
            error(f"Failed to create ZFS pool: {str(e)}")
            raise

    def create_datasets(self, prefix: str) -> None:
        debug(f"Creating dataset structure with prefix: {prefix}")
        datasets: List[Tuple[str, Optional[Dict[str, str]]]] = [
            (f"zroot/data_{prefix}", {"mountpoint": "none"}),
            (f"zroot/ROOT_{prefix}", {"mountpoint": "none"}),
            (f"zroot/ROOT_{prefix}/default", {"mountpoint": "/", "canmount": "noauto"}),
            (f"zroot/data_{prefix}/home", {"mountpoint": "/home"}),
            (f"zroot/data_{prefix}/root", {"mountpoint": "/root"}),
            (f"zroot/var_{prefix}", {"mountpoint": "/var", "canmount": "off"}),
            (f"zroot/var_{prefix}/lib", {"mountpoint": "/var/lib", "canmount": "off"}),
            (f"zroot/var_{prefix}/lib/libvirt", None),
            (f"zroot/var_{prefix}/lib/docker", None),
            (f"zroot/vm_{prefix}", {"mountpoint": "/vm"}),
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
        SysCommand(f"zpool set bootfs=zroot/ROOT_{prefix}/default zroot")
        info("Dataset structure created successfully")

    def import_pool(self, prefix: str, mountpoint: Path) -> None:
        debug(f"Importing pool to mountpoint: {mountpoint}")
        try:
            SysCommand(f"zpool import -N -R {mountpoint} zroot")
            SysCommand("zfs load-key zroot")
            SysCommand(f"zfs mount zroot/ROOT_{prefix}/default")
            SysCommand("zfs mount -a")

            debug("Setting pool cache file")
            SysCommand(f"zpool set cachefile={self.pool_cache_path} zroot")

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

    def export_pool(self) -> None:
        debug("Exporting ZFS pool")
        try:
            SysCommand("zfs umount -a")
            SysCommand("zpool export zroot")
            info("Pool exported successfully")
        except SysCallError as e:
            error(f"Failed to export pool: {str(e)}")
            raise
