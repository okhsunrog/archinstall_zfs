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

## Building and Testing

The `gen_iso` directory contains all the necessary tools and profiles to build custom Arch Linux ISOs and test them with QEMU. The process is managed through `just` commands for clarity and ease of use.

### ISO Profiles

There are two ISO profiles available:
- **`main_profile`**: Based on `releng`, this is for building a production-ready ISO for installation on real hardware.
- **`testing_profile`**: Based on `baseline`, this is for building a development ISO for testing in QEMU.

### Prerequisites

To build the ISOs, you'll need to be running Arch Linux and have the following packages installed:

```bash
sudo pacman -S qemu-desktop edk2-ovmf archiso grub just
```

**Note on `grub`:** The `grub` package is required on the host system because `mkarchiso` may fail to create a bootable image without it.

### Building the ISOs

- **Build the main production ISO:**
  ```bash
  just build-main-iso
  ```

- **Build the testing ISO for development:**
  ```bash
  just build-testing-iso
  ```

The output ISOs will be placed in the `gen_iso/out` directory.

### Testing with QEMU

For development and testing, you should use the testing ISO.

1.  **Set up the QEMU environment:**
    ```bash
    just qemu-setup
    ```
    This will create a disk image and UEFI variables file.

2.  **Build the testing ISO:**
    ```bash
    just build-testing-iso
    ```

3.  **Install in QEMU:**
    - With a GUI:
      ```bash
      just qemu-install
      ```
    - With a serial console:
      ```bash
      just qemu-install-serial
      ```

4.  **Run an existing QEMU installation:**
    - With a GUI:
      ```bash
      just qemu-run
      ```
    - With a serial console:
      ```bash
      just qemu-run-serial
      ```

### Advanced Usage

The `gen_iso/run-qemu.sh` script is highly configurable via command-line options. You can run it directly for more advanced scenarios. Use `gen_iso/run-qemu.sh -h` to see all available options.

### Notes

- SSH is forwarded to port 2222 on the host (`ssh -p 2222 root@localhost`). The root password on the ISO is empty.
- The default VM has 4GB RAM and 2 CPU cores. These can be changed in the `justfile` or via script parameters.
- UEFI boot is enabled by default.

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
