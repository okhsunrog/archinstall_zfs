## Archinstall‑ZFS ✨

ZFS‑first Arch Linux installer with batteries included. Effortless ZFS root, automatic ZFSBootMenu, and a fast, friendly TUI.

[![Demo](assets/archinstall-demo.svg)](https://asciinema.org/a/Lt0B9qvvu9bLPpkAV96SC5prq)

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
python -m archinstall_zfs
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

```bash
just build-main-iso       # Production ISO (releng profile)
just build-testing-iso    # Dev/testing ISO (baseline profile)
```

Artifacts land in `gen_iso/out`.

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

The testing ISO is tuned for fast iterations: instant boot, auto‑login as root, full serial console, SSH on port 2222.

### Inside the ISO

```bash
./installer           # Shortcut wrapper
python -m archinstall_zfs
```

The source is available at `/root/archinstall_zfs` in both ISO profiles.

### All `just` recipes

```bash
# Quality
just format            # Ruff format
just lint              # Ruff lint (auto-fix)
just type-check        # MyPy
just test              # Pytest
just all               # Run all of the above
just clean             # Clean caches

# ISO build
just build-main-iso
just build-testing-iso
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

## Contributing 🤝

Issues and PRs are welcome. Typical flow: fork → branch → PR.

## License 📄

GPL‑3.0. See `LICENSE`.

## Links 🔗

- Project repository: [okhsunrog/archinstall_zfs](https://github.com/okhsunrog/archinstall_zfs)
- Releases: [downloads](https://github.com/okhsunrog/archinstall_zfs/releases)

Demo animation was recorded with [asciinema](https://asciinema.org/), edited using [asciinema-scene](https://github.com/jdum/asciinema-scene), and converted to SVG using [my fork of svg-term-cli](https://github.com/okhsunrog/svg-term-cli).
