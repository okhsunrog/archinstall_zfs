set shell := ["bash", "-euo", "pipefail", "-c"]

# Paths
PROFILE_DIR := "gen_iso/profile"
PROFILE_OUT := "gen_iso/profile_rendered"
ISO_OUT := "gen_iso/out"
DISK_IMAGE := "gen_iso/arch.qcow2"
UEFI_VARS := "gen_iso/my_vars.fd"
QEMU_SCRIPT := "gen_iso/run-qemu.sh"
BINARY := "target/release/azfs-tui"
BINARY_SLINT := "target/release/azfs"
CONTAINER_IMAGE := "ghcr.io/okhsunrog/archinstall_zfs/ci:latest"
PACMAN_CACHE_VOLUME := "archzfs-pacman-cache"
CARGO_TARGET_VOLUME := "archzfs-cargo-target"
CARGO_REGISTRY_VOLUME := "archzfs-cargo-registry"

# ─── Cargo ──────────────────────────────────────────────

# Requires libalpm on the host (Arch, or any distro inside `nix develop`).
# Native release build of azfs + azfs-tui + xtask.
cargo-build:
    cargo build --release --bin azfs --bin azfs-tui --bin xtask

# For non-Arch hosts without native libalpm. Target tree and cargo registry
# persist across runs via named volumes.
# Release build of azfs + azfs-tui + xtask inside the CI container (Arch-glibc).
cargo-build-container:
    sudo podman run --rm \
        -e CARGO_HOME=/cargo-registry \
        -e CARGO_TARGET_DIR=/cargo-target \
        -e HOST_UID="$(id -u)" \
        -e HOST_GID="$(id -g)" \
        -v "$(pwd):/work" \
        -v "{{CARGO_TARGET_VOLUME}}:/cargo-target" \
        -v "{{CARGO_REGISTRY_VOLUME}}:/cargo-registry" \
        -w /work \
        {{CONTAINER_IMAGE}} \
        bash -c 'cargo build --release --bin azfs --bin azfs-tui --bin xtask && \
                 mkdir -p target/release && \
                 cp /cargo-target/release/azfs /cargo-target/release/azfs-tui /cargo-target/release/xtask target/release/ && \
                 chown "$HOST_UID:$HOST_GID" target/release/azfs target/release/azfs-tui target/release/xtask'

# Run cargo unit tests
cargo-test:
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
check: fmt-check lint cargo-test

# ─── ISO Building ──────────────────────────────────────

# Internal: render profile templates using the prebuilt xtask binary.
# Requires cargo-build or cargo-build-container to have run first.
_render-profile MODE="precompiled" KERNEL="linux-lts" FAST="":
    ./target/release/xtask render-profile \
        --profile-dir {{PROFILE_DIR}} \
        --out-dir {{PROFILE_OUT}} \
        --kernel {{KERNEL}} \
        --zfs {{MODE}} \
        {{FAST}}

# Internal: copy installer binaries into rendered profile
_prepare-binary:
    @mkdir -p {{PROFILE_OUT}}/airootfs/usr/local/bin/
    install -m 0755 {{BINARY}} {{PROFILE_OUT}}/airootfs/usr/local/bin/azfs-tui
    @if [ -f {{BINARY_SLINT}} ]; then \
        install -m 0755 {{BINARY_SLINT}} {{PROFILE_OUT}}/airootfs/usr/local/bin/azfs; \
    fi

# Fast, minimal packages, serial+SSH enabled. Skips wifi/bluetooth/firmware.
# For QEMU iteration and CI.
# Usage: just iso-test [--mode precompiled|dkms] [--kernel linux|linux-lts|linux-zen]
[arg("MODE", long="mode")]
[arg("KERNEL", long="kernel")]
iso-test MODE="precompiled" KERNEL="linux-lts":
    @echo "Building testing ISO (mode={{MODE}}, kernel={{KERNEL}})"
    just cargo-build
    just _render-profile {{MODE}} {{KERNEL}} "--fast"
    just _prepare-binary
    @echo "Building ISO..."
    sudo rm -rf gen_iso/workdir
    sudo mkarchiso -v -w "gen_iso/workdir" -o {{ISO_OUT}} {{PROFILE_OUT}}
    sudo chown -R "$(id -u):$(id -g)" {{ISO_OUT}} gen_iso/workdir
    @echo "Testing ISO built in {{ISO_OUT}}"

# Same package set as CI releases (iwd, wireless-regdb, linux-firmware, etc).
# Slower than iso-test: larger squashfs, more packages to pacstrap.
# For bare-metal testing of features QEMU can't exercise.
# Usage: just iso-full [--mode precompiled|dkms] [--kernel linux|linux-lts|linux-zen]
[arg("MODE", long="mode")]
[arg("KERNEL", long="kernel")]
iso-full MODE="precompiled" KERNEL="linux-lts":
    @echo "Building full ISO (mode={{MODE}}, kernel={{KERNEL}})"
    just cargo-build
    just _render-profile {{MODE}} {{KERNEL}}
    just _prepare-binary
    @echo "Building ISO..."
    sudo rm -rf gen_iso/workdir
    sudo mkarchiso -v -w "gen_iso/workdir" -o {{ISO_OUT}} {{PROFILE_OUT}}
    sudo chown -R "$(id -u):$(id -g)" {{ISO_OUT}} gen_iso/workdir
    @echo "Full ISO built in {{ISO_OUT}}"

# List available ISOs
iso-list:
    @ls -lht {{ISO_OUT}}/*.iso 2>/dev/null || echo "No ISOs found. Run 'just iso-test' first."

# Clean ISO build artifacts
iso-clean:
    rm -rf {{PROFILE_OUT}} gen_iso/workdir
    @echo "ISO build artifacts cleaned"

# ─── Container-based mkarchiso (cross-distro, for non-Arch hosts) ──

# Pull the prebuilt CI image from ghcr.io (same image CI uses for build + check)
builder-pull:
    sudo podman pull {{CONTAINER_IMAGE}}

# Inspect podman-side state (image + all volumes: pacman cache, cargo target, cargo registry)
builder-info:
    #!/usr/bin/env bash
    set -eu
    sudo podman image ls --filter reference={{CONTAINER_IMAGE}} --format 'image:  {{"{{.Repository}}:{{.Tag}}"}}  size: {{"{{.Size}}"}}'
    for vol in {{PACMAN_CACHE_VOLUME}} {{CARGO_TARGET_VOLUME}} {{CARGO_REGISTRY_VOLUME}}; do
        if sudo podman volume exists "$vol"; then
            mp=$(sudo podman volume inspect "$vol" --format '{{"{{.Mountpoint}}"}}')
            if sudo test -d "$mp"; then
                size=$(sudo du -sh "$mp" | awk '{print $1}')
                echo "volume: $vol  size: $size"
            else
                echo "volume: $vol  (metadata only; on-disk dir not yet materialized)"
            fi
        else
            echo "volume: $vol  (not yet created)"
        fi
    done

# Remove the image and all named volumes (pacman cache, cargo target, cargo registry)
builder-clean:
    -sudo podman image rm {{CONTAINER_IMAGE}}
    -sudo podman volume rm {{PACMAN_CACHE_VOLUME}} {{CARGO_TARGET_VOLUME}} {{CARGO_REGISTRY_VOLUME}}

# Testing ISO fully inside the CI container: cargo, xtask render, mkarchiso.
# Produces Arch-glibc binaries that work both on the host and inside the ISO.
# For non-Arch hosts (Fedora/Debian/…). Only requires podman + qemu on the host.
# Usage: just iso-test-podman [--mode precompiled|dkms] [--kernel linux|linux-lts|linux-zen]
[arg("MODE", long="mode")]
[arg("KERNEL", long="kernel")]
iso-test-podman MODE="precompiled" KERNEL="linux-lts":
    @echo "Building testing ISO via podman (mode={{MODE}}, kernel={{KERNEL}})"
    just cargo-build-container
    just _render-profile {{MODE}} {{KERNEL}} "--fast"
    just _prepare-binary
    just _mkarchiso-container

# Full ISO via podman. Same build path as iso-test-podman, full package set.
# Usage: just iso-full-podman [--mode precompiled|dkms] [--kernel linux|linux-lts|linux-zen]
[arg("MODE", long="mode")]
[arg("KERNEL", long="kernel")]
iso-full-podman MODE="precompiled" KERNEL="linux-lts":
    @echo "Building full ISO via podman (mode={{MODE}}, kernel={{KERNEL}})"
    just cargo-build-container
    just _render-profile {{MODE}} {{KERNEL}}
    just _prepare-binary
    just _mkarchiso-container

# Internal: invoke mkarchiso inside the container with correct mounts + chown.
_mkarchiso-container:
    sudo rm -rf gen_iso/workdir
    mkdir -p gen_iso/workdir {{ISO_OUT}}
    sudo podman run --rm \
        --privileged \
        -e HOST_UID="$(id -u)" \
        -e HOST_GID="$(id -g)" \
        -v "$(pwd)/{{PROFILE_OUT}}:/profile:ro" \
        -v "$(pwd)/gen_iso/workdir:/workdir" \
        -v "$(pwd)/{{ISO_OUT}}:/out" \
        -v "{{PACMAN_CACHE_VOLUME}}:/var/cache/pacman/pkg" \
        {{CONTAINER_IMAGE}} \
        bash -c 'mkarchiso -v -w /workdir -o /out /profile && chown -R "$HOST_UID:$HOST_GID" /workdir /out'
    @echo "ISO built in {{ISO_OUT}}"

# ─── QEMU Setup ────────────────────────────────────────

# Create fresh 20G qcow2 disk
qemu-create-disk:
    qemu-img create -f qcow2 {{DISK_IMAGE}} 20G

# Arch ships OVMF_VARS.4m.fd; Fedora/Debian ship OVMF_VARS.fd (no 4m suffix).
# Search for the 4m variant first, fall back to the bare name. Skip secboot.
# Copy OVMF UEFI variables template into the workspace.
qemu-setup-uefi:
    #!/usr/bin/env bash
    OVMF_ROOTS=(/usr/share/edk2 /usr/share/edk2-ovmf /usr/share/OVMF)
    OVMF_VARS=$(find "${OVMF_ROOTS[@]}" -name "OVMF_VARS*.4m.fd" ! -name "*secboot*" -print -quit 2>/dev/null)
    if [[ -z "$OVMF_VARS" ]]; then
        OVMF_VARS=$(find "${OVMF_ROOTS[@]}" -name "OVMF_VARS.fd" -print -quit 2>/dev/null)
    fi
    if [[ -z "$OVMF_VARS" ]]; then echo "ERROR: OVMF_VARS{,.4m}.fd not found. Install edk2-ovmf."; exit 1; fi
    cp "$OVMF_VARS" {{UEFI_VARS}}
    echo "UEFI vars file created at {{UEFI_VARS}} (source: $OVMF_VARS)"

# Full QEMU setup (disk + UEFI vars)
qemu-setup: qemu-create-disk qemu-setup-uefi
    @echo "QEMU setup complete."

# Delete and recreate QEMU disk + UEFI vars
qemu-refresh:
    rm -f {{DISK_IMAGE}} {{UEFI_VARS}}
    just qemu-setup
    @echo "QEMU refresh complete."

# ─── QEMU Execution ───────────────────────────────────

# Boot latest ISO in QEMU with GUI
qemu-install:
    #!/usr/bin/env bash
    if [[ ! -f {{DISK_IMAGE}} ]]; then just qemu-create-disk; fi
    if [[ ! -f {{UEFI_VARS}} ]]; then just qemu-setup-uefi; fi
    ISO=$(ls -1t {{ISO_OUT}}/archzfs-*.iso 2>/dev/null | head -n1)
    if [[ -z "$ISO" ]]; then echo "No ISO found. Run 'just iso-test' or 'just iso-full'."; exit 1; fi
    bash {{QEMU_SCRIPT}} -i "$ISO" -D {{DISK_IMAGE}} -U {{UEFI_VARS}}

# Boot latest ISO in QEMU with serial console
qemu-install-serial:
    #!/usr/bin/env bash
    if [[ ! -f {{DISK_IMAGE}} ]]; then just qemu-create-disk; fi
    if [[ ! -f {{UEFI_VARS}} ]]; then just qemu-setup-uefi; fi
    ISO=$(ls -1t {{ISO_OUT}}/archzfs-*.iso 2>/dev/null | head -n1)
    if [[ -z "$ISO" ]]; then echo "No ISO found. Run 'just iso-test' or 'just iso-full'."; exit 1; fi
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
        {{BINARY}} root@localhost:/usr/local/bin/azfs-tui
    @if [ -f {{BINARY_SLINT}} ]; then \
        scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2222 \
            {{BINARY_SLINT}} root@localhost:/usr/local/bin/azfs; \
    fi
    @echo "Uploaded to QEMU VM"

# ─── Integration Tests ─────────────────────────────────

# Requires cargo-build or cargo-build-container first.
# Full cycle: fresh disk, install, boot, verify.
test-vm *ARGS:
    ./target/release/xtask test-vm {{ARGS}}

# Requires cargo-build or cargo-build-container first.
# Install only: fresh disk, run installer, verify exit code.
test-install *ARGS:
    ./target/release/xtask test-install {{ARGS}}

# Requires cargo-build or cargo-build-container first.
# Boot only: boot existing disk, verify system health.
test-boot *ARGS:
    ./target/release/xtask test-boot {{ARGS}}

# Install with pool-level ZFS encryption; regression cover for load-key-after-reimport
test-install-encrypted-pool *ARGS:
    just test-install --encryption pool --zfs-mode dkms {{ARGS}}

# Install with dataset-level ZFS encryption; regression cover for load-key-after-reimport
test-install-encrypted-dataset *ARGS:
    just test-install --encryption dataset --zfs-mode dkms {{ARGS}}

# ─── Cleanup ───────────────────────────────────────────

# Clean all build artifacts
clean: iso-clean
    cargo clean
    rm -f {{DISK_IMAGE}} {{UEFI_VARS}}
