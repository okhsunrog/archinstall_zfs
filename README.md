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
   > **Ventoy users**: When selecting the image, choose GRUB2 boot mode for proper UEFI booting.
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

| Mode | Description | Best for |
|------|-------------|----------|
| **Full Disk** | Complete disk takeover with automated partitioning. Clears GPT/MBR signatures, creates fresh GPT table, partitions (EFI 500MB, optional swap, remainder for ZFS) | Clean installs, single-purpose machines, maximum automation |
| **New Pool** | Creates ZFS pool on an existing partition. Uses your existing partition layout, creates ZFS pool on selected partition | Dual-boot scenarios, custom partitioning schemes, preserving existing OS installations |
| **Existing Pool** | Installs into an existing ZFS pool as a new boot environment. Creates new BE datasets within your existing pool structure | Experiments, testing different configurations, multiple Arch installations |

> **Pro tip**: Existing Pool mode is excellent for trying different desktop environments or system configurations without risk - each installation becomes its own boot environment selectable from ZFSBootMenu.

### Kernel support
- `linux-lts` + `zfs-linux-lts`
- `linux` + `zfs-linux`
- `linux-zen` + `zfs-linux-zen`
- `linux-hardened` + `zfs-linux-hardened`

> All kernel options automatically fall back to `zfs-dkms` if precompiled modules are unavailable.

### Kernel/ZFS compatibility validation

One of the key challenges with ZFS on Arch is compatibility between kernel versions and ZFS modules. The installer includes a sophisticated validation system that:

- **Parses OpenZFS releases**: Checks https://github.com/openzfs/zfs/releases for supported kernel version ranges
- **Validates current packages**: Cross-references with actual kernel versions available in Arch repositories
- **Checks precompiled availability**: Determines if precompiled ZFS modules exist for your chosen kernel
- **Assesses DKMS feasibility**: Analyzes whether DKMS compilation will work with bleeding-edge kernels
- **Provides smart fallbacks**: Automatically suggests compatible alternatives when conflicts are detected

The validation runs in two places:
1. **In the installer TUI** - Shows only compatible kernel options and warns about potential issues
2. **During ISO building** - Ensures ISOs are built with working kernel/ZFS combinations

This prevents common installation failures like:
```
warning: cannot resolve "linux-lts=6.12.41-1", a dependency of "zfs-linux-lts"
:: Do you want to skip the above package for this upgrade? [y/N]
```

Advanced users can bypass validation with `export ARCHINSTALL_ZFS_SKIP_DKMS_VALIDATION=1`, but this may result in DKMS compilation failures.

### Encryption options
- Pool‑wide encryption
- Per‑boot environment encryption (encrypts the base dataset, all datasets within the boot environment inherit encryption)
- No encryption

### Swap and memory management
The installer offers flexible swap configuration options:

**No swap + ZRAM (recommended)**
- Uses compressed swap in RAM via `systemd-zram-generator`
- Default size: `min(ram / 2, 4096)` MB configured in `/etc/systemd/zram-generator.conf`
- zswap is disabled to avoid double compression
- Best for most desktop/laptop scenarios

**Classical swap partition**
- Creates dedicated swap partition on disk
- Enables zswap for compressed swap cache in RAM
- For full disk mode: specify swap size during installation
- For other modes: select existing partition in TUI
- Supports encryption via `cryptswap` in `/etc/crypttab`

**No swap**
- Pure RAM-only operation
- Suitable for systems with abundant RAM or specific workloads

> **Note**: Swap on ZFS (zvol/swapfile) is not supported due to potential deadlock issues. Hibernation is not currently supported.

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
- [ ] Hostid improvements (hostname‑based)

### Future enhancements
- [ ] Advanced ZFS tuning (compression algorithms, block sizes)
- [ ] Multi‑language support (archinstall integration)

---

## Links 🔗

### Project
- [Releases](https://github.com/okhsunrog/archinstall_zfs/releases)
- [Issues](https://github.com/okhsunrog/archinstall_zfs/issues)
- [Discussions](https://github.com/okhsunrog/archinstall_zfs/discussions)

### Articles
- [Meet archinstall_zfs: The TUI That Tames Arch Linux ZFS Installation](https://okhsunrog.dev/posts/archinstall-zfs/) (English)
- [Arch Linux на ZFS для людей: новый TUI-установщик archinstall_zfs](https://habr.com/ru/articles/942396/) (Habr, Russian)
- [Arch Linux на ZFS для людей: новый TUI-установщик archinstall_zfs](https://okhsunrog.dev/ru/posts/archinstall-zfs/) (Russian)

### Resources
- [Arch Wiki: ZFS](https://wiki.archlinux.org/title/ZFS)
- [Arch Wiki: Install Arch Linux on ZFS](https://wiki.archlinux.org/title/Install_Arch_Linux_on_ZFS)
- [ZFSBootMenu: Boot Environments and You](https://docs.zfsbootmenu.org/en/v2.3.x/general/bootenvs-and-you.html)
- [Awesome ZFS](https://github.com/ankek/awesome-zfs)
- [OpenZFS Documentation](https://openzfs.github.io/openzfs-docs/)

---

## License 📄

GPL‑3.0 — see [`LICENSE`](LICENSE).

---

<p align="center">
<sub>Demo animation created with <a href="https://asciinema.org/">asciinema</a>, edited with <a href="https://github.com/jdum/asciinema-scene">asciinema-scene</a>, and converted to SVG with <a href="https://github.com/okhsunrog/svg-term-cli">svg-term-cli</a>.</sub>
</p>
