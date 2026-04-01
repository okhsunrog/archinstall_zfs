# archinstall-zfs-rs — Architecture & Implementation Plan

> This document serves as the primary reference for implementing the Rust rewrite
> of archinstall_zfs. It should be given as context to Claude Code alongside access
> to the `archinstall` and `archinstall_zfs` Python repos for reference.

## 1. Project Overview

Complete rewrite of `archinstall_zfs` in Rust, dropping the Python `archinstall`
library dependency entirely. The result is a single static binary — a standalone
Arch Linux installer with first-class ZFS support and a ratatui-based TUI.

### Boundary: what's Rust, what stays Python

- **Rust** — everything that runs on the live ISO: the installer binary, TUI,
  all ZFS/disk/pacstrap/chroot logic, config handling, test harness.
- **Python/uv** — everything that runs on the dev machine before booting: ISO
  profile rendering (Jinja2 templates), `mkarchiso` orchestration, kernel/ZFS
  compatibility validation for ISO builds. Users never see Python.

### What we keep from the Python codebase (logic, not code)

- All ZFS-specific logic: pool/dataset creation, encryption, ZFSBootMenu, zrepl
- Installation flow: disk partitioning → pacstrap → chroot config → bootloader
- Config wizard UX (wizard steps, validation gates, preview)
- JSON config import/export for unattended installs (`--config` + `--silent`)
- AUR helper installation (yay-bin)
- dracut / mkinitcpio initramfs handlers
- Kernel/ZFS compatibility scanning with precompiled↔DKMS fallback
- QEMU test infrastructure (run-qemu.sh, test ISO builds, justfile recipes)

### What we drop

- `archinstall` library dependency (reimplemented as thin wrappers)
- Python profile system with dynamic module loading → static profile registry
- Plugin system
- Translation system (English-only for v1)
- LUKS/LVM/btrfs paths (ZFS-only installer)
- GRUB, rEFInd, Limine, efistub bootloaders (ZFSBootMenu only)

### What improves

- TUI: ratatui gives real-time progress, scrollable logs, split panes
- Type safety: serde structs with validation instead of pydantic + runtime casts
- Error handling: `Result<T, E>` chains instead of scattered try/except
- Single static binary (musl): no Python, no pip, no venv on the live ISO
- Sub-second startup
- ZFS JSON output (`-j` flag from OpenZFS 2.3+) replaces text parsing
- `alpm` crate replaces shelling out to `pacman -Si` for package queries
- `pacmanconf` crate replaces manual pacman.conf string manipulation

---

## 2. Crate Dependencies

```toml
[dependencies]
# TUI
ratatui = "0.29"
crossterm = "0.28"
tui-textarea = "0.7"          # text input (hostname, passwords, package lists)
tui-widget-list = "0.13"      # scrollable select list with mouse support
tui-scrollview = "0.6"        # scrollable log view for install progress
ratatui-macros = "0.6"        # layout boilerplate reduction

# Arch Linux
alpm = "4"                    # libalpm bindings — package version queries
pacmanconf = "3"              # parse/modify pacman.conf natively

# System
sysinfo = "0.34"              # CPU vendor (microcode), disk info, UEFI detection
blockdev = "0.3"              # lsblk --json parsing
nix = { version = "0.29", features = ["fs", "process", "signal"] }  # sync(), signals

# Config / CLI
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }

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
├── app.rs                       # Top-level state machine (Menu ↔ Install ↔ Done)
│
├── config/
│   ├── mod.rs
│   ├── types.rs                 # GlobalConfig + all sub-structs, enums (serde)
│   ├── validation.rs            # validate_for_install() → Vec<String>
│   └── io.rs                    # JSON load/save/merge
│
├── tui/
│   ├── mod.rs                   # Terminal init/restore, main event loop
│   ├── event.rs                 # Input event handling (key, mouse, resize)
│   ├── theme.rs                 # Color palette, style constants
│   ├── screens/
│   │   ├── mod.rs
│   │   ├── main_menu.rs         # Top-level config menu
│   │   ├── wizard.rs            # Storage & ZFS wizard (multi-step)
│   │   ├── locale.rs            # Locale/keyboard/language selection
│   │   ├── mirrors.rs           # Mirror region selection
│   │   ├── network.rs           # Network config (copy ISO / NetworkManager)
│   │   ├── auth.rs              # Root password + user accounts
│   │   ├── kernel.rs            # Kernel + ZFS mode selector
│   │   ├── profile.rs           # Desktop/server profile picker
│   │   ├── applications.rs      # Audio, bluetooth, power toggles
│   │   ├── packages.rs          # Additional pacman + AUR packages
│   │   ├── confirm.rs           # Pre-install summary + confirm
│   │   └── install_progress.rs  # Live log + progress bar
│   └── widgets/
│       ├── mod.rs
│       ├── password_input.rs    # Masked input with confirm step
│       └── info_dialog.rs       # Modal OK dialog
│       # tui-textarea handles text input
│       # tui-widget-list handles select lists
│       # tui-scrollview handles log view
│
├── system/
│   ├── mod.rs
│   ├── cmd.rs                   # CommandRunner trait + RealRunner impl
│   ├── pacman.rs                # pacstrap, pacman -Sy, db lock handling, repo config
│   ├── chroot.rs                # arch-chroot wrapper
│   ├── sysinfo.rs               # UEFI check, CPU vendor (microcode)
│   └── net.rs                   # Internet connectivity check
│
├── disk/
│   ├── mod.rs
│   ├── by_id.rs                 # Enumerate /dev/disk/by-id (disks vs partitions)
│   └── partition.rs             # sgdisk: zap, create GPT, EFI+ZFS+swap partitions
│
├── zfs/
│   ├── mod.rs                   # ZfsManager: setup_for_installation(), finish()
│   ├── cli.rs                   # run_zfs(), run_zpool() helpers (append -j for queries)
│   ├── models.rs                # Serde structs for ZFS JSON output
│   ├── pool.rs                  # zpool create/import/export/list/status
│   ├── dataset.rs               # zfs create/list/get/set/mount/umount
│   ├── encryption.rs            # Pool/dataset encryption, key file management
│   ├── kmod.rs                  # modprobe zfs, archzfs repo setup
│   ├── cache.rs                 # hostid, zfs-list.cache, misc file copies
│   └── bootmenu.rs              # ZFSBootMenu: download EFI images, efibootmgr
│
├── installer/
│   ├── mod.rs                   # Installer struct, orchestrates the full pipeline
│   ├── base.rs                  # pacstrap, set_mirrors, microcode detection
│   ├── locale.rs                # hostname, locale-gen, timezone, keyboard, NTP
│   ├── users.rs                 # useradd, passwd, sudoers
│   ├── services.rs              # systemctl enable in chroot
│   ├── network.rs               # Copy ISO network config / install NetworkManager
│   ├── initramfs/
│   │   ├── mod.rs               # InitramfsHandler trait
│   │   ├── dracut.rs            # Dracut config, hooks, generation
│   │   └── mkinitcpio.rs        # mkinitcpio config, hooks, generation
│   ├── aur.rs                   # Temp user, yay-bin, AUR package install
│   └── fstab.rs                 # genfstab + swap/crypttab entries
│
├── kernel/
│   ├── mod.rs                   # KernelInfo registry, package mapping, fallback
│   └── scanner.rs               # Compatibility scan via alpm (version queries)
│
├── swap/
│   └── mod.rs                   # zram-generator config, zswap partition setup
│
├── profile/
│   ├── mod.rs                   # ProfileRegistry, static PROFILES array
│   ├── desktop.rs               # Desktop profiles (gnome, plasma, sway, hyprland...)
│   └── server.rs                # Server profiles (sshd, docker, postgresql...)
│
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

pub struct CmdOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

pub struct RealRunner;         // std::process::Command — production
pub struct RecordingRunner;    // Records calls, returns canned responses — tests
```

Convenience wrappers:

```rust
pub fn chroot(runner: &dyn CommandRunner, target: &Path, cmd: &str) -> Result<CmdOutput>;
pub fn chroot_streaming(
    runner: &dyn CommandRunner, target: &Path, cmd: &str, tx: &Sender<String>,
) -> Result<CmdOutput>;
```

### 4.2 ZFS JSON integration

OpenZFS 2.3+ supports `-j` on query commands. We define serde models and
deserialize directly. The actual JSON schema must be captured from real
ZFS 2.3+ output as test fixtures — don't guess the schema.

Mutating commands (`zpool create`, `zfs create`, `zfs set`) don't output JSON.
For those we check exit code + stderr.

### 4.3 Config types

Direct mapping from `archinstall_zfs/menu/models.py` `GlobalConfig`. All enums
use serde string serialization. Validation returns `Vec<String>` of errors,
same as Python `validate_for_install()`.

### 4.4 Profile system

Static `&[Profile]` array replacing archinstall's dynamic Python module loading.
Each `archinstall/default_profiles/desktops/*.py` becomes a const entry.

### 4.5 TUI architecture

Screen-based state machine with ratatui immediate-mode rendering. Installation
runs in a background thread communicating via `mpsc` channels (log lines, phase
updates, completion).

### 4.6 Installation pipeline

Linear pipeline matching `archinstall_zfs/main.py::perform_installation()`:

```
Phase 1:  Disk preparation (sgdisk)
Phase 2:  ZFS pool + datasets + encryption
Phase 3:  Mount EFI partition
Phase 4:  pacstrap base system
Phase 5:  System config (hostname, locale, timezone, mirrors, network)
Phase 6:  archzfs repo on target + ZFS packages (precompiled→DKMS fallback)
Phase 7:  Initramfs (dracut or mkinitcpio)
Phase 8:  Users + authentication
Phase 9:  Profile packages + services
Phase 10: Additional packages + AUR packages
Phase 11: Swap configuration (zram or zswap partition)
Phase 12: ZFS services + genfstab + misc files
Phase 13: ZFSBootMenu bootloader
Phase 14: Cleanup (umount, zpool export)
```

---

## 5. Testing Strategy

### Unit tests (~80% of codebase)

Config validation, serde round-trips, ZFS JSON parsing with fixtures,
command construction with mock runner, file content generation with tempdir.

### Integration tests with mock CommandRunner

Full subsystem tests recording expected command sequences and verifying order.

### QEMU VM integration tests

Existing infrastructure: test ISO with SSH, headless QEMU with KVM, stable
`/dev/disk/by-id/virtio-archzfs-test-disk`. Fresh qcow2 + UEFI vars per test.
Run `--config <json> --silent`, check exit code, optionally verify boot.

### CI: split runners

GitHub-hosted for `cargo test`/`clippy`/`fmt`. Self-hosted with KVM for VM tests.

---

## 6. Python → Rust Mapping Reference

| Python source | Rust module | Notes |
|---|---|---|
| `archinstall/lib/command.py` | `system::cmd` | `CommandRunner` trait |
| `archinstall/lib/installer.py` | `installer/*` | Only used methods |
| `archinstall/lib/pacman/` | `system::pacman` + `pacmanconf` crate | |
| `archinstall/tui/` | `tui/*` | ratatui replacement |
| `archinstall/default_profiles/` | `profile/` | Static data |
| `archinstall_zfs/main.py` | `app.rs` + `installer/mod.rs` | |
| `archinstall_zfs/menu/global_config.py` | `tui/screens/main_menu.rs` + `wizard.rs` | |
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

---

## 7. Implementation Plan

Phases are ordered so each produces a working, testable deliverable.
A test VM should be running before starting (see Phase 0).

### Phase 0: Capture ZFS JSON fixtures from test VM

**Goal:** Collect real ZFS `-j` output to build serde models against.

**Prerequisites:** The developer runs `just build-test pre && just qemu-setup &&
just qemu-install-serial` before starting this phase. The test VM must be
accessible via `ssh -o StrictHostKeyChecking=no -p 2222 root@localhost`.

**Steps (all via SSH into the test VM):**

1. SSH into the VM
2. Create test pools on loopback files — both unencrypted and encrypted:
   ```bash
   truncate -s 1G /tmp/test.img
   zpool create testpool /tmp/test.img
   zfs create testpool/data
   zfs create testpool/data/home
   zfs set compression=lz4 testpool/data
   zfs set mountpoint=/mnt/test testpool/data
   ```
3. Capture query output for all commands we'll need to parse:
   ```bash
   zpool list -j          > /tmp/fixtures/zpool_list.json
   zpool status -j        > /tmp/fixtures/zpool_status.json
   zpool get -j all testpool > /tmp/fixtures/zpool_get_all.json
   zfs list -j            > /tmp/fixtures/zfs_list.json
   zfs list -j -t all     > /tmp/fixtures/zfs_list_all.json
   zfs get -j all testpool > /tmp/fixtures/zfs_get_all.json
   zfs get -j encryption testpool > /tmp/fixtures/zfs_get_encryption_off.json
   zfs mount -j           > /tmp/fixtures/zfs_mount.json
   ```
4. Export and capture importable pool listing:
   ```bash
   zpool export testpool
   zpool import -j        > /tmp/fixtures/zpool_import.json
   zpool import testpool
   ```
5. Create an encrypted pool variant and capture its properties:
   ```bash
   truncate -s 1G /tmp/test_enc.img
   echo "testpassword123" | zpool create \
       -O encryption=aes-256-gcm -O keyformat=passphrase \
       testpool_enc /tmp/test_enc.img
   zfs create testpool_enc/data
   zfs get -j encryption,keystatus,keyformat testpool_enc \
       > /tmp/fixtures/zfs_get_encrypted.json
   zpool status -j testpool_enc > /tmp/fixtures/zpool_status_encrypted.json
   ```
6. Cleanup test pools, copy all fixtures to the host project:
   ```bash
   zpool destroy testpool_enc
   zpool destroy testpool
   ```
   Then from the host: `scp -P 2222 -r root@localhost:/tmp/fixtures/ tests/fixtures/`

**Deliverable:** `tests/fixtures/` directory with real JSON output from every
ZFS/zpool query command. These files drive the serde model definitions in Phase 2
and serve as permanent test fixtures.

### Phase 1: Project skeleton + core infrastructure

**Goal:** Compilable project, one working TUI screen, config validation tests.

1. `cargo init`, `Cargo.toml` with all dependencies
2. `system::cmd` — `CommandRunner` trait, `RealRunner`, `CmdOutput`
3. `config::types` — all enums + `GlobalConfig` with serde + defaults
4. `config::validation` — port `validate_for_install()` from Python
5. `config::io` — JSON load/save
6. `main.rs` — clap CLI: `--config`, `--silent`, `--dry-run`
7. `tui/mod.rs` — terminal init/restore, event loop
8. `tui/theme.rs` — color palette
9. `tui/screens/main_menu.rs` — renders menu, stubs for navigation
10. Unit tests: config serde round-trip, validation errors

### Phase 2: System + disk + ZFS (no TUI)

**Goal:** Core operations work with mock runner.

1. `system::pacman` — pacstrap, sync_db, archzfs repo
2. `system::chroot`, `sysinfo`, `net`
3. `disk::by_id` + `disk::partition`
4. `zfs::models` — serde structs derived from the Phase 0 fixture files.
   Write the structs, then `#[test]` that every fixture deserializes cleanly.
5. `zfs::cli`, `pool`, `dataset`, `encryption`, `cache`, `bootmenu`, `kmod`

All tested with `RecordingRunner` + JSON fixtures from Phase 0.

### Phase 3: Installer pipeline

**Goal:** Full install logic, end-to-end testable with mocks.

1. `installer/mod.rs` — `Installer` + `perform_installation()`
2. `installer/base.rs`, `locale.rs`, `users.rs`, `services.rs`, `network.rs`
3. `installer/initramfs/` — trait + dracut + mkinitcpio
4. `installer/aur.rs`, `fstab.rs`
5. `kernel/` — package mapping, scanner (alpm), fallback
6. `swap/`, `profile/`, `zrepl.rs`

### Phase 4: TUI screens

**Goal:** All screens functional, wired to config state.

1. Custom widgets (password_input, info_dialog)
2. Storage wizard, all config screens
3. Confirm + install progress screens
4. Wire navigation + background install thread

### Phase 5: Integration + polish

**Goal:** VM-tested, CI-ready.

1. QEMU test harness + configs
2. CI workflow (split runners)
3. Error recovery, edge cases
4. `--dry-run` mode, musl static binary
5. Documentation

---

## 8. Notes for Claude Code

### Test VM access

A QEMU test VM is running with the archzfs live ISO. Access it via:
```
ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2222 root@localhost
```
Empty root password. The VM has ZFS tools installed and a 20G virtio disk at
`/dev/disk/by-id/virtio-archzfs-test-disk`. Use it to:
- Capture ZFS `-j` JSON output for test fixtures (Phase 0)
- Test commands and verify JSON schemas
- Create/destroy test pools on loopback files

### Reference repos

- **`archinstall_zfs/`** — Python codebase being rewritten. Primary reference
  for business logic: `main.py`, `menu/`, `zfs/`, `disk/`.
- **`archinstall/`** — upstream library. Reference for `lib/installer.py`,
  `lib/command.py`, `lib/pacman/`. We're replacing this, not wrapping it.

### Style preferences

- No excessive comments — code should be self-documenting
- No emojis in code or output strings
- Professional, minimal formatting
- `thiserror` for domain errors, `color_eyre` at binary boundaries
- `&dyn CommandRunner` over generics for simplicity
- All command-executing functions take `&dyn CommandRunner`
- All file-writing functions take `&Path` target (testable with tempdir)
- Split files at ~300 lines

### What NOT to implement

- Anything from archinstall not used by archinstall_zfs
- Async runtime
- i18n/translation
- Non-UEFI boot
- Bootloaders other than ZFSBootMenu
- LVM, LUKS, or btrfs support
