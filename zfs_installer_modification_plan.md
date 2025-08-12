# ZFSInstaller Modification Plan

## Overview

This document details the modifications needed to the `ZFSInstaller` class to work with the new initramfs handler architecture.

## Current Implementation Analysis

The current `ZFSInstaller` class:
1. Extends archinstall's `Installer` class
2. Customizes base packages to include "dracut"
3. Works with archinstall's `minimal_installation()` method
4. Currently doesn't have explicit initramfs handling

## Required Modifications

### 1. Constructor Changes

The constructor should be modified to:
- Accept an `InitramfsHandler` instance as a parameter
- Use the handler's `install_packages()` method to determine required packages
- Maintain compatibility with existing parameters

```python
def __init__(
    self,
    target: Path,
    disk_config: DiskLayoutConfiguration,
    initramfs_handler: InitramfsHandler,
    base_packages: list[str] | None = None,
    kernels: list[str] | None = None,
):
```

### 2. Package Installation Integration

Instead of hardcoding "dracut" in base packages:
- Remove "dracut" from hardcoded base packages
- Use `initramfs_handler.install_packages()` to get required packages
- Add these packages either to base packages or install them separately

### 3. Mkinitcpio Method Override

Override the `mkinitcpio()` method to delegate to the initramfs handler:
- Call `self.initramfs_handler.generate_initramfs(kernel)` for each kernel
- Maintain the same return type and error handling

### 4. Hook Setup Integration

Integrate the initramfs handler's hook setup:
- Call `self.initramfs_handler.setup_hooks()` at the appropriate time
- Ensure this happens after base system installation but before finalization

## Implementation Steps

### Step 1: Modify Constructor

```python
def __init__(
    self,
    target: Path,
    disk_config: DiskLayoutConfiguration,
    initramfs_handler: InitramfsHandler,
    base_packages: list[str] | None = None,
    kernels: list[str] | None = None,
):
    # Get packages from initramfs handler
    initramfs_packages = initramfs_handler.install_packages()
    
    # Define ZFS-specific base packages without initramfs tool
    if base_packages is None:
        base_packages = ["base", "base-devel", "linux-firmware", "linux-firmware-marvell", "sof-firmware"]
    
    # Add initramfs packages
    base_packages.extend(initramfs_packages)
    
    # Default to linux-lts for ZFS compatibility
    if kernels is None:
        kernels = ["linux-lts"]
        
    # Call parent constructor
    super().__init__(target=target, disk_config=disk_config, base_packages=base_packages, kernels=kernels)
    
    # Store initramfs handler
    self.initramfs_handler = initramfs_handler
    
    # Configure the initramfs handler
    self.initramfs_handler.configure()
```

### Step 2: Override Mkinitcpio Method

```python
def mkinitcpio(self, flags: list[str]) -> bool:
    """
    Override mkinitcpio to delegate to the initramfs handler.
    """
    try:
        # Generate initramfs for each kernel using the handler
        for kernel in self.kernels:
            if not self.initramfs_handler.generate_initramfs(kernel):
                return False
        return True
    except SysCallError as e:
        if e.worker_log:
            log(e.worker_log.decode())
        return False
```

### Step 3: Hook Setup Integration

Ensure hooks are set up at the appropriate time in the installation process, likely in the constructor after calling the parent constructor.

## Integration Points

### 1. With Main Installation Flow

The modified `ZFSInstaller` should:
- Work seamlessly with the existing installation flow in `main.py`
- Maintain all existing functionality except for improved initramfs handling
- Provide clear error messages if initramfs generation fails

### 2. With Archinstall's Workflow

The modifications should:
- Not break archinstall's existing functionality
- Properly integrate with archinstall's plugin/hooks system if needed
- Follow archinstall's error handling patterns

## Backward Compatibility

Consider maintaining backward compatibility by:
- Providing default initramfs handler if none is specified
- Keeping existing constructor parameters where possible
- Ensuring the class can still be used without the new initramfs handler system

## Testing Considerations

1. Verify constructor properly initializes with initramfs handler
2. Verify base packages are correctly assembled
3. Verify mkinitcpio override works correctly
4. Verify hook setup integration works
5. Verify compatibility with both Dracut and Mkinitcpio handlers
6. Verify error handling in failure scenarios