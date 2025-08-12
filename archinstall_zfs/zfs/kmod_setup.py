import re
import tempfile
from pathlib import Path
from typing import Any, cast

from archinstall import debug, error, info, warn
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
    # First check if ZFS modules are already available (built into ISO)
    if check_zfs_module():
        info("ZFS modules already available, no initialization needed")
        return

    # ZFS modules not available, need to install them
    info("ZFS modules not found, attempting to install")
    add_archzfs_repo()

    if not check_zfs_module():
        info("ZFS module not loaded after repo setup, initializing")
        zfs_init = ZFSInitializer()
        if not zfs_init.run():
            raise RuntimeError("Failed to initialize ZFS support")


def add_archzfs_repo(target_path: Path = Path("/"), installation: Any = None) -> None:
    """Add archzfs repository to pacman.conf if not already present"""
    info("Adding archzfs repository")

    pacman_conf = target_path / "etc/pacman.conf"

    with open(pacman_conf) as f:
        content = f.read()
        if "[archzfs]" in content:
            info("archzfs repository already configured")
            return

    # Initialize keyring if needed - handled by systemd pacman-init on ISO
    try:
        if installation:
            installation.arch_chroot("pacman-key --init")
            installation.arch_chroot("pacman-key --populate archlinux")
        else:
            # Live ISO path: skip keyring work and proceed to write repo only
            pass
    except SysCallError as e:
        error(f"Failed to initialize keyring: {e}")
        if installation:
            raise RuntimeError("Cannot proceed without working keyring") from e
        # Live ISO: skip keyring errors

    key_id = "DDF7DB817396A49B2A2723F7403BD972F75D9D76"
    key_sign = f"pacman-key --lsign-key {key_id}"
    keyservers = [
        "hkps://keyserver.ubuntu.com",
        "hkps://pgp.mit.edu",
        "hkps://pool.sks-keyservers.net",
        "hkps://keys.openpgp.org",
    ]

    key_received = False
    if installation:
        for keyserver in keyservers:
            key_receive = f"pacman-key --keyserver {keyserver} -r {key_id}"
            try:
                installation.arch_chroot(key_receive)
                key_received = True
                info(f"Successfully received key from {keyserver}")
                break
            except SysCallError as e:
                error(f"Failed to receive key from {keyserver}: {e}")
                continue

        if not key_received:
            raise RuntimeError("Cannot proceed without archzfs repository key")

        try:
            installation.arch_chroot(key_sign)
            info("Successfully signed archzfs key")
        except SysCallError as e:
            raise RuntimeError("Cannot proceed without signed archzfs key") from e

    repo_config = [
        "\n[archzfs]\n",
        "Server = https://archzfs.com/$repo/$arch\n",
        "Server = https://mirror.sum7.eu/archlinux/archzfs/$arch\n",
        "Server = https://mirror.biocrafting.net/archlinux/archzfs/$arch\n",
    ]

    with open(pacman_conf, "a") as f:
        f.writelines(repo_config)

    if installation:
        try:
            installation.arch_chroot("pacman -Sy")
            info("Successfully synced package databases on target")
        except SysCallError as e:
            error(f"Failed to sync databases on target: {e}")


class ZFSInitializer:
    def __init__(self, verbose: bool = False):
        self.verbose = verbose
        self.kernel_version = self._get_running_kernel_version()

    def _get_running_kernel_version(self) -> str:
        return cast(str, SysCommand("uname -r").decode().strip())

    def increase_cowspace(self) -> None:
        info("Increasing cowspace to half of RAM")
        SysCommand("mount -o remount,size=50% /run/archiso/cowspace")

    def _setup_archive_repository(self) -> None:
        """Setup Archlinux Archive repository matching archiso version for DKMS consistency."""
        info("Setting up Archlinux Archive repository for DKMS")

        try:
            # Get archiso version from /version file
            version_file = Path("/version")
            if version_file.exists():
                archiso_version = version_file.read_text().strip()
                debug(f"Detected archiso version: {archiso_version}")

                # Skip archive setup for testing/development builds
                if archiso_version in ["testing", "latest", "git", "devel"]:
                    info(f"Detected {archiso_version} build, skipping archive setup")
                    SysCommand("pacman -Sy --noconfirm", peek_output=True)
                    return

                # Convert dots to slashes (e.g., "2024.01.01" -> "2024/01/01")
                archive_date = archiso_version.replace(".", "/")

                # Workaround for specific date (from bash script)
                if archive_date == "2022/02/01":
                    archive_date = "2022/02/02"

                archive_url = f"https://archive.archlinux.org/repos/{archive_date}/"
                debug(f"Testing archive URL: {archive_url}")

                # Test if archive exists
                test_result = SysCommand(f"curl -s --head {archive_url}")
                if "200 OK" in test_result.decode():
                    info(f"Using Archlinux Archive for date: {archive_date}")

                    # Update mirrorlist to use archive
                    mirrorlist_content = f"Server={archive_url}$repo/os/$arch\n"
                    Path("/etc/pacman.d/mirrorlist").write_text(mirrorlist_content)

                    # Now safely upgrade to archive versions
                    SysCommand("pacman -Syyuu --noconfirm", peek_output=True)
                    info("Successfully upgraded to archive repository versions")
                else:
                    warn(f"Archive repository for {archive_date} not accessible, using current repos")
                    SysCommand("pacman -Sy --noconfirm", peek_output=True)
            else:
                warn("Could not find /version file, using current repos")
                SysCommand("pacman -Sy --noconfirm", peek_output=True)

        except Exception as e:
            warn(f"Failed to setup archive repository: {e}, using current repos")
            SysCommand("pacman -Sy --noconfirm", peek_output=True)

    def extract_pkginfo(self, package_path: Path) -> str:
        pkginfo = SysCommand(f"bsdtar -qxO -f {package_path} .PKGINFO").decode()
        match = re.search(r"depend = zfs-utils=(.*)", pkginfo)
        if match:
            return match.group(1)
        raise ValueError("Could not extract zfs-utils version from package info")

    def install_zfs(self) -> bool:
        kernel_version_fixed = self.kernel_version.replace("-", ".")

        # Detect kernel type and search for appropriate ZFS package
        if "lts" in self.kernel_version:
            primary_package = "zfs-linux-lts"
            fallback_package = "zfs-linux"
            info("Detected LTS kernel, searching for zfs-linux-lts...")
        else:
            primary_package = "zfs-linux"
            fallback_package = "zfs-linux-lts"
            info("Detected regular kernel, searching for zfs-linux...")

        # Try primary package first
        package_info = self.search_zfs_package(primary_package, kernel_version_fixed)

        # If primary not found, try fallback
        if not package_info:
            info(f"{primary_package} not found, trying {fallback_package}...")
            package_info = self.search_zfs_package(fallback_package, kernel_version_fixed)

        if package_info:
            url, package = package_info
            package_url = f"{url}{package}"
            package_type = "zfs-linux-lts" if "lts" in package else "zfs-linux"
            info(f"Found {package_type} package: {package}")

            with tempfile.TemporaryDirectory() as tmpdir:
                package_path = Path(tmpdir) / package
                SysCommand(f"curl -s -o {package_path} {package_url}")

                zfs_utils_version = self.extract_pkginfo(package_path)
                utils_info = self.search_zfs_package("zfs-utils", zfs_utils_version)

                if utils_info:
                    utils_url = f"{utils_info[0]}{utils_info[1]}"
                    info(f"Installing zfs-utils and {package_type}")
                    SysCommand(f"pacman -U {utils_url} --noconfirm", peek_output=True)
                    SysCommand(f"pacman -U {package_url} --noconfirm", peek_output=True)
                    return True

        info("Falling back to DKMS method")
        try:
            # Set up Archlinux Archive repository to match archiso version
            self._setup_archive_repository()
            SysCommand("pacman -S --noconfirm --needed base-devel linux-headers git", peek_output=True)

            # Disable mkinitcpio hooks during DKMS installation on live system
            # This prevents initramfs rebuild which fails on archiso
            info("Temporarily disabling mkinitcpio hooks for live system")
            hooks_dir = Path("/etc/pacman.d/hooks")
            disabled_hooks = []

            if hooks_dir.exists():
                for hook_file in hooks_dir.glob("*mkinitcpio*"):
                    disabled_file = hook_file.with_suffix(hook_file.suffix + ".disabled")
                    hook_file.rename(disabled_file)
                    disabled_hooks.append((hook_file, disabled_file))
                    debug(f"Disabled hook: {hook_file}")

            try:
                SysCommand("pacman -S zfs-dkms --noconfirm", peek_output=True)

                # Re-enable hooks
                for original, disabled in disabled_hooks:
                    disabled.rename(original)
                    debug(f"Re-enabled hook: {original}")

                return True
            except Exception as e:
                # Re-enable hooks even if installation failed
                for original, disabled in disabled_hooks:
                    if disabled.exists():
                        disabled.rename(original)
                raise e

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
