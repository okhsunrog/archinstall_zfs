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
import urllib.request
import urllib.parse
from pathlib import Path
from typing import Optional, Tuple


class ZFSPackageBuilder:
    def __init__(self):
        self.project_root = Path(__file__).parent.parent
        self.local_repo_dir = self.project_root / "local_repo"
        self.gen_iso_dir = Path(__file__).parent
        
    def run_command(self, cmd: list[str], check: bool = True, capture: bool = True) -> subprocess.CompletedProcess:
        """Run a command and return the result."""
        print(f"Running: {' '.join(cmd)}")
        if capture:
            return subprocess.run(cmd, check=check, capture_output=True, text=True)
        else:
            return subprocess.run(cmd, check=check)
    
    def get_archzfs_version(self) -> Optional[str]:
        """Get the current zfs-linux-lts version from archzfs repository."""
        try:
            with urllib.request.urlopen("https://archzfs.com/archzfs/x86_64/") as response:
                content = response.read().decode('utf-8')
                
            # Look for zfs-linux-lts package
            matches = re.findall(r'zfs-linux-lts-([^"]+)\.pkg\.tar\.zst', content)
            if matches:
                return matches[0]  # Return first match
            return None
        except Exception as e:
            print(f"Error fetching archzfs version: {e}")
            return None
    
    def get_current_kernel_version(self) -> str:
        """Get the current linux-lts kernel version."""
        result = self.run_command(["pacman", "-Si", "linux-lts"])
        for line in result.stdout.split('\n'):
            if line.startswith('Version'):
                return line.split(':')[1].strip()
        raise RuntimeError("Could not determine linux-lts version")
    
    def get_aur_version(self) -> Optional[str]:
        """Get the latest zfs-linux-lts version from AUR."""
        try:
            url = "https://aur.archlinux.org/rpc/?v=5&type=info&arg=zfs-linux-lts"
            with urllib.request.urlopen(url) as response:
                data = json.loads(response.read().decode('utf-8'))
                
            if data['resultcount'] > 0:
                return data['results'][0]['Version']
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
            result = subprocess.run(["git", "log", "--oneline", "--follow", "PKGBUILD"], cwd=aur_dir, capture_output=True, text=True, check=True)
            
            versions = []
            for line in result.stdout.split('\n'):
                if not line.strip():
                    continue
                    
                commit_hash = line.split()[0]
                # Get PKGBUILD content for this commit
                try:
                    pkgbuild_result = subprocess.run(["git", "show", f"{commit_hash}:PKGBUILD"], cwd=aur_dir, capture_output=True, text=True, check=True)
                    pkgbuild_content = pkgbuild_result.stdout
                    
                    # Extract version info from PKGBUILD
                    zfs_ver_match = re.search(r'_zfsver="([^"]+)"', pkgbuild_content)
                    kernel_ver_match = re.search(r'_kernelver="([^"]+)"', pkgbuild_content)
                    
                    if zfs_ver_match and kernel_ver_match:
                        zfs_ver = zfs_ver_match.group(1)
                        kernel_ver = kernel_ver_match.group(1)
                        
                        # Construct package version
                        pkg_version = f"{zfs_ver}_{kernel_ver.replace('-', '.')}-1"
                        
                        versions.append({
                            'commit': commit_hash,
                            'zfs_version': zfs_ver,
                            'kernel_version': kernel_ver,
                            'package_version': pkg_version
                        })
                        
                except subprocess.CalledProcessError:
                    continue  # Skip commits where we can't read PKGBUILD
            
            # Remove duplicates and sort by commit order (newest first)
            seen_versions = set()
            unique_versions = []
            for v in versions:
                if v['package_version'] not in seen_versions:
                    seen_versions.add(v['package_version'])
                    unique_versions.append(v)
            
            return unique_versions[:20]  # Return recent 20 versions
            
        except Exception as e:
            print(f"Error getting AUR package history: {e}")
            return []
    
    def get_archive_linux_versions(self) -> list[str]:
        """Get available linux-lts versions from archive.archlinux.org."""
        try:
            # Get list of dates from archive
            url = "https://archive.archlinux.org/repos/"
            with urllib.request.urlopen(url) as response:
                content = response.read().decode('utf-8')
            
            # Extract date directories (format: YYYY/MM/DD/)
            date_matches = re.findall(r'(\d{4}/\d{2}/\d{2})/', content)
            recent_dates = sorted(date_matches)[-60:]  # Last 60 days of archives
            
            linux_versions = []
            for date in recent_dates:
                try:
                    # Check what linux-lts version was available on this date
                    archive_url = f"https://archive.archlinux.org/repos/{date}/core/os/x86_64/"
                    with urllib.request.urlopen(archive_url) as response:
                        archive_content = response.read().decode('utf-8')
                    
                    # Look for linux-lts packages
                    linux_matches = re.findall(r'linux-lts-(\d+\.\d+\.\d+-\d+)-x86_64\.pkg\.tar\.', archive_content)
                    for version in linux_matches:
                        if version not in [v['version'] for v in linux_versions]:
                            linux_versions.append({
                                'version': version,
                                'date': date,
                                'archive_url': f"https://archive.archlinux.org/repos/{date}/"
                            })
                            
                except:
                    continue  # Skip dates where archive is not accessible
            
            return linux_versions
            
        except Exception as e:
            print(f"Error getting archive linux versions: {e}")
            return []
    
    def extract_kernel_version(self, zfs_version: str) -> Optional[str]:
        """Extract kernel version from zfs package version."""
        # zfs-linux-lts versions look like: 2.3.3_6.12.41.1-1
        match = re.search(r'_(\d+\.\d+\.\d+(?:\.\d+)?)', zfs_version)
        if match:
            return match.group(1)
        return None
    
    def kernel_versions_exact_match(self, zfs_kernel_version: str, linux_kernel_version: str) -> bool:
        """Check if ZFS package kernel version exactly matches linux kernel version."""
        # Remove arch suffix from linux version
        linux_base = linux_kernel_version.split('-')[0] + '-' + linux_kernel_version.split('-')[1]
        return zfs_kernel_version == linux_base
    
    def find_compatible_combination(self) -> Optional[dict]:
        """Find the best compatible combination of linux-lts and zfs-linux-lts versions."""
        current_kernel = self.get_current_kernel_version()
        print(f"Current linux-lts: {current_kernel}")
        
        # Strategy 1: Check if current archzfs has exact match
        archzfs_version = self.get_archzfs_version()
        if archzfs_version:
            zfs_kernel = self.extract_kernel_version(archzfs_version)
            if zfs_kernel and self.kernel_versions_exact_match(zfs_kernel, current_kernel):
                print("✅ Current ArchZFS repo has exact match")
                return {
                    'strategy': 'archzfs',
                    'linux_version': current_kernel,
                    'zfs_version': archzfs_version,
                    'source': 'archzfs'
                }
        
        # Strategy 2: Check if current AUR package matches current kernel
        aur_version = self.get_aur_version()
        if aur_version:
            zfs_kernel = self.extract_kernel_version(aur_version)
            if zfs_kernel and self.kernel_versions_exact_match(zfs_kernel, current_kernel):
                print("✅ Current AUR package matches current kernel")
                return {
                    'strategy': 'aur_current',
                    'linux_version': current_kernel,
                    'zfs_version': aur_version,
                    'source': 'aur_current'
                }
        
        # Strategy 3: Find older AUR version that matches current kernel
        print("🔍 Searching AUR package history for compatible versions...")
        aur_history = self.get_aur_package_history()
        for aur_pkg in aur_history:
            if self.kernel_versions_exact_match(aur_pkg['kernel_version'], current_kernel):
                print(f"✅ Found matching AUR version: {aur_pkg['package_version']}")
                return {
                    'strategy': 'aur_historical',
                    'linux_version': current_kernel,
                    'zfs_version': aur_pkg['package_version'],
                    'aur_commit': aur_pkg['commit'],
                    'source': 'aur_historical'
                }
        
        # Strategy 4: Find compatible combination using archive linux-lts
        print("🔍 Searching for compatible combination with archive repositories...")
        archive_kernels = self.get_archive_linux_versions()
        for aur_pkg in aur_history:
            for archive_kernel in archive_kernels:
                if self.kernel_versions_exact_match(aur_pkg['kernel_version'], archive_kernel['version']):
                    print(f"✅ Found compatible combination: linux-lts {archive_kernel['version']} + ZFS {aur_pkg['package_version']}")
                    return {
                        'strategy': 'archive_combination',
                        'linux_version': archive_kernel['version'],
                        'zfs_version': aur_pkg['package_version'],
                        'aur_commit': aur_pkg['commit'],
                        'archive_url': archive_kernel['archive_url'],
                        'source': 'archive_combination'
                    }
        
        print("❌ No compatible combination found")
        return None
    
    def setup_archive_repository(self, archive_url: str) -> bool:
        """Setup archive repository for specific date."""
        try:
            print(f"🔧 Setting up archive repository: {archive_url}")
            
            # Backup current pacman.conf
            subprocess.run(["sudo", "cp", "/etc/pacman.conf", "/etc/pacman.conf.backup"], check=True)
            
            # Create new pacman.conf with archive repositories
            archive_conf = f"""#
# /etc/pacman.conf
#
# See the pacman.conf(5) manpage for option and repository directives

[options]
HoldPkg     = pacman glibc
Architecture = auto
CheckSpace
ParallelDownloads = 5
SigLevel    = Required DatabaseOptional
LocalFileSigLevel = Optional

# Archive repositories for specific kernel version
[core]
Server = {archive_url}core/os/$arch

[extra]
Server = {archive_url}extra/os/$arch

# Current archzfs repository
[archzfs]
SigLevel = Optional TrustAll
Server = https://archzfs.com/$repo/$arch
"""
            
            with open("/tmp/pacman.conf.archive", "w") as f:
                f.write(archive_conf)
            
            subprocess.run(["sudo", "cp", "/tmp/pacman.conf.archive", "/etc/pacman.conf"], check=True)
            subprocess.run(["sudo", "pacman", "-Sy"], check=True)
            print("✅ Archive repository configured")
            return True
            
        except Exception as e:
            print(f"Error setting up archive repository: {e}")
            return False
    
    def restore_original_repositories(self) -> None:
        """Restore original pacman.conf."""
        try:
            subprocess.run(["sudo", "cp", "/etc/pacman.conf.backup", "/etc/pacman.conf"], check=True)
            subprocess.run(["sudo", "rm", "-f", "/etc/pacman.conf.backup", "/tmp/pacman.conf.archive"], check=True)
            subprocess.run(["sudo", "pacman", "-Sy"], check=True)
            print("🔧 Restored original repositories")
        except Exception as e:
            print(f"Warning: Could not restore repositories: {e}")
    
    def build_aur_package(self, combination: dict) -> bool:
        """Build zfs-linux-lts from AUR using specified combination."""
        try:
            # Create local repo directory
            self.local_repo_dir.mkdir(exist_ok=True)
            
            # Setup build directory
            build_dir = self.local_repo_dir / "zfs-linux-lts-build"
            if build_dir.exists():
                subprocess.run(["rm", "-rf", str(build_dir)], check=True)
            
            # Clone and checkout specific commit if needed
            if combination.get('aur_commit'):
                print(f"📦 Building AUR package from commit {combination['aur_commit']}")
                self.run_command(["git", "clone", "https://aur.archlinux.org/zfs-linux-lts.git", str(build_dir)])
                subprocess.run(["git", "checkout", combination['aur_commit']], cwd=build_dir, check=True)
            else:
                print("📦 Building latest AUR package")
                self.run_command(["git", "clone", "https://aur.archlinux.org/zfs-linux-lts.git", str(build_dir)])
            
            # Build package
            print("🔨 Building ZFS package...")
            subprocess.run(["makepkg", "-s", "--noconfirm"], cwd=build_dir, check=True)
            
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
        content = re.sub(r'\n\[archzfs-local\]\n[^\[]*', '', content)
        
        # Add local repo if we built packages
        if combination['source'] in ['aur_current', 'aur_historical', 'archive_combination']:
            local_repo_section = f"""
[archzfs-local]
SigLevel = Optional TrustAll
Server = file://{self.local_repo_dir}

"""
            # Insert before [archzfs] section
            content = content.replace("[archzfs]", local_repo_section + "[archzfs]")
        
        # For archive combinations, also set up archive repos in ISO
        if combination['source'] == 'archive_combination':
            # Replace core/extra repos with archive versions
            archive_url = combination['archive_url']
            content = re.sub(r'\[core\]\nServer = [^\n]+', f'[core]\nServer = {archive_url}core/os/$arch', content)
            content = re.sub(r'\[extra\]\nServer = [^\n]+', f'[extra]\nServer = {archive_url}extra/os/$arch', content)
        
        pacman_conf.write_text(content)
        print(f"Updated {pacman_conf}")
    
    def update_packages_file(self, packages_file: Path, combination: dict) -> None:
        """Update packages.x86_64 to always use precompiled ZFS packages (never DKMS)."""
        lines = packages_file.read_text().splitlines()
        new_lines = []
        
        for line in lines:
            line = line.strip()
            if not line:
                new_lines.append(line)
                continue
            
            # Remove DKMS packages - we always use precompiled now
            if line in ["base-devel", "linux-lts-headers", "zfs-dkms"]:
                continue  # Skip DKMS components
            elif line == "zfs-utils":
                # Add precompiled ZFS packages
                new_lines.extend(["zfs-linux-lts", "zfs-utils"])
            elif line == "zfs-linux-lts":
                # Don't duplicate if already present
                if "zfs-linux-lts" not in new_lines:
                    new_lines.append(line)
            else:
                new_lines.append(line)
        
        # For archive combinations, ensure we use the specific linux-lts version
        if combination['source'] == 'archive_combination':
            # Replace linux-lts with specific version
            linux_version = combination['linux_version']
            for i, line in enumerate(new_lines):
                if line == "linux-lts":
                    new_lines[i] = f"linux-lts={linux_version}"
                    break
        
        # Remove duplicates while preserving order
        seen = set()
        final_lines = []
        for line in new_lines:
            if line not in seen:
                seen.add(line)
                final_lines.append(line)
        
        packages_file.write_text('\n'.join(final_lines) + '\n')
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
            
            # Setup archive repository if needed
            archive_setup = False
            if combination['source'] == 'archive_combination':
                archive_setup = self.setup_archive_repository(combination['archive_url'])
                if not archive_setup:
                    print("❌ Failed to setup archive repository")
                    return 1
            
            try:
                # Build packages if needed
                if combination['source'] in ['aur_current', 'aur_historical', 'archive_combination']:
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
                # Always restore repositories if we modified them
                if archive_setup:
                    self.restore_original_repositories()
        
        except Exception as e:
            print(f"💥 Unexpected error: {e}")
            # Try to restore repositories if something went wrong
            try:
                self.restore_original_repositories()
            except:
                pass
            return 1


if __name__ == "__main__":
    builder = ZFSPackageBuilder()
    sys.exit(builder.run())