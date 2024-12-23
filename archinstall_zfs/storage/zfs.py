from pathlib import Path

from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.tui.curses_menu import EditMenu
import os

class ZFSManager:
    def __init__(self):
        self.zfs_key_path = Path('/etc/zfs/zroot.key')
        self.pool_cache_path = Path('/etc/zfs/zpool.cache')
        self.hostid_path = Path('/etc/hostid')

    def get_encryption_password(self) -> str:
        while True:
            try:
                password_menu = EditMenu(
                    "ZFS Encryption Password",
                    header="Enter password for ZFS encryption",
                    hide_input=True
                )
                verify_menu = EditMenu(
                    "Verify Password",
                    header="Enter password again",
                    hide_input=True
                )
                
                password = password_menu.input().text()
                verify = verify_menu.input().text()
                
                if password == verify and password:  # Also check password is not empty
                    return password
                print("Passwords do not match or empty! Try again.")
            except KeyboardInterrupt:
                raise SystemExit("Installation cancelled by user")

    def setup_encryption(self, password: str):
        # Create zfs config directory
        self.zfs_key_path.parent.mkdir(parents=True, exist_ok=True)
        
        # Save encryption key
        self.zfs_key_path.write_text(password)
        self.zfs_key_path.chmod(0o000)

    def create_pool(self, partition: str, prefix: str, encryption_password: str):
        # Change to root directory to avoid working directory issues
        os.chdir('/')
        print("Starting ZFS pool creation")

        self.setup_encryption(encryption_password)
        try:
            SysCommand('zgenhostid')
        except SysCallError as e:
            if "File exists" not in str(e):
                raise  # Re-raise if it's not the "already exists" error
        
        # Create ZFS pool with encryption
        pool_cmd = (
            'zpool create -f -o ashift=12 '
            '-O acltype=posixacl -O relatime=on -O xattr=sa '
            '-o autotrim=on -O dnodesize=auto -O normalization=formD '
            '-O devices=off -O compression=lz4 '
            '-O encryption=aes-256-gcm -O keyformat=passphrase '
            f'-O keylocation=file://{self.zfs_key_path} '
            '-m none '
            'zroot /dev/disk/by-partlabel/rootpart'
        )
        SysCommand(pool_cmd)
        
        # Create base datasets
        datasets = [
            (f'zroot/data_{prefix}', {'mountpoint': 'none'}),
            (f'zroot/ROOT_{prefix}', {'mountpoint': 'none'}),
            (f'zroot/ROOT_{prefix}/default', {'mountpoint': '/', 'canmount': 'noauto'}),
            (f'zroot/data_{prefix}/home', {'mountpoint': '/home'}),
            (f'zroot/data_{prefix}/root', {'mountpoint': '/root'}),
            (f'zroot/var_{prefix}', {'mountpoint': '/var', 'canmount': 'off'}),
            (f'zroot/var_{prefix}/lib', {'mountpoint': '/var/lib', 'canmount': 'off'}),
            (f'zroot/var_{prefix}/lib/libvirt', None),
            (f'zroot/var_{prefix}/lib/docker', None),
            (f'zroot/vm_{prefix}', {'mountpoint': '/vm'})
        ]
        
        for dataset, props in datasets:
            if props:
                props_str = ' '.join(f'-o {k}={v}' for k, v in props.items())
                SysCommand(f'zfs create {props_str} {dataset}')
            else:
                SysCommand(f'zfs create {dataset}')

        # Set bootfs and export pool
        SysCommand(f'zpool set bootfs=zroot/ROOT_{prefix}/default zroot')
        SysCommand('zpool export zroot')

    def import_pool(self, prefix: str, mountpoint: Path = Path('/mnt')):
        SysCommand(f'zpool import -N -R {mountpoint} zroot')
        SysCommand('zfs load-key zroot')
        SysCommand(f'zfs mount zroot/ROOT_{prefix}/default')
        SysCommand('zfs mount -a')
        
        # Set and copy cache file
        SysCommand(f'zpool set cachefile={self.pool_cache_path} zroot')
        
        target_zfs = mountpoint / 'etc/zfs'
        target_zfs.mkdir(parents=True, exist_ok=True)
        
        SysCommand(f'cp {self.pool_cache_path} {target_zfs}/')
        SysCommand(f'cp {self.hostid_path} {mountpoint}/etc/')
        SysCommand(f'cp {self.zfs_key_path} {target_zfs}/')

    def export_pool(self):
        SysCommand('zfs umount -a')
        SysCommand('zpool export zroot')

