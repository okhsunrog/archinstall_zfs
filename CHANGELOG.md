# Changelog

User-visible changes to archinstall_zfs, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The monthly ISO builds repackage the
latest release with fresh packages and ZFS modules, so they are not listed here.

## [Unreleased]

### Changed

- The graphical installer takes the screen and keyboard through libseat and
  logind. Start it from a console login rather than over SSH; on the official Arch
  ISO, install `seatd` along with its other runtime dependencies.

### Fixed

- Killing the graphical installer together with its supervisor left the console
  keyboard switched off; logind now restores it.

## [0.5.1] - 2026-09-23

### Changed

- Tap-to-click and natural scrolling are turned on for touchpads in the live
  installer; mouse wheels keep the traditional direction. The
  `SLINT_LIBINPUT_TAP_TO_CLICK` and `SLINT_LIBINPUT_ACCEL_SPEED` variables are no
  longer read.
- CachyOS installs include `cachyos-hooks`, so the installed system calls itself
  CachyOS rather than Arch Linux in `/etc/os-release`.

### Fixed

- Installing a new boot environment into a pool that already had one: the first
  boot mounted the other environment's `/home`, so logging in failed and COSMIC
  never started. Only the new environment's datasets are now handed to the target.
- `/root` failed to mount on every boot of a system with flatpak, such as a COSMIC
  install, because flatpak's environment generator created `/root/.cache` first.
- USB keyboards, including wireless receivers that need a vendor driver, did not
  work in ZFSBootMenu: dracut 111 left every HID driver out of the image.
- "Choose an existing pool" and Inspect ZFS failed with "failed to parse zpool list
  output" while no pool was imported, which is the usual state of the live ISO.
- The Import read-only button in Inspect ZFS ran past the edge of the dialog.
- CachyOS installs no longer warn that `linux-cachyos-zfs` is missing from the
  repositories; the check can only see the live system's Arch repositories.

## [0.5.0] - 2026-09-16

Five months of work. The full release notes, with the reasoning behind each
change, are in the [GitHub release](https://github.com/okhsunrog/archinstall_zfs/releases/tag/v0.5.0).

### Changed

- Saved configurations no longer contain passwords. They go to a companion
  `*.secrets.json`; pass `--secrets` for an unattended install. Configurations with
  inline secrets are still read.
- The configuration fields `*_by_id` are now `disk`, `efi_partition`,
  `zfs_partition` and `swap_partition`. The old names are still accepted.
- A redacted copy of the configuration is written into the installed system at
  `/etc/archinstall-zfs/installation.json`.
- NetworkManager is installed and enabled unless the live medium's networkd and iwd
  setup is copied across.
- The KDE profile installs the plasma group. The Hyprland profile installs
  hyprlauncher, waybar and hyprpolkitagent instead of wofi and polkit-kde-agent.
- Seat access defaults to polkit instead of seatd.
- Parallel downloads default to 5 rather than 10, and mirrors are ranked by the
  installer itself instead of reflector.
- The ESP is mounted, and written into fstab, readable by root only.
- The terminal interface's `--dry-run` is now `--demo`, and it does not install.
- CachyOS installs require a processor supporting x86-64-v3 or better.

### Added

- Alongside mode: install next to an existing system by taking unallocated space or
  shrinking an NTFS or ext4 partition, with the allocation shown to scale first.
- The Disk step shows each disk's partitions and contents and suggests a mode.
- CachyOS as a target distribution, with repositories chosen by processor level.
- An interrupted installation can be continued from the welcome screen.
- A shell inside the installed system from the completion screen.
- Saving and loading a configuration from the graphical review screen.
- A safe demo mode and ISO boot entry that never installs.
- Updating the live installer from a Ventoy stick without rebuilding the ISO.
- A manual walkthrough of the whole installation in `docs/zfs-root-install-guide.md`.

### Fixed

- Encrypted installs stopped with "encryption key not loaded", or completed and
  could not be unlocked at boot.
- Package groups, renamed packages and oversized transactions failed part way
  through; names are now checked before any disk change.
- ZRAM swap and swap partitions produced a system without swap.
- Only one configured kernel got an initramfs and a ZFS module.
- A KDE install could finish without its display manager, and a desktop without a
  running network service.
- Cancelling waited for dkms, dracut or ZFSBootMenu builds to finish.
- A stray directory or pool from an earlier attempt failed the install after the
  disk had been changed.
- Wi-Fi scans and connections on adapters that answer iwd badly, such as the
  Realtek RTL8723BE.
- VirtIO disks were not listed.
- Several ZED cache hook defects that could mount another boot environment's
  datasets.

## [0.4.1] - 2026-04-08

### Changed

- Kernel selection detects whether a precompiled ZFS module or DKMS applies.
- Profiles were redesigned, with follow-up questions in the graphical installer.
- One package search for repository and AUR packages, with fuzzy matching.

## [0.4.0] - 2026-04-05

A complete rewrite in Rust. The Python version is preserved on the
[`old_python`](https://github.com/okhsunrog/archinstall_zfs/tree/old_python) branch.

### Changed

- No dependency on archinstall or Python: a single binary that drives libalpm
  directly, with parallel downloads and native AUR support.
- The binaries are `archinstall-zfs-tui` and `archinstall-zfs-slint`.

### Added

- A graphical installer that renders directly through Linux KMS, with no X11 or
  Wayland on the live ISO.
- Cancelling an installation, and a welcome screen that checks the network, UEFI
  and ZFS first.

## [0.3.5] - 2025-09-02

### Fixed

- User services are enabled, profile post-install steps run, and hardware key
  authentication is set up.

## [0.3.4] - 2025-08-30

### Added

- zrepl and AUR package support.

## [0.3.3] - 2025-08-23

### Added

- A ZFS compression option.

## [0.3.2] - 2025-08-20

### Fixed

- The ZFSBootMenu command line carried a shell substitution instead of the hostid.

## [0.3.1] - 2025-08-15

### Fixed

- Kernel and ZFS compatibility is respected and validated before installing.

## [0.3.0] - 2025-08-15

### Changed

- Kernel and ZFS module management was reworked, and installing on an existing pool
  got a clearer interface.
- reflector is stopped before pacman is configured.

## [0.2.2] - 2025-08-14

### Changed

- The installer is found in the same place on every medium.

## [0.2.1] - 2025-08-14

### Fixed

- Mirror handling on the live system is more robust.

## [0.2.0] - 2025-08-14

### Changed

- ZFS is set up on the live system itself, preferring a precompiled module and
  falling back to DKMS, with the repositories pinned to a matching Arch Archive
  snapshot while it builds.

## [0.1.0] - 2025-08-13

The first release: a ZFS-on-root installer built on archinstall.

[Unreleased]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.3.5...v0.4.0
[0.3.5]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/okhsunrog/archinstall_zfs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/okhsunrog/archinstall_zfs/tree/v0.1.0
