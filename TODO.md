# Archinstall-ZFS — Unified TODO and Refactor Plan

## Goals

- Introduce a clean initramfs handler abstraction to support both Dracut and Mkinitcpio in a maintainable way
- Remove ad-hoc Dracut configuration in the main flow; route all initramfs work through a handler
- Let the user choose ZFS kernel module packaging strategy:
  - Use precompiled `zfs-linux-lts` (with automatic fallback to `zfs-dkms` if installation fails)
  - Use `zfs-dkms` directly
- Keep ZFS native encryption handling as-is (mkinitcpio does not need special encryption hooks)

## Architecture

### Initramfs Handlers

- Create abstract `InitramfsHandler` with methods:
  - `configure() -> None`
  - `generate_initramfs(kernel: str) -> bool`
  - `install_packages() -> list[str]`
  - `setup_hooks() -> None`

- Implement concrete handlers:
  - `DracutInitramfsHandler` (replaces current `DracutSetup` logic)
    - Reuse current config/scripts/hooks creation
    - `install_packages() -> ["dracut"]` (and optionally `dracut-live` in the future)
    - `generate_initramfs(kernel)`: call dracut to build initramfs
  - `MkinitcpioInitramfsHandler`
    - Configure ZFS support via `MODULES+=(zfs)` (no extra encryption hooks)
    - `install_packages() -> ["mkinitcpio"]`
    - `generate_initramfs(kernel)`: run `mkinitcpio -P` in chroot

### Installer Integration

- Update `ZFSInstaller`:
  - Accept `initramfs_handler: InitramfsHandler`
  - Merge `initramfs_handler.install_packages()` into base packages
  - Remove hardcoded `"dracut"` from default base packages
  - Call `initramfs_handler.configure()` and `initramfs_handler.setup_hooks()`
  - Override `mkinitcpio(self, flags: list[str]) -> bool` to delegate to `initramfs_handler.generate_initramfs()` per kernel

### Menu Integration

- Extend `ZFSInstallerMenu`:
  - Add enum `ZFSModuleMode { PRECOMPILED, DKMS }`
  - Add menu item "ZFS Modules Source" with two choices
  - Add `create_initramfs_handler(target: Path, encryption_enabled: bool) -> InitramfsHandler`
  - Expose selection via `get_zfs_config()`

### Main Flow Changes

- `ask_user_questions(arch_config) -> ZFSInstallerMenu` (return menu instance)
- `perform_installation(disk_manager, zfs_manager, installer_menu)` (accept menu)
- Remove direct use of `DracutSetup`
- Build handler via `installer_menu.create_initramfs_handler(mountpoint, encryption_enabled)` and pass it to `ZFSInstaller`
- After adding ArchZFS repo on target, install ZFS packages according to user choice:
  - If `PRECOMPILED`: try `zfs-linux-lts zfs-utils`, on failure fall back to `zfs-dkms zfs-utils linux-lts-headers`
  - If `DKMS`: install `zfs-dkms zfs-utils linux-lts-headers`

## Non-Goals / Clarifications

- No mkinitcpio encryption hook configuration is needed (ZFS native encryption is handled by the ZFS module)
- Keep `initialize_zfs()` flow for the live ISO as-is

## Implementation Tasks

1) Implement `archinstall_zfs/initramfs/base.py`
2) Implement `DracutInitramfsHandler` in `archinstall_zfs/initramfs/dracut.py`
3) Implement `MkinitcpioInitramfsHandler` in `archinstall_zfs/initramfs/mkinitcpio.py`
4) Update `archinstall_zfs/menu/zfs_installer_menu.py`:
   - Add `ZFSModuleMode` enum and menu entry
   - Add `create_initramfs_handler()` and include module mode in `get_zfs_config()`
5) Update `archinstall_zfs/installer.py` to accept and use `initramfs_handler`
6) Update `archinstall_zfs/main.py` to return menu from `ask_user_questions`, accept it in `perform_installation`, remove direct Dracut config, create handler, and wire ZFS package choice + fallback
7) Basic tests and lint pass

## Testing Checklist

- Handlers create expected files/dirs under target
- `install_packages()` returns correct lists
- `ZFSInstaller` merges handler packages and overrides `mkinitcpio()`
- Main flow installs ZFS packages according to selection with fallback


