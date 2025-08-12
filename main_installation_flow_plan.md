# Main Installation Flow Update Plan

## Overview

This document details the modifications needed to update the main installation flow in `main.py` to work with the new initramfs handler architecture.

## Current Implementation Analysis

The current main installation flow in `main.py`:
1. Calls `ask_user_questions()` to get user configuration
2. Creates a separate `DracutSetup` instance and calls `configure()`
3. Creates `ZFSInstaller` with hardcoded packages including "dracut"
4. Calls `minimal_installation(mkinitcpio=False)` to disable archinstall's default mkinitcpio
5. Does not properly integrate initramfs generation with the installer

## Required Modifications

### 1. Modify ask_user_questions Function

The `ask_user_questions` function needs to:
- Return the menu instance or configuration
- Store the menu instance for later use

### 2. Update perform_installation Function

The `perform_installation` function needs to:
- Remove the separate Dracut configuration step
- Get the initramfs handler from the menu configuration
- Pass the handler to the ZFSInstaller constructor
- Ensure proper integration with the installation workflow

### 3. Update Main Function

The `main` function may need minor adjustments to support the new flow.

## Implementation Steps

### Step 1: Modify ask_user_questions Function

Update the function to return the menu instance:

```python
def ask_user_questions(arch_config: ArchConfig) -> ZFSInstallerMenu:
    """Ask user questions via ZFS installer menu and return the menu instance."""
    installer_menu = ZFSInstallerMenu(arch_config)
    installer_menu.run()
    return installer_menu
```

### Step 2: Update perform_installation Function Signature

Modify the function to accept the menu instance:

```python
def perform_installation(disk_manager: DiskManager, zfs_manager: ZFSManager, installer_menu: ZFSInstallerMenu) -> bool:
```

### Step 3: Remove Old Dracut Configuration

Remove this code from `perform_installation`:

```python
# REMOVE THIS CODE:
# Adding dracut configuration
dracut = DracutSetup(str(mountpoint), encryption_enabled=bool(zfs_manager.encryption_handler.password))
dracut.configure()
```

### Step 4: Create Initramfs Handler

Add code to create the initramfs handler:

```python
# Create initramfs handler from menu configuration
initramfs_handler = installer_menu.create_initramfs_handler(
    mountpoint, 
    bool(zfs_manager.encryption_handler.password)
)
```

### Step 5: Update ZFSInstaller Creation

Modify the ZFSInstaller creation to pass the initramfs handler:

```python
with ZFSInstaller(
    mountpoint, 
    disk_config=disk_cfg, 
    initramfs_handler=initramfs_handler
) as installation:
```

### Step 6: Ensure Hook Setup

Make sure the initramfs handler's hooks are properly set up, either:
- Through the ZFSInstaller's integration with the handler
- Or by explicitly calling setup_hooks() at the appropriate time

## Detailed Code Changes

### In main.py:

1. **Update ask_user_questions function**:
   ```python
   def ask_user_questions(arch_config: ArchConfig) -> ZFSInstallerMenu:
       """Ask user questions via ZFS installer menu."""
       installer_menu = ZFSInstallerMenu(arch_config)
       installer_menu.run()
       return installer_menu
   ```

2. **Update perform_installation function signature**:
   ```python
   def perform_installation(disk_manager: DiskManager, zfs_manager: ZFSManager, installer_menu: ZFSInstallerMenu) -> bool:
   ```

3. **Remove old Dracut configuration**:
   ```python
   # REMOVE:
   # dracut = DracutSetup(str(mountpoint), encryption_enabled=bool(zfs_manager.encryption_handler.password))
   # dracut.configure()
   ```

4. **Add initramfs handler creation**:
   ```python
   # Create initramfs handler from menu configuration
   initramfs_handler = installer_menu.create_initramfs_handler(
       mountpoint, 
       bool(zfs_manager.encryption_handler.password)
   )
   ```

5. **Update ZFSInstaller creation**:
   ```python
   with ZFSInstaller(
       mountpoint, 
       disk_config=disk_cfg, 
       initramfs_handler=initramfs_handler
   ) as installation:
   ```

6. **Update main function**:
   ```python
   def main() -> bool:
       # ... existing code ...
       
       # Get menu instance from ask_user_questions
       installer_menu = ask_user_questions(arch_config)
       
       # ... existing code ...
       
       # Pass menu instance to perform_installation
       success = perform_installation(disk_manager, zfs_manager, installer_menu)
       
       # ... existing code ...
   ```

## Integration Points

### 1. With ZFSInstaller

The main flow should properly integrate with the updated ZFSInstaller:
- Pass the initramfs handler to the constructor
- Let the installer handle initramfs generation through its mkinitcpio override
- Ensure hooks are set up at the right time

### 2. With Menu System

The main flow should properly integrate with the menu system:
- Get configuration from the menu instance
- Create initramfs handlers based on user selections
- Handle any errors in the process

### 3. With Archinstall

The main flow should maintain compatibility with archinstall:
- Not break existing archinstall functionality
- Properly use archinstall's APIs and workflows
- Follow archinstall's error handling patterns

## Error Handling

The updated flow should handle:

1. **Menu Errors**: If the menu fails to run or return valid configuration
2. **Handler Creation Errors**: If initramfs handlers cannot be created
3. **Installation Errors**: If the installation process fails
4. **Integration Errors**: If components don't work together properly

## Backward Compatibility

Consider maintaining backward compatibility by:

1. Keeping existing function signatures where possible with defaults
2. Ensuring the code can still run without the new initramfs handler system
3. Providing fallbacks for error conditions

## Testing Considerations

1. Verify the menu configuration is properly passed to the main flow
2. Verify the correct initramfs handler is created based on user selection
3. Verify the handler is properly passed to ZFSInstaller
4. Verify initramfs generation works correctly during installation
5. Verify both Dracut and Mkinitcpio options work in the full flow
6. Verify error handling in failure scenarios
7. Verify integration with archinstall's existing features