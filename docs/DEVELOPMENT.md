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

Key build parameters:
- `kernel`: Target kernel (linux, linux-lts, linux-zen, linux-hardened)
- `use_precompiled_zfs`: Use precompiled ZFS packages vs DKMS
- `fast_build`: Minimal package set for development builds
- `include_headers`: Include kernel headers for DKMS builds

### Release process

Releases are automated via GitHub Actions:

1. **Monthly builds**: Automatically build fresh ISOs monthly
2. **Tag-based releases**: Create releases when new tags are pushed
3. **Compatibility checking**: Validate kernel/ZFS compatibility before building
4. **Artifact generation**: Upload ISO files as release artifacts

The CI pipeline handles fallback scenarios:
- If precompiled ZFS packages aren't available, automatically switch to DKMS
- Skip incompatible kernel versions with appropriate warnings
- Generate multiple ISO variants (different kernels, ZFS installation methods)
