# Archinstall-ZFS

# WARNING: This is work-in-progress and doesn't work on the latest commit for now

A ZFS-focused Arch Linux installer built on top of archinstall. This installer provides a streamlined way to set up Arch Linux with ZFS as the root filesystem, featuring automatic ZFSBootMenu configuration.

## Features

- Multiple installation modes:
  - Full disk installation with automatic partitioning
  - New ZFS pool creation on existing partition
  - Installation alongside existing distros on existing ZFS pool
- Automatic ZFSBootMenu setup with recovery options
- Native ZFS encryption support (pool-wide or dataset-specific)
- Seamless integration with archinstall's profile system
- User-friendly TUI interface
- Comprehensive error handling and logging

## Installation

To run the installer:

1. Boot into the Arch Linux live environment
2. Clone the repository
3. Navigate to the project directory
4. Run:
   ```python
    python -m archinstall_zfs
    ```

## Testing with QEMU

For testing purposes, you can run the installer in QEMU using the provided scripts in the `qemu_scripts` directory:

- `qemu_arch_install.sh` - Boot from ISO to install Arch with GUI
- `qemu_arch_install_serial.sh` - Boot from ISO to install Arch with serial console
- `qemu_arch_run.sh` - Boot from existing disk image with GUI
- `qemu_arch_run_serial.sh` - Boot from existing disk image with serial console

### Prerequisites

1. Install QEMU and OVMF firmware:
   - On Arch Linux: `pacman -S qemu-desktop edk2-ovmf`
   - On Ubuntu/Debian: `apt install qemu-system-x86 ovmf`

2. Download an Arch Linux ISO to `~/tmp_zfs/archiso.iso` (or pass as first argument)

3. Create a disk image in the qemu_scripts directory: `qemu-img create -f qcow2 qemu_scripts/arch.qcow2 20G`

4. Create UEFI variables file in the qemu_scripts directory:
   ```bash
   # On Arch Linux:
   cp /usr/share/edk2-ovmf/x64/OVMF_VARS.4m.fd qemu_scripts/my_vars.fd
   
   # On Ubuntu/Debian:
   cp /usr/share/OVMF/OVMF_VARS.fd qemu_scripts/my_vars.fd
   ```

### Understanding UEFI Files

The QEMU scripts use UEFI boot mode, which requires two files:

- **OVMF_CODE.4m.fd**: The UEFI firmware code (read-only)
  - Contains the actual UEFI implementation
  - Provided by the OVMF package at `/usr/share/edk2-ovmf/x64/OVMF_CODE.4m.fd`
  - Used directly from the system installation

- **my_vars.fd**: UEFI variables storage (read-write)
  - Stores UEFI settings, boot entries, and secure boot keys
  - Must be a writable copy of the OVMF_VARS template
  - Created by copying from the system's OVMF_VARS file
  - Gets modified during VM operation to persist UEFI settings

### Usage

```bash
# Navigate to qemu_scripts directory
cd qemu_scripts

# Make scripts executable
chmod +x *.sh

# Run installer with GUI
./qemu_arch_install.sh

# Run installer with serial console
./qemu_arch_install_serial.sh

# Run existing installation with GUI
./qemu_arch_run.sh

# Run existing installation with serial console
./qemu_arch_run_serial.sh
```

### Notes

- SSH is forwarded to port 2222 on host (`ssh -p 2222 user@localhost`)
- The scripts allocate 4GB RAM and 2 CPU cores to the VM
- UEFI boot is enabled with secure boot support

## Why Choose This Installer?

- Simplifies the complex process of setting up ZFS on Arch Linux
- Integrates seamlessly with archinstall's existing functionality
- Provides secure encryption options out of the box
- Handles all ZFS-specific configuration automatically
- Uses ZFSBootMenu for flexible boot management

## Contributing

I welcome contributions! Whether it's bug fixes, feature additions, or documentation improvements, your help is appreciated.

1. Fork the repository
2. Create your feature branch
3. Submit a pull request

This project is licensed under the GNU General Public License v3.0. See the LICENSE file for details.

## TODO

1. Configuration Improvements
   - Integrate with archinstall's global menu system
   - Implement unified configuration handling
   - Add support for custom dataset configurations via JSON
   - Add menu cancellation options

2. System Enhancements
   - Evaluate replacing mkinitcpio with dracut
   - Implement smarter hostid generation, based on hostname
   - Add local ZFSBootMenu building support
   - Add swap configuration options (zswap, zram, swap patition)

3. Additional Features
   - Expand post-installation customization options
   - Add more ZFS optimization options (configurable compression, DirectIO, etc.)
   - Install and configure zrepl for backup and replication

## Support

For issues, questions, or suggestions, please open an issue on our repository. I'm committed to maintaining and improving this installer for the Arch Linux community.
