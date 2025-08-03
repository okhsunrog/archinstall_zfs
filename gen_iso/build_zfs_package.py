#!/usr/bin/env python3
"""
ZFS Package Builder for archinstall_zfs ISOs

This script ensures ISOs ALWAYS use precompiled ZFS packages (no DKMS) by:
1. Checking if current archzfs repo has compatible zfs-linux-lts
2. If not, finding historical AUR package versions that match available linux-lts
3. If needed, using archive.archlinux.org to get older linux-lts versions
4. Building perfectly matched ZFS packages for slim, fast ISOs
5. Never falling back to DKMS - always precompiled packages
"""

import json
import re
import subprocess
import sys
import urllib.parse
import urllib.request
from pathlib import Path


class ZFSPackageBuilder:
    def __init__(self) -> None:
        self.project_root = Path(__file__).parent.parent
        self.local_repo_dir = self.project_root / "local_repo"
        self.gen_iso_dir = Path(__file__).parent

    def run_command(self, cmd: list[str], check: bool = True, capture: bool = True) -> subprocess.CompletedProcess:
        """Run a command and return the result."""
        print(f"Running: {' '.join(cmd)}")
        if capture:
            # Ruff S603: Commands are constructed from static strings in controlled contexts
            return subprocess.run(cmd, check=check, capture_output=True, text=True)  # noqa: S603
        # Ruff S603: Commands are constructed from static strings in controlled contexts
        return subprocess.run(cmd, check=check)  # noqa: S603

    def get_archzfs_version(self) -> str | None:
        """Get the current zfs-linux-lts version from archzfs repository."""
        try:
            with urllib.request.urlopen("https://archzfs.com/archzfs/x86_64/") as response:
                content: str = response.read().decode("utf-8")

            # Look for zfs-linux-lts package
            matches = re.findall(r'zfs-linux-lts-([^"]+)\.pkg\.tar\.zst', content)
            if isinstance(matches, list) and matches:
                first = matches[0]
                if isinstance(first, str):
                    return first  # Return first match
            return None
        except Exception as e:
            print(f"Error fetching archzfs version: {e}")
            return None

    def get_current_kernel_version(self) -> str:
        """Get the current linux-lts kernel version."""
        result = self.run_command(["pacman", "-Si", "linux-lts"])
        stdout: str = getattr(result, "stdout", "")
        for line in stdout.split("\n"):
            if line.startswith("Version"):
                parts = line.split(":")
                if len(parts) > 1:
                    return parts[1].strip()
        raise RuntimeError("Could not determine linux-lts version")

    def get_aur_version(self) -> str | None:
        """Get the latest zfs-linux-lts version from AUR."""
        try:
            url = "https://aur.archlinux.org/rpc/?v=5&type=info&arg=zfs-linux-lts"
            with urllib.request.urlopen(url) as response:  # noqa: S310
                data = json.loads(response.read().decode("utf-8"))
                if isinstance(data, dict) and data.get("resultcount", 0) > 0:
                    results = data.get("results")
                    if isinstance(results, list) and results:
                        ver = results[0].get("Version")
                        if isinstance(ver, str):
                            return ver
            return None
        except Exception as e:
            print(f"Error fetching AUR version: {e}")
            return None

    def get_aur_package_history(self) -> list[dict]:
        """Get historical versions of zfs-linux-lts AUR package by examining git commits."""
        try:
            aur_dir = self.local_repo_dir / "zfs-linux-lts-git"
            if not aur_dir.exists():
                print("Cloning AUR git history...")
                self.run_command(["git", "clone", "https://aur.archlinux.org/zfs-linux-lts.git", str(aur_dir)])

            # Get git log with PKGBUILD changes
            result = subprocess.run(["git", "log", "--oneline", "--follow", "PKGBUILD"], cwd=aur_dir, capture_output=True, text=True, check=True)  # noqa: S607

            versions = []
            for line in result.stdout.split("\n"):
                if not line.strip():
                    continue

                commit_hash = line.split()[0]
                # Get PKGBUILD content for this commit
                try:
                    pkgbuild_result = subprocess.run(["git", "show", f"{commit_hash}:PKGBUILD"], cwd=aur_dir, capture_output=True, text=True, check=True)  # noqa: S603, S607
                    pkgbuild_content = pkgbuild_result.stdout

                    # Extract version info from PKGBUILD
                    zfs_ver_match = re.search(r'_zfsver="([^"]+)"', pkgbuild_content)
                    kernel_ver_match = re.search(r'_kernelver="([^"]+)"', pkgbuild_content)

                    if zfs_ver_match and kernel_ver_match:
                        zfs_ver = zfs_ver_match.group(1)
                        kernel_ver = kernel_ver_match.group(1)

                        # Construct package version
                        pkg_version = f"{zfs_ver}_{kernel_ver.replace('-', '.')}-1"

                        versions.append({"commit": commit_hash, "zfs_version": zfs_ver, "kernel_version": kernel_ver, "package_version": pkg_version})

                except subprocess.CalledProcessError:
                    continue  # Skip commits where we can't read PKGBUILD

            # Remove duplicates and sort by commit order (newest first)
            seen_versions = set()
            unique_versions = []
            for v in versions:
                if v["package_version"] not in seen_versions:
                    seen_versions.add(v["package_version"])
                    unique_versions.append(v)

            return unique_versions[:20]  # Return recent 20 versions

        except Exception as e:
            print(f"Error getting AUR package history: {e}")
            return []

    def get_archive_linux_versions(self) -> list[dict[str, str]]:
        """Get available linux-lts versions from archive.archlinux.org."""
        try:
            # Get list of dates from archive
            url = "https://archive.archlinux.org/repos/"
            with urllib.request.urlopen(url) as response:  # noqa: S310
                content = response.read().decode("utf-8")

            # Extract date directories (format: YYYY/MM/DD/)
            date_matches = re.findall(r"(\d{4}/\d{2}/\d{2})/", content)
            recent_dates = sorted(date_matches)[-60:]  # Last 60 days of archives

            linux_versions: list[dict[str, str]] = []
            for date in recent_dates:
                try:
                    # Check what linux-lts version was available on this date
                    archive_url = f"https://archive.archlinux.org/repos/{date}/core/os/x86_64/"
                    # Audit: only https scheme is allowed here
                    assert archive_url.startswith("https://"), "unsupported scheme"
                    with urllib.request.urlopen(archive_url) as response:  # noqa: S310
                        archive_content = response.read().decode("utf-8")

                    # Look for linux-lts packages
                    linux_matches = re.findall(r"linux-lts-(\d+\.\d+\.\d+-\d+)-x86_64\.pkg\.tar\.", archive_content)
                    for version in linux_matches:
                        if version not in [v["version"] for v in linux_versions]:
                            linux_versions.append(
                                {
                                    "version": version,
                                    "date": date,
                                    "archive_url": f"https://archive.archlinux.org/repos/{date}/",
                                }
                            )

                except Exception as e:
                    # Skip dates where archive is not accessible
                    print(f"Archive access issue for {date}: {e}")
                    continue

            return linux_versions

        except Exception as e:
            print(f"Error getting archive linux versions: {e}")
            return []

    def extract_kernel_version(self, zfs_version: str) -> str | None:
        """Extract kernel version from zfs package version."""
        # zfs-linux-lts versions look like: 2.3.3_6.12.41.1-1
        match = re.search(r"_(\d+\.\d+\.\d+(?:\.\d+)?)", zfs_version)
        if match:
            return match.group(1)
        return None

    def kernel_versions_exact_match(self, zfs_kernel_version: str, linux_kernel_version: str) -> bool:
        """Check if ZFS package kernel version exactly matches linux kernel version."""
        # Remove arch suffix from linux version
        linux_base = linux_kernel_version.split("-")[0] + "-" + linux_kernel_version.split("-")[1]
        return zfs_kernel_version == linux_base

    def find_compatible_combination(self) -> dict | None:
        """Find the best compatible combination of linux-lts and zfs-linux-lts versions."""
        current_kernel = self.get_current_kernel_version()
        print(f"Current linux-lts: {current_kernel}")

        # Strategy 1: Check if current archzfs has exact match
        archzfs_version = self.get_archzfs_version()
        if archzfs_version:
            zfs_kernel = self.extract_kernel_version(archzfs_version)
            if zfs_kernel and self.kernel_versions_exact_match(zfs_kernel, current_kernel):
                print("✅ Current ArchZFS repo has exact match")
                return {"strategy": "archzfs", "linux_version": current_kernel, "zfs_version": archzfs_version, "source": "archzfs"}

        # Strategy 2: Check if current AUR package matches current kernel
        aur_version = self.get_aur_version()
        if aur_version:
            zfs_kernel = self.extract_kernel_version(aur_version)
            if zfs_kernel and self.kernel_versions_exact_match(zfs_kernel, current_kernel):
                print("✅ Current AUR package matches current kernel")
                return {"strategy": "aur_current", "linux_version": current_kernel, "zfs_version": aur_version, "source": "aur_current"}

        # Strategy 3: Find older AUR version that matches current kernel
        print("🔍 Searching AUR package history for compatible versions...")
        aur_history = self.get_aur_package_history()
        for aur_pkg in aur_history:
            if self.kernel_versions_exact_match(aur_pkg["kernel_version"], current_kernel):
                print(f"✅ Found matching AUR version: {aur_pkg['package_version']}")
                return {
                    "strategy": "aur_historical",
                    "linux_version": current_kernel,
                    "zfs_version": aur_pkg["package_version"],
                    "aur_commit": aur_pkg["commit"],
                    "source": "aur_historical",
                }

        # Strategy 4: Find compatible combination using archive linux-lts
        print("🔍 Searching for compatible combination with archive repositories...")
        archive_kernels = self.get_archive_linux_versions()
        # archive_kernels elements are dicts with keys: version, date, archive_url
        for aur_pkg in aur_history:
            for ak in archive_kernels:
                version = ak["version"]
                archive_url = ak["archive_url"]
                if self.kernel_versions_exact_match(aur_pkg["kernel_version"], version):
                    print(f"✅ Found compatible combination: linux-lts {version} + ZFS {aur_pkg['package_version']}")
                    return {
                        "strategy": "archive_combination",
                        "linux_version": version,
                        "zfs_version": aur_pkg["package_version"],
                        "aur_commit": aur_pkg["commit"],
                        "archive_url": archive_url,
                        "source": "archive_combination",
                    }

        print("❌ No compatible combination found")
        return None

    def _generate_temp_pacman_conf(self, archive_url: str | None, include_local_repo: bool) -> Path:
        """
        Generate a temporary pacman.conf without modifying system files.
        - If archive_url is provided, core/extra will point to the archive URLs.
        - Always adds [archzfs]; optionally adds [archzfs-local] to file:// local_repo_dir.
        Returns the path to the generated config.
        """
        temp_conf_dir = self.local_repo_dir / "tmp_conf"
        temp_conf_dir.mkdir(parents=True, exist_ok=True)
        conf_path = temp_conf_dir / "pacman.conf"

        options = """[options]
HoldPkg = pacman glibc
Architecture = auto
CheckSpace
ParallelDownloads = 5
SigLevel = Required DatabaseOptional
LocalFileSigLevel = Optional
"""

        if archive_url:
            core = f"[core]\nServer = {archive_url}core/os/$arch\n\n"
            extra = f"[extra]\nServer = {archive_url}extra/os/$arch\n\n"
        else:
            core = "[core]\nInclude = /etc/pacman.d/mirrorlist\n\n"
            extra = "[extra]\nInclude = /etc/pacman.d/mirrorlist\n\n"

        archzfs = "[archzfs]\nSigLevel = Optional TrustAll\nServer = https://archzfs.com/$repo/$arch\n\n"

        local_repo = ""
        if include_local_repo:
            local_repo = f"[archzfs-local]\nSigLevel = Optional TrustAll\nServer = file://{self.local_repo_dir}\n\n"

        conf_content = options + core + extra + local_repo + archzfs
        conf_path.write_text(conf_content)
        return conf_path

    def _pacman_with_config(self, args: list[str], conf_path: Path) -> subprocess.CompletedProcess:
        """
        Run pacman with a specific --config file. Does not modify system configuration.
        """
        cmd = ["pacman", "--config", str(conf_path), *args]
        return self.run_command(cmd, check=True, capture=True)

    def build_aur_package(self, combination: dict) -> bool:
        """Build zfs-linux-lts from AUR using specified combination."""
        try:
            # Create local repo directory
            self.local_repo_dir.mkdir(exist_ok=True)

            # Setup build directory
            build_dir = self.local_repo_dir / "zfs-linux-lts-build"
            if build_dir.exists():
                subprocess.run(["rm", "-rf", str(build_dir)], check=True)  # noqa: S603, S607

            # Clone and checkout specific commit if needed
            if combination.get("aur_commit"):
                print(f"📦 Building AUR package from commit {combination['aur_commit']}")
                self.run_command(["git", "clone", "https://aur.archlinux.org/zfs-linux-lts.git", str(build_dir)])
                subprocess.run(["git", "checkout", combination["aur_commit"]], cwd=build_dir, check=True)  # noqa: S603, S607
            else:
                print("📦 Building latest AUR package")
                self.run_command(["git", "clone", "https://aur.archlinux.org/zfs-linux-lts.git", str(build_dir)])

            # If using archive combination, sync deps with the same temporary config
            if combination.get("source") == "archive_combination":
                conf = self._generate_temp_pacman_conf(combination.get("archive_url"), include_local_repo=False)
                try:
                    self._pacman_with_config(["-Sy"], conf)
                except subprocess.CalledProcessError:
                    print("❌ Failed to sync dependencies for makepkg with temporary archive config")
                    return False

            # Build package
            print("🔨 Building ZFS package...")
            subprocess.run(["makepkg", "-s", "--noconfirm"], cwd=build_dir, check=True)  # noqa: S607

            # Move built packages to local repo
            for pkg_file in build_dir.glob("*.pkg.tar.*"):
                pkg_file.rename(self.local_repo_dir / pkg_file.name)
                print(f"📦 Built package: {pkg_file.name}")

            return True

        except subprocess.CalledProcessError as e:
            print(f"Error building AUR package: {e}")
            return False

    def create_local_repository(self) -> bool:
        """Create a local pacman repository from built packages."""
        try:
            # Create repository database
            pkg_files = list(self.local_repo_dir.glob("*.pkg.tar.*"))
            if not pkg_files:
                print("No package files found for repository creation")
                return False

            print("Creating local repository database...")
            cmd = ["repo-add", str(self.local_repo_dir / "archzfs-local.db.tar.xz")] + [str(f) for f in pkg_files]
            self.run_command(cmd)

            return True

        except subprocess.CalledProcessError as e:
            print(f"Error creating local repository: {e}")
            return False

    def update_iso_configs(self, combination: dict) -> None:
        """Update ISO configurations to use optimal ZFS packages (never DKMS)."""
        profiles = ["main_profile", "testing_profile"]

        for profile in profiles:
            # Update pacman.conf
            pacman_conf = self.gen_iso_dir / profile / "pacman.conf"
            self.update_pacman_conf(pacman_conf, combination)

            # Update packages.x86_64
            packages_file = self.gen_iso_dir / profile / "packages.x86_64"
            self.update_packages_file(packages_file, combination)

    def update_pacman_conf(self, pacman_conf: Path, combination: dict) -> None:
        """Update pacman.conf for optimal ZFS package source."""
        content = pacman_conf.read_text()

        # Remove existing local repo section
        content = re.sub(r"\n\[archzfs-local\]\n[^\[]*", "", content)

        # Add local repo if we built packages
        if combination["source"] in ["aur_current", "aur_historical", "archive_combination"]:
            local_repo_section = f"""
[archzfs-local]
SigLevel = Optional TrustAll
Server = file://{self.local_repo_dir}

"""
            # Insert before [archzfs] section
            content = content.replace("[archzfs]", local_repo_section + "[archzfs]")

        # For archive combinations, also set up archive repos in ISO
        if combination["source"] == "archive_combination":
            # Replace core/extra repos with archive versions
            archive_url = combination["archive_url"]
            content = re.sub(r"\[core\]\nServer = [^\n]+", f"[core]\nServer = {archive_url}core/os/$arch", content)
            content = re.sub(r"\[extra\]\nServer = [^\n]+", f"[extra]\nServer = {archive_url}extra/os/$arch", content)

        pacman_conf.write_text(content)
        print(f"Updated {pacman_conf}")

    def update_packages_file(self, packages_file: Path, combination: dict) -> None:
        """Update packages.x86_64 to always use precompiled ZFS packages (never DKMS)."""
        lines = packages_file.read_text().splitlines()
        new_lines: list[str] = []

        for raw_line in lines:
            stripped = raw_line.strip()
            if not stripped:
                new_lines.append(stripped)
                continue

            # Remove DKMS packages - we always use precompiled now
            if stripped in ["base-devel", "linux-lts-headers", "zfs-dkms"]:
                continue  # Skip DKMS components
            if stripped == "zfs-utils":
                # Add precompiled ZFS packages
                new_lines.extend(["zfs-linux-lts", "zfs-utils"])
            elif stripped == "zfs-linux-lts":
                # Don't duplicate if already present
                if "zfs-linux-lts" not in new_lines:
                    new_lines.append(stripped)
            else:
                new_lines.append(stripped)

        # For archive combinations, ensure we use the specific linux-lts version
        if combination.get("source") == "archive_combination":
            # Replace linux-lts with specific version
            linux_version = str(combination.get("linux_version"))
            for i, entry in enumerate(new_lines):
                if entry == "linux-lts":
                    new_lines[i] = f"linux-lts={linux_version}"
                    break

        # Remove duplicates while preserving order
        seen: set[str] = set()
        final_lines: list[str] = []
        for entry in new_lines:
            if entry not in seen:
                seen.add(entry)
                final_lines.append(entry)

        packages_file.write_text("\n".join(final_lines) + "\n")
        print(f"Updated {packages_file}")

    def run(self) -> int:
        """Main script execution - Always finds precompiled ZFS packages."""
        print("=== Smart ZFS Package Builder ===")
        print("🎯 Goal: ALWAYS use precompiled packages (never DKMS)")

        try:
            # Find the best compatible combination
            combination = self.find_compatible_combination()

            if not combination:
                print("💥 CRITICAL: No compatible combination found!")
                print("This should never happen with our robust approach.")
                return 1

            print(f"\n🎯 Selected strategy: {combination['strategy']}")
            print(f"📦 Linux version: {combination['linux_version']}")
            print(f"🗂️ ZFS version: {combination['zfs_version']}")
            print(f"🔧 Source: {combination['source']}")

            # Prepare a temporary pacman.conf for any operations that need archive repos
            temp_conf: Path | None = None
            if combination["source"] == "archive_combination":
                temp_conf = self._generate_temp_pacman_conf(combination["archive_url"], include_local_repo=False)
                # Sync databases using the temporary configuration
                try:
                    self._pacman_with_config(["-Sy"], temp_conf)
                except subprocess.CalledProcessError:
                    print("❌ Failed to sync pacman databases with temporary archive config")
                    return 1

            try:
                # Build packages if needed
                if combination["source"] in ["aur_current", "aur_historical", "archive_combination"]:
                    print("\n🔨 Building ZFS packages...")
                    if not self.build_aur_package(combination):
                        print("❌ Failed to build AUR package")
                        return 1

                    if not self.create_local_repository():
                        print("❌ Failed to create local repository")
                        return 1

                    print("✅ Successfully built and packaged ZFS modules")

                # Update ISO configurations
                print("\n⚙️ Configuring ISO profiles...")
                self.update_iso_configs(combination)

                # Success message
                print("\n🎉 SUCCESS: ISO configured with precompiled ZFS packages!")
                print("📏 Result: Slim ISO without build tools")
                print(f"🚀 Strategy: {combination['strategy']}")

                return 0

            finally:
                # No system repository restoration needed when using --config
                pass

        except Exception as e:
            print(f"💥 Unexpected error: {e}")
            return 1


if __name__ == "__main__":
    builder = ZFSPackageBuilder()
    sys.exit(builder.run())
