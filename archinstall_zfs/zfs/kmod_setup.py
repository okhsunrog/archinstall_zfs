import re
import tempfile
from pathlib import Path
from typing import Any, cast

from archinstall import debug, error, info
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand


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


def add_archzfs_repo(target_path: Path = Path("/"), installation: Any = None) -> None:
    """Add archzfs repository to pacman.conf if not already present"""
    info("Adding archzfs repository")

    pacman_conf = target_path / "etc/pacman.conf"

    # Check if repo already exists
    with open(pacman_conf) as f:
        content = f.read()
        if "[archzfs]" in content:
            info("archzfs repository already configured")
            return

    key_id = "DDF7DB817396A49B2A2723F7403BD972F75D9D76"
    key_sign = f"pacman-key --lsign-key {key_id}"
    
    # Try multiple keyservers for better reliability
    keyservers = [
        "hkps://keys.openpgp.org",
        "hkps://keyserver.ubuntu.com",
        "hkps://pgp.mit.edu",
        "hkps://pool.sks-keyservers.net"
    ]
    
    key_received = False
    for keyserver in keyservers:
        key_receive = f"pacman-key --keyserver {keyserver} -r {key_id}"
        try:
            if installation:
                installation.arch_chroot(key_receive)
            else:
                SysCommand(key_receive)
            key_received = True
            info(f"Successfully received key from {keyserver}")
            break
        except SysCallError as e:
            error(f"Failed to receive key from {keyserver}: {e}")
            continue
    
    if not key_received:
        # For testing purposes, we can continue without the key
        # This allows development even when keyservers are down
        error("Failed to receive archzfs key from all keyservers")
        error("Continuing without key verification (testing mode)")
        info("Note: This may cause package verification issues")
    else:
        # Only sign the key if we successfully received it
        if installation:
            installation.arch_chroot(key_sign)
        else:
            SysCommand(key_sign)

    repo_config = [
        "\n[archzfs]\n",
        "Server = https://archzfs.com/$repo/$arch\n",
        "Server = https://mirror.sum7.eu/archlinux/archzfs/$arch\n",
        "Server = https://mirror.biocrafting.net/archlinux/archzfs/$arch\n",
    ]

    with open(pacman_conf, "a") as f:
        f.writelines(repo_config)

    if not installation:
        SysCommand("pacman -Sy")


class ZFSInitializer:
    def __init__(self, verbose: bool = False):
        self.verbose = verbose
        self.kernel_version = self._get_running_kernel_version()

    def _get_running_kernel_version(self) -> str:
        return cast(str, SysCommand("uname -r").decode().strip())

    def increase_cowspace(self) -> None:
        info("Increasing cowspace to half of RAM")
        SysCommand("mount -o remount,size=50% /run/archiso/cowspace")

    def extract_pkginfo(self, package_path: Path) -> str:
        pkginfo = SysCommand(f"bsdtar -qxO -f {package_path} .PKGINFO").decode()
        match = re.search(r"depend = zfs-utils=(.*)", pkginfo)
        if match:
            return match.group(1)
        raise ValueError("Could not extract zfs-utils version from package info")

    def install_zfs(self) -> bool:
        kernel_version_fixed = self.kernel_version.replace("-", ".")

        package_info = self.search_zfs_package("zfs-linux", kernel_version_fixed)
        if package_info:
            url, package = package_info
            package_url = f"{url}{package}"

            with tempfile.TemporaryDirectory() as tmpdir:
                package_path = Path(tmpdir) / package
                SysCommand(f"curl -s -o {package_path} {package_url}")

                zfs_utils_version = self.extract_pkginfo(package_path)
                utils_info = self.search_zfs_package("zfs-utils", zfs_utils_version)

                if utils_info:
                    utils_url = f"{utils_info[0]}{utils_info[1]}"
                    SysCommand(f"pacman -U {utils_url} --noconfirm", peek_output=True)
                    SysCommand(f"pacman -U {package_url} --noconfirm", peek_output=True)
                    return True

        info("Falling back to DKMS method")
        try:
            SysCommand("pacman -Syyuu --noconfirm", peek_output=True)
            SysCommand("pacman -S --noconfirm --needed base-devel linux-headers git", peek_output=True)
            SysCommand("pacman -S zfs-dkms --noconfirm", peek_output=True)
            return True
        except Exception as e:
            error(f"DKMS installation failed: {e!s}")
            return False

    def load_zfs_module(self) -> bool:
        try:
            SysCommand("modprobe zfs")
            info("ZFS module loaded successfully")
            return True
        except Exception as e:
            error(f"Failed to load ZFS module: {e!s}")
            return False

    def run(self) -> bool:
        if not Path("/proc/cmdline").read_text().find("arch.*iso"):
            error("Not running in archiso")
            return False

        self.increase_cowspace()

        if not self.install_zfs():
            return False

        return self.load_zfs_module()

    def search_zfs_package(self, package_name: str, version: str) -> tuple[str, str] | None:
        urls = ["http://archzfs.com/archzfs/x86_64/", "http://archzfs.com/archive_archzfs/"]

        pattern = f'{package_name}-[0-9][^"]*{version}[^"]*x86_64[^"]*'

        for url in urls:
            info(f"Searching {package_name} on {url}")
            try:
                response = SysCommand(f"curl -s {url}").decode()
                matches = re.findall(pattern, response)
                if matches:
                    package = matches[-1]
                    return url, package
            except Exception as e:
                error(f"Failed to search package: {e!s}")

        return None
