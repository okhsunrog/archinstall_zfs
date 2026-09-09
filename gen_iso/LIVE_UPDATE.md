# Updating the live installer from Ventoy

ISOs built with `azfs-live-update.service` can load a newer GUI installer from
the Ventoy data partition at boot. Place your trusted, LinuxKMS release binary
at `/azfs-update/azfs` on that partition (alongside the ISO, outside it).

For example, after `just cargo-build`, with Ventoy already mounted:

```sh
mkdir -p /run/media/$USER/Ventoy/azfs-update
cp target/release/azfs /run/media/$USER/Ventoy/azfs-update/azfs.part
mv /run/media/$USER/Ventoy/azfs-update/azfs.part /run/media/$USER/Ventoy/azfs-update/azfs
```

Safely unmount/eject the USB drive before booting it. Publishing with the final
rename avoids picking up an incomplete transfer. Remove or rename `azfs-update`
to use the installer embedded in the ISO again.

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
journalctl -b -u azfs-live-update.service
```

The destination is the live root's writable OverlayFS layer, normally backed
by tmpfs. The ISO and the Ventoy filesystem are never modified by the service.
With the standard non-persistent boot, the replacement disappears on reboot
and is loaded again from USB. No separate executable mount on exFAT is needed.

This updates only `azfs`, not the TUI, kernel, ZFS or shared libraries. Build a
new base ISO when those dependencies change incompatibly. An older ISO without
this service needs one initial update before binary-only updates work.

Run the unprivileged regression checks with:

```sh
uv run python gen_iso/test_live_update.py
shellcheck gen_iso/profile/airootfs/usr/local/libexec/azfs-live-update
```
