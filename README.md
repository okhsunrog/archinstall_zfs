# Archinstall‑ZFS ⚡

> **ZFS‑first Arch Linux installer with batteries included**  
> Effortless ZFS root, automatic ZFSBootMenu, and a fast, friendly TUI.

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

**Perfect for:** Home labs, servers, workstations, or anyone who wants ZFS on Arch without the headaches.

---

## ⚡ Quick Start

### 📦 Option A: Prebuilt ISO *(Recommended)*

1. **Download** the latest ISO from [**Releases**](https://github.com/okhsunrog/archinstall_zfs/releases)
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

**Intelligent fallback:** If precompiled fails → automatic DKMS with same kernel ✅

### 🔐 **ZFS Encryption Options**
- **Pool-wide**: Everything encrypted from the start
- **Per-dataset**: Selective encryption (e.g., encrypt `/home`, leave `/var/log` plain)
- **No encryption**: Maximum performance

### 🧯 **Boot Environment Features**
- **ZFSBootMenu** with recovery options out of the box
- **Smart mounting** via custom ZED hook (only mounts active BE + shared datasets)
- **Snapshot-aware** filesystem navigation

### 💾 **Swap Configurations**
| Type | Description | Best For |
|------|-------------|----------|
| **No Swap** | Skip swap entirely | High-memory systems |
| **ZRAM** | Compressed RAM-based swap | Most desktops/laptops |
| **Swap Partition** | Traditional partition swap | Servers, hibernation needs |

> 📝 **Note:** Swap-on-ZFS (zvol/swapfiles) not supported. Hibernation not supported in current release.

---

## 🛠️ Development

### 🏗️ **Building Custom ISOs**

**Prerequisites** (Arch Linux host):
```bash
sudo pacman -S qemu-desktop edk2-ovmf archiso grub just rsync
curl -LsSf https://astral.sh/uv/install.sh | sh  # Optional: fast Python workflow
just setup  # Install dev dependencies
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

**Solution:** Press `N` when prompted. The installer will automatically switch to DKMS mode and continue installation successfully.

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
- [ ] **Backup integration** (zrepl setup wizard)
- [ ] **Multi-language support** (archinstall integration)
- [ ] **Enhanced monitoring** (ZED notification setup)

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
just setup                       # Install dependencies
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
- 🏗️ **Build**: ArchISO, QEMU
- 📄 **License**: GPL-3.0

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