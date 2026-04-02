# archinstall-zfs-rs — Architecture

> Primary reference for the Rust rewrite of archinstall_zfs. Give this as
> context to Claude Code alongside the `archinstall_zfs` Python repo.

## 1. Project Overview

Complete rewrite of `archinstall_zfs` in Rust, dropping the Python `archinstall`
library dependency entirely. The result is a single binary — a standalone
Arch Linux installer with first-class ZFS support and a ratatui-based TUI.

Everything is Rust: the installer binary, TUI, all ZFS/disk/pacstrap/chroot
logic, config handling, ISO profile rendering (minijinja), and the QEMU test
harness (xtask). No Python anywhere.

### What we keep from the Python codebase (logic, not code)

- All ZFS-specific logic: pool/dataset creation, encryption, ZFSBootMenu, zrepl
- Installation flow: disk partitioning -> pacstrap -> chroot config -> bootloader
- Config wizard UX (wizard steps, validation gates, preview)
- JSON config import/export for unattended installs (`--config` + `--silent`)
- AUR helper installation (yay-bin)
- dracut / mkinitcpio initramfs handlers
- Kernel/ZFS compatibility scanning with precompiled<->DKMS fallback
- QEMU test infrastructure (run-qemu.sh, test ISO builds, justfile recipes)
- Custom ZED boot-environment-aware cache hook
- zfs-list.cache mountpoint rewriting

### What we drop

- `archinstall` library dependency (reimplemented as thin wrappers)
- Python profile system with dynamic module loading -> static profile registry
- Plugin system
- Translation system (English-only for v1)
- LUKS/LVM/btrfs paths (ZFS-only installer)
- GRUB, rEFInd, Limine, efistub bootloaders (ZFSBootMenu only)
- Pre-built ZFSBootMenu EFI downloads (replaced with local `generate-zbm`)

### What improves

- TUI: ratatui gives real-time progress, scrollable logs, split panes
- Type safety: serde structs with validation instead of pydantic + runtime casts
- Error handling: `Result<T, E>` chains instead of scattered try/except
- Single binary: no Python, no pip, no venv on the live ISO
- Sub-second startup
- ZFS JSON output (`-j` flag from OpenZFS 2.3+) replaces text parsing
- `alpm` crate replaces shelling out to `pacman -Si` for package queries
- `pacmanconf` crate replaces manual pacman.conf string manipulation
- ZFSBootMenu built locally with `generate-zbm` — same kernel and ZFS version
  as the installed system, embedded cmdline (zbm.timeout, import_policy),
  pacman hook for automatic regeneration on kernel/ZFS updates
- Boot-environment-aware ZED cache hook prevents cross-BE mount issues
- Passwords passed via stdin (not visible in process args)

---

## 2. Crate Dependencies

```toml
[dependencies]
# TUI
ratatui = "0.30"
crossterm = "0.29"
tui-textarea = "0.7"          # text input (hostname, passwords, package lists)
tui-widget-list = "0.13"      # scrollable select list with mouse support
tui-scrollview = "0.6"        # scrollable log view for install progress
ratatui-macros = "0.6"        # layout boilerplate reduction

# Arch Linux
alpm = "5"                    # libalpm bindings — package version queries
pacmanconf = "3"              # parse/modify pacman.conf natively

# System
sysinfo = "0.38"              # CPU vendor (microcode), UEFI detection
blockdev = "0.3"              # device info queries
nix = { version = "0.31", features = ["fs", "process", "signal"] }  # sync(), signals

# Config / CLI
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }

# ISO profile rendering
minijinja = "2"               # Jinja2-compatible templates for archiso profiles

# Error handling / logging
thiserror = "2"
color-eyre = "0.6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
```

**No async runtime.** Installation is sequential. TUI runs synchronously with
non-blocking crossterm input polling. Long-running commands (pacstrap, dkms)
run in a background thread with stdout piped to a channel for live display.

**No ZFS C bindings.** OpenZFS 2.3+ supports `-j` (JSON output) for all query
commands. We call the CLI tools and deserialize with serde. Mutating commands
(`zpool create`, `zfs set`) don't need JSON — we just check exit codes.

**`alpm` requires libalpm** — always present on Arch live ISOs. Used only for
querying package versions (kernel scanner), not for installation (that's pacstrap).

---

## 3. Module Tree

```
src/
├── main.rs                      # CLI parsing (clap), entry point
├── app.rs                       # Headless install orchestrator (14 phases)
│
├── config/
│   ├── mod.rs
│   ├── types.rs                 # GlobalConfig + all enums (serde, defaults)
│   ├── validation.rs            # validate_for_install() -> Vec<String>
│   └── io.rs                    # JSON load/save/merge
│
├── tui/
│   ├── mod.rs                   # Terminal init/restore, main event loop
│   ├── event.rs                 # Input event handling (key, mouse, resize)
│   ├── theme.rs                 # Color palette, style constants
│   ├── screens/
│   │   ├── mod.rs
│   │   ├── main_menu.rs         # Top-level config menu (read-only for now)
│   │   ├── wizard.rs            # Storage & ZFS wizard (multi-step) [TODO]
│   │   ├── locale.rs            # Locale/keyboard/language selection [TODO]
│   │   ├── mirrors.rs           # Mirror region selection [TODO]
│   │   ├── auth.rs              # Root password + user accounts [TODO]
│   │   ├── kernel.rs            # Kernel + ZFS mode selector [TODO]
│   │   ├── profile.rs           # Desktop/server profile picker [TODO]
│   │   ├── applications.rs      # Audio, bluetooth toggles [TODO]
│   │   ├── packages.rs          # Additional pacman + AUR packages [TODO]
│   │   ├── confirm.rs           # Pre-install summary + confirm [TODO]
│   │   └── install_progress.rs  # Live log + progress bar [TODO]
│   └── widgets/
│       └── mod.rs               # Custom widgets [TODO]
│
├── system/
│   ├── mod.rs
│   ├── cmd.rs                   # CommandRunner trait + RealRunner + chroot()
│   ├── pacman.rs                # pacstrap, archzfs repo, parallel downloads
│   ├── sysinfo.rs               # UEFI check, CPU vendor (microcode)
│   └── net.rs                   # Internet connectivity check
│
├── disk/
│   ├── mod.rs
│   ├── by_id.rs                 # Enumerate /dev/disk/by-id (disks vs partitions)
│   └── partition.rs             # sgdisk: zap, create GPT, EFI+ZFS+swap partitions
│
├── zfs/
│   ├── mod.rs                   # ZFS_SERVICES constant
│   ├── cli.rs                   # run_zfs(), run_zpool() helpers (-j for queries)
│   ├── models.rs                # Serde structs for ZFS JSON output
│   ├── pool.rs                  # zpool create/import/export/set
│   ├── dataset.rs               # zfs create, default dataset layout, mount ordering
│   ├── encryption.rs            # Pool/dataset encryption, key file management
│   ├── kmod.rs                  # modprobe zfs, archzfs repo, reflector, host init
│   ├── cache.rs                 # hostid, zfs-list.cache rewriting, ZED cache hook
│   └── bootmenu.rs              # generate-zbm config, pacman hook, efibootmgr, ZBM properties
│
├── installer/
│   ├── mod.rs                   # Installer struct, phases 4-12 pipeline
│   ├── base.rs                  # pacstrap base system + kernels + microcode
│   ├── locale.rs                # hostname, locale-gen, timezone, keyboard, NTP
│   ├── users.rs                 # useradd, chpasswd (via stdin), sudoers
│   ├── services.rs              # systemctl enable in chroot
│   ├── network.rs               # Copy ISO network config / install NetworkManager
│   ├── initramfs/
│   │   ├── mod.rs
│   │   ├── dracut.rs            # Dracut config, pacman hooks, generation
│   │   └── mkinitcpio.rs        # mkinitcpio config (systemd->udev for ZFS), generation
│   ├── aur.rs                   # Temp user, yay-bin, AUR package install
│   └── fstab.rs                 # genfstab + EFI nofail fix + swap/crypttab entries
│
├── kernel/
│   ├── mod.rs                   # KernelInfo registry, package mapping, fallback
│   └── scanner.rs               # Compatibility scan via alpm (version queries)
│
├── swap/
│   └── mod.rs                   # zram-generator config, zswap partition setup
│
├── profile/
│   ├── mod.rs                   # Profile registry, get_profile()
│   ├── desktop.rs               # Desktop profiles (gnome, plasma, sway, hyprland...)
│   └── server.rs                # Server profiles (sshd, docker, postgresql...)
│
├── iso.rs                       # archiso profile template rendering (minijinja)
└── zrepl.rs                     # zrepl YAML config generation
```

---

## 4. Key Design Decisions

### 4.1 CommandRunner trait (testability foundation)

Every module that executes external commands goes through this trait:

```rust
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput>;
    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CmdOutput>;
    fn run_streaming(&self, program: &str, args: &[&str], tx: &Sender<String>) -> Result<CmdOutput>;
}
```

Two implementations:
- `RealRunner` — `std::process::Command` for production
- `RecordingRunner` — records calls, returns canned responses for tests

Convenience wrapper: `chroot()` calls `arch-chroot <target> bash -c <cmd>`.

### 4.2 ZFS JSON integration

OpenZFS 2.3+ supports `-j` on query commands. We define serde models and
deserialize directly. Fixtures captured from real ZFS output live in
`tests/fixtures/`.

Mutating commands (`zpool create`, `zfs create`, `zfs set`) don't output JSON.
For those we check exit code + stderr.

### 4.3 Config types

Direct mapping from `archinstall_zfs/menu/models.py` `GlobalConfig`. All enums
use serde string serialization. The `InitSystem` enum is used throughout
(not stringly-typed). Validation returns `Vec<String>` of errors.

Key config options:
- `set_bootfs` (default: true) — sets pool bootfs for ZBM auto-boot with
  countdown. Users can disable for fully interactive ZBM menu.

### 4.4 ZFSBootMenu — locally built

Instead of downloading pre-built EFI binaries, we:
1. Install `zfsbootmenu` from AUR (provides `generate-zbm`)
2. Write `/etc/zfsbootmenu/config.yaml` matching the chosen init system
3. Run `generate-zbm` in chroot to build an EFI bundle using the system's
   own kernel and ZFS modules
4. Copy the EFI to `EFI/BOOT/BOOTX64.EFI` as UEFI fallback
5. Create efibootmgr entries (no `-u` needed — cmdline is embedded)
6. Install a pacman hook (`95-zfsbootmenu.hook`) for auto-regeneration

The EFI bundle embeds `zbm.timeout=10 zbm.import_policy=hostid` so it
works even when booting from the fallback path after UEFI vars reset.

ZFS dataset properties set on the root dataset:
- `org.zfsbootmenu:commandline` — `spl.spl_hostid=0x00bab10c zswap.enabled=0|1 rw`
  (never includes `root=` — ZBM adds it)
- `org.zfsbootmenu:rootprefix` — `root=ZFS=` (dracut) or `zfs=` (mkinitcpio)
- Pool `bootfs` — set to the root dataset (configurable via `set_bootfs`)

### 4.5 Boot environment isolation

Custom ZED hook (`history_event-zfs-list-cacher.sh`) filters
`/etc/zfs/zfs-list.cache/<pool>` to only include datasets from the currently
booted BE. Without this, systemd would try to mount datasets from other BEs.
The hook is installed with `chattr +i` to survive ZFS package updates.

The `zfs-list.cache` file is also rewritten during installation to strip the
`/mnt` mountpoint prefix before copying to the target.

### 4.6 mkinitcpio + ZFS

The archzfs `zfs` hook is a legacy (udev-based) hook, incompatible with
systemd-based initramfs. When mkinitcpio is selected, the installer replaces
`systemd`/`sd-vconsole` hooks with `udev`/`keymap` equivalents, then inserts
`zfs` before `filesystems`. Both init systems set `COMPRESSION="cat"` to avoid
double compression on ZFS.

### 4.7 Profile system

Static `&[Profile]` array replacing archinstall's dynamic Python module loading.
Each desktop/server profile is a const entry with packages and services.

### 4.8 TUI architecture

Screen-based state machine with ratatui immediate-mode rendering. Installation
runs in a background thread communicating via `mpsc` channels (log lines, phase
updates, completion).

### 4.9 Installation pipeline

Linear pipeline matching `archinstall_zfs/main.py::perform_installation()`:

```
Phase 0:  Pre-install checks (internet, UEFI, ZFS module)
Phase 1:  Disk preparation (sgdisk)
Phase 2:  ZFS pool + datasets + encryption
Phase 3:  Mount EFI partition
Phase 4:  pacstrap base system
Phase 5:  System config (hostname, locale, timezone, mirrors, network)
Phase 6:  archzfs repo on target + ZFS packages (precompiled->DKMS fallback)
Phase 7:  Initramfs (dracut or mkinitcpio)
Phase 8:  Users + authentication
Phase 9:  Profile packages + services
Phase 10: Additional packages + AUR packages
Phase 11: Swap configuration (zram or zswap partition)
Phase 12: ZFS services + genfstab + misc files + ZED hook
Phase 13: ZFSBootMenu (generate-zbm + efibootmgr + dataset properties)
Phase 14: Cleanup (umount, zpool export)
```

---

## 5. Testing Strategy

### Unit tests

Config validation, serde round-trips, ZFS JSON parsing with fixtures,
command construction with mock runner, file content generation with tempdir.
Currently 123 tests.

### QEMU VM integration tests (xtask)

Full install-and-boot cycle via `cargo xtask test-vm`:
1. Fresh 20G qcow2 disk + UEFI vars
2. Boot testing ISO, SSH in, upload binary + config
3. Run `--config <json> --silent`, verify exit code
4. Reset UEFI vars, boot from disk (EFI/BOOT/BOOTX64.EFI fallback)
5. SSH in, run 13 verification checks

Both dracut and mkinitcpio configs tested. `--tmpfs` flag for RAM-backed disk.

### CI

GitHub-hosted for `cargo test`/`clippy`/`fmt`. Self-hosted with KVM for VM tests.

---

## 6. Implementation Status

### Done

- [x] Phase 0: ZFS JSON fixtures
- [x] Phase 1: Project skeleton, CLI, config types, validation
- [x] Phase 2: System, disk, ZFS operations, JSON models
- [x] Phase 3: Full installer pipeline (all 14 phases)
- [x] QEMU xtask tests (dracut + mkinitcpio, 13/13 checks)
- [x] ISO profile rendering (minijinja, `render-profile` subcommand)
- [x] Local ZFSBootMenu build with generate-zbm
- [x] Boot-environment-aware ZED cache hook
- [x] Headless `--config --silent` mode

### Next: TUI screens (Phase 4)

1. Custom widgets (password_input, info_dialog)
2. Storage & ZFS wizard (multi-step: mode, disk, pool, encryption, swap)
3. All config screens (locale, mirrors, auth, kernel, profile, packages)
4. Confirm + install progress screens
5. Wire navigation + background install thread

### Future

- CI workflow (GitHub Actions + self-hosted KVM runner)
- Error recovery and edge cases
- `--dry-run` mode
- Mirror region selection

---

## 7. Python -> Rust Mapping Reference

| Python source | Rust module | Notes |
|---|---|---|
| `archinstall/lib/command.py` | `system::cmd` | `CommandRunner` trait |
| `archinstall/lib/installer.py` | `installer/*` | Only used methods |
| `archinstall/lib/pacman/` | `system::pacman` + `pacmanconf` crate | |
| `archinstall/tui/` | `tui/*` | ratatui replacement |
| `archinstall/default_profiles/` | `profile/` | Static data |
| `archinstall_zfs/main.py` | `app.rs` + `installer/mod.rs` | |
| `archinstall_zfs/menu/global_config.py` | `tui/screens/main_menu.rs` + wizard | |
| `archinstall_zfs/menu/models.py` | `config/types.rs` | |
| `archinstall_zfs/zfs/__init__.py` | `zfs/*` | |
| `archinstall_zfs/zfs/kmod_setup.py` | `zfs/kmod.rs` | |
| `archinstall_zfs/disk/__init__.py` | `disk/*` | |
| `archinstall_zfs/kernel.py` | `kernel/mod.rs` | |
| `archinstall_zfs/kernel_scanner.py` | `kernel/scanner.rs` | `alpm` crate |
| `archinstall_zfs/installer.py` | `installer/mod.rs` | |
| `archinstall_zfs/initramfs/*.py` | `installer/initramfs/*` | |
| `archinstall_zfs/aur.py` | `installer/aur.rs` | |
| `archinstall_zfs/zrepl.py` | `zrepl.rs` | |
| `archinstall_zfs/assets/zed/*` | `zfs/cache.rs` (embedded const) | |
| `archinstall_zfs/utils/__init__.py` | `zfs/cache.rs` (rewrite_cache_mountpoints) | |
| `iso_builder.py` | `iso.rs` | minijinja replaces Jinja2 |

---

## 8. Notes for Claude Code

### Reference repos

- **`archinstall_zfs/`** — Python codebase being rewritten. Primary reference
  for business logic: `main.py`, `menu/`, `zfs/`, `disk/`.
- **`archinstall/`** — upstream library. Reference for `lib/installer.py`,
  `lib/pacman/`. We're replacing this, not wrapping it.
- **`/tmp/zfsbootmenu/`** — ZFSBootMenu source. Reference for `generate-zbm`,
  `config.yaml`, dracut/mkinitcpio modules, boot behavior, ZFS properties.

### Style preferences

- No excessive comments -- code should be self-documenting
- No emojis in code or output strings
- Professional, minimal formatting
- `thiserror` for domain errors, `color_eyre` at binary boundaries
- `&dyn CommandRunner` over generics for simplicity
- All command-executing functions take `&dyn CommandRunner`
- All file-writing functions take `&Path` target (testable with tempdir)
- Use `match` on `InitSystem` enum, not string comparisons
- Passwords via `run_with_stdin`, never in command args
- Split files at ~300 lines

### What NOT to implement

- Anything from archinstall not used by archinstall_zfs
- Async runtime
- i18n/translation
- Non-UEFI boot
- Bootloaders other than ZFSBootMenu
- LVM, LUKS, or btrfs support
- musl static binary (requires glibc for libalpm)
