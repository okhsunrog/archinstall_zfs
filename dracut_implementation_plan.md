# DracutInitramfsHandler Implementation Plan

## Overview

This document details the implementation of the `DracutInitramfsHandler` class, which will replace the current `DracutSetup` class and properly integrate with the new initramfs handler architecture.

## Current Implementation Analysis

The current `DracutSetup` class in `archinstall_zfs/initramfs/dracut.py` provides:

1. Configuration file creation (`/etc/dracut.conf.d/zfs.conf`)
2. Script creation for initramfs generation (`/usr/local/bin/dracut-install.sh`, `/usr/local/bin/dracut-remove.sh`)
3. Pacman hook creation for automatic initramfs regeneration
4. Directory setup

## Required Changes

### 1. Inherit from InitramfsHandler

The new `DracutInitramfsHandler` must inherit from the abstract `InitramfsHandler` base class and implement all required methods.

### 2. Method Implementation

#### `configure()` method
- Keep existing functionality for directory creation
- Keep existing functionality for dracut.conf creation
- Keep existing functionality for script creation
- Keep existing functionality for hook creation

#### `generate_initramfs(kernel: str)` method
- Replace direct SysCommand calls with proper error handling
- Generate initramfs for a specific kernel
- Return success/failure status

#### `install_packages()` method
- Return list of packages needed for dracut: `["dracut", "dracut-live"]`
- May include additional packages based on system configuration

#### `setup_hooks()` method
- Keep existing pacman hook creation functionality
- Ensure hooks are properly configured for the target system

### 3. Constructor Changes

The constructor should:
- Accept target path (from base class)
- Accept encryption_enabled parameter
- Initialize all required paths and attributes

### 4. Integration with Existing Code

The implementation should:
- Reuse existing logic from `DracutSetup` where appropriate
- Maintain compatibility with existing configuration options
- Ensure proper error handling and logging

## Implementation Steps

1. Create `DracutInitramfsHandler` class inheriting from `InitramfsHandler`
2. Implement constructor with proper parameter handling
3. Implement `configure()` method reusing existing logic
4. Implement `generate_initramfs()` method with proper error handling
5. Implement `install_packages()` method returning required packages
6. Implement `setup_hooks()` method reusing existing hook logic
7. Test integration with ZFSInstaller
8. Update documentation and comments

## Code Structure

```python
from pathlib import Path
from archinstall.lib.general import SysCommand
from .base import InitramfsHandler

class DracutInitramfsHandler(InitramfsHandler):
    def __init__(self, target: Path, encryption_enabled: bool = False):
        super().__init__(target)
        self.encryption_enabled = encryption_enabled
        self.scripts_dir = self.target / "usr/local/bin"
        self.hooks_dir = self.target / "etc/pacman.d/hooks"
        self.conf_dir = self.target / "etc/dracut.conf.d"
    
    def configure(self) -> None:
        # Implementation based on existing DracutSetup.configure()
        pass
    
    def generate_initramfs(self, kernel: str) -> bool:
        # Generate initramfs for specific kernel
        pass
    
    def install_packages(self) -> list[str]:
        # Return required packages
        return ["dracut"]
    
    def setup_hooks(self) -> None:
        # Implementation based on existing DracutSetup._create_hooks()
        pass
```

## Testing Considerations

1. Verify proper package installation
2. Verify configuration file creation
3. Verify script creation and permissions
4. Verify hook creation and functionality
5. Verify initramfs generation for different kernels
6. Verify error handling in failure scenarios