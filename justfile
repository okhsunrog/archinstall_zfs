# Variables
GEN_ISO_DIR := "gen_iso"
QEMU_SCRIPT := "gen_iso/run-qemu.sh"
ISO_OUT_DIR := "gen_iso/out"
DISK_IMAGE := "gen_iso/arch.qcow2"
UEFI_VARS := "gen_iso/my_vars.fd"
MAIN_PROFILE_DIR := "gen_iso/profile"
ISO_WORK_DIR := "/tmp/archiso-tmp"
TEMP_PROFILE_DIR := "/tmp/archzfs-profile"

# Default recipe to display available commands
default:
    @just --list

# Format code with ruff
format:
    uv run ruff format .

# Lint and auto-fix with ruff
lint:
    uv run ruff check --fix .

# Type check with mypy
type-check:
    uv run mypy .

# Run tests with pytest
test:
    uv run pytest

# Run all quality checks
all: format lint type-check test

# Clean up cache and build artifacts
clean:
    rm -rf .mypy_cache/
    rm -rf .pytest_cache/
    rm -rf .ruff_cache/
    rm -rf htmlcov/
    rm -rf build/
    rm -rf dist/
    rm -rf *.egg-info/
    find . -type d -name __pycache__ -exec rm -rf {} +
    find . -type f -name "*.pyc" -delete
    sudo rm -rf {{ISO_WORK_DIR}}

# Clean up ISO build artifacts only
clean-iso:
    sudo rm -rf {{ISO_OUT_DIR}}
    sudo rm -rf {{ISO_WORK_DIR}}
    sudo rm -rf {{TEMP_PROFILE_DIR}}

# Install development dependencies
install-dev:
    uv pip install -e '.[dev]'

# Setup development environment
setup: install-dev
    @echo "Development environment setup complete!"
    @echo "Run 'just all' to check code quality"

# Run a quick check (format + lint only)
quick: format lint

# Run tests with coverage report
test-cov:
    uv run pytest --cov=archinstall_zfs --cov-report=html --cov-report=term-missing

# Run tests with coverage report (XML format for CI)
test-cov-xml:
    uv run pytest --cov=archinstall_zfs --cov-report=xml --cov-report=html --cov-report=term-missing

# Check code without making changes
check:
    uv run ruff format --check .
    uv run ruff check .
    uv run mypy .

# Run all checks for CI
ci-check: check test

# ISO and QEMU Recipes

# Prepare source code for inclusion in ISO
_prepare-source PROFILE_DIR:
    @echo "Preparing archinstall_zfs source code for {{PROFILE_DIR}}..."
    @mkdir -p {{PROFILE_DIR}}/airootfs/root
    @rsync -a --exclude='.git' --exclude='__pycache__' --exclude='.pytest_cache' \
        --exclude='.mypy_cache' --exclude='.ruff_cache' --exclude='build' \
        --exclude='dist' --exclude='*.egg-info' --exclude='gen_iso' \
        --exclude='tests' archinstall_zfs {{PROFILE_DIR}}/airootfs/root/archinstall_zfs/
    @cp pyproject.toml README.md LICENSE {{PROFILE_DIR}}/airootfs/root/archinstall_zfs/
    @echo '#!/bin/bash' > {{PROFILE_DIR}}/airootfs/root/installer
    @echo 'set -euo pipefail' >> {{PROFILE_DIR}}/airootfs/root/installer
    @echo 'cd /root/archinstall_zfs' >> {{PROFILE_DIR}}/airootfs/root/installer
    @echo 'exec python -m archinstall_zfs' >> {{PROFILE_DIR}}/airootfs/root/installer
    @chmod +x {{PROFILE_DIR}}/airootfs/root/installer

# Clean up source code copy
_cleanup-source PROFILE_DIR:
    @echo "Cleaning up source code copy from {{PROFILE_DIR}}..."
    @rm -rf {{PROFILE_DIR}}/airootfs/root/archinstall_zfs
    @rm -f {{PROFILE_DIR}}/airootfs/root/installer

render-main-profile PRECOMPILED KERNEL HEADERS FAST:
    @echo "Rendering main profile (precompiled={{PRECOMPILED}}, kernel={{KERNEL}}, headers={{HEADERS}}, fast={{FAST}}) into {{TEMP_PROFILE_DIR}}..."
    @rm -rf {{TEMP_PROFILE_DIR}}
    @ZFS_MODE="precompiled"; if [ "{{PRECOMPILED}}" != "true" ]; then ZFS_MODE="dkms"; fi; \
    FAST_FLAG=""; if [ "{{FAST}}" = "true" ]; then FAST_FLAG="--fast"; fi; \
    uv run python archinstall_zfs/builder.py --profile-dir {{MAIN_PROFILE_DIR}} --out-dir {{TEMP_PROFILE_DIR}} --kernel "{{KERNEL}}" --zfs "$ZFS_MODE" --headers "{{HEADERS}}" $FAST_FLAG

# Build main ISO (parametric)
# Usage: just build-main [pre|dkms] [linux|linux-lts|linux-zen]
build-main MODE="pre" KERNEL="linux-lts":
    @echo "Building main ISO (mode={{MODE}}, kernel={{KERNEL}})"
    @just _prepare-source {{MAIN_PROFILE_DIR}}
    @PRE="true"; HEAD="auto"; if [ "{{MODE}}" = "dkms" ]; then PRE="false"; HEAD="true"; fi; \
    just render-main-profile $PRE {{KERNEL}} $HEAD false
    @echo "Building main ISO from rendered profile..."
    sudo mkarchiso -v -r -w {{ISO_WORK_DIR}} -o {{ISO_OUT_DIR}} {{TEMP_PROFILE_DIR}}
    @just _cleanup-source {{MAIN_PROFILE_DIR}}

# Build testing ISO (parametric)
# Usage: just build-test [pre|dkms] [linux|linux-lts|linux-zen]
build-test MODE="pre" KERNEL="linux-lts":
    @echo "Building testing ISO (mode={{MODE}}, kernel={{KERNEL}})"
    @just _prepare-source {{MAIN_PROFILE_DIR}}
    @PRE="true"; HEAD="auto"; if [ "{{MODE}}" = "dkms" ]; then PRE="false"; HEAD="true"; fi; \
    just render-main-profile $PRE {{KERNEL}} $HEAD true
    @echo "Building testing ISO from rendered profile..."
    sudo mkarchiso -v -r -w {{ISO_WORK_DIR}} -o {{ISO_OUT_DIR}} {{TEMP_PROFILE_DIR}}
    @just _cleanup-source {{MAIN_PROFILE_DIR}}

# Back-compat wrappers (deprecated)
build-main-iso:
    @echo "[DEPRECATION] Use: just build-main [pre|dkms] [kernel]"
    @just build-main pre linux-lts

build-main-iso-dkms:
    @echo "[DEPRECATION] Use: just build-main [pre|dkms] [kernel]"
    @just build-main dkms linux-lts

build-testing-iso:
    @echo "[DEPRECATION] Use: just build-test [pre|dkms] [kernel]"
    @just build-test pre linux-lts

build-testing-iso-dkms:
    @echo "[DEPRECATION] Use: just build-test [pre|dkms] [kernel]"
    @just build-test dkms linux-lts

# List available ISO files
list-isos:
    @echo "Available ISO files:"
    @ls -lh {{ISO_OUT_DIR}}/*.iso 2>/dev/null || echo "No ISO files found in {{ISO_OUT_DIR}}"


# Create disk image for QEMU
qemu-create-disk:
    @mkdir -p {{GEN_ISO_DIR}}
    qemu-img create -f qcow2 {{DISK_IMAGE}} 20G

# Setup UEFI vars for QEMU
qemu-setup-uefi:
    @mkdir -p {{GEN_ISO_DIR}}
    @OVMF_VARS_PATH=`find /usr/share/edk2 /usr/share/edk2-ovmf /usr/share/OVMF -name "OVMF_VARS*.fd" -print -quit`; \
     if [ -z "$OVMF_VARS_PATH" ]; then \
         echo "Error: OVMF_VARS.fd not found. Please install edk2-ovmf."; \
         exit 1; \
     fi; \
     cp "$OVMF_VARS_PATH" {{UEFI_VARS}}
    @echo "UEFI vars file created at {{UEFI_VARS}}"

# Reset UEFI vars to factory defaults (fixes boot issues)
qemu-reset-uefi:
    @echo "Resetting UEFI vars to factory defaults..."
    @if [ -f {{UEFI_VARS}} ]; then \
        echo "Removing existing UEFI vars file: {{UEFI_VARS}}"; \
        rm {{UEFI_VARS}}; \
    fi
    @just qemu-setup-uefi
    @echo "UEFI vars reset complete. ISO should now boot correctly."

# A setup recipe for qemu
qemu-setup: qemu-create-disk qemu-setup-uefi
    @echo "QEMU environment is set up."
    @echo "Now run 'just build-testing-iso'"
    @echo "Then 'just qemu-install' or 'just qemu-install-serial'"

# Remove existing QEMU artifacts and set up fresh ones
qemu-refresh:
    @echo "Refreshing QEMU disk image and UEFI vars..."
    @if [ -f {{DISK_IMAGE}} ]; then \
        echo "Removing existing disk image: {{DISK_IMAGE}}"; \
        rm -f {{DISK_IMAGE}}; \
    fi
    @if [ -f {{UEFI_VARS}} ]; then \
        echo "Removing existing UEFI vars file: {{UEFI_VARS}}"; \
        rm -f {{UEFI_VARS}}; \
    fi
    @just qemu-setup
    @echo "QEMU refresh complete."

# Install Arch Linux in QEMU with GUI from the generated testing ISO
qemu-install:
    @if [ ! -f {{DISK_IMAGE}} ]; then just qemu-create-disk; fi
    @if [ ! -f {{UEFI_VARS}} ]; then just qemu-setup-uefi; fi
    @ISO_PATH=$(ls -1t {{ISO_OUT_DIR}}/archzfs-*-testing-*.iso 2>/dev/null | head -n 1); \
      if [ -z "$ISO_PATH" ]; then \
        echo "Testing ISO not found in {{ISO_OUT_DIR}}. Run 'just build-testing-iso' first."; \
        exit 1; \
      fi; \
      echo "Using testing ISO: $ISO_PATH"; \
    {{QEMU_SCRIPT}} -i "$ISO_PATH" -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Install Arch Linux in QEMU with serial console from the generated testing ISO
qemu-install-serial:
    @if [ ! -f {{DISK_IMAGE}} ]; then just qemu-create-disk; fi
    @if [ ! -f {{UEFI_VARS}} ]; then just qemu-setup-uefi; fi
    @ISO_PATH=$(ls -1t {{ISO_OUT_DIR}}/archzfs-*-testing-*.iso 2>/dev/null | head -n 1); \
      if [ -z "$ISO_PATH" ]; then \
        echo "Testing ISO not found in {{ISO_OUT_DIR}}. Run 'just build-testing-iso' first."; \
        exit 1; \
      fi; \
      echo "Using testing ISO: $ISO_PATH"; \
      {{QEMU_SCRIPT}} -i "$ISO_PATH" -D {{DISK_IMAGE}} -U {{UEFI_VARS}} -S

# Run existing Arch Linux installation in QEMU with GUI
qemu-run:
    @if [ ! -f {{DISK_IMAGE}} ]; then echo "Disk image not found. Run 'just qemu-install' first."; exit 1; fi
    {{QEMU_SCRIPT}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Run existing Arch Linux installation in QEMU with serial console
qemu-run-serial:
    @if [ ! -f {{DISK_IMAGE}} ]; then echo "Disk image not found. Run 'just qemu-install' first."; exit 1; fi
    {{QEMU_SCRIPT}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}} -S

# Sync source code and SSH into running QEMU VM
ssh:
    @echo "Syncing archinstall_zfs repo to VM (incremental, in-place)..."
    @ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@localhost -p 2222 "mkdir -p /root/archinstall_zfs/archinstall_zfs"
    @if rsync -av --delete -e "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2222" \
        --exclude='.git' --exclude='__pycache__' --exclude='.pytest_cache' \
        --exclude='.mypy_cache' --exclude='.ruff_cache' --exclude='build' \
        --exclude='dist' --exclude='*.egg-info' --exclude='gen_iso' \
        --exclude='tests' archinstall_zfs/ root@localhost:/root/archinstall_zfs/archinstall_zfs/ 2>/dev/null \
        && rsync -av -e "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2222" \
        pyproject.toml README.md LICENSE root@localhost:/root/archinstall_zfs/ 2>/dev/null; then \
        echo "Source code synced with rsync!"; \
    else \
        echo "rsync not available in VM, using scp fallback..."; \
        just ssh-scp; \
    fi
    @echo "Connecting to VM..."
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@localhost -p 2222

# Sync source code using scp (fallback when rsync not available)
ssh-scp:
    @echo "Syncing source code using scp (in-place)..."
    @ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@localhost -p 2222 "mkdir -p /root/archinstall_zfs/archinstall_zfs"
    scp -r -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2222 archinstall_zfs/* root@localhost:/root/archinstall_zfs/archinstall_zfs/
    scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2222 pyproject.toml README.md LICENSE root@localhost:/root/archinstall_zfs/

# SSH into running QEMU VM without syncing source code
ssh-only:
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@localhost -p 2222
