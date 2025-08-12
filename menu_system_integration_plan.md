# Menu System Integration Plan

## Overview

This document details the modifications needed to integrate the new initramfs handler architecture with the menu system.

## Current Implementation Analysis

The current `ZFSInstallerMenu` class:
1. Has an `InitSystem` enum with `DRACUT` and `MKINITCPIO` values
2. Stores the selected init system in `self.init_system`
3. Has a `get_zfs_config()` method that returns the init system choice
4. Does not currently create or return initramfs handler instances

## Required Modifications

### 1. Add Method to Create Initramfs Handlers

Add a method to create the appropriate initramfs handler based on user selection:

```python
def create_initramfs_handler(self, target: Path, encryption_enabled: bool = False) -> InitramfsHandler:
    """Create an initramfs handler based on user selection."""
    if self.init_system == InitSystem.DRACUT:
        from archinstall_zfs.initramfs.dracut import DracutInitramfsHandler
        return DracutInitramfsHandler(target, encryption_enabled)
    else:  # MKINITCPIO
        from archinstall_zfs.initramfs.mkinitcpio import MkinitcpioInitramfsHandler
        return MkinitcpioInitramfsHandler(target, encryption_enabled)
```

### 2. Update Configuration Return Method

Modify or extend the `get_zfs_config()` method to include information needed to create initramfs handlers:

```python
def get_zfs_config(self) -> dict[str, Any]:
    """Get ZFS-specific configuration."""
    return {
        "dataset_prefix": self.dataset_prefix,
        "init_system": self.init_system,
        "encryption_mode": self.zfs_encryption_mode,
        "encryption_password": self.zfs_encryption_password,
    }
```

## Integration with Main Installation Flow

### 1. Pass Configuration to Main Flow

The main installation flow in `main.py` needs to:
1. Get the initramfs choice from the menu
2. Create the appropriate initramfs handler
3. Pass it to the ZFSInstaller constructor

### 2. Modify perform_installation Function

Update the `perform_installation` function in `main.py`:

```python
def perform_installation(disk_manager: DiskManager, zfs_manager: ZFSManager) -> bool:
    # ... existing code ...
    
    # Get initramfs handler from menu configuration
    # This requires passing the menu config to this function or accessing it differently
    
    # Create initramfs handler
    initramfs_handler = menu_config.create_initramfs_handler(
        zfs_manager.config.mountpoint, 
        bool(zfs_manager.encryption_handler.password)
    )
    
    # ... existing code ...
    
    # Pass initramfs handler to ZFSInstaller
    with ZFSInstaller(
        mountpoint, 
        disk_config=disk_cfg, 
        initramfs_handler=initramfs_handler
    ) as installation:
        # ... rest of installation ...
```

## Implementation Steps

### Step 1: Add Handler Creation Method

Add the `create_initramfs_handler` method to `ZFSInstallerMenu`:

```python
def create_initramfs_handler(self, target: Path, encryption_enabled: bool = False) -> InitramfsHandler:
    """Create an initramfs handler based on user selection."""
    if self.init_system == InitSystem.DRACUT:
        from archinstall_zfs.initramfs.dracut import DracutInitramfsHandler
        return DracutInitramfsHandler(target, encryption_enabled)
    else:  # MKINITCPIO
        from archinstall_zfs.initramfs.mkinitcpio import MkinitcpioInitramfsHandler
        return MkinitcpioInitramfsHandler(target, encryption_enabled)
```

### Step 2: Update Main Installation Flow

Modify the main installation flow to use the new method:

1. In `ask_user_questions` function, store the menu instance
2. In `perform_installation` function, use the menu to create the handler
3. Pass the handler to ZFSInstaller

### Step 3: Remove Old Dracut Configuration

Remove the separate Dracut configuration step that currently exists in `perform_installation`:

```python
# REMOVE THIS CODE:
# Adding dracut configuration
dracut = DracutSetup(str(mountpoint), encryption_enabled=bool(zfs_manager.encryption_handler.password))
dracut.configure()
```

## Error Handling

The integration should handle:

1. **Import Errors**: If initramfs handler modules cannot be imported
2. **Initialization Errors**: If handlers cannot be created
3. **Configuration Errors**: If user selections are invalid
4. **Runtime Errors**: If handler methods fail during execution

## Backward Compatibility

Consider maintaining backward compatibility by:

1. Providing default values for new parameters
2. Keeping existing method signatures where possible
3. Ensuring the menu can still be used without the new initramfs handler system

## Testing Considerations

1. Verify the menu correctly stores initramfs selection
2. Verify the correct handler is created based on selection
3. Verify encryption parameters are properly passed to handlers
4. Verify integration with main installation flow
5. Verify error handling in failure scenarios
6. Verify both Dracut and Mkinitcpio options work correctly