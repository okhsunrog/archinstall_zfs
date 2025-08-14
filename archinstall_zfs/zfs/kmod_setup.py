import contextlib
import re
import tempfile
import time
from pathlib import Path
from shutil import which
from typing import Any, cast

from archinstall import debug, error, info, warn
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand


def _service_substate(name: str) -> str:
    """Return systemd SubState for a unit, mirroring archinstall's check.

    Accepts plain unit base name (e.g. "reflector") and appends .service by default.
    """
    if not name.endswith((".service", ".target", ".timer")):
        name += ".service"
    return cast(
        str,
        SysCommand(
            f"systemctl show --no-pager -p SubState --value {name}",
            environment_vars={"SYSTEMD_COLORS": "0"},
        ).decode(),
    )


def wait_for_reflector_to_finish() -> None:
    """Block until reflector has reached a finished state.

    Finished states follow archinstall's logic: 'dead', 'failed', or 'exited'.
    """
    info("Waiting for automatic mirror selection (reflector) to complete.")
    while _service_substate("reflector") not in ("dead", "failed", "exited"):
        time.sleep(1)


def stop_reflector_units() -> None:
    """Stop reflector service and timer to keep it inactive during installation."""
    for unit in ("reflector.service", "reflector.timer"):
        with contextlib.suppress(SysCallError):
            SysCommand(f"systemctl stop {unit}", peek_output=True)
    info("Ensured reflector.service and reflector.timer are stopped")


def ensure_reflector_finished_and_stopped() -> None:
    """Wait for reflector to finish, then stop it and its timer.

    This keeps reflector inactive and avoids it racing with repository/mirror changes.
    """
    with contextlib.suppress(Exception):
        wait_for_reflector_to_finish()
    with contextlib.suppress(Exception):
        stop_reflector_units()


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
    # Make reflector behavior simple and robust: wait for completion, then stop it
    ensure_reflector_finished_and_stopped()

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

            # Ensure reflector is finished and stopped before pinning mirrorlist
            ensure_reflector_finished_and_stopped()

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
        # Intentionally keep reflector stopped; no restart attempts here

    def extract_pkginfo(self, package_path: Path) -> str:
        pkginfo = SysCommand(f"bsdtar -qxO -f {package_path} .PKGINFO").decode()
        match = re.search(r"depend = zfs-utils=(.*)", pkginfo)
        if match:
            return match.group(1)
        raise ValueError("Could not extract zfs-utils version from package info")

    def install_zfs(self) -> bool:
        """Install ZFS using the new kernel-aware system."""
        # Detect running kernel variant
        kernel_name = self._detect_kernel_variant()
        from archinstall_zfs.kernel import EnhancedZFSInstaller, get_kernel_registry
        from archinstall_zfs.menu.models import ZFSModuleMode

        registry = get_kernel_registry()

        # Try precompiled first, fallback to DKMS with same kernel
        installer = EnhancedZFSInstaller(registry)
        result = installer.install_with_fallback(
            kernel_name,
            ZFSModuleMode.PRECOMPILED,  # Always try precompiled first
            None,  # Host installation, not target
        )

        if result.success:
            info(result.get_summary())
            return True
        error(f"ZFS installation failed: {result.get_summary()}")
        return False

    def _detect_kernel_variant(self) -> str:
        """Detect kernel variant from running kernel version."""
        kernel_version = self.kernel_version.lower()

        if "lts" in kernel_version:
            return "linux-lts"
        if "zen" in kernel_version:
            return "linux-zen"
        if "hardened" in kernel_version:
            return "linux-hardened"
        if "rt" in kernel_version:
            if "lts" in kernel_version:
                return "linux-rt-lts"
            return "linux-rt"
        return "linux"

    def _install_zfs_legacy(self) -> bool:
        """Legacy ZFS installation method (kept for rollback capability)."""
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
            # Keep reflector stopped; do not try to restart it

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
