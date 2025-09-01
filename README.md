<h1 align="center">Archinstall‑ZFS 🚀</h1>

> ZFS‑first Arch Linux installer with ZFSBootMenu and a straightforward TUI.

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/okhsunrog/archinstall_zfs)
[![GitHub](https://img.shields.io/github/license/okhsunrog/archinstall_zfs)](https://github.com/okhsunrog/archinstall_zfs/blob/main/LICENSE)
[![GitHub Release](https://img.shields.io/github/v/release/okhsunrog/archinstall_zfs)](https://github.com/okhsunrog/archinstall_zfs/releases)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/okhsunrog/archinstall_zfs/ci.yml)](https://github.com/okhsunrog/archinstall_zfs/actions)
[![Python Version](https://img.shields.io/badge/python-3.13-blue.svg)](https://www.python.org/downloads/)

[![Demo](assets/archinstall-demo.svg)](https://asciinema.org/a/IgofIGOQP9AXUCVbDHIAstlPz)

---

## Overview

Setting up ZFS on Arch involves kernel selection, ZFS module installation, bootloader configuration, and optional encryption. Archinstall‑ZFS automates these steps, integrates with archinstall profiles, and provides a TUI‑based workflow. It can build ISOs for repeatable installs and includes helpers for QEMU testing.

---

## Quick start ⚡

### Option A: Prebuilt ISO (recommended)
1. Download the latest ISO from the releases page.
2. Boot on a UEFI machine and connect to the network.
3. Run:

```bash
./installer
# or
cd /root/archinstall_zfs && python -m archinstall_zfs
```

> Why recommended: the ISO already contains ZFS components and this installer, so startup is faster and avoids on-the-fly package installation.

### Option B: Official Arch ISO
```bash
# 1) Boot the official Arch ISO and connect to the network
pacman -Sy git

# 2) Get the installer
git clone --depth 1 https://github.com/okhsunrog/archinstall_zfs
cd archinstall_zfs
python -m archinstall_zfs
```

> Note: This path installs ZFS components during the run, so it usually takes longer than Option A.

---

## Features 🧩

### Device naming: uses `/dev/disk/by-id` for stable device references.

### Installation modes
| Mode | Description | Use case |
|------|-------------|----------|
| Full disk | Wipe disk, auto‑partition, new ZFS pool | Clean installs, single‑purpose machines |
| New pool | Create ZFS pool on an existing partition | Dual‑boot, custom partitioning |
| Existing pool | Install to an existing ZFS pool | Additional boot environments |

### Kernel support
- `linux-lts` + `zfs-linux-lts`
- `linux` + `zfs-linux`
- `linux-zen` + `zfs-linux-zen`
- `linux-hardened` + `zfs-linux-hardened`

The installer validates kernel/ZFS compatibility (via the OpenZFS API) and selects precompiled packages when available, falling back to DKMS when required.

### Encryption options
- Pool‑wide encryption
- Per‑dataset encryption (for example, encrypt `/home`, leave `/var/log` plain)
- No encryption

### Boot environments and layout

Boot Environments (BE) are a way to maintain multiple independent systems on a single ZFS pool. Each system is housed in its own root dataset and can be selected at boot through ZFSBootMenu. For multi-boot scenarios, you can install multiple distributions in one pool — each becomes a separate BE.

ZFSBootMenu is a bootloader designed specifically for ZFS. Unlike traditional bootloaders, it natively understands ZFS structure and can display boot environments in a beautiful ncurses menu, create snapshots, and clone boot environments directly at boot time. Need to roll back to a week-old snapshot? Simply select it from the menu and the system boots in that exact state. Want to experiment without breaking your current system? Clone a boot environment right from the bootloader and boot into the copy.

#### Dataset structure
The installer creates a consistent dataset layout for each boot environment:

**Real example from a production system:**
```bash
❯ zfs list
NAME                       USED  AVAIL  REFER  MOUNTPOINT
novafs                    1.09T   361G   192K  none

# Current active BE "arch0" (container, not mounted itself)
novafs/arch0               609G   361G   192K  none
novafs/arch0/data          421G   361G   192K  none
novafs/arch0/data/home     421G   361G   344G  /home    # /home dataset for arch0
novafs/arch0/data/root     120M   361G  45.3M  /root    # root user data for current BE
novafs/arch0/root          170G   361G   142G  /        # root filesystem of active BE
novafs/arch0/vm           18.8G   361G  18.8G  /vm      # separate dataset for VMs

# Previous BE "archold" (inactive but ready to boot)
novafs/archold             227G   361G   192K  none
novafs/archold/data        143G   361G   192K  none
novafs/archold/data/home   141G   361G   119G  /home    # /home dataset for archold
novafs/archold/data/root  1.81G   361G  1.81G  /root    # root user data for second BE
novafs/archold/root       83.7G   361G  83.7G  /        # root filesystem of second BE

# Global datasets (outside BE, mounted by all boot environments)
novafs/tmp_zfs            7.08G   361G  7.08G  /tmp_zfs        # temporary data
```

**Standard layout created by the installer:**
```
pool/prefix/root       → /          (root filesystem, canmount=noauto)
pool/prefix/data/home  → /home      (user data)
pool/prefix/data/root  → /root      (root user data)
pool/prefix/vm         → /vm        (virtual machines)
```

#### Boot process and systemd integration

The boot process works as follows:

1. **ZFSBootMenu**: Acts as the bootloader; when selecting a boot environment, it launches the Linux kernel via kexec and passes kernel command line parameters
2. **Root filesystem mounting**: The zfs hook in initramfs mounts the root filesystem according to command line parameters: `root=ZFS=...` (dracut) or `zfs=...` (mkinitcpio)
3. **Pool import**: systemd handles pool import via `zfs-import-scan.service`; pools are created with `zpool set cachefile=none <pool>` to avoid using `zpool.cache`
4. **Dataset mounting**: Other datasets are mounted by systemd through `zfs-mount-generator`, which reads `/etc/zfs/zfs-list.cache/<pool>` and generates mount units on the fly

#### Cross-environment mount prevention

By default, ZFS sees all datasets in the pool, which could cause systemd to attempt mounting filesystems from other boot environments (e.g., `/home` from a neighboring BE). This is solved by a custom ZED hook (`history_event-zfs-list-cacher.sh`) that:

- Monitors ZFS events and detects the currently booted root dataset
- Derives the active boot environment prefix
- Filters datasets to include only the current BE hierarchy and shared datasets
- Atomically updates `/etc/zfs/zfs-list.cache/<pool>` when content changes, using locks to prevent races
- Is installed to `/etc/zfs/zed.d/` and marked immutable (`chattr +i`) to prevent package updates from overwriting it

#### ZFS properties and kernel parameters
- Sets kernel parameters such as `spl.spl_hostid=$(hostid)` and optional zswap settings
- Configures `root=ZFS=` (dracut) or `zfs=` (mkinitcpio)
- Adds the root dataset to `/etc/fstab` to support snapshot navigation
- Compression options in the TUI: `lz4` (default), `zstd` (levels), or `off`

### Snapshot management (optional)
- zrepl support for automatic snapshot creation and retention
- Schedules: 15‑minute intervals with tiered retention (4×15m, 24×1h, 3×1d)
- Generates configuration based on your pool and dataset prefix
- Installs zrepl-bin package and enables relevant systemd service

### AUR packages (optional)
- Installs an AUR helper (`yay`) and selected AUR packages during installation (if chosen in the installer)
- Builds using a temporary user (`aurinstall`) with temporary passwordless sudo for package builds
- Restores sudo configuration and removes the temporary user after installation; build artifacts are cleaned up


---

## Development 🛠️

For detailed development information, architecture notes, and contribution guidelines, see [**DEVELOPMENT.md**](docs/DEVELOPMENT.md).



---

## Troubleshooting 🔧

<details>
<summary><strong>ZFS package dependency issues</strong></summary>

If a precompiled ZFS package for your exact kernel version is not available, you may see:
```
warning: cannot resolve "linux-lts=6.12.41-1", a dependency of "zfs-linux-lts"
:: Do you want to skip the above package for this upgrade? [y/N]
```

Choose `N`. The installer will detect this and switch to DKMS mode. The validation step reduces the chance of encountering this, but it can still occur right after kernel releases.

</details>

<details>
<summary><strong>Kernel options missing from the menu</strong></summary>

The installer validates kernel/ZFS compatibility using the OpenZFS API and hides combinations that are not available. When that happens, you will see a notice in the TUI.

Options:
1. Choose a compatible kernel (for example, `linux-lts`)
2. Use precompiled ZFS instead of DKMS (if available)
3. Disable validation (advanced users only):
   ```bash
   export ARCHINSTALL_ZFS_SKIP_DKMS_VALIDATION=1
   ./installer
   ```

Disabling validation may result in DKMS compilation failures during installation.

</details>

<details>
<summary><strong>Installation fails in QEMU</strong></summary>

Common causes:
- UEFI not enabled in VM settings
- Insufficient RAM (< 2GB)
- No network connectivity

Tips:
1. Use `just qemu-install-serial` for better error visibility
2. Check QEMU logs in the terminal
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

## Roadmap 🗺️

### Next release
- [ ] Secure Boot support (sign kernels/ZBM, manage keys)
- [ ] Local ZFSBootMenu builds (no internet dependency)
- [ ] Hostid improvements (hostname‑based)

### Future enhancements
- [ ] Advanced ZFS tuning (compression algorithms, block sizes)
- [ ] Multi‑language support (archinstall integration)

---

## Links 🔗

- Releases: `https://github.com/okhsunrog/archinstall_zfs/releases`
- Issues: `https://github.com/okhsunrog/archinstall_zfs/issues`
- Discussions: `https://github.com/okhsunrog/archinstall_zfs/discussions`
- Arch Wiki: `https://wiki.archlinux.org/title/ZFS`

---

## License 📄

GPL‑3.0 — see [`LICENSE`](LICENSE).

---

<p align="center">
<sub>Demo animation created with <a href="https://asciinema.org/">asciinema</a>, edited with <a href="https://github.com/jdum/asciinema-scene">asciinema-scene</a>, and converted to SVG with <a href="https://github.com/okhsunrog/svg-term-cli">svg-term-cli</a>.</sub>
</p>