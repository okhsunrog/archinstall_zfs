# Alongside installation development

The core planner and execution helper are under development in
`core/src/disk/alongside/`. They are not yet connected to the installation flow
or graphical wizard. This document does not claim end-to-end dual-boot support.

## Storage contract

- Require UEFI and GPT. Reuse the existing UEFI detector; reject MBR and hybrid
  layouts rather than converting them while preserving data.
- Only ordinary, unmounted NTFS and ext4 partitions can be shrunk. LUKS,
  BitLocker, LVM, RAID and Btrfs resizing are not implemented. Missing tools
  disable the operation with a package hint, not the whole application.
- Existing unallocated extents are selectable independently. Never combine
  gaps across an intervening partition or move a partition's starting sector.
- Check current usage, geometry, GUIDs and filesystem minimum before writing.
  Shrink the filesystem first, then its partition. A failed shrink must not be
  followed by partition creation or formatting.
- Preserve existing ESP and recovery partitions. Save the original partition
  table outside the target disk. This is not a backup of filesystem data.
- Do not cancel between filesystem shrink and partition-boundary update.
  Report partial failures and retain recovery information; never automatically
  restore GPT or retry a stale plan.

## EFI policy

Reuse an existing ESP on the selected disk by default. Only offer an additional
ESP when a successful capacity check establishes insufficient space, and require
an explicit selection. A corrupt or unreadable ESP is not evidence of low space.
Repeat capacity validation before applying the plan.

Space calculation takes the actual EFI bundle size, reserves one replacement
image and 8 MiB for allocation overhead, and adds persistent backup/fallback
copies only when enabled. A permanent previous-version backup is optional.
The inspected host bundle was 50,681,856 bytes; this measurement is a reference,
not a constant to assume for other builds.

The source of the pre-installation bundle remains to be integrated: the current
installer builds ZBM in the target after installation, too late for exact
pre-resize capacity checks. The live-image packaging or a preliminary build
must supply the actual artifact before the new workflow can be enabled.

## Disposable execution tests

Build the fixture as the normal development user:

```sh
cargo build -p archinstall-zfs-core --example alongside_fixture --locked
```

Then run the built executable as root (adapt the binary path if overriding
`CARGO_TARGET_DIR`):

```sh
sudo target/debug/examples/alongside_fixture ext4
sudo target/debug/examples/alongside_fixture ntfs
sudo target/debug/examples/alongside_fixture ext4 free
sudo target/debug/examples/alongside_fixture ext4 new-esp
```

The fixture accepts filesystem/mode names, never a target disk argument. It
creates a sparse image, attaches its own loop device, and detaches it before
removing the image. It checks retained file contents, existing ESP contents,
recovery-partition metadata, new partition geometry and stale-plan rejection.
The `new-esp` fixture fills the existing ESP before explicitly requesting a
second one. It does not simulate a booted Windows installation or prove Windows
bootability.

NTFS shrinking intentionally schedules a Windows consistency check. The test
uses `ntfscat --force` only to read its known payload afterwards; production
resizing never forces dirty/hibernated NTFS or clears its check-required flag.

The disk lock stays held during mutations. Do not run `udevadm settle` while
holding it: udev may be waiting for that lock. Check kernel partition geometry
and devtmpfs nodes directly; aliases can be populated after releasing the lock.

Validated on 2026-09-10: all four fixture commands above passed, including file
content comparisons and stale-plan rejection. Workspace tests, formatting and
clippy with warnings denied also passed. No installation or guest-OS boot was
performed by these fixtures; Windows boot, full installer integration and GUI
review remain outstanding.
