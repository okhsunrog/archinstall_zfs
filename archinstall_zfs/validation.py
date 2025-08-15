"""
Proactive DKMS compatibility validation for ZFS and kernel combinations.

This module provides validation to prevent DKMS compilation failures by checking
kernel and ZFS version compatibility before installation begins.
"""

import json
import re
from typing import Optional

from archinstall import debug, warn
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand

try:
    from packaging.version import parse as parse_version
except ImportError:
    # Fallback for systems without packaging library
    def parse_version(version_str: str) -> tuple[int, ...]:
        """Simple version parsing fallback."""
        # Extract numeric parts only
        version_str = re.sub(r'[^\d.]', '', version_str.split('-')[0])
        return tuple(int(x) for x in version_str.split('.') if x.isdigit())


def get_package_version(package_name: str) -> Optional[str]:
    """
    Gets the version of a package from the pacman sync database.
    
    Args:
        package_name: Name of the package to query
        
    Returns:
        Version string (e.g. "2.3.3-1") or None if package not found
    """
    try:
        # Use -Si to get sync database info without requiring root
        output = SysCommand(f"pacman -Si {package_name}").decode()
        match = re.search(r"Version\s*:\s*(.+)", output)
        if match:
            version = match.group(1).strip()
            debug(f"Found {package_name} version: {version}")
            return version
    except Exception as e:
        debug(f"Failed to get version for {package_name}: {e}")
        return None
    return None


def fetch_zfs_kernel_compatibility(zfs_version: str) -> Optional[tuple[str, str]]:
    """
    Fetches OpenZFS release data from the GitHub API and parses the kernel compatibility range.
    
    Args:
        zfs_version: Version string from zfs-dkms package (e.g. "2.3.3-1")
        
    Returns:
        Tuple of (min_kernel_version, max_kernel_version) or None if not found
    """
    # Extract base version (remove package revision)
    base_zfs_version = zfs_version.split('-')[0]
    tag_name = f"zfs-{base_zfs_version}"
    api_url = f"https://api.github.com/repos/openzfs/zfs/releases/tags/{tag_name}"
    
    debug(f"Fetching compatibility info from API: {api_url}")
    
    try:
        # Use curl to avoid additional Python dependencies
        cmd = f'curl -sL -H "Accept: application/vnd.github.v3+json" "{api_url}"'
        api_response_str = SysCommand(cmd).decode()
        release_data = json.loads(api_response_str)
        
        # Check for API error
        if "message" in release_data:
            debug(f"GitHub API returned error: {release_data.get('message')}")
            return None
            
        release_body = release_data.get("body", "")
        if not release_body:
            debug(f"No release body found for {tag_name}")
            return None
            
        # Parse kernel compatibility from release notes
        # Based on actual ZFS release format: "**Linux**: compatible with 4.18 - 6.15 kernels"
        compatibility_patterns = [
            r"\*\*Linux\*\*:\s*compatible with\s*([\d.]+)\s*-\s*([\d.]+)\s*kernels",
            r"Linux.*?compatible with.*?([\d.]+)\s*-\s*([\d.]+)\s*kernels",
            r"Kernel.*?compatibility.*?([\d.]+)\s*-\s*([\d.]+)",
            r"Linux kernel.*?([\d.]+)\s*-\s*([\d.]+)"
        ]
        
        for pattern in compatibility_patterns:
            match = re.search(pattern, release_body, re.IGNORECASE | re.DOTALL)
            if match:
                min_kernel, max_kernel = match.groups()
                debug(f"Found compatible kernel range for {tag_name}: {min_kernel} - {max_kernel}")
                return min_kernel, max_kernel
                
        debug(f"No kernel compatibility information found in release notes for {tag_name}")
        return None
            
    except (Exception, json.JSONDecodeError) as e:
        warn(f"Failed to get compatibility data for ZFS tag {tag_name}: {e}")
        return None


def validate_kernel_zfs_compatibility(kernel_name: str, zfs_mode: str) -> tuple[bool, list[str]]:
    """
    Validates compatibility between a kernel and ZFS DKMS.
    
    Args:
        kernel_name: Name of the kernel package (e.g. "linux-zen")
        zfs_mode: ZFS module mode ("precompiled" or "dkms")
        
    Returns:
        Tuple of (is_compatible, warnings_list)
    """
    warnings = []
    
    # Only validate DKMS mode - precompiled is always compatible if package exists
    if zfs_mode != "dkms":
        return True, warnings
    
    # Get ZFS DKMS version
    zfs_pkg_ver = get_package_version("zfs-dkms")
    if not zfs_pkg_ver:
        warnings.append("Could not determine zfs-dkms version - assuming compatible")
        return True, warnings

    # Get kernel version  
    kernel_pkg_ver = get_package_version(kernel_name)
    if not kernel_pkg_ver:
        warnings.append(f"Could not determine {kernel_name} version - assuming compatible")
        return True, warnings
    
    # Fetch compatibility range from OpenZFS
    compatibility_range = fetch_zfs_kernel_compatibility(zfs_pkg_ver)
    if not compatibility_range:
        warnings.append("Could not fetch ZFS kernel compatibility - assuming compatible")
        return True, warnings
        
    min_kernel_ver, max_kernel_ver = compatibility_range
    
    try:
        # Parse kernel version (remove package suffix)
        kernel_base_ver = kernel_pkg_ver.split('-')[0]
        
        # Use packaging library if available, fallback to simple parsing
        if hasattr(parse_version, '__module__'):
            # packaging library available
            kernel_version = parse_version(kernel_base_ver)
            min_version = parse_version(min_kernel_ver)
            max_version = parse_version(max_kernel_ver)
        else:
            # Fallback to simple tuple comparison
            kernel_version = parse_version(kernel_base_ver)
            min_version = parse_version(min_kernel_ver)
            max_version = parse_version(max_kernel_ver)
        
        if min_version <= kernel_version <= max_version:
            debug(f"Kernel {kernel_name} ({kernel_base_ver}) is compatible with ZFS DKMS")
            return True, warnings
        else:
            warning_msg = (
                f"Kernel {kernel_name} ({kernel_base_ver}) is outside the supported range "
                f"for ZFS DKMS ({min_kernel_ver} - {max_kernel_ver})"
            )
            warnings.append(warning_msg)
            return False, warnings
            
    except Exception as e:
        warn(f"Error parsing version information: {e}")
        warnings.append("Version parsing failed - assuming compatible")
        return True, warnings


def get_compatible_kernels(available_kernels: list[str]) -> tuple[list[str], list[str]]:
    """
    Check all available kernels for DKMS compatibility.
    
    Args:
        available_kernels: List of kernel names to check
        
    Returns:
        Tuple of (compatible_kernels, incompatible_kernels)
    """
    compatible = []
    incompatible = []
    
    for kernel in available_kernels:
        is_compatible, warnings = validate_kernel_zfs_compatibility(kernel, "dkms")
        
        if warnings:
            for warning in warnings:
                debug(f"Validation warning for {kernel}: {warning}")
        
        if is_compatible:
            compatible.append(kernel)
        else:
            incompatible.append(kernel)
            
    return compatible, incompatible


def should_filter_kernel_options() -> bool:
    """
    Determine if kernel filtering should be enabled.
    
    This allows for easy disabling of the feature via environment variable
    or other configuration if needed.
    """
    import os
    # Allow disabling via environment variable for debugging/testing
    return os.getenv("ARCHINSTALL_ZFS_DISABLE_KERNEL_FILTERING", "").lower() not in ("1", "true", "yes")
