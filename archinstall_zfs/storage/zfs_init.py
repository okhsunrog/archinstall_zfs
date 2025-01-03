import tempfile
from pathlib import Path
from typing import Optional, Tuple
import re
from archinstall import SysCommand, debug, info, error
from archinstall.lib.exceptions import SysCallError


def check_zfs_module() -> bool:
    debug("Checking ZFS kernel module")
    try:
        SysCommand("modprobe zfs")
        info("ZFS module loaded successfully")
        return True
    except SysCallError:
        return False


def initialize_zfs() -> None:
    add_archzfs_repo()
    if not check_zfs_module():
        info("ZFS module not loaded, initializing")
        zfs_init = ZFSInitializer()
        if not zfs_init.run():
            raise RuntimeError("Failed to initialize ZFS support")


def add_archzfs_repo(target_path: str = "/") -> None:
    """Add archzfs repository to pacman.conf"""
    info("Adding archzfs repository")

    SysCommand('pacman-key -r DDF7DB817396A49B2A2723F7403BD972F75D9D76')
    SysCommand('pacman-key --lsign-key DDF7DB817396A49B2A2723F7403BD972F75D9D76')

    pacman_conf = f"{str(target_path).rstrip('/')}/etc/pacman.conf"
    with open(pacman_conf, "a") as f:
        f.write('\n[archzfs]\n')
        f.write('Server = http://archzfs.com/$repo/$arch\n')
        f.write('Server = http://mirror.sum7.eu/archlinux/$repo/$repo/$arch\n')
        f.write('Server = https://mirror.biocrafting.net/archlinux/$repo/$repo/$arch\n')

    SysCommand('pacman -Sy')

class ZFSInitializer:
    def __init__(self, verbose: bool = False):
        self.verbose = verbose
        self.kernel_version = self._get_running_kernel_version()

    def _get_running_kernel_version(self) -> str:
        return SysCommand('uname -r').decode().strip()

    def increase_cowspace(self) -> None:
        info("Increasing cowspace to half of RAM")
        SysCommand('mount -o remount,size=50% /run/archiso/cowspace')

    def extract_pkginfo(self, package_path: Path) -> str:
        pkginfo = SysCommand(f'bsdtar -qxO -f {package_path} .PKGINFO').decode()
        return re.search(r'depend = zfs-utils=(.*)', pkginfo).group(1)

    def install_zfs(self) -> bool:
        kernel_version_fixed = self.kernel_version.replace('-', '.')

        package_info = self.search_zfs_package("zfs-linux", kernel_version_fixed)
        if package_info:
            url, package = package_info
            package_url = f"{url}{package}"

            with tempfile.TemporaryDirectory() as tmpdir:
                package_path = Path(tmpdir) / package
                SysCommand(f'curl -s -o {package_path} {package_url}')

                zfs_utils_version = self.extract_pkginfo(package_path)
                utils_info = self.search_zfs_package("zfs-utils", zfs_utils_version)

                if utils_info:
                    utils_url = f"{utils_info[0]}{utils_info[1]}"
                    SysCommand(f'pacman -U {utils_url} --noconfirm', peek_output=True)
                    SysCommand(f'pacman -U {package_url} --noconfirm', peek_output=True)
                    return True

        info("Falling back to DKMS method")
        try:
            SysCommand('pacman -Syyuu --noconfirm', peek_output=True)
            SysCommand('pacman -S --noconfirm --needed base-devel linux-headers git', peek_output=True)
            SysCommand('pacman -S zfs-dkms --noconfirm', peek_output=True)
            return True
        except Exception as e:
            error(f"DKMS installation failed: {str(e)}")
            return False

    def load_zfs_module(self) -> bool:
        try:
            SysCommand('modprobe zfs')
            info("ZFS module loaded successfully")
            return True
        except Exception as e:
            error(f"Failed to load ZFS module: {str(e)}")
            return False

    def run(self) -> bool:
        if not Path('/proc/cmdline').read_text().find('arch.*iso'):
            error("Not running in archiso")
            return False

        self.increase_cowspace()

        if not self.install_zfs():
            return False

        return self.load_zfs_module()

    def search_zfs_package(self, package_name: str, version: str) -> Optional[Tuple[str, str]]:
        urls = [
            "http://archzfs.com/archzfs/x86_64/",
            "http://archzfs.com/archive_archzfs/"
        ]

        pattern = f'{package_name}-[0-9][^"]*{version}[^"]*x86_64[^"]*'

        for url in urls:
            info(f"Searching {package_name} on {url}")
            try:
                response = SysCommand(f'curl -s {url}').decode()
                matches = re.findall(pattern, response)
                if matches:
                    package = matches[-1]
                    return url, package
            except Exception as e:
                error(f"Failed to search package: {str(e)}")

        return None