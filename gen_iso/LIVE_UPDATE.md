# Updating the live installer from Ventoy

ISOs built with `azfs-live-update.service` can load a newer GUI installer from
the Ventoy data partition at boot. Place your trusted, LinuxKMS release binary
at `/azfs-update/azfs` on that partition (alongside the ISO, outside it).

## Publish an update

From the repository root, run `just cargo-build` to produce the default
LinuxKMS/iwd release binary. Do not use a desktop-mock or MCP preview build.
Confirm the mounted data partition belongs to the intended USB drive with
`lsblk -o NAME,PATH,MODEL,SERIAL,FSTYPE,LABEL,MOUNTPOINTS` and `findmnt`.
Do not reuse a device name from a previous connection without checking it.

With Ventoy already mounted, adjust the path and run this block:

```bash
(
    set -euo pipefail
    VENTOY_MOUNT="/run/media/$USER/Ventoy"
    mountpoint -q "$VENTOY_MOUNT"
    UPDATE_DIR="$VENTOY_MOUNT/azfs-update"
    mkdir -p "$UPDATE_DIR"
    cp target/release/azfs "$UPDATE_DIR/azfs.part"
    sync -f "$UPDATE_DIR/azfs.part"
    SOURCE_HASH=$(sha256sum target/release/azfs | cut -d ' ' -f 1)
    COPY_HASH=$(sha256sum "$UPDATE_DIR/azfs.part" | cut -d ' ' -f 1)
    test "$SOURCE_HASH" = "$COPY_HASH"
    mv "$UPDATE_DIR/azfs.part" "$UPDATE_DIR/azfs"
    printf '%s  azfs\n' "$SOURCE_HASH" > "$UPDATE_DIR/azfs.sha256"
    sync -f "$UPDATE_DIR"
)
```

Wait for both flushes to finish. If any command fails, investigate before
reporting the transfer complete. The checksum file is for manual verification;
the boot service does not use it as a signature or trust source. Publishing with
the final rename avoids picking up an incomplete transfer.

Unmount the data partition using the desktop's safe removal action or
`udisksctl unmount -b <verified-data-partition>`. For stronger delivery validation,
remount it read-only and compare the destination hash again, then unmount all USB
partitions and eject/power off the verified drive. Stop any VM using it first.
See [ISO and USB delivery](../docs/development.md#iso-and-usb-delivery).

Boot the existing base ISO normally. Remove or rename `azfs-update` to use the
installer embedded in the ISO again. A base ISO without this service needs one
initial replacement; copying a binary beside such an ISO cannot enable updates.

## Boot behavior and limits

The oneshot service runs before login consoles and a display manager. The
current ISO starts at a console: run `azfs` as usual. The service does not launch
the GUI itself. Any future GUI unit must also order itself after
`azfs-live-update.service`.

The helper follows `/dev/mapper/ventoy` to its backing disk and reads partition
1 in a private, read-only mount. It does not search other disks by label. Booting
the ISO directly, or booting without a discoverable Ventoy mapping, does not
load an external binary.

On current Ventoy versions the helper uses the partition's
`/dev/mapper/<partition>` device, after checking that it maps the complete
selected partition. See [Ventoy Linux Remount](https://www.ventoy.net/en/doc_linux_remount.html).

The binary is staged next to `/usr/local/bin/azfs`, checked for an ELF header,
and run with `--help` under a timeout to check runtime compatibility. Only then
is it atomically renamed over the existing file. A failed copy or check keeps
the built-in installer. These are integrity/startup checks, not authentication:
putting a binary in this directory authorizes running it as root. The service
logs the loaded binary's SHA-256 and any errors:

```sh
journalctl -b -u azfs-live-update.service --no-pager
sha256sum /usr/local/bin/azfs
```

The destination is the live root's writable OverlayFS layer, normally backed
by tmpfs. The ISO and the Ventoy filesystem are never modified by the service.
With the standard non-persistent boot, the replacement disappears on reboot
and is loaded again from USB. No separate executable mount on exFAT is needed.

This updates only `azfs`, not the TUI, kernel, ZFS or shared libraries. Build a
new base ISO when those dependencies change incompatibly. An older ISO without
this service needs one initial update before binary-only updates work.

## Implementation and validation

The source lives in the ISO profile:

- [Loader](profile/airootfs/usr/local/libexec/azfs-live-update)
- [Oneshot service](profile/airootfs/etc/systemd/system/azfs-live-update.service)
- [Regression tests](test_live_update.py)

The unit pulls in the passive `getty-pre.target` with `Wants=` as well as ordering
itself `Before=` that target. Keep both: ordering alone does not start a passive
target and cannot establish the intended login ordering in that case.

Run the unprivileged regression checks with:

```sh
uv run python gen_iso/test_live_update.py
shellcheck gen_iso/profile/airootfs/usr/local/libexec/azfs-live-update
```

`just test-live-update` runs the same Python tests. Also boot with Ventoy and
exercise an accepted update, no update and a rejected update. Confirm that the
console remains available, the loaded SHA-256 matches, and the service finishes
before getty. The [boot guide](../docs/debugging-boot.md#testing-a-physical-ventoy-drive-without-writing-to-it)
shows a VM setup that protects the physical medium while allowing guest writes.
