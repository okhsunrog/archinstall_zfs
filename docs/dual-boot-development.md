# Alongside installation development

The planner in `core/src/disk/alongside/` is connected to the graphical wizard
and installation pipeline. Integration validation is in progress; this document
does not claim verified bootability of an existing operating system.

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

Space planning reserves **100 MiB for one locally built ZFSBootMenu image plus
8 MiB overhead**. This is a growth allowance, not an exact prediction: the host
image measured 48.33 MiB; published ordinary EFI bundles through v3.1.0 reached
63.56 MiB. A backup or temporary second copy is not a prerequisite for reuse.

The installed system still builds ZBM locally. `azfs-update-zbm` generates it in
`/var/lib/zfsbootmenu` and then runs `azfs-install-zbm`. Publication checks the
actual file and available FAT space. With room for two images it stages and
renames; otherwise it saves the current loader on the root filesystem, removes
that loader from the ESP and writes its replacement. The latter has a short
power-loss window requiring recovery from live media. It is an accepted tradeoff,
not a reason to create a second ESP by default. No persistent backup is created
on the ESP. A foreign removable-media fallback is preserved; an optional ZBM
fallback must not block updating the main image.

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
performed by these fixtures; These original fixture runs did not cover guest boot; the GUI and VM checks
below were added subsequently. Windows boot remains unverified.

## Graphical editor and integration checks

The GUI's **Install alongside** mode shows proportional current/planned maps,
separate filesystem/free-space choices, and an allocation slider plus exact GiB
input. Disk discovery and filesystem checks run off the UI thread. Errors on
one disk leave the disk selector available. The TUI currently directs interactive
resizing users to `azfs`; its config-driven installer uses the same core plan.

Safe visual fixtures:

```sh
SLINT_EMIT_DEBUG_INFO=1 cargo build -p archinstall-zfs-slint \
  --no-default-features --features desktop-mock,slint/mcp --locked
uv run slint-ui/scripts/alongside_review.py --output /tmp/alongside-review
SLINT_BACKEND=headless target/debug/azfs --preview alongside
```

`AZFS_PREVIEW_ESP=small` exercises explicit additional-ESP consent.
`AZFS_PREVIEW_ALONGSIDE=ext4|missing-tools|no-efi|mbr` selects edge cases without
probing or changing the host. Preview mode remains mandatory.

After a production build, run the disposable installation and boot test:

```sh
just cargo-build
just test-vm --alongside --zfs-mode dkms --tmpfs --timeout 1800
```

This option creates an 80 GiB QEMU disk with a preserved ext4 filesystem and EFI
fixture, allocates 32 GiB by shrinking ext4, and checks retained payloads before
booting the newly installed system. It does not represent a bootable Windows or
second Linux installation. Do not claim existing-OS bootability from this test.

The boot publisher has an independent real FAT test, using only its own image:

```sh
sudo bash core/tests/zbm-update-fixture.sh
shellcheck core/assets/azfs-install-zbm core/assets/azfs-update-zbm \
  core/tests/zbm-update-fixture.sh
```

## Visual review coverage (2026-09-10)

Real headless Slint captures and callbacks were exercised at 1920×1080 / 100%,
1366×768 / 100%, and 1920×1080 / 150%. Individual captures were inspected,
including maps, the compact layout and the review summary.

| Case | Interaction checked |
| --- | --- |
| Reuse ESP | Select source, enter allocation, switch to unallocated space |
| Swap | Choose disk swap on Disk; map and Review subtract its size from the pool |
| Whole extent | Default to all MiB-aligned space; preserve fractional-GiB capacity |
| Insufficient ESP | Reach consent by keyboard, explicitly enable second ESP |
| Return navigation | Open Review, return to Disk, switch installation modes and return |
| Missing NTFS tools | Select unavailable source; readable package hint; Install disabled |
| Missing ESP / MBR | Readable reason; Install disabled; disk selector remains available |
| ext4 | Alternate filesystem fixture |

Large and small partition widths represent disk proportions; small EFI/recovery
regions cannot carry a full label inside the bar. Their role is identified by
color and the EFI selector. A free extent is preferred over shrinking when one
can satisfy the minimum allocation. These preview results do not establish real
filesystem safety or existing-OS bootability; use the execution tests separately.


Allocation defaults to the entire selected free extent (or the maximum safe
shrink allocation), aligned down to MiB without discarding a fractional GiB.
The Disk page owns swap selection for alongside installs. None/ZRAM consume no
disk space; plain/encrypted swap reserves the chosen GiB inside the allocation.
A minimum of 32 GiB remains for ZFS after subtracting swap and an optional new
ESP. The later ZFS page shows the same setting read-only. Encrypted swap uses a
random key per boot and is not a hibernation target.

Additional tests:

```sh
sudo target/debug/examples/alongside_fixture ext4 swap
sudo target/debug/examples/alongside_fixture ntfs swap
just test-vm --alongside --config xtask/configs/alongside-swap.json --tmpfs --timeout 1800
```

The swap fixture reserves 8 GiB inside a 40 GiB allocation, leaving 32 GiB for
ZFS. Both filesystem fixtures passed with retained payload comparisons. The
first complete alongside VM install with ZRAM passed; the installed system
booted and all 13 health checks passed after updating the check for the new
ZBM wrapper. The additional disk-swap VM run is a separate verification.
