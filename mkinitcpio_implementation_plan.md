# MkinitcpioInitramfsHandler Implementation Plan

## Overview

This document details the implementation of the `MkinitcpioInitramfsHandler` class, which will provide mkinitcpio support as an alternative to dracut in the ZFS installer.

## Requirements

The `MkinitcpioInitramfsHandler` must:
1. Inherit from the abstract `InitramfsHandler` base class
2. Implement all required abstract methods
3. Provide ZFS-specific configuration for mkinitcpio
4. Handle encryption scenarios properly
5. Integrate with archinstall's existing mkinitcpio workflow

## Implementation Details

### 1. Class Structure

```python
from pathlib import Path
from .base import InitramfsHandler

class MkinitcpioInitramfsHandler(InitramfsHandler):
    def __init__(self, target: Path, encryption_enabled: bool = False):
        super().__init__(target)
        self.encryption_enabled = encryption_enabled
        self.zfs_modules = ["zfs"]
```

### 2. Method Implementation

#### `configure()` method
- Modify `/etc/mkinitcpio.conf` to include ZFS modules
- Add ZFS-specific hooks if needed
- Configure encryption hooks if encryption is enabled

#### `generate_initramfs(kernel: str)` method
- Use archinstall's existing mkinitcpio functionality
- Or directly call mkinitcpio with appropriate parameters
- Handle errors appropriately

#### `install_packages()` method
- Return list of packages needed for mkinitcpio: `["mkinitcpio"]`
- Include additional packages if needed for ZFS support

#### `setup_hooks()` method
- Leverage archinstall's existing hook system
- Or create custom hooks if needed for ZFS

### 3. ZFS-Specific Configuration

For ZFS support, mkinitcpio needs:
1. **Modules**: Add "zfs" to the MODULES array
2. **Hooks**: May need custom hooks for ZFS import
3. **Files**: Include encryption key files if encryption is enabled
4. **Binaries**: Include ZFS binaries if needed

### 4. Encryption Support

When encryption is enabled:
1. Add encryption hooks (encrypt, or systemd-based hooks)
2. Include key files in the initramfs
3. Configure proper kernel parameters

## Implementation Steps

1. Create `MkinitcpioInitramfsHandler` class inheriting from `InitramfsHandler`
2. Implement constructor with proper parameter handling
3. Implement `configure()` method to modify mkinitcpio.conf
4. Implement `generate_initramfs()` method
5. Implement `install_packages()` method
6. Implement `setup_hooks()` method
7. Test integration with ZFSInstaller
8. Update documentation and comments

## Code Structure

```python
from pathlib import Path
from .base import InitramfsHandler

class MkinitcpioInitramfsHandler(InitramfsHandler):
    def __init__(self, target: Path, encryption_enabled: bool = False):
        super().__init__(target)
        self.encryption_enabled = encryption_enabled
        self.zfs_modules = ["zfs"]
    
    def configure(self) -> None:
        # Modify /etc/mkinitcpio.conf to include ZFS modules and hooks
        pass
    
    def generate_initramfs(self, kernel: str) -> bool:
        # Generate initramfs using mkinitcpio
        pass
    
    def install_packages(self) -> list[str]:
        # Return required packages
        return ["mkinitcpio"]
    
    def setup_hooks(self) -> None:
        # Set up any custom hooks needed
        pass
```

## Integration with Archinstall

The implementation should:
1. Work with archinstall's existing mkinitcpio workflow
2. Not conflict with other archinstall features
3. Properly handle errors and edge cases
4. Follow archinstall's coding conventions

## Testing Considerations

1. Verify proper modification of mkinitcpio.conf
2. Verify ZFS modules are included
3. Verify initramfs generation for different kernels
4. Verify encryption support when enabled
5. Verify compatibility with archinstall's existing features
6. Verify error handling in failure scenarios