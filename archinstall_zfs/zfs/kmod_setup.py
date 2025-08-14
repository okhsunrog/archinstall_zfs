import contextlib
import re
import tempfile
from pathlib import Path
from shutil import which
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


def check_zfs_utils() -> bool:
    """Return True if zfs/zpool utilities are available on the host."""
    return bool(which("zpool") and which("zfs"))


def initialize_zfs() -> None:
    """Ensure ZFS is available on the live system (host).

    - If the kernel module and utils are available, do nothing
    - Otherwise, add archzfs repo and install precompiled or DKMS fallback
    - Finally, load the module
    """
    if check_zfs_module() and check_zfs_utils():
        info("ZFS already available on host")
        return

    info("Preparing live system for ZFS (installing packages if needed)")
    add_archzfs_repo()

    zfs_init = ZFSInitializer()
    if not zfs_init.run():
        raise RuntimeError("Failed to initialize ZFS support on host")


def _rewrite_archzfs_repo_block(content: str) -> str:
    """Ensure [archzfs] block uses the official archzfs.com repo.

    Replaces any existing [archzfs] block with a canonical stanza to avoid
    pointing to unsupported or experimental sources that break dependency
    resolution.
    """
    lines = content.splitlines()
    start = None
    end = None
    for idx, line in enumerate(lines):
        if line.strip().lower() == "[archzfs]":
            start = idx
            # Find the end of the block (next section or EOF)
            for j in range(idx + 1, len(lines)):
                if lines[j].startswith("[") and lines[j].endswith("]"):
                    end = j
                    break
            if end is None:
                end = len(lines)
            break
    canonical = [
        "[archzfs]",
        "SigLevel = Never",
        "Server = https://github.com/archzfs/archzfs/releases/download/experimental",
        "",
    ]
    if start is None:
        # No existing block, append at the end
        if lines and lines[-1].strip():
            lines.append("")
        lines.extend(canonical)
        return "\n".join(lines) + "\n"

    # Replace existing block with canonical
    new_lines = lines[:start] + canonical + lines[end:]
    return "\n".join(new_lines) + "\n"


def add_archzfs_repo(target_path: Path = Path("/"), installation: Any = None) -> None:
    """Add archzfs repository to pacman.conf and ensure key is trusted.

    - If installation is provided, performs operations inside the target chroot.
    - If installation is None, performs operations on the live system (host), which is
      what pacstrap will use for fetching packages.
    """
    info("Adding archzfs repository")

    pacman_conf = target_path / "etc/pacman.conf"

    with open(pacman_conf) as f:
        content = f.read()

    # Always rewrite or append archzfs block to use GitHub experimental
    updated = _rewrite_archzfs_repo_block(content)
    if updated != content:
        info("Writing archzfs repo configuration (GitHub experimental)")
        pacman_conf.write_text(updated)
    else:
        info("archzfs repository already configured (GitHub experimental)")

    # Initialize keyring
    try:
        if installation:
            installation.arch_chroot("pacman-key --init")
            installation.arch_chroot("pacman-key --populate archlinux")
        else:
            SysCommand("pacman-key --init", peek_output=True)
            SysCommand("pacman-key --populate archlinux", peek_output=True)
    except SysCallError as e:
        error(f"Failed to initialize keyring: {e}")
        raise RuntimeError("Cannot proceed without working keyring") from e

    # Known archzfs signing keys (maintainers rotate keys periodically)
    # Keep this list updated when archzfs rotates keys to avoid interactive prompts
    archzfs_key_ids = [
        "3A9917BF0DED5C13F69AC68FABEC0A1208037BE9",
        "DDF7DB817396A49B2A2723F7403BD972F75D9D76",
    ]
    keyservers = [
        "hkps://keyserver.ubuntu.com",
        "hkps://pgp.mit.edu",
        "hkps://pool.sks-keyservers.net",
        "hkps://keys.openpgp.org",
    ]

    # Receive and locally sign all known archzfs keys
    for key_id in archzfs_key_ids:
        key_received = False
        for keyserver in keyservers:
            key_receive = f"pacman-key --keyserver {keyserver} -r {key_id}"
            try:
                if installation:
                    installation.arch_chroot(key_receive)
                else:
                    SysCommand(key_receive, peek_output=True)
                key_received = True
                info(f"Successfully received key {key_id[-8:]} from {keyserver}")
                break
            except SysCallError as e:
                warn(f"Failed to receive key {key_id[-8:]} from {keyserver}: {e}")
                continue

        if not key_received:
            raise RuntimeError(f"Cannot proceed without archzfs repository key {key_id}")

        key_sign = f"pacman-key --lsign-key {key_id}"
        try:
            if installation:
                installation.arch_chroot(key_sign)
            else:
                SysCommand(key_sign, peek_output=True)
            info(f"Successfully signed archzfs key {key_id[-8:]}")
        except SysCallError as e:
            raise RuntimeError(f"Cannot proceed without signed archzfs key {key_id}") from e

    # Repo block written above; now sync dbs non-interactively

    try:
        info("Syncing package databases...")
        if installation:
            # Use non-interactive pacman in the target
            installation.arch_chroot("pacman -Sy --noconfirm")
            info("Successfully synced package databases on target")
        else:
            SysCommand("pacman -Sy --noconfirm", peek_output=True)
            info("Successfully synced package databases on host")
    except SysCallError as e:
        error(f"Failed to sync databases: {e}")


class ZFSInitializer:
    def __init__(self, verbose: bool = False):
        self.verbose = verbose
        self.kernel_version = self._get_running_kernel_version()
        self._mirrorlist_path: Path = Path("/etc/pacman.d/mirrorlist")
        self._mirrorlist_backup: str | None = self._mirrorlist_path.read_text() if self._mirrorlist_path.exists() else None
        self._archive_set: bool = False
        # Reflector service state tracking
        self._reflector_changed: bool = False
        self._reflector_was_active: bool = False
        self._reflector_was_enabled: bool = False

    def _is_service_active(self, name: str) -> bool:
        try:
            SysCommand(f"systemctl is-active --quiet {name}")
            return True
        except SysCallError:
            return False

    def _is_service_enabled(self, name: str) -> bool:
        try:
            SysCommand(f"systemctl is-enabled --quiet {name}")
            return True
        except SysCallError:
            return False

    def _stop_reflector_for_archive(self) -> None:
        """Stop/disable reflector while we pin mirrorlist to the Archive.

        Saves original state so we can restore later.
        """
        try:
            self._reflector_was_active = self._is_service_active("reflector.service")
            self._reflector_was_enabled = self._is_service_enabled("reflector.service")
            # Disable and stop to avoid auto-restart loops during repo pinning/downgrade
            SysCommand("systemctl disable --now reflector.service", peek_output=True)
            SysCommand("systemctl reset-failed reflector.service", peek_output=True)
            self._reflector_changed = True
            info("Temporarily disabled reflector.service during Archive pinning")
        except Exception as e:
            warn(f"Failed to disable reflector.service: {e!s}")

    def _restore_reflector(self) -> None:
        if not self._reflector_changed:
            return
        try:
            # Restore previous enabled/active state
            if self._reflector_was_enabled:
                SysCommand("systemctl enable reflector.service", peek_output=True)
            else:
                # Ensure it's not enabled if it wasn't
                SysCommand("systemctl disable reflector.service", peek_output=True)
            if self._reflector_was_active:
                SysCommand("systemctl start reflector.service", peek_output=True)
            else:
                SysCommand("systemctl stop reflector.service", peek_output=True)
            info("Restored reflector.service to previous state")
        except Exception as e:
            warn(f"Failed to restore reflector.service: {e!s}")

    def _get_running_kernel_version(self) -> str:
        return cast(str, SysCommand("uname -r").decode().strip())

    def increase_cowspace(self) -> None:
        info("Increasing cowspace to half of RAM")
        SysCommand("mount -o remount,size=50% /run/archiso/cowspace")

    def _setup_archive_repository(self) -> None:
        """Pin pacman to Archlinux Archive matching the ISO date to align headers with running kernel."""
        info("Setting up Archlinux Archive repository for DKMS")

        try:
            version_file = Path("/version")
            if not version_file.exists():
                warn("/version not found; using current repos")
                SysCommand("pacman -Sy --noconfirm", peek_output=True)
                return

            archiso_version = version_file.read_text().strip()
            debug(f"Detected archiso version: {archiso_version}")

            # Skip archive setup for non-date tags
            if archiso_version in ["testing", "latest", "git", "devel"]:
                info(f"Detected {archiso_version} build, using current repos")
                SysCommand("pacman -Sy --noconfirm", peek_output=True)
                return

            # Convert dots to slashes (e.g., 2024.01.01 -> 2024/01/01)
            archive_date = archiso_version.replace(".", "/")
            if archive_date == "2022/02/01":
                archive_date = "2022/02/02"

            archive_url = f"https://archive.archlinux.org/repos/{archive_date}/"
            info(f"Using Archlinux Archive date: {archive_date}")

            # Stop reflector while we pin mirrorlist and downgrade to the archive snapshot
            self._stop_reflector_for_archive()

            # Force mirrorlist to archive and full downgrade/upgrade to that snapshot
            mirrorlist_content = f"Server={archive_url}$repo/os/$arch\n"
            # Backup is taken in __init__; write pinned mirrorlist
            self._mirrorlist_path.write_text(mirrorlist_content)
            self._archive_set = True
            SysCommand("pacman -Syyuu --noconfirm", peek_output=True)
            info("Successfully aligned to archive repository versions")
        except Exception as e:
            warn(f"Archive setup failed ({e!s}); continuing with current repos")
            SysCommand("pacman -Sy --noconfirm", peek_output=True)

    def _restore_archive_mirrorlist(self) -> None:
        """Restore original mirrorlist if we pinned to Archive, and resync."""
        if self._archive_set and self._mirrorlist_backup is not None:
            try:
                info("Restoring original pacman mirrorlist")
                self._mirrorlist_path.write_text(self._mirrorlist_backup)
                SysCommand("pacman -Syy --noconfirm", peek_output=True)
                info("Restored current repositories")
            except Exception as e:
                warn(f"Failed to restore mirrorlist: {e!s}")
        # Regardless of archive pin, restore reflector if we changed it
        self._restore_reflector()

    def _finalize_reflector_and_mirrors(self) -> None:
        """Best-effort guard to exit with sane mirrorlist and reflector state.

        This is called after normal restoration to cover edge cases where the
        process was interrupted mid-way or network flakiness left the service
        in a failed state. It avoids changing the user's preferences unless we
        explicitly altered them earlier in this run.
        """
        try:
            # If mirrorlist still points at the Archive unexpectedly and we have
            # a backup, restore it and resync databases.
            try:
                current_ml = self._mirrorlist_path.read_text()
            except Exception:
                current_ml = ""

            if "archive.archlinux.org/repos/" in current_ml and self._mirrorlist_backup:
                info("Detected Archive mirrorlist still active; restoring backup")
                try:
                    self._mirrorlist_path.write_text(self._mirrorlist_backup)
                    SysCommand("pacman -Syy --noconfirm", peek_output=True)
                except Exception as e:
                    warn(f"Failed to restore backup mirrorlist: {e!s}")

            # Clear failed state and attempt to return reflector to its previous state
            with contextlib.suppress(Exception):
                SysCommand("systemctl reset-failed reflector.service", peek_output=True)

            # If we changed reflector earlier, we already tried to restore it.
            # As an extra safety, if it was previously active, try to restart it
            # to avoid lingering 'failed' status in logs.
            if self._reflector_was_active:
                with contextlib.suppress(Exception):
                    SysCommand("systemctl try-restart reflector.service", peek_output=True)
        except Exception as e:
            warn(f"Finalization of mirrors/reflector encountered an issue: {e!s}")

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
            # Set up Archlinux Archive repository to match archiso version (best-effort)
            self._setup_archive_repository()

            # Ensure the kernel modules directory exists for the running kernel
            modules_dir = Path(f"/usr/lib/modules/{self.kernel_version}")
            if not modules_dir.exists():
                debug(f"Creating missing modules directory: {modules_dir}")
                modules_dir.mkdir(parents=True, exist_ok=True)

            # Install toolchain and headers matching the running kernel; reinstall to ensure presence
            SysCommand("pacman -S --noconfirm --needed base-devel linux-headers git", peek_output=True)
            SysCommand("pacman -S --noconfirm linux-headers", peek_output=True)

            # Temporarily disable mkinitcpio hooks (both etc and share locations) to avoid live ISO errors
            info("Temporarily disabling mkinitcpio hooks for live system")
            hooks_locations = [Path("/etc/pacman.d/hooks"), Path("/usr/share/libalpm/hooks")]
            disabled_hooks: list[tuple[Path, Path]] = []
            for loc in hooks_locations:
                if not loc.exists():
                    continue
                for hook_file in loc.glob("*mkinitcpio*"):
                    disabled_file = hook_file.with_suffix(hook_file.suffix + ".disabled")
                    try:
                        hook_file.rename(disabled_file)
                        disabled_hooks.append((hook_file, disabled_file))
                        debug(f"Disabled hook: {hook_file}")
                    except Exception as e:
                        warn(f"Failed to disable hook {hook_file}: {e}")

            try:
                # Install DKMS ZFS which will trigger DKMS build for the running kernel
                SysCommand("pacman -S zfs-dkms --noconfirm", peek_output=True)

                return True
            finally:
                # Re-enable hooks regardless of success
                for original, disabled in disabled_hooks:
                    with contextlib.suppress(Exception):
                        disabled.rename(original)
                        debug(f"Re-enabled hook: {original}")

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
        # No archiso detection; just prepare and ensure ZFS is present
        with contextlib.suppress(Exception):
            self.increase_cowspace()

        try:
            if not self.install_zfs():
                return False
            return self.load_zfs_module()
        finally:
            # Always attempt to restore mirrorlist so pacstrap uses stock repos
            self._restore_archive_mirrorlist()
            # And best-effort ensure reflector/mirrorlist are in a sane state
            with contextlib.suppress(Exception):
                self._finalize_reflector_and_mirrors()

    def search_zfs_package(self, package_name: str, version: str) -> tuple[str, str] | None:
        urls = ["https://github.com/archzfs/archzfs/releases/download/experimental/"]

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
