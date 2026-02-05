# Development Guide

This document covers development workflows, architecture details, and contribution guidelines for archinstall_zfs.

---

## Development 🛠️

### Building custom ISOs

Prerequisites (Arch Linux host):
```bash
sudo pacman -S qemu-desktop edk2-ovmf archiso grub just rsync uv
just install-dev  # Install dev dependencies
```

Build commands:
```bash
# Production ISOs
just build-main pre              # Precompiled ZFS + linux-lts
just build-main dkms linux       # DKMS + linux kernel

# Development ISOs (faster builds)
just build-test pre              # Minimal package set for testing
just build-test dkms linux-zen   # Test with zen kernel

just list-isos                   # Show built ISOs
```

### QEMU testing workflow

Quick loop:
```bash
just qemu-setup                  # Create test disk + UEFI vars
just build-test pre              # Build minimal testing ISO
just qemu-install-serial         # Boot with serial console

# In another terminal:
just ssh                         # Sync source code and connect
./installer                      # Run the installer
```

Other QEMU commands:
```bash
just qemu-install                # GUI install flow
just qemu-run                    # Boot existing installation
just qemu-refresh                # Reset test environment
```

### Quality checks
```bash
just format                      # Format code (ruff)
just lint                        # Lint and auto‑fix
just type-check                  # MyPy type checking
just test                        # Run tests
just all                         # All checks
```

---

## Architecture notes 💡

### Code architecture and design

archinstall_zfs uses archinstall as a library while implementing custom components for ZFS-specific functionality:

**Core philosophy**: Leverage archinstall's strengths (TUI components, configuration system, package installation) while replacing parts that don't understand ZFS.

**Key custom components:**

- **`ZFSInstaller`** (inherits from `archinstall.Installer`)
  - Adds ZFS-specific packages (kernel modules, utilities)
  - Handles initramfs generation (dracut or mkinitcpio with ZFS hooks)
  - Configures ZFS-specific services and kernel parameters
  - Manages pool creation and dataset layout

- **`DiskManager`** (custom implementation)
  - Replaces archinstall's disk management entirely
  - Uses `sgdisk` for precise partition creation
  - Handles GPT/MBR signature cleanup
  - Manages EFI, swap, and ZFS partition layouts

- **`GlobalConfigMenu`** (custom TUI)
  - Completely custom menu replacing archinstall's standard flow
  - ZFS-specific options (pool names, encryption, compression)
  - Kernel/ZFS compatibility validation integration
  - Installation mode selection and validation

- **Configuration system** (Pydantic models)
  - Type-safe configuration handling
  - Replaces archinstall's dict-based approach for ZFS settings
  - Validation at the model level prevents runtime errors

**Integration points with archinstall:**
- `SelectMenu`, `EditMenu` - Reused for consistent TUI experience
- Package installation system - Leveraged for base system setup
- Profile system - Extended for ZFS-aware profiles
- Service management - Used for systemd service configuration

### Templating system
Jinja2 templates are used to generate ISO profiles:

Variables:
- `kernel`: target kernel variant
- `use_precompiled_zfs` / `use_dkms`: ZFS installation method
- `include_headers`: include kernel headers
- `fast_build`: minimal vs full package set

Key templates:
- `packages.x86_64.j2` → Package selection
- `profiledef.sh.j2` → ISO metadata
- `pacman.conf.j2` → Repository configuration

### Task runner (just)
Workflows are orchestrated via [`just`](https://github.com/casey/just) recipes:

```bash
just --list                      # See available commands
just build-main pre linux-zen    # Parameterized builds
just qemu-install-serial         # Serial‑console QEMU setup
```

---

## Contributing 🤝

We welcome issues and pull requests.

Development flow:
```bash
git clone https://github.com/okhsunrog/archinstall_zfs
cd archinstall_zfs
just install-dev                 # Install dependencies
just qemu-setup                  # Set up test environment
# Make changes
just all                         # Run quality checks
just qemu-install-serial         # Test in VM
```

### Code style and quality

The project uses several tools to maintain code quality:

- **Formatting**: `ruff` for code formatting
- **Linting**: `ruff` for linting and auto-fixing issues
- **Type checking**: `mypy` for static type analysis
- **Testing**: `pytest` for unit tests

All checks can be run with `just all`, or individually:
- `just format` - Format code
- `just lint` - Run linting
- `just type-check` - Run type checking
- `just test` - Run tests

### Project structure

```
archinstall_zfs/
├── archinstall_zfs/           # Main package
│   ├── __main__.py           # Entry point
│   ├── main.py               # Main installer logic
│   ├── installer.py          # ZFS-specific installer
│   ├── menu/                 # TUI menus
│   ├── disk/                 # Disk management
│   ├── initramfs/            # Initramfs configuration
│   └── zfs/                  # ZFS utilities
├── gen_iso/                  # ISO generation
│   ├── profile/              # archiso profile templates
│   └── run-qemu.sh          # QEMU testing script
├── tests/                    # Test suite
├── docs/                     # Documentation
└── justfile                  # Task definitions
```

### Testing

The project includes several types of tests:

1. **Unit tests** - Test individual components in isolation
2. **Integration tests** - Test component interactions
3. **Validation tests** - Test kernel/ZFS compatibility validation

Run tests with:
```bash
just test                        # Run all tests
python -m pytest tests/         # Run tests directly
python -m pytest tests/test_validation.py -v  # Run specific test file
```

### ISO building process

The ISO building process uses Jinja2 templates to generate archiso profiles:

1. **Template rendering**: `iso_builder.py` processes templates with build parameters
2. **Profile generation**: Creates archiso profile in `gen_iso/out/`
3. **ISO creation**: Uses `mkarchiso` to build the final ISO
4. **Validation**: Checks kernel/ZFS compatibility before building

**Build variants explained:**

**`build-main` (Production ISOs)**
- **Purpose**: Release-quality ISOs for end users
- **Package set**: Full package selection including diagnostic tools, network utilities, editors, development tools
- **Compression**: `squashfs` for optimal size reduction
- **Boot behavior**: Standard boot timeouts and behavior
- **Target**: End-user installations and releases
- **Build time**: Longer due to comprehensive package set

**`build-test` (Development ISOs)**
- **Purpose**: Fast iteration for development and testing
- **Package set**: Minimal packages (base system + installer only)
- **Compression**: `erofs` for faster compression (larger files, faster builds)
- **Boot behavior**: 
  - Disabled boot timeouts for immediate testing
  - Serial console output enabled for QEMU debugging
  - SSH server auto-started with root login enabled
- **Target**: QEMU testing, development iteration
- **Build time**: Much faster due to minimal packages

**Key build parameters:**
- `kernel`: Target kernel (linux, linux-lts, linux-zen, linux-hardened)
- `use_precompiled_zfs`: Use precompiled ZFS packages vs DKMS
- `fast_build`: Toggles between full (`build-main`) and minimal (`build-test`) package sets
- `include_headers`: Include kernel headers for DKMS builds

**Template system:**
- `packages.x86_64.j2` - Conditional package inclusion based on `fast_build`
- `profiledef.sh.j2` - Build flags, compression settings, file permissions
- `pacman.conf.j2` - Repository configuration and package sources

### Release process

Releases are automated via GitHub Actions with sophisticated fallback handling:

**Automated build triggers:**
1. **Monthly builds**: Scheduled builds on the 4th of each month at 4:30 AM UTC to ensure fresh packages
2. **Tag-based releases**: Automatic release creation when version tags (v*) are pushed
3. **Manual triggers**: Can be triggered manually with kernel selection options and optional release creation

**Build process and validation:**

1. **Smart compatibility validation**: 
   - Tests precompiled ZFS compatibility first by attempting a validation build
   - Falls back to DKMS if precompiled modules are incompatible
   - Uses `iso_builder.py` validation runs to determine the best ZFS installation method

2. **Multi-kernel building**:
   - **linux kernel**: Built with smart fallback (precompiled → DKMS → skip if incompatible)
   - **linux-lts kernel**: Built with smart fallback (precompiled → DKMS → fail if neither works)
   - Uses `continue-on-error: true` for linux kernel to allow partial failures

3. **Fallback logic**:
   - linux kernel build can fail without blocking the release (expected when kernel is too new)
   - linux-lts kernel build must succeed (fails the entire workflow if both precompiled and DKMS fail)
   - Detailed logging explains which ZFS method was selected and why

4. **Build artifacts**:
   - Generates SHA256 checksums for each built ISO
   - Lists ISO files with sizes for verification
   - Uploads both ISOs and checksum files to GitHub releases

**Release artifacts:**
- ISO files for each successfully built kernel variant
- SHA256 checksum files for integrity verification
- Automated release notes (for tag-based releases) or descriptive titles

**Failure handling:**
- linux kernel builds can fail gracefully without blocking releases (common when kernel is too new for ZFS)
- linux-lts builds must succeed or the entire workflow fails (ensures at least one working ISO)
- Detailed console output shows exactly which ZFS method was selected and why
- Manual workflow dispatch allows testing specific kernel combinations

**Release types:**
- **Tag releases**: Created from version tags (v*) with full release notes
- **Monthly releases**: Automated builds on the 4th of each month with latest packages
- **Manual releases**: Created via workflow dispatch, marked as draft/prerelease for testing

This approach ensures users always have at least one working ISO (linux-lts), while allowing the latest linux kernel to be skipped when ZFS compatibility issues arise.

---

## TODO / Future improvements

- **Host-to-target (H2T) mode support**: archinstall 3.0.15+ exposes `running_from_host()` to detect if running from an installed system vs ISO. Could be used to:
  - Warn users about H2T mode limitations
  - Skip live-ISO-only operations
  - Better handle ZFS module availability (host might have them pre-loaded)
