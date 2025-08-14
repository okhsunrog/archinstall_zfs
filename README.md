## Archinstall‑ZFS ✨

ZFS‑first Arch Linux installer with batteries included. Effortless ZFS root, automatic ZFSBootMenu, and a fast, friendly TUI.

[![Demo](assets/archinstall-demo.svg)](https://asciinema.org/a/IgofIGOQP9AXUCVbDHIAstlPz)

### Highlights

- 🚀 Fast path to an Arch+ZFS system
- 🧰 Three install modes: full‑disk, new pool on a partition, or use an existing pool
- 🧯 Automatic ZFSBootMenu with recovery options
- 🧪 Boot‑environment aware mounts via a custom ZED hook (mounts only datasets from the active BE + shared ones)
- 🔐 Native ZFS encryption (pool‑wide or per‑dataset)
- 🧩 Seamless integration with archinstall profiles
- 🖥️ Smooth TUI experience with clear logging

Requirements: UEFI firmware and internet connectivity (both are validated by the installer).

## Quickstart ⚡

### Option A — Prebuilt ISO (recommended)

1) Download the latest ISO from [Releases](https://github.com/okhsunrog/archinstall_zfs/releases).
2) Boot it on your machine (UEFI).
3) Connect to the network.
4) Run the installer:

```bash
./installer
# or
cd /root/archinstall_zfs && python -m archinstall_zfs
```

Notes:
- The ISO already contains ZFS modules and this installer.
- Source code is available at `/root/archinstall_zfs` inside the live system.

### Option B — Official Arch ISO

1) Boot the official Arch ISO and connect to the network.
2) Install minimal deps:

```bash
pacman -Sy git
```

3) Fetch and run the installer:

```bash
git clone --depth 1 https://github.com/okhsunrog/archinstall_zfs
cd archinstall_zfs
python -m archinstall_zfs
```

This path takes a bit longer because it installs ZFS modules on the fly. The prebuilt ISO already includes them.

## Features ✨

- 📦 Full‑disk auto‑partitioning, new‑pool on partition, or use existing pool
- 🧭 ZFSBootMenu setup with sensible defaults
- 🧩 Profile support via archinstall
- 🧱 ZED hook to keep zfs‑list.cache scoped to the active boot environment
- 🔐 ZFS encryption: Pool or per‑dataset
- 🧾 Robust logs and error handling

### 🚀 Enhanced Kernel Support (v2.0)

**Precompiled ZFS for All Kernels:**
- `linux-lts` + `zfs-linux-lts` (recommended)
- `linux` + `zfs-linux` ✨ **NEW!**
- `linux-zen` + `zfs-linux-zen` ✨ **NEW!**

**Intelligent Fallback Logic:**
- Maintains kernel consistency during fallback
- `linux-lts` precompiled fails → `linux-lts` + DKMS ✅
- No more unexpected kernel changes during installation

**Extensible Architecture:**
- Easy to add support for new kernel variants
- Centralized kernel configuration management
- Comprehensive error handling and reporting

See [KERNEL_ARCHITECTURE.md](docs/KERNEL_ARCHITECTURE.md) for detailed technical information.

## Troubleshooting 🔧

### ZFS Package Dependency Issues

If you encounter an error like this during installation:

```
warning: cannot resolve "linux-lts=6.12.41-1", a dependency of "zfs-linux-lts"
:: The following package cannot be upgraded due to unresolvable dependencies:
      zfs-linux-lts

:: Do you want to skip the above package for this upgrade? [y/N] 
error: failed to prepare transaction (could not satisfy dependencies)
:: unable to satisfy dependency 'linux-lts=6.12.41-1' required by zfs-linux-lts
==> ERROR: Failed to install packages to new root
```

**What's happening:** The precompiled ZFS package for your kernel version isn't available or compatible with the current kernel.

**Solution:** Press `N` when prompted. The installer will automatically detect this issue and switch to the DKMS fallback path, which will install ZFS using DKMS instead of the precompiled package. This ensures your installation continues successfully while maintaining kernel consistency.

## Swap options

- **No swap**: Skip configuring swap.
- **ZRAM only**: Compressed, RAM-backed swap using zram-generator. We disable zswap in this mode.
- **ZSWAP + swap partition**:
  - Full-disk installs: creates a tail swap partition at the end of the disk.
  - New/existing pool modes: select an existing partition by-id to use as swap.
  - Two variants: unencrypted (plain mkswap) or encrypted with a random key each boot (dm-crypt via crypttab; mapped as `/dev/mapper/cryptswap`).

Notes:
- Swap on ZFS zvol and swapfiles on ZFS are not supported.
- Hibernation/resume is not supported in the current release.

## Development 🧑‍💻

The `gen_iso` directory ships everything to build custom ISOs and test them in QEMU. Use `just` to orchestrate common workflows.

### Host prerequisites (Arch Linux)

```bash
sudo pacman -S qemu-desktop edk2-ovmf archiso grub just rsync
```

Why grub? mkarchiso may fail to produce a bootable image without it installed on the host.

Optional: Install uv for a fast Python workflow.

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
just setup   # installs dev deps via uv
```

### Build ISOs

Unified templated profile at `gen_iso/profile` is rendered to `/tmp/archzfs-profile` and then built.

```bash
# Full (main) ISO
just build-main pre            # Precompiled ZFS, kernel=linux-lts (default)
just build-main dkms           # DKMS + linux-lts-headers (default kernel)
just build-main dkms linux     # DKMS + linux-headers, kernel=linux

# Minimal (testing) ISO, faster to build
just build-test pre             # Precompiled ZFS, kernel=linux-lts, minimal packages
just build-test dkms            # DKMS + linux-lts-headers, minimal packages
just build-test dkms linux      # DKMS + linux-headers, minimal packages, kernel=linux

just list-isos                  # List built ISOs in gen_iso/out
```

Notes:
- Testing builds use a minimal package set for faster iterations.
- Main builds include the full set. When `kernel=linux`, `broadcom-wl` and `b43-fwcutter` are included; they are omitted for `linux-lts`/others.
- Artifacts land in `gen_iso/out`.

### Templating model (Jinja2)

We use Jinja2 templates in `gen_iso/profile` to produce a concrete ArchISO profile before building.

- Templates:
  - `packages.x86_64.j2` (package list)
  - `pacman.conf.j2`
  - `profiledef.sh.j2`
  - `efiboot/loader/entries/01-archiso-x86_64-linux.conf.j2`
- Builder context keys:
  - `kernel`: `linux`, `linux-lts`, or `linux-zen`
  - `use_precompiled_zfs` / `use_dkms` (mutually exclusive)
  - `include_headers`: whether to add `{{kernel}}-headers` (auto=true for DKMS)
  - `fast_build`: minimal testing build when true; full main build when false
- Package split logic:
  - Testing-only: inside `{% if fast_build %}`
  - Main-only: inside `{% else %}`
  - Common to both: present in both branches
  - Kernel-specific: guarded (e.g., `{% if kernel == "linux" %}` for `broadcom-wl`, `b43-fwcutter`)

### Test in QEMU

Quick path for development:

```bash
just qemu-setup           # Create disk + UEFI vars
just build-testing-iso
just qemu-install-serial  # Headless serial console (recommended for dev)

# In a second terminal
just ssh                  # Sync source into VM and connect via SSH
./installer               # Run the installer in the SSH session
```

Helpful commands:

```bash
just qemu-install         # GUI install flow
just qemu-run             # Boot existing install (GUI)
just qemu-run-serial      # Boot existing install (serial)
just qemu-refresh         # Reset disk + UEFI vars
```

The testing ISO is tuned for faster build times by using a minimal package list. It is rendered from the same profile with a fast mode flag.

### Inside the ISO

```bash
./installer           # Shortcut wrapper (cds into /root/archinstall_zfs)
cd /root/archinstall_zfs && python -m archinstall_zfs
```

The source is available at `/root/archinstall_zfs` in both ISO profiles.

### All `just` recipes (high level)

```bash
# Quality
just format            # Ruff format
just lint              # Ruff lint (auto-fix)
just type-check        # MyPy
just test              # Pytest
just all               # Run all of the above
just clean             # Clean caches

# ISO build (parametric)
just build-main [pre|dkms] [linux|linux-lts|linux-zen]
just build-test [pre|dkms] [linux|linux-lts|linux-zen]
just list-isos
just clean-iso

# QEMU
just qemu-setup
just qemu-create-disk
just qemu-setup-uefi
just qemu-reset-uefi
just qemu-refresh
just qemu-install
just qemu-install-serial
just qemu-run
just qemu-run-serial
just ssh
just ssh-only
```

## Why this installer? 💡

- Simplifies ZFS on Arch end‑to‑end
- ZFSBootMenu and encryption handled for you
- Boot‑env aware mounts to prevent cross‑environment surprises
- Smooth dev loop with a testing ISO and `just ssh` syncing

## Roadmap / TODO 🗺️

1. System Enhancements
   - Smarter hostid generation (based on hostname)
   - Local ZFSBootMenu build support
   - Secure Boot support (sign kernels/ZBM and manage keys)

2. Additional Features
   - More ZFS tuning options (compression, DirectIO, etc.)
   - zrepl support: guided setup for backup/replication
   - Archinstall language selection in the menu

3. User Experience Improvements
   - [Proactive DKMS compatibility validation](docs/TODO_PROACTIVE_DKMS_VALIDATION.md) - Prevent kernel/ZFS compatibility issues before they occur

## Contributing 🤝

Issues and PRs are welcome. Typical flow: fork → branch → PR.

## License 📄

GPL‑3.0. See `LICENSE`.

## Links 🔗

- Project repository: [okhsunrog/archinstall_zfs](https://github.com/okhsunrog/archinstall_zfs)
- Releases: [downloads](https://github.com/okhsunrog/archinstall_zfs/releases)

Demo animation was recorded with [asciinema](https://asciinema.org/), edited using [asciinema-scene](https://github.com/jdum/asciinema-scene), and converted to SVG using [my fork of svg-term-cli](https://github.com/okhsunrog/svg-term-cli).
