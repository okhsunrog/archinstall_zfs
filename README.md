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
- **`testing_profile`**: Based on `baseline`, this is a streamlined development ISO optimized for QEMU testing with:
  - Auto-login as root (no username/password required)
  - Instant boot (0 timeout on all bootloaders)
  - Serial console support with kernel output
  - Passwordless SSH access on port 22

Both profiles automatically include the current archinstall_zfs source code at `/root/archinstall_zfs` during the build process, ensuring you always have the latest version available inside the ISO.

### Prerequisites

To build the ISOs, you'll need to be running Arch Linux and have the following packages installed:

```bash
sudo pacman -S qemu-desktop edk2-ovmf archiso grub just rsync
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

For development and testing, you should use the testing ISO which is optimized for automated workflows.

#### Quick Start

1.  **Set up the QEMU environment:**
    ```bash
    just qemu-setup
    ```
    This will create a disk image and UEFI variables file.

2.  **Build the testing ISO:**
    ```bash
    just build-testing-iso
    ```

3.  **Install and run in QEMU:**
    - **Recommended: Serial console mode** (headless, perfect for development):
      ```bash
      just qemu-install-serial
      ```
    - **GUI mode** (if you prefer a graphical interface):
      ```bash
      just qemu-install
      ```

4.  **Run an existing QEMU installation:**
    - **Serial console:**
      ```bash
      just qemu-run-serial
      ```
    - **GUI:**
      ```bash
      just qemu-run
      ```

#### Testing ISO Features

The testing profile provides a fully automated development experience:

- **🚀 Instant Boot**: Boots immediately without waiting (0 timeout on all bootloaders)
- **🔑 Auto-Login**: Automatically logs in as root - no username or password required
- **📟 Serial Console**: Full kernel output and console access via serial port
- **🌐 SSH Ready**: Passwordless SSH access on port 22

#### Accessing the System

Once booted, you have multiple ways to interact with the system:

1. **Serial Console** (when using `-serial` commands):
   - Direct console access in the terminal
   - All kernel messages visible
   - Auto-logged in as root

2. **SSH Access** (available in both modes):
   ```bash
   ssh root@localhost -p 2222
   ```
   No password required - connects immediately.

#### Development Workflow

For rapid development cycles:

```bash
# Clean previous builds
just clean-iso

# Build new testing ISO
just build-testing-iso

# Test with serial console (recommended)
just qemu-install-serial
```

The system will boot instantly and log you in automatically, ready for testing.

#### Using archinstall_zfs Inside the ISO

Both ISO profiles include the complete archinstall_zfs source code in `/root/archinstall_zfs`. The source is copied fresh from your working directory during each build using archiso's standard `airootfs` mechanism, ensuring it's always current. Once booted, you can:

```bash
# Install the package (recommended - one time setup)
./install-archinstall-zfs.sh

# Then run the installer
python -m archinstall_zfs

# Alternative: Run directly without installing
python archinstall_zfs/main.py

# Or examine the source code
ls -la archinstall_zfs/
```

This follows archiso best practices as documented in the [Arch Wiki](https://wiki.archlinux.org/title/Archiso), where the `airootfs` directory serves as the starting point for the live system's root filesystem. The source is automatically prepared during each build using dedicated justfile recipes, ensuring it stays synchronized with your development work without creating static copies that could become outdated.

### Available Commands

Here's a quick reference of all available `just` commands:

#### ISO Building
```bash
just build-main-iso      # Build production ISO (releng profile)
just build-testing-iso   # Build testing ISO (baseline profile)
just list-isos          # List available ISO files
just clean-iso          # Clean ISO build artifacts
```

#### QEMU Setup and Testing
```bash
just qemu-setup         # Create disk image and UEFI vars
just qemu-create-disk   # Create disk image only
just qemu-setup-uefi    # Setup UEFI vars only
just qemu-reset-uefi    # Reset UEFI vars to defaults

# Installation (boots from ISO)
just qemu-install       # Install with GUI
just qemu-install-serial # Install with serial console

# Run existing installation (boots from disk)
just qemu-run           # Run with GUI
just qemu-run-serial    # Run with serial console
```

#### Development
```bash
just format             # Format code with ruff
just lint               # Lint and auto-fix with ruff
just type-check         # Type check with mypy
just test               # Run tests with pytest
just all                # Run all quality checks
just clean              # Clean up cache and build artifacts
```

### Advanced Usage

The `gen_iso/run-qemu.sh` script is highly configurable via command-line options. You can run it directly for more advanced scenarios. Use `gen_iso/run-qemu.sh -h` to see all available options.

### Configuration Details

#### Network and SSH
- SSH is forwarded to port 2222 on the host: `ssh root@localhost -p 2222`
- No password required for root access (testing profile only)
- Network configured with DHCP automatically

#### Hardware Settings
- Default VM: 4GB RAM, 2 CPU cores
- Configurable via `justfile` variables or script parameters (`-m` for memory, `-p` for cores)
- KVM acceleration enabled automatically

#### Boot Configuration
- **Testing Profile**: Instant boot, auto-login, serial console support
- **Main Profile**: Standard timeout and login behavior
- UEFI boot enabled by default (BIOS boot available with `-b` flag)

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
