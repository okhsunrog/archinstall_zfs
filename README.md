# Archinstall-ZFS

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

3. Development Infrastructure
   - Implement comprehensive linting
   - Add CI/CD pipeline
   - Improve code quality checks
   - Enhance error handling

4. Additional Features
   - Expand post-installation customization options
   - Add more ZFS optimization options (configurable compression, DirectIO, etc.)
   - Install and configure zrepl for backup and replication

## Support

For issues, questions, or suggestions, please open an issue on our repository. I'm committed to maintaining and improving this installer for the Arch Linux community.