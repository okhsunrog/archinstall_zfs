<h1 align="center">Archinstall‑ZFS 🚀</h1>

> **ZFS‑first Arch Linux installer with batteries included**  
> Effortless ZFS root, automatic ZFSBootMenu, and a fast, friendly TUI.

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/okhsunrog/archinstall_zfs)
[![GitHub](https://img.shields.io/github/license/okhsunrog/archinstall_zfs)](https://github.com/okhsunrog/archinstall_zfs/blob/main/LICENSE)
[![GitHub Release](https://img.shields.io/github/v/release/okhsunrog/archinstall_zfs)](https://github.com/okhsunrog/archinstall_zfs/releases)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/okhsunrog/archinstall_zfs/ci.yml)](https://github.com/okhsunrog/archinstall_zfs/actions)
[![Python Version](https://img.shields.io/badge/python-3.13-blue.svg)](https://www.python.org/downloads/)

[![Demo](assets/archinstall-demo.svg)](https://asciinema.org/a/IgofIGOQP9AXUCVbDHIAstlPz)

---

## 🌟 Why Archinstall-ZFS?

**Traditional ZFS on Arch setup is complex** — managing kernel compatibility, ZFS modules, bootloaders, and encryption by hand. **This installer handles all of that for you.**

✨ **What you get:**
- 🚀 **One-command install** from bare metal to working ZFS system
- 🧰 **Flexible deployment**: Full-disk, partition, or existing pool
- 🛡️ **Production-ready**: ZFSBootMenu, encryption, boot environments
- 🧩 **Arch-native**: Full archinstall integration with profiles
- 🔧 **Just works**: Smart kernel/ZFS matching with automatic fallbacks
- 📸 **Snapshot management**: Built-in zrepl support for automated snapshots
- 📦 **AUR integration**: Seamless AUR package installation during setup

**Perfect for:** Home labs, servers, workstations, or anyone who wants ZFS on Arch without the headaches.

---

## ⚡ Quick Start

### 📦 Option A: Prebuilt ISO *(Recommended)*

1. **Download** the latest ISO from [**Releases**](https://github.com/okhsunrog/archinstall_zfs/releases)
   - 🆕 **Monthly automated builds** on the 4th of each month
   - Both `linux` and `linux-lts` kernels with smart ZFS mode selection
2. **Boot** it on your UEFI machine and connect to network
3. **Run** the installer:

```bash
./installer
# or
cd /root/archinstall_zfs && python -m archinstall_zfs
```

> 💡 **Why the prebuilt ISO?** It includes ZFS modules and this installer pre-configured, saving you 5-10 minutes of setup time.

### 🛠️ Option B: Official Arch ISO

```bash
# 1. Boot official Arch ISO and connect to network
pacman -Sy git

# 2. Get the installer
git clone --depth 1 https://github.com/okhsunrog/archinstall_zfs
cd archinstall_zfs
python -m archinstall_zfs
```

> ⏱️ This takes a bit longer as it installs ZFS modules on the fly.

---

## 🎯 Core Features

### 🖥️ **Installation Modes**
| Mode | Description | Use Case |
|------|-------------|----------|
| **Full Disk** | Wipe disk, auto-partition, new ZFS pool | Clean installs, single-purpose machines |
| **New Pool** | Create ZFS pool on existing partition | Dual-boot, custom partitioning |
| **Existing Pool** | Install to existing ZFS pool | Upgrades, additional boot environments |

### 🧭 **Smart Kernel Support**
**Precompiled ZFS for all major kernels:**
- **`linux-lts`** + `zfs-linux-lts` *(recommended for stability)*
- **`linux`** + `zfs-linux` *(latest features)*
- **`linux-zen`** + `zfs-linux-zen` *(desktop optimized)*  
- **`linux-hardened`** + `zfs-linux-hardened` *(security focused)*

**🔍 Proactive validation:** Real-time compatibility checking against OpenZFS API  
**🔄 Intelligent fallback:** If precompiled fails → automatic DKMS with same kernel ✅

### 🔐 **ZFS Encryption Options**
- **Pool-wide**: Everything encrypted from the start
- **Per-dataset**: Selective encryption (e.g., encrypt `/home`, leave `/var/log` plain)
- **No encryption**: Maximum performance

### 🧯 **ZFSBootMenu & Boot Environments** *(The Main Feature!)*

**🎯 This is what sets this installer apart** - complete ZFSBootMenu integration with boot environment support:

#### **Automatic ZFSBootMenu Setup**
- **Zero-config installation**: Downloads and installs ZFSBootMenu EFI files automatically
- **Dual boot entries**: Main (`ZFSBootMenu`) + Recovery (`ZFSBootMenu-Recovery`) 
- **UEFI integration**: Automatically adds boot entries to firmware
- **Online updates**: Downloads latest ZFSBootMenu from official releases

#### **Production-Ready Boot Environment Architecture**
- **Structured datasets**: Automatic creation of optimal ZFS dataset hierarchy:
  ```
  pool/prefix/root       → /          (root filesystem, canmount=noauto)
  pool/prefix/data/home  → /home      (user data)
  pool/prefix/data/root  → /root      (root user data)  
  pool/prefix/vm         → /vm        (virtual machines)
  ```
- **Boot environment isolation**: Each installation becomes a separate boot environment
- **Snapshot navigation**: ZFSBootMenu automatically discovers all snapshots and clones

#### **Smart Dataset Mounting (Custom ZED Hook)**
- **Boot environment aware**: Only mounts datasets from the active boot environment
- **Shared data handling**: Automatically mounts shared datasets (like `/home`) across all BEs
- **Clean isolation**: Prevents cross-BE contamination and surprises
- **Zero configuration**: Works out of the box with optimal defaults

#### **ZFS Properties Optimization**
- **ZFSBootMenu integration**: Automatically sets `org.zfsbootmenu:commandline` and `org.zfsbootmenu:rootprefix`
- **Kernel parameter optimization**: Includes `spl.spl_hostid=$(hostid)` and optimal `zswap` settings
- **Init system awareness**: Configures `root=ZFS=` (dracut) or `zfs=` (mkinitcpio) automatically
- **fstab stability**: Adds root dataset to `/etc/fstab` to prevent snapshot navigation bugs
- **Configurable ZFS compression**: Choose compression in TUI — `lz4` (default), `zstd` (levels), or `off`

#### **What This Means for You**
- **🔄 Easy rollbacks**: Boot from any snapshot if an update breaks your system
- **🏠 Multiple environments**: Install different Arch configurations on the same pool
- **🛡️ System isolation**: Boot environments don't interfere with each other
- **📸 Snapshot workflows**: Take snapshots before major changes, rollback instantly if needed

### 💾 **Swap Configurations**
| Type | Description | Best For |
|------|-------------|----------|
| **No Swap** | Skip swap entirely | High-memory systems |
| **ZRAM** | Compressed RAM-based swap | Most desktops/laptops |
| **Swap Partition** | Traditional partition swap | Servers, hibernation needs |

> 📝 **Note:** Swap-on-ZFS (zvol/swapfiles) not supported. Hibernation not supported in current release.

### ⚙️ **Advanced Features**

#### **🔧 Initramfs Optimization**
- **Dracut support**: Optimized dracut configuration with ZFS-specific settings
- **Mkinitcpio support**: Alternative initramfs with proper ZFS integration
- **Smart compression**: Disables double compression (ZFS + initramfs)
- **Encryption key handling**: Automatic inclusion of ZFS encryption keys
- **Minimal footprint**: Excludes unnecessary modules (network, plymouth, etc.)

#### **🌐 Network & Connectivity**
- **Internet validation**: Checks connectivity before starting installation
- **Network config preservation**: Option to copy live ISO network settings to target
- **Archzfs repository**: Automatic setup of ZFS package repositories
- **Mirror configuration**: Full archinstall mirror selection integration

#### **📦 Smart Package Management & Compatibility Validation**
- **🔍 Proactive compatibility checking**: Real-time validation against OpenZFS GitHub API prevents installation failures
- **🎯 Smart kernel selection**: Automatically filters incompatible kernel/ZFS combinations in TUI with clear explanations
- **⚡ Fail-fast ISO builds**: Pre-validates compatibility before building ISOs, saving time on doomed builds
- **🔄 Intelligent fallbacks**: Seamless precompiled → DKMS switching when kernel versions don't match
- **📦 Repository management**: Handles archzfs repo setup with fallback to direct GitHub release parsing
- **💡 User-friendly feedback**: Clear warnings and suggestions when compatibility issues are detected
- **🏗️ AUR integration**: Built-in AUR helper (yay) with secure temporary user management for AUR package installation

#### **📸 ZFS Snapshot Management**
- **🔄 zrepl support**: Automated ZFS snapshot creation and replication with sensible defaults
- **⏰ Smart scheduling**: 15-minute snapshot intervals with intelligent pruning (4×15m, 24×1h, 3×1d retention)
- **🎛️ Zero-config**: Automatically generates configuration based on your ZFS pool and dataset layout
- **🔧 Service management**: Automatic package installation and systemd service enablement
- **📋 Flexible configuration**: Uses your actual pool name and dataset prefix for targeted snapshots

#### **💽 Disk Management Excellence**  
- **By-ID partition handling**: Uses `/dev/disk/by-id` for stable device references
- **Smart partition waiting**: Waits for udev to create partition symlinks
- **EFI integration**: Automatic EFI partition mounting and configuration
- **Signature cleaning**: Proper disk signature clearing to prevent conflicts

#### **🔐 Security & Reliability**
- **Static hostid**: Generates consistent system identification
- **Secure key storage**: Proper file permissions (000) for encryption keys
- **Cache management**: Smart ZFS cache file handling and mountpoint modification
- **Service integration**: Enables all necessary ZFS systemd services

---

## 🛠️ Development

### 🏗️ **Building Custom ISOs**

**Prerequisites** (Arch Linux host):
```bash
sudo pacman -S qemu-desktop edk2-ovmf archiso grub just rsync uv
just install-dev  # Install dev dependencies
```

**Build commands:**
```bash
# Production ISOs
just build-main pre              # Precompiled ZFS + linux-lts
just build-main dkms linux       # DKMS + linux kernel

# Development ISOs (faster builds)
just build-test pre              # Minimal package set for testing
just build-test dkms linux-zen   # Test with zen kernel

just list-isos                   # See what you've built
```

### 🧪 **QEMU Testing Workflow**

**Quick development loop:**
```bash
just qemu-setup                  # Create test disk + UEFI vars
just build-test pre              # Build minimal testing ISO
just qemu-install-serial         # Boot with serial console

# In another terminal:
just ssh                         # Sync source code and connect
./installer                      # Test your changes
```

**Other QEMU commands:**
```bash
just qemu-install                # GUI install flow
just qemu-run                    # Boot existing installation
just qemu-refresh                # Reset test environment
```

### ⚙️ **Quality Assurance**

```bash
just format                      # Format code (ruff)
just lint                        # Lint and auto-fix
just type-check                  # MyPy type checking
just test                        # Run test suite
just all                         # All quality checks
```

---

## 🔧 Troubleshooting

<details>
<summary><strong>🚨 ZFS Package Dependency Issues</strong></summary>

**Problem:** You see an error like:
```
warning: cannot resolve "linux-lts=6.12.41-1", a dependency of "zfs-linux-lts"
:: Do you want to skip the above package for this upgrade? [y/N] 
```

**What's happening:** The precompiled ZFS package isn't available for your exact kernel version.

**Solution:** Press `N` when prompted. The installer will automatically detect this and switch to DKMS mode.

**🎯 Note:** With our new validation system, this scenario should be much rarer as incompatible combinations are detected upfront and handled automatically!

</details>

<details>
<summary><strong>⚠️ Kernel Options Missing from Menu</strong></summary>

**Problem:** Some kernel DKMS options are missing from the installer menu.

**What's happening:** The installer automatically validates kernel/ZFS compatibility using the OpenZFS GitHub API and hides incompatible combinations to prevent installation failures.

**You'll see a notice like:**
```
NOTICE: The following kernels are temporarily unavailable for DKMS
as they are not yet supported by the current ZFS version:
  - Linux Zen
```

**Solutions:**
1. **Choose a compatible kernel** (like `linux-lts` which is usually most compatible)
2. **Use precompiled ZFS** instead of DKMS (if available for your kernel)
3. **Disable validation** (advanced users only):
   ```bash
   export ARCHINSTALL_ZFS_SKIP_DKMS_VALIDATION=1
   ./installer
   ```

**Note:** Disabling validation may result in DKMS compilation failures during installation.

</details>

<details>
<summary><strong>🐛 Installation Fails in QEMU</strong></summary>

**Common causes:**
- UEFI not enabled in VM settings
- Insufficient RAM (< 2GB)
- Network not connected

**Debug steps:**
1. Use `just qemu-install-serial` for better error visibility
2. Check QEMU logs in the terminal
3. Verify UEFI firmware is loaded

</details>

<details>
<summary><strong>⚡ Boot Issues After Installation</strong></summary>

**If ZFSBootMenu doesn't appear:**
1. Check UEFI boot order in firmware
2. Verify EFI partition is properly mounted
3. Check if ZFSBootMenu EFI files exist in `/boot/efi/EFI/ZBM/`

**Recovery:** Boot from installer USB and run repair commands via chroot.

</details>

---

## 🗺️ Roadmap

### 🎯 **Next Release**
- [ ] **Secure Boot support** (sign kernels/ZBM, manage keys)
- [ ] **Local ZFSBootMenu builds** (no internet dependency)
- [ ] **Smarter hostid generation** (hostname-based)

### 🚀 **Future Enhancements**
- [ ] **Advanced ZFS tuning** (compression algorithms, block sizes)
- [ ] **Multi-language support** (archinstall integration)

---

## 💡 Architecture Insights

### 🧩 **Templating System**
We use **Jinja2 templates** to generate ISO profiles dynamically:

**Template variables:**
- `kernel`: Target kernel variant
- `use_precompiled_zfs` / `use_dkms`: ZFS installation method
- `include_headers`: Whether to include kernel headers
- `fast_build`: Minimal vs full package set

**Key templates:**
- `packages.x86_64.j2` → Package selection
- `profiledef.sh.j2` → ISO metadata  
- `pacman.conf.j2` → Repository configuration

### 🏗️ **Just Task Runner**
All workflows orchestrated via [`just`](https://github.com/casey/just) recipes:

```bash
just --list                      # See all available commands
just build-main pre linux-zen    # Parameterized builds
just qemu-install-serial         # Complex QEMU setups
```

---

## 🤝 Contributing

**We welcome contributions!** Here's how to help:

1. **🐛 Bug Reports**: Include system info, error logs, and reproduction steps
2. **💡 Feature Requests**: Describe your use case and proposed solution  
3. **🔧 Code Contributions**: Fork → branch → test → pull request
4. **📖 Documentation**: Help improve this README or add examples

**Development flow:**
```bash
git clone https://github.com/okhsunrog/archinstall_zfs
cd archinstall_zfs
just install-dev                 # Install dependencies
just qemu-setup                  # Set up test environment
# Make your changes
just all                         # Run quality checks
just qemu-install-serial         # Test in VM
```

---

## 📊 Project Stats

- 🗂️ **Languages**: Python, Shell, Jinja2
- 🧪 **Testing**: Pytest, MyPy, Ruff
- 📦 **Dependencies**: archinstall, ZFS utilities
- 🏗️ **Build**: ArchISO, QEMU, GitHub Actions
- 📄 **License**: GPL-3.0
- 🚀 **CI/CD**: Automated tag releases + monthly builds

---

## 🔗 Links & Resources

- 📦 **Releases**: [Download ISOs](https://github.com/okhsunrog/archinstall_zfs/releases)
- 🐛 **Issues**: [Report bugs](https://github.com/okhsunrog/archinstall_zfs/issues)
- 💬 **Discussions**: [Get help](https://github.com/okhsunrog/archinstall_zfs/discussions)
- 📖 **Arch Wiki**: [ZFS on Arch Linux](https://wiki.archlinux.org/title/ZFS)

---

## 📄 License

**GPL-3.0** - See [`LICENSE`](LICENSE) file for details.

---

<p align="center">
<sub>Demo animation created with <a href="https://asciinema.org/">asciinema</a>, edited with <a href="https://github.com/jdum/asciinema-scene">asciinema-scene</a>, and converted to SVG with <a href="https://github.com/okhsunrog/svg-term-cli">svg-term-cli</a>.</sub>
</p>