# Graphical installer development

Use the deterministic preview to review the installer without root, ZFS, disks,
iwd, NetworkManager, or internet access. The preview uses the real components and
editing controllers with simulated inventory and installation callbacks.

```sh
SLINT_EMIT_DEBUG_INFO=1 cargo build -p archinstall-zfs-slint \
  --no-default-features --features desktop-mock,slint/mcp
SLINT_BACKEND=headless SLINT_MCP_PORT=9315 target/debug/azfs \
  --preview welcome --preview-size 1280x800
```

Omit `SLINT_BACKEND=headless` to interact in a desktop window. To reproduce a
1920×1080 display with a 200% UI scale, use `--preview-size 1920x1080 --ui-scale 2`.

Preview scenes: `welcome`, `offline`, `disk`, `new-pool`, `existing-pool`, `zfs`,
`system`, `users`, `desktop`, `review`, `install`, `done`, `failed`, `cancelled`,
`inspect` (simulated pool import, export, and selection), and `invalid` (an
incomplete configuration for validation checks).
The three simulated drives include NVMe, SATA, and removable USB, with long serials
and persistent paths. Each drive has two simulated partitions. The wizard starts
with a configured user, KDE Plasma, packages, and services; edits remain interactive.
Wi-Fi uses the existing mock backend, including known, secured, open, and enterprise
networks. Installation advances through simulated phases and supports cancellation.

`--preview` is accepted only by a `desktop-mock` build and conflicts with `--config`,
`--secrets`, `--silent`, and `--demo`. It replaces system probes, storage enumeration,
package searches, installation, and reboot. No installation or storage commands are
started. Logs are still written to `/tmp`.

Capture all scenes at four display configurations:

```sh
uv run slint-ui/scripts/review.py --output /tmp/azfs-ui-review
```

The output contains PNG screenshots, JSON element trees, process logs, and an
`index.html` gallery. Open the images and inspect them; successful capture does not
establish that a layout is correct. Use `--scenes disk review` or
`--sizes 800x600@1 1920x1080@2` to narrow a repeat run.

Run repeatable interaction checks at 800×600 and 1920×1080 with 200% scale:

```sh
uv run slint-ui/scripts/interactions.py --output /tmp/azfs-ui-interactions
```

These cover editing, focus recovery, keyboard scrolling, Wi-Fi credentials,
forget/reconnect/disconnect, enterprise errors, pool inspection, installation
progress, completion, and cancellation. All input values are disposable fixtures.

For additional interaction checks, use Slint MCP's `get_element_tree`, `click_element`,
`set_element_value`, and `dispatch_key_event`. Check password entry, Wi-Fi connect,
verification, success, disconnect, forget, and unsupported enterprise networks.
Check keyboard navigation and every popup at the smallest supported window size.

The preview exercises desktop rendering. Validate input isolation and cursor
rendering separately with the default LinuxKMS build in a VM or on the target
machine. Start it from an actual VT shell to test that GUI keystrokes cannot reach
the shell after exit.

The KMS backend disables VT keyboard translation while rendering and restores it
on exit. A small installer supervisor also flushes input and restores keyboard
and display modes when the GUI process aborts or receives SIGKILL. It forwards
SIGINT, SIGTERM, SIGHUP, and SIGQUIT to the GUI. Run this before starting any threads.
The supervisor must remain alive for crash recovery; SIGKILL of both processes
cannot run userspace cleanup.

Libinput uses neutral pointer acceleration by default. Override it with
`SLINT_LIBINPUT_ACCEL_SPEED=0.3` (finite values from -1 to 1). Test the resulting
feel on the target mouse or touchpad; headless rendering cannot validate it.
