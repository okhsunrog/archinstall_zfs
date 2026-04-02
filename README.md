<h1 align="center">archinstall-zfs-rs</h1>

> ZFS-first Arch Linux installer with ZFSBootMenu — rewritten in Rust.

[![GitHub](https://img.shields.io/github/license/okhsunrog/archinstall-zfs-rs)](https://github.com/okhsunrog/archinstall-zfs-rs/blob/main/LICENSE)

---

## Overview

Setting up ZFS on Arch involves kernel selection, ZFS module installation, bootloader configuration, and optional encryption. archinstall-zfs-rs automates these steps and provides both a ratatui TUI and an experimental Slint GUI. It uses direct libalpm bindings for package management (no `pacman`/`pacstrap` shell calls), resolves AUR dependency chains via `raur`/`aur-depends`, and validates kernel/ZFS compatibility against OpenZFS release data.

This is a Rust rewrite of [archinstall_zfs](https://github.com/okhsunrog/archinstall_zfs). Key improvements over the Python version:

- **No archinstall dependency** — fully standalone, no dependency on the official Arch installer framework or its Python ecosystem
- **Single binary** with no Python/pip/venv dependencies
- **Direct libalpm** for all package management with per-package progress callbacks
- **No external package manager binaries** needed at runtime (no `pacman`, `pacstrap`, `yay`)
- **Proper AUR dependency resolution** via `raur` + `aur-depends` crates
- **Trace-level file logging** (`/tmp/archinstall-zfs.log`) for post-mortem analysis

---

## Quick start

### Option A: Prebuilt ISO (recommended)
1. Download the latest ISO from the releases page.
2. Boot on a UEFI machine and connect to the network.
3. Run:

```bash
archinstall-zfs
```

### Option B: Official Arch ISO
```bash
# Boot the official Arch ISO and connect to the network
# Download the binary from releases, or build from source:
pacman -Sy git base-devel
git clone --depth 1 https://github.com/okhsunrog/archinstall-zfs-rs
cd archinstall-zfs-rs
cargo build --release -p archinstall-zfs-tui
./target/release/archinstall-zfs-tui
```

### Silent mode (for automation)
```bash
archinstall-zfs --config config.json --silent
```

---

## Features

### Installation modes

| Mode | Description | Best for |
|------|-------------|----------|
| **Full Disk** | Complete disk takeover with automated partitioning. Creates EFI (500MB), optional swap, remainder for ZFS | Clean installs, single-purpose machines |
| **New Pool** | Creates ZFS pool on an existing partition. Uses your existing partition layout | Dual-boot scenarios, custom partitioning |
| **Existing Pool** | Installs into an existing ZFS pool as a new boot environment | Multiple Arch installations, experiments |

### Kernel support
- `linux-lts` + `zfs-linux-lts`
- `linux` + `zfs-linux`
- `linux-zen` + `zfs-linux-zen`
- `linux-hardened` + `zfs-linux-hardened`

All kernel options automatically fall back to `zfs-dkms` if precompiled modules are unavailable.

### Kernel/ZFS compatibility validation

The installer validates kernel/ZFS compatibility by:

- **Parsing OpenZFS releases**: Fetches supported kernel version ranges from GitHub API
- **Checking precompiled availability**: Verifies version match between kernel and precompiled ZFS packages
- **DKMS range validation**: Ensures kernel is within the supported range for zfs-dkms
- **archzfs.db fallback**: Downloads the archzfs package database directly when the repo isn't configured locally

The validation runs in two places:
1. **In the TUI/GUI** — shows compatibility status (`[OK]`/`[INCOMPATIBLE]`) next to each kernel
2. **Before installation** — warns about potential issues in Phase 0

### Encryption options
- Pool-wide encryption (all datasets inherit)
- Per-boot environment encryption (encrypts the base dataset)
- No encryption

### Swap and memory management

**ZRAM (recommended)**: Compressed swap in RAM via `systemd-zram-generator`. Default size: `min(ram / 2, 4096)` MB. zswap is disabled to avoid double compression.

**Swap partition**: Dedicated partition with zswap enabled. Supports encryption via `cryptswap` in `/etc/crypttab`.

**No swap**: Pure RAM-only operation.

> Swap on ZFS (zvol/swapfile) is not supported due to potential deadlock issues.

### Boot environments and layout

The installer creates a consistent dataset layout for each boot environment:

```
pool/prefix/root       → /          (root filesystem, canmount=noauto)
pool/prefix/data/home  → /home      (user data)
pool/prefix/data/root  → /root      (root user data)
pool/prefix/vm         → /vm        (virtual machines)
```

ZFSBootMenu is built locally via `generate-zbm` (AUR package) with a pacman hook for automatic regeneration on kernel updates.

A custom ZED hook (`history_event-zfs-list-cacher.sh`) ensures only the current boot environment's datasets are mounted, preventing cross-environment mount conflicts.

### AUR packages
- Resolves AUR-to-AUR dependency chains via `raur` + `aur-depends`
- Builds using a temporary user (`aurinstall`) with temporary passwordless sudo
- Cleans up temp user and build artifacts after installation

### Snapshot management (optional)
- zrepl support for automatic snapshot creation and retention
- Schedules: 15-minute intervals with tiered retention (4x15m, 24x1h, 3x1d)

---

## Architecture

```
archinstall-zfs-rs/
  core/       # Library crate — all installation logic, config, validation
  tui/        # ratatui-based terminal UI
  slint-ui/   # Experimental Slint GUI (Linux KMS backend)
  xtask/      # Development tasks (QEMU testing)
  gen_iso/    # ISO building templates and scripts
```

### Package management

All package installation uses direct libalpm bindings (`alpm` crate):

- `AlpmContext::for_host()` — installs packages on the live ISO
- `AlpmContext::for_target()` — installs packages into the target chroot
- `TargetMounts` — manages API filesystem mounts (proc/sys/dev) matching pacstrap's `chroot_setup()`
- Progress/download/event/log callbacks wired to `tracing`

The only remaining shell calls are:
- `pacman-key` for GPG keyring operations (no libalpm equivalent)
- `makepkg` for building AUR packages (bash script by design)
- `arch-chroot` for non-package chroot commands

---

## Development

### Prerequisites
- Arch Linux (for libalpm headers)
- Rust 2024 edition
- `just` (task runner)

### Common commands
```bash
just build          # Build release binaries
just test           # Run cargo tests
just lint           # Run clippy
just fmt            # Format code

just build-test     # Build testing ISO
just test-vm        # Full cycle: fresh disk, install, boot, verify (13 checks)
just test-install   # Install only
just test-boot      # Boot and verify existing installation

just qemu-install   # Boot ISO in QEMU with GUI
just ssh            # SSH into running QEMU VM
just upload         # Upload binaries to running VM
```

### Testing
The xtask test suite boots a QEMU VM, runs the installer, reboots from the installed disk, and verifies 13 system health checks (kernel, ZFS pool, sshd, fstab, initramfs, zram, mounts, hostid, ZED hook, bootfs, rootprefix, ZBM build, ZBM pacman hook).

Installer logs are automatically pulled from the VM to `test-install.log` for analysis.

---

## Troubleshooting

<details>
<summary><strong>ZFS package dependency issues</strong></summary>

If a precompiled ZFS package for your exact kernel version is not available, the installer automatically falls back to DKMS. The kernel compatibility validation reduces the chance of encountering build failures.

</details>

<details>
<summary><strong>Installation fails in QEMU</strong></summary>

Common causes:
- UEFI not enabled in VM settings
- Insufficient RAM (< 2GB)
- No network connectivity

Tips:
1. Use `just qemu-install-serial` for better error visibility
2. Check `test-install.log` for detailed trace-level logs
3. Verify UEFI firmware is loaded

</details>

<details>
<summary><strong>Boot issues after installation</strong></summary>

If ZFSBootMenu does not appear:
1. Check UEFI boot order in firmware
2. Verify the EFI partition is mounted
3. Confirm ZFSBootMenu files exist in `/boot/efi/EFI/ZBM/`

Recovery: Boot from the installer USB and run repair commands via chroot.

</details>

---

## Links

### Project
- [Releases](https://github.com/okhsunrog/archinstall-zfs-rs/releases)
- [Issues](https://github.com/okhsunrog/archinstall-zfs-rs/issues)
- [Python version](https://github.com/okhsunrog/archinstall_zfs)

### Resources
- [Arch Wiki: ZFS](https://wiki.archlinux.org/title/ZFS)
- [Arch Wiki: Install Arch Linux on ZFS](https://wiki.archlinux.org/title/Install_Arch_Linux_on_ZFS)
- [ZFSBootMenu documentation](https://docs.zfsbootmenu.org/)
- [OpenZFS documentation](https://openzfs.github.io/openzfs-docs/)

---

## License

GPL-3.0 — see [`LICENSE`](LICENSE).
