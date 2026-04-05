set shell := ["bash", "-euo", "pipefail", "-c"]

# Paths
PROFILE_DIR := "gen_iso/profile"
PROFILE_OUT := "gen_iso/profile_rendered"
ISO_OUT := "gen_iso/out"
DISK_IMAGE := "gen_iso/arch.qcow2"
UEFI_VARS := "gen_iso/my_vars.fd"
QEMU_SCRIPT := "gen_iso/run-qemu.sh"
BINARY := "target/release/archinstall-zfs-tui"
BINARY_SLINT := "target/release/archinstall-zfs-slint"

# ─── Build ──────────────────────────────────────────────

# Build installer binaries (release)
build:
    cargo build --release -p archinstall-zfs-tui -p archinstall-zfs-slint

# Run cargo tests
test:
    cargo test --workspace

# Run clippy
lint:
    cargo clippy --workspace -- -D warnings

# Format check
fmt-check:
    cargo fmt --all -- --check

# Format
fmt:
    cargo fmt --all

# All checks
check: fmt-check lint test

# ─── ISO Building ──────────────────────────────────────

# Internal: render profile templates
_render-profile MODE="precompiled" KERNEL="linux-lts" FAST="":
    cargo run --release -p archinstall-zfs-tui -- render-profile \
        --profile-dir {{PROFILE_DIR}} \
        --out-dir {{PROFILE_OUT}} \
        --kernel {{KERNEL}} \
        --zfs {{MODE}} \
        {{FAST}}

# Internal: copy installer binaries into rendered profile
_prepare-binary:
    @mkdir -p {{PROFILE_OUT}}/airootfs/usr/local/bin/
    install -m 0755 {{BINARY}} {{PROFILE_OUT}}/airootfs/usr/local/bin/archinstall-zfs-tui
    @if [ -f {{BINARY_SLINT}} ]; then \
        install -m 0755 {{BINARY_SLINT}} {{PROFILE_OUT}}/airootfs/usr/local/bin/archinstall-zfs-slint; \
    fi

# Build production ISO
# Usage: just build-main [pre|dkms] [linux|linux-lts|linux-zen]
build-main MODE="precompiled" KERNEL="linux-lts":
    @echo "Building production ISO (mode={{MODE}}, kernel={{KERNEL}})"
    just build
    just _render-profile {{MODE}} {{KERNEL}}
    just _prepare-binary
    @echo "Building ISO..."
    sudo mkarchiso -v -w "gen_iso/workdir" -o {{ISO_OUT}} {{PROFILE_OUT}}
    @echo "ISO built in {{ISO_OUT}}"

# Build testing ISO (fast, minimal packages, serial+SSH enabled)
# Usage: just build-test [pre|dkms] [linux|linux-lts|linux-zen]
build-test MODE="precompiled" KERNEL="linux-lts":
    @echo "Building testing ISO (mode={{MODE}}, kernel={{KERNEL}})"
    just build
    just _render-profile {{MODE}} {{KERNEL}} "--fast"
    just _prepare-binary
    @echo "Building ISO..."
    sudo mkarchiso -v -w "gen_iso/workdir" -o {{ISO_OUT}} {{PROFILE_OUT}}
    @echo "Testing ISO built in {{ISO_OUT}}"

# List available ISOs
list-isos:
    @ls -lht {{ISO_OUT}}/*.iso 2>/dev/null || echo "No ISOs found. Run 'just build-test' first."

# Clean ISO build artifacts
clean-iso:
    rm -rf {{PROFILE_OUT}} gen_iso/workdir
    @echo "ISO build artifacts cleaned"

# ─── QEMU Setup ────────────────────────────────────────

# Create fresh 20G qcow2 disk
qemu-create-disk:
    qemu-img create -f qcow2 {{DISK_IMAGE}} 20G

# Copy OVMF UEFI variables file
qemu-setup-uefi:
    #!/usr/bin/env bash
    OVMF_VARS=$(find /usr/share/edk2 /usr/share/edk2-ovmf /usr/share/OVMF -name "OVMF_VARS*.4m.fd" ! -name "*secboot*" -print -quit 2>/dev/null)
    if [[ -z "$OVMF_VARS" ]]; then echo "ERROR: OVMF_VARS.fd not found. Install edk2-ovmf."; exit 1; fi
    cp "$OVMF_VARS" {{UEFI_VARS}}
    echo "UEFI vars file created at {{UEFI_VARS}}"

# Full QEMU setup (disk + UEFI vars)
qemu-setup: qemu-create-disk qemu-setup-uefi
    @echo "QEMU setup complete."

# Delete and recreate QEMU disk + UEFI vars
qemu-refresh:
    rm -f {{DISK_IMAGE}} {{UEFI_VARS}}
    just qemu-setup
    @echo "QEMU refresh complete."

# ─── QEMU Execution ───────────────────────────────────

# Boot latest testing ISO in QEMU with GUI
qemu-install:
    #!/usr/bin/env bash
    if [[ ! -f {{DISK_IMAGE}} ]]; then just qemu-create-disk; fi
    if [[ ! -f {{UEFI_VARS}} ]]; then just qemu-setup-uefi; fi
    ISO=$(ls -1t {{ISO_OUT}}/archzfs-*-testing-*.iso 2>/dev/null | head -n1)
    if [[ -z "$ISO" ]]; then echo "No testing ISO found. Run 'just build-test'."; exit 1; fi
    bash {{QEMU_SCRIPT}} -i "$ISO" -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Boot latest testing ISO in QEMU with serial console
qemu-install-serial:
    #!/usr/bin/env bash
    if [[ ! -f {{DISK_IMAGE}} ]]; then just qemu-create-disk; fi
    if [[ ! -f {{UEFI_VARS}} ]]; then just qemu-setup-uefi; fi
    ISO=$(ls -1t {{ISO_OUT}}/archzfs-*-testing-*.iso 2>/dev/null | head -n1)
    if [[ -z "$ISO" ]]; then echo "No testing ISO found. Run 'just build-test'."; exit 1; fi
    bash {{QEMU_SCRIPT}} -i "$ISO" -D {{DISK_IMAGE}} -U {{UEFI_VARS}} -S

# Boot existing installation in QEMU with GUI
qemu-run:
    #!/usr/bin/env bash
    if [[ ! -f {{DISK_IMAGE}} ]]; then echo "No disk. Run 'just qemu-install' first."; exit 1; fi
    bash {{QEMU_SCRIPT}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Boot existing installation in QEMU with serial console
qemu-run-serial:
    #!/usr/bin/env bash
    if [[ ! -f {{DISK_IMAGE}} ]]; then echo "No disk. Run 'just qemu-install' first."; exit 1; fi
    bash {{QEMU_SCRIPT}} -D {{DISK_IMAGE}} -U {{UEFI_VARS}} -S

# SSH into running VM
ssh:
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2222 root@localhost

# Upload latest binaries to running QEMU VM
upload:
    scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2222 \
        {{BINARY}} root@localhost:/usr/local/bin/archinstall-zfs-tui
    @if [ -f {{BINARY_SLINT}} ]; then \
        scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2222 \
            {{BINARY_SLINT}} root@localhost:/usr/local/bin/archinstall-zfs-slint; \
    fi
    @echo "Uploaded to QEMU VM"

# ─── Integration Tests ─────────────────────────────────

# Full cycle: fresh disk, install, boot, verify
test-vm *ARGS:
    just build
    cargo xtask test-vm {{ARGS}}

# Install only: fresh disk, run installer, verify exit code
test-install *ARGS:
    just build
    cargo xtask test-install {{ARGS}}

# Boot only: boot existing disk, verify system health
test-boot *ARGS:
    cargo xtask test-boot {{ARGS}}

# ─── Cleanup ───────────────────────────────────────────

# Clean all build artifacts
clean: clean-iso
    cargo clean
    rm -f {{DISK_IMAGE}} {{UEFI_VARS}}
