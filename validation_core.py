"""
Core validation logic for kernel/ZFS compatibility checking.

This module provides validation functions that work in any context without
depending on archinstall, making it suitable for both the TUI installer
and standalone scripts like iso_builder.
"""

import json
import lzma
import os
import re
import subprocess
import sys
import tarfile
from io import BytesIO


def debug_print(msg: str) -> None:
    """Simple debug output that respects DEBUG environment variable."""
    if os.getenv("DEBUG", "").lower() in ("1", "true", "yes"):
        print(f"DEBUG: {msg}", file=sys.stderr)


def warn_print(msg: str) -> None:
    """Simple warning output."""
    print(f"WARN: {msg}", file=sys.stderr)


def get_archzfs_package_version(package_name: str) -> str | None:
    """
    Gets package version from archzfs repository database by downloading and parsing it directly.

    This method works even when the archzfs repository is not configured locally,
    making it suitable for CI environments.

    Args:
        package_name: Name of the package to query (e.g., "zfs-dkms")

    Returns:
        Package version string if found, None otherwise
    """
    archzfs_db_url = "https://github.com/archzfs/archzfs/releases/download/experimental/archzfs.db"

    try:
        debug_print(f"Downloading archzfs database from {archzfs_db_url}")
        result = subprocess.run(["curl", "-sL", archzfs_db_url], capture_output=True, check=False)  # noqa: S603, S607

        if result.returncode != 0:
            debug_print(f"Failed to download archzfs.db: {result.stderr.decode('utf-8', errors='ignore')}")
            return None

        # Decompress the XZ data
        debug_print("Decompressing archzfs database")
        decompressed_data = lzma.decompress(result.stdout)

        # Parse the tar archive
        with tarfile.open(fileobj=BytesIO(decompressed_data), mode="r") as tar:
            # Look for package directories that match our package name
            for member in tar.getmembers():
                if member.isdir() and member.name.startswith(f"{package_name}-"):
                    # Extract the desc file for this package
                    desc_path = f"{member.name}/desc"
                    try:
                        desc_file = tar.extractfile(desc_path)
                        if desc_file:
                            desc_content = desc_file.read().decode("utf-8")

                            # Parse the package description format
                            lines = desc_content.strip().split("\n")
                            in_version_section = False

                            for line in lines:
                                if line.strip() == "%VERSION%":
                                    in_version_section = True
                                    continue
                                if line.startswith("%") and in_version_section:
                                    # End of version section
                                    break
                                if in_version_section and line.strip():
                                    version = line.strip()
                                    debug_print(f"Found {package_name} version from archzfs.db: {version}")
                                    return version
                    except KeyError:
                        continue  # No desc file for this entry

        debug_print(f"Package {package_name} not found in archzfs database")
        return None

    except Exception as e:
        debug_print(f"Failed to parse archzfs database: {e}")
        return None


def get_package_version(package_name: str) -> str | None:  # noqa: PLR0911
    """
    Gets the version of a package from the pacman sync database.
    For ZFS packages, falls back to downloading archzfs.db directly if pacman fails.

    Args:
        package_name: Name of the package to query

    Returns:
        Package version string if found, None otherwise
    """
    try:
        # Use -Si to get sync database info without requiring root
        result = subprocess.run(["pacman", "-Si", package_name], capture_output=True, text=True, check=False)  # noqa: S603, S607
        if result.returncode != 0:
            debug_print(f"pacman -Si {package_name} failed with exit code {result.returncode}")

            # For ZFS packages, try fallback to archzfs database
            if package_name.startswith(("zfs-", "spl-")):
                debug_print(f"Trying archzfs database fallback for {package_name}")
                return get_archzfs_package_version(package_name)

            return None

        match = re.search(r"Version\s*:\s*(.+)", result.stdout)
        if match:
            version = match.group(1).strip()
            debug_print(f"Found {package_name} version: {version}")
            return version
        debug_print(f"No version line found in pacman output for {package_name}")

        # For ZFS packages, try fallback to archzfs database
        if package_name.startswith(("zfs-", "spl-")):
            debug_print(f"Trying archzfs database fallback for {package_name}")
            return get_archzfs_package_version(package_name)

    except Exception as e:
        debug_print(f"Failed to get version for {package_name}: {e}")

        # For ZFS packages, try fallback to archzfs database
        if package_name.startswith(("zfs-", "spl-")):
            debug_print(f"Trying archzfs database fallback for {package_name}")
            return get_archzfs_package_version(package_name)

        return None
    return None


def fetch_zfs_kernel_compatibility(zfs_version: str) -> tuple[str, str] | None:
    """
    Fetches OpenZFS release data from the GitHub API and parses the kernel compatibility range.

    Args:
        zfs_version: ZFS version string (e.g., "2.3.3-1")

    Returns:
        Tuple of (min_kernel_version, max_kernel_version) if found, None otherwise
    """
    base_zfs_version = zfs_version.split("-")[0]
    tag_name = f"zfs-{base_zfs_version}"
    api_url = f"https://api.github.com/repos/openzfs/zfs/releases/tags/{tag_name}"

    debug_print(f"Fetching compatibility info from API: {api_url}")

    try:
        result = subprocess.run(["curl", "-sL", "-H", "Accept: application/vnd.github.v3+json", api_url], capture_output=True, text=True, check=False)  # noqa: S603, S607

        if result.returncode != 0:
            debug_print(f"curl failed for {api_url}")
            return None

        release_data = json.loads(result.stdout)
        release_body = release_data.get("body", "")
        if not release_body:
            debug_print(f"No release body found for {tag_name}")
            return None

        # Parse kernel compatibility from release notes with multiple patterns for robustness
        compatibility_patterns = [
            r"\*\*Linux\*\*:\s*compatible with\s*([\d.]+)\s*-\s*([\d.]+)\s*kernels",
            r"Linux.*?compatible with.*?([\d.]+)\s*-\s*([\d.]+)\s*kernels",
            r"Kernel.*?compatibility.*?([\d.]+)\s*-\s*([\d.]+)",
            r"Linux kernel.*?([\d.]+)\s*-\s*([\d.]+)",
        ]

        for pattern in compatibility_patterns:
            match = re.search(pattern, release_body, re.IGNORECASE | re.DOTALL)
            if match:
                min_kernel, max_kernel = match.groups()
                debug_print(f"Found compatible kernel range via API: {min_kernel} - {max_kernel}")
                return min_kernel, max_kernel

        debug_print(f"No kernel compatibility information found in release notes for {tag_name}")
        return None

    except (json.JSONDecodeError, Exception) as e:
        warn_print(f"Failed to get compatibility data for ZFS tag {tag_name}: {e}")
        return None


def parse_version(version_str: str) -> tuple[int, ...]:
    """
    Parse version string into comparable tuple, normalizing for kernel compatibility.

    For kernel compatibility, we only care about major.minor for range checking,
    since patch versions (6.15.x) should be compatible with 6.15.

    Args:
        version_str: Version string to parse (e.g., "6.8.arch1", "2.3.3")

    Returns:
        Tuple of version components, normalized for compatibility checking
    """
    try:
        from packaging.version import parse as _parse_packaging  # noqa: PLC0415

        try:
            parsed = _parse_packaging(version_str)
            # For kernel compatibility, only use major.minor (ignore patch/micro)
            # This treats 6.15.9 as equivalent to 6.15.0 for range checking
            return (parsed.major, parsed.minor, 0)
        except Exception:
            # Fallback to simple parsing if packaging fails
            return _parse_version_fallback(version_str)
    except ImportError:
        # Fallback for systems without packaging library
        return _parse_version_fallback(version_str)


def _parse_version_fallback(version_str: str) -> tuple[int, ...]:
    """
    Fallback version parsing when packaging library is unavailable.
    
    Normalizes to major.minor.0 for kernel compatibility checking.
    """
    # Remove non-numeric chars except dots, handle kernel suffixes
    clean_version = re.sub(r"[^\d.]", "", version_str.split("-")[0])
    parts = [int(x) for x in clean_version.split(".") if x.isdigit()]
    
    if len(parts) == 0:
        return (0, 0, 0)
    elif len(parts) == 1:
        return (parts[0], 0, 0)
    elif len(parts) == 2:
        return (parts[0], parts[1], 0)
    else:
        # For kernel compatibility, normalize patch versions to 0
        # This makes 6.15.9 equivalent to 6.15.0 for range checking
        return (parts[0], parts[1], 0)


def validate_kernel_zfs_compatibility(kernel_name: str, zfs_mode: str) -> tuple[bool, list[str]]:  # noqa: PLR0911
    """
    Validates compatibility between a kernel and ZFS module mode.

    Args:
        kernel_name: Name of the kernel package (e.g., "linux", "linux-lts", "linux-zen")
        zfs_mode: ZFS module mode ("precompiled" or "dkms")

    Returns:
        Tuple of (is_compatible, warnings_list)
        - is_compatible: True if compatible, False if incompatible or validation failed
        - warnings_list: List of warning messages
    """
    warnings: list[str] = []

    # Only validate DKMS mode - precompiled is always compatible if package exists
    if zfs_mode != "dkms":
        debug_print(f"Skipping validation for {zfs_mode} mode")
        return True, warnings

    debug_print(f"Validating DKMS compatibility for {kernel_name}")

    # Get package versions and compatibility data
    zfs_pkg_ver = get_package_version("zfs-dkms")
    kernel_pkg_ver = get_package_version(kernel_name)
    compatibility_range = fetch_zfs_kernel_compatibility(zfs_pkg_ver) if zfs_pkg_ver else None

    # Handle cases where we can't get required information (fail hard for critical data)
    if not zfs_pkg_ver:
        warnings.append("Could not determine zfs-dkms version - ZFS repository may not be configured or package unavailable")
        return False, warnings
    if not kernel_pkg_ver:
        warnings.append(f"Could not determine {kernel_name} version - package repository issue")
        return False, warnings
    if not compatibility_range:
        warnings.append("Could not fetch ZFS kernel compatibility data from GitHub API - network or API issue")
        return False, warnings

    min_kernel_ver, max_kernel_ver = compatibility_range

    try:
        # Parse kernel version (remove package suffix)
        kernel_base_ver = kernel_pkg_ver.split("-")[0]

        kernel_version = parse_version(kernel_base_ver)
        min_version = parse_version(min_kernel_ver)
        max_version = parse_version(max_kernel_ver)

        if min_version <= kernel_version <= max_version:
            debug_print(f"Kernel {kernel_name} ({kernel_base_ver}) is compatible with ZFS DKMS")
            return True, warnings

        warning_msg = f"Kernel {kernel_name} ({kernel_base_ver}) is outside the supported range for ZFS DKMS ({min_kernel_ver} - {max_kernel_ver})"
        warnings.append(warning_msg)
        return False, warnings

    except Exception as e:
        warn_print(f"Error parsing version information: {e}")
        warnings.append("Version parsing failed - cannot determine compatibility")
        return False, warnings


def get_compatible_kernels(kernel_names: list[str]) -> tuple[list[str], list[str]]:
    """
    Get lists of compatible and incompatible kernels for ZFS DKMS.

    Args:
        kernel_names: List of kernel names to check

    Returns:
        Tuple of (compatible_kernels, incompatible_kernels)
    """
    compatible_kernels = []
    incompatible_kernels = []

    for kernel_name in kernel_names:
        is_compatible, _ = validate_kernel_zfs_compatibility(kernel_name, "dkms")
        if is_compatible:
            compatible_kernels.append(kernel_name)
        else:
            incompatible_kernels.append(kernel_name)

    return compatible_kernels, incompatible_kernels


def validate_precompiled_zfs_compatibility(kernel_name: str) -> tuple[bool, list[str]]:
    """
    Validate compatibility between a kernel and its precompiled ZFS package.
    
    Args:
        kernel_name: Name of the kernel (e.g., "linux-zen")
        
    Returns:
        Tuple of (is_compatible, warnings)
    """
    warnings = []
    
    # Get the precompiled package name
    precompiled_pkg_name = f"zfs-{kernel_name}"
    
    # Get versions
    kernel_pkg_ver = get_package_version(kernel_name)
    zfs_pkg_ver = get_package_version(precompiled_pkg_name)
    
    if not kernel_pkg_ver:
        warnings.append(f"Could not determine {kernel_name} version")
        return False, warnings
        
    if not zfs_pkg_ver:
        warnings.append(f"Could not determine {precompiled_pkg_name} version - precompiled package may not be available")
        return False, warnings
    
    try:
        # Keep full kernel version including package release
        kernel_full_ver = kernel_pkg_ver
        
        # Parse ZFS package version: format is {zfs_version}_{supported_kernel_version}-{pkg_release}
        # Example: "2.3.3_6.15.9.zen1.1-1" -> supported kernel is "6.15.9.zen1.1-1"
        if "_" not in zfs_pkg_ver:
            warnings.append(f"Unexpected {precompiled_pkg_name} version format: {zfs_pkg_ver}")
            return False, warnings
            
        _, supported_kernel_part = zfs_pkg_ver.split("_", 1)
        # Keep the full supported kernel version including package release
        supported_kernel_ver = supported_kernel_part
        
        # Compare versions - precompiled ZFS must match EXACTLY
        if kernel_full_ver == supported_kernel_ver:
            debug_print(f"Kernel {kernel_name} ({kernel_full_ver}) matches exactly with precompiled ZFS ({supported_kernel_ver})")
            return True, warnings
        else:
            warning_msg = f"Kernel {kernel_name} ({kernel_full_ver}) does not match precompiled ZFS (requires exactly {supported_kernel_ver})"
            warnings.append(warning_msg)
            return False, warnings
            
    except Exception as e:
        warn_print(f"Error parsing precompiled ZFS version information: {e}")
        warnings.append("Version parsing failed - cannot determine precompiled ZFS compatibility")
        return False, warnings


def should_filter_kernel_options() -> bool:
    """
    Determines whether kernel options should be filtered based on compatibility.

    Returns:
        True if filtering should be enabled, False otherwise
    """
    # Check environment variable for override
    filter_env = os.getenv("ARCHINSTALL_ZFS_FILTER_KERNELS", "").lower()
    if filter_env in ("0", "false", "no", "off", "disable"):
        debug_print("Kernel filtering disabled via environment variable")
        return False

    # Default: enable filtering
    return True
