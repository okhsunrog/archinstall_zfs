# Design Proposal: Proactive DKMS Compatibility Check

**Status:** TODO (Future Enhancement)  
**Date:** 2025-08-14

## 1. Problem Statement

Currently, a user can select a kernel (e.g., `linux-zen`) and the `zfs-dkms` module type even if the latest version of that kernel is too new and not yet supported by the ZFS source code.

The `zfs-dkms` package itself has no explicit version dependency on kernel headers, so `pacman` will successfully install all the packages. The failure only occurs during a post-transaction hook when the DKMS framework attempts to compile the ZFS module against the incompatible kernel source, leading to a failed installation.

This is a frustrating user experience because the failure happens late in the process and the reason is not immediately obvious.

## 2. Proposed Solution

To prevent this failure, the installer will perform a proactive, pre-flight compatibility check *before* presenting the kernel selection menu to the user.

The core logic is as follows:
1. **Determine Target Versions:** For each available kernel (`linux`, `linux-lts`, `linux-zen`), query `pacman` to find the exact version that would be installed. Also, query for the version of `zfs-dkms`.
2. **Fetch Upstream Compatibility:** Using the `zfs-dkms` version, query the official OpenZFS GitHub API to retrieve the supported Linux kernel version range from the release notes.
3. **Validate and Filter:** Compare each kernel's version against the supported range.
4. **Guide the User:** Dynamically generate the kernel selection menu, showing only the compatible kernel options. If any kernels are filtered out, display a clear, informational message explaining why.

This approach transforms the user experience from reactive (failing late) to guided (preventing an invalid choice).

## 3. Key Design Principles

* **Use the Right Source of Truth:** `pacman` is the source of truth for package versions. The OpenZFS GitHub API is the source of truth for source code compatibility.
* **Fail Open:** In case of any error during the validation check (e.g., network issues, API changes), the check should be aborted, and all kernel options should be presented to the user. A degraded experience (showing all options) is better than a broken one (showing no options).
* **Clear User Communication:** The user must be informed when and why options are being hidden from them.

## 4. Detailed Implementation Plan

### Step 1: Create a New Validation Module

A new module, `archinstall_zfs/kernel/validation.py`, will house the core logic. It will contain three key functions.

#### A. `get_package_version(package_name)`

This helper function will query `pacman` for the version of a given package.

```python
# In archinstall_zfs/kernel/validation.py

import re
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand

def get_package_version(package_name: str) -> str | None:
    """Gets the version of a package from the pacman sync database."""
    try:
        # Use -Syi to get detailed info and refresh DB if needed
        output = SysCommand(f"pacman -Syi {package_name}").decode()
        match = re.search(r"Version\s+:\s+(.+)", output)
        if match:
            # Returns version string like "2.3.3-1" or "6.9.1.arch1-1"
            return match.group(1).strip()
    except SysCallError:
        return None
    return None
```

#### B. `fetch_zfs_kernel_compatibility(zfs_version)`

This function will use the GitHub API to get the compatibility range for a specific ZFS version.

```python
# In archinstall_zfs/kernel/validation.py

import json
from archinstall import debug, warn

def fetch_zfs_kernel_compatibility(zfs_version: str) -> tuple[str, str] | None:
    """
    Fetches OpenZFS release data from the GitHub API and parses the kernel compatibility range.
    """
    base_zfs_version = zfs_version.split('-')[0]  # Remove arch suffix
    tag_name = f"zfs-{base_zfs_version}"
    api_url = f"https://api.github.com/repos/openzfs/zfs/releases/tags/{tag_name}"
    
    debug(f"Fetching compatibility info from API: {api_url}")
    
    try:
        cmd = f'curl -sL -H "Accept: application/vnd.github.v3+json" "{api_url}"'
        api_response_str = SysCommand(cmd).decode()
        release_data = json.loads(api_response_str)
        
        release_body = release_data.get("body", "")
        if not release_body:
            return None
            
        match = re.search(r"Linux:.*?compatible with.*?([\d.]+)\s*-\s*([\d.]+)\s*kernels", release_body, re.IGNORECASE)
        
        if match:
            min_kernel, max_kernel = match.groups()
            debug(f"Found compatible kernel range via API: {min_kernel} - {max_kernel}")
            return min_kernel, max_kernel
            
    except (SysCallError, json.JSONDecodeError) as e:
        warn(f"Failed to get compatibility data for ZFS tag {tag_name}: {e}")
        return None
    
    return None
```

#### C. `get_compatible_kernel_variants(registry)`

This orchestrator function will use the helpers above to return lists of compatible and incompatible kernels.

```python
# In archinstall_zfs/kernel/validation.py

from packaging.version import parse as parse_version
from .registry import KernelRegistry, KernelVariant

def get_compatible_kernel_variants(registry: KernelRegistry) -> tuple[list[KernelVariant], list[KernelVariant]]:
    """
    Checks all registered kernel variants for DKMS compatibility.
    Returns a tuple of (compatible_variants, incompatible_variants).
    """
    zfs_pkg_ver = get_package_version("zfs-dkms")
    if not zfs_pkg_ver:
        warn("Could not determine zfs-dkms version. Assuming all kernels are compatible.")
        return registry.get_supported_variants(), []

    compatibility_range = fetch_zfs_kernel_compatibility(zfs_pkg_ver)
    if not compatibility_range:
        warn("Could not fetch ZFS compatibility. Assuming all kernels are compatible.")
        return registry.get_supported_variants(), []
        
    min_kernel_ver, max_kernel_ver = (parse_version(v) for v in compatibility_range)
    
    compatible, incompatible = [], []
    for variant in registry.get_supported_variants():
        kernel_pkg_ver = get_package_version(variant.kernel_package)
        if not kernel_pkg_ver:
            compatible.append(variant) # Fail open
            continue
        
        try:
            kernel_base_ver = parse_version(kernel_pkg_ver.split('-')[0])  # Remove arch suffix
            if min_kernel_ver <= kernel_base_ver <= max_kernel_ver:
                compatible.append(variant)
            else:
                incompatible.append(variant)
        except Exception:
            compatible.append(variant) # Fail open
            
    return compatible, incompatible
```

### Step 2: Integrate into the TUI Menu

Modify `_configure_kernels` in `archinstall_zfs/menu/global_config.py` to use the validation logic.

```python
# In archinstall_zfs/menu/global_config.py

# from archinstall_zfs.kernel.validation import get_compatible_kernel_variants

def _configure_kernels(self, *_: Any) -> None:
    registry = get_kernel_registry()
    
    # Run the compatibility check
    compatible_variants, incompatible_variants = get_compatible_kernel_variants(registry)
    
    # Prepare the informational header for the menu
    menu_header = "Select kernel and ZFS module mode"
    if incompatible_variants:
        incompatible_names = [v.display_name for v in incompatible_variants]
        warning_msg = (
            "\n\nNOTICE: The following kernels are temporarily unavailable for DKMS\n"
            "as they are not yet supported by the current ZFS version:\n"
            f"  - {', '.join(incompatible_names)}"
        )
        menu_header += warning_msg

    items = []
    # Generate menu items ONLY from the compatible list
    for variant in compatible_variants:
        # ... (logic to create MenuItem for precompiled and DKMS) ...
    
    if not items:
        # Handle edge case where no kernels are compatible
        # ...
        return

    # ... (rest of the function to show the menu) ...
```

## 5. User Experience (UX) Impact

### Scenario A: All Kernels Compatible

* **Behavior:** The validation check runs silently in the background. The kernel selection menu appears exactly as it does now, with all options available.
* **User Impact:** None. The experience is seamless.

### Scenario B: `linux-zen` Kernel is Incompatible

* **Behavior:** The validation check identifies that the latest `linux-zen` version is outside the supported range of the current `zfs-dkms` package.
* **User Impact:**
  1. The kernel selection menu appears.
  2. A clear, informational header is displayed at the top:
     > NOTICE: The following kernels are temporarily unavailable for DKMS
     > as they are not yet supported by the current ZFS version:
     >  - Linux Zen
  3. The menu items for "Linux Zen + ZFS DKMS" and "Linux Zen + precompiled ZFS" are **not present** in the list of choices.
  4. The user can only select from the remaining, guaranteed-to-be-compatible options (`linux-lts`, `linux`).

## 6. Acceptance Criteria

- [ ] A new module `kernel/validation.py` is created with the specified functions.
- [ ] The `_configure_kernels` menu function calls `get_compatible_kernel_variants` before building the menu.
- [ ] The menu correctly filters out incompatible kernel variants from the list of choices.
- [ ] If kernels are filtered, an informational message is displayed in the menu header.
- [ ] If the validation check fails for any reason (network, API, etc.), all kernel options are presented to the user ("Fail Open").
- [ ] Comprehensive test coverage for the validation logic.
- [ ] Documentation updates to explain the new proactive validation feature.
