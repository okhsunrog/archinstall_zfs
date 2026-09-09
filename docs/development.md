# Developer guide

Run commands from the repository root. Use the checked-out source, `Cargo.lock`,
and `just --list` as the authority for current versions and available tasks.
The native Arch and container/Nix setup options are in the
[main README](../README.md#development).

## Choose the right environment

| Work | Environment | What it proves |
| --- | --- | --- |
| Composition, controls, dialogs, error states | `desktop-mock` **with `--preview`**, Slint MCP | Real UI and editing callbacks on deterministic fixtures |
| Host NetworkManager integration | `desktop` | Host networking behavior; not ISO/iwd behavior |
| LinuxKMS rendering, VT ownership, shell handoff | Default release build in a disposable VM | Actual KMS and process lifecycle on virtual hardware |
| Touchpad feel, hardware cursor artifacts, real iwd | ISO `azfs --demo` on the target computer | Behavior on that device; not an installation test |
| Partitioning, ZFS, bootability, cleanup | Dedicated disposable installation VM | The specific tested installation scenario |

`desktop-mock` alone only selects a mock Wi-Fi backend. Pass `--preview` to
replace storage probes, package lookup, installation, and reboot too. Do not
run the ordinary installer on the development host to obtain screenshots.

## Source map

- `core/`: configuration, validation, disk/ZFS/network operations and installation.
- `tui/`: terminal frontend.
- `slint-ui/ui/`: real Slint views, dialogs, shared controls and globals.
- `slint-ui/src/`: controllers, UI adapters, preview fixtures and VT supervisor.
- `slint-ui/scripts/`: repeatable capture and interaction flows.
- `xtask/`: profile rendering and QEMU installation harness.
- `gen_iso/profile/`: versioned ISO input. `profile_rendered/`, workdirs and
  generated ISOs are outputs; do not implement changes only in those trees.

For Slint implementation and design review, follow
[Slint coding and visual review](slint-ui-review.md). The
[GUI README](../slint-ui/README.md) lists all available preview states and flows.

## Build and check

```sh
just cargo-build
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
just check-features
```

`just cargo-build` produces production `target/release/azfs`, `azfs-tui`, and
`xtask`. `just check` also runs the dependency audit. On a non-Arch host, use the
documented container build for binaries intended for an Arch ISO; a host-linked
binary may require a different dynamic loader or shared libraries.

When adding dependencies, use the package manager (`cargo add`, `uv add`, etc.)
and inspect the resulting manifest/lockfile changes. Prefer Rust edition 2024.
Run Python tooling through `uv run`; the UI review scripts use the standard
library and do not require adding a Python dependency for each review.

Select checks by the changed behavior. A docs-only change needs working links
and accurate commands, not a new ISO. Shared UI controls need checks of their
consumers, not just the page edited. A boot helper needs failure-path tests and
a boot check, not only Rust tests. Do not treat ignored hardware tests as passed.

## Slint fork and release builds

The GUI runtime and `slint-build` use the same Slint fork branch, currently
`feat/linuxkms-integration`. Read their manifests and the git source revision
in `Cargo.lock` before updating or diagnosing backend behavior. An upstream
branch moving does not change the revision used by a locked build.

Keep mock/MCP builds separate from production artifacts. Before distributing a
binary, run `just cargo-build` with the default LinuxKMS/iwd features. Do not copy
`target/debug/azfs` from a preview run onto the installer USB. `slint/mcp` is a
debug build feature, not a feature to add permanently to the shipping manifest.

## Updating the installer on Ventoy

Use [Live binary updates](../gen_iso/LIVE_UPDATE.md) for the exact USB workflow.
One base ISO containing `azfs-live-update.service` is required. Afterwards:

1. Build the LinuxKMS release binary.
2. Publish it as `/azfs-update/azfs` on the Ventoy data partition using a
   temporary filename, verify the copy, and safely unmount the USB drive.
3. Boot the ISO and run `azfs` from the console as usual.
4. Check `journalctl -b -u azfs-live-update.service` for the loaded SHA-256.

The oneshot runs before login consoles. It reads only the boot disk's data
partition and stages the binary in `/usr/local/bin` before an atomic rename.
The live root's OverlayFS normally stores that replacement in RAM; the ISO is
unchanged. An absent or rejected update leaves the built-in installer available.
The service neither starts the GUI nor periodically reloads a running process.

Changes to the service, ISO profile, kernel, ZFS, TUI or incompatible system
libraries require updating the base image. Replacing `azfs` cannot change those.

```sh
just test-live-update
shellcheck gen_iso/profile/airootfs/usr/local/libexec/azfs-live-update
```

For boot changes, test the actual service in a disposable VM with Ventoy, not
only a direct CD-ROM boot. Exercise valid, absent and invalid updates, check the
installed hash and login ordering, and verify fallback still reaches the console.

## ISO and USB delivery

```sh
just iso-full --mode dkms --kernel linux
```

This is a full build. Repacking a known base can save package-build time, but
still requires replacing the compressed root image, regenerating its checksum,
preserving BIOS/UEFI boot metadata, and checking the final image. It is not a
binary-sized in-place edit. Never ship temporary boot collectors or automatic
shutdown fixtures used during testing.

Before writing physical media, inspect `lsblk` and identify the drive by model,
serial, partition and mount source. Copy a file onto Ventoy's data filesystem;
do not overwrite the entire disk with `dd`. Preserve unrelated images. Delete
the superseded installer image only after the new copy is verified.

Wait for writes to finish, compare source/destination hashes, unmount and
recheck the hashes after mounting read-only. Cached reads alone are weaker
evidence than a direct read. `oflag=direct` has alignment constraints; do not
blindly apply an ISO-copy command to an arbitrarily sized binary. Unmount all
partitions and power off/eject the USB only after every VM using it has exited.
Do not report safe removal until these steps succeed.

A stalled flush is not proof of a defective USB drive. Inspect the kernel log
and existing hung-task/panic settings when diagnosing a write stall; do not
change host panic policy as a routine part of copying an image.

For QEMU booting from physical media, keep the physical backing device read-only
and direct guest writes into a local qcow2 overlay. A fully read-only virtual
disk can fail during Ventoy's device-mapper setup. The working block-device
arrangement and shutdown procedure are in [Boot debugging](debugging-boot.md).

## Installation and shell lifecycle

The disposable encrypted-install harness is separate from UI preview:

```sh
just cargo-build
just test-install-encrypted-pool --tmpfs --timeout 1800
```

See [Post-install shell](../slint-ui/POST_INSTALL_SHELL.md) for the real VT/chroot
round trip and fixture. The GUI child exits before the shell opens. The parent
restores the console, manages owned mounts/processes, cleans up, then starts a
new completion GUI. Simulated `shell` screenshots do not prove this lifecycle.

## Reporting completed work

Record the revision, feature set, commands, tested states/display configurations,
and artifact paths. Separate code checks, visual inspection, VM behavior and
physical-device results. A green screenshot capture is not a design review; a
successful GUI boot is not a successful disk installation. Dated review reports
are historical evidence, not a standing claim that future changes are covered.
