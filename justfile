# Variables
GEN_ISO_DIR := "gen_iso"
QEMU_SCRIPT := "gen_iso/run-qemu.sh"
ISO_OUT_DIR := "gen_iso/out"
DISK_IMAGE := "gen_iso/arch.qcow2"
UEFI_VARS := "gen_iso/my_vars.fd"
MAIN_PROFILE_DIR := "gen_iso/main_profile"
TESTING_PROFILE_DIR := "gen_iso/testing_profile"
TESTING_ISO_PATH := "gen_iso/out/archzfs-testing-x86_64.iso"
ISO_WORK_DIR := "/tmp/archiso-tmp"

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
    rm -rf {{ISO_WORK_DIR}}

# Clean up ISO build artifacts only
clean-iso:
    sudo rm -rf {{ISO_WORK_DIR}}

# Install development dependencies
install-dev:
    uv pip install -e .[dev]

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

# Build the main ISO for production release
build-main-iso:
    @echo "Building main ISO from 'releng' profile..."
    sudo mkarchiso -v -r -w {{ISO_WORK_DIR}} -o {{ISO_OUT_DIR}} {{MAIN_PROFILE_DIR}}

# Build the testing ISO for QEMU
build-testing-iso:
    @echo "Building testing ISO from 'baseline' profile..."
    sudo mkarchiso -v -r -w {{ISO_WORK_DIR}} -o {{ISO_OUT_DIR}} {{TESTING_PROFILE_DIR}}

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

# Install Arch Linux in QEMU with GUI from the generated testing ISO
qemu-install:
    @if [ ! -f {{DISK_IMAGE}} ]; then just qemu-create-disk; fi
    @if [ ! -f {{UEFI_VARS}} ]; then just qemu-setup-uefi; fi
    @if [ ! -f {{TESTING_ISO_PATH}} ]; then echo "Testing ISO not found. Run 'just build-testing-iso' first."; exit 1; fi
    {{QEMU_SCRIPT}} -i {{TESTING_ISO_PATH}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Install Arch Linux in QEMU with serial console from the generated testing ISO
qemu-install-serial:
    @if [ ! -f {{DISK_IMAGE}} ]; then just qemu-create-disk; fi
    @if [ ! -f {{UEFI_VARS}} ]; then just qemu-setup-uefi; fi
    @if [ ! -f {{TESTING_ISO_PATH}} ]; then echo "Testing ISO not found. Run 'just build-testing-iso' first."; exit 1; fi
    {{QEMU_SCRIPT}} -i {{TESTING_ISO_PATH}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}} -S

# Run existing Arch Linux installation in QEMU with GUI
qemu-run:
    @if [ ! -f {{DISK_IMAGE}} ]; then echo "Disk image not found. Run 'just qemu-install' first."; exit 1; fi
    {{QEMU_SCRIPT}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Run existing Arch Linux installation in QEMU with serial console
qemu-run-serial:
    @if [ ! -f {{DISK_IMAGE}} ]; then echo "Disk image not found. Run 'just qemu-install' first."; exit 1; fi
    {{QEMU_SCRIPT}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}} -S