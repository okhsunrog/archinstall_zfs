# Graphical installer development

Follow [Slint coding and visual review](../docs/slint-ui-review.md) for design
criteria, coverage, full-size screenshot inspection and implementation rules.
This page is the executable preview/fixture reference; the
[developer guide](../docs/development.md) covers production and VM workflows.

Use the deterministic preview to review the installer without root, ZFS, disks,
iwd, NetworkManager, or internet access. The preview uses the real components and
editing controllers with simulated inventory and installation callbacks.

```sh
SLINT_EMIT_DEBUG_INFO=1 cargo build -p archinstall-zfs-slint \
  --no-default-features --features desktop-mock,slint/mcp --locked
SLINT_BACKEND=headless SLINT_MCP_PORT=9315 target/debug/azfs \
  --preview welcome --preview-size 1920x1080 --ui-scale 1
```

Use **1920×1080 at 100% scale** as the primary design and screenshot baseline.
This is also the default preview size and the first configuration in the review
and interaction scripts. Smaller windows and higher scales remain secondary
checks for clipping and accessibility, not the main design reference.

Omit `SLINT_BACKEND=headless` to interact in a desktop window. To reproduce a
1920×1080 display with a 200% UI scale, use `--preview-size 1920x1080 --ui-scale 2`.

Preview scenes: `welcome`, `offline`, `no-uefi`, `zfs-preparing`, `zfs-failed`,
`wifi-empty`, `wifi-unavailable`, `wifi-no-internet`, `wifi-verifying`,
`disk`, `new-pool`, `existing-pool`, `zfs`, `system`, `users`, `desktop`, `review`,
`install`, `done`, `failed`, `cancelling`, `cancelled`,
`inspect` (simulated pool import, export, and selection), and `invalid` (an
incomplete configuration for validation checks).
The three simulated drives include NVMe, SATA, and removable USB, with long serials
and persistent paths. The SATA drive has four partitions; NVMe and USB have two each. Filesystem, label,
EFI type, and installer-media usage are included. The wizard starts
with a configured user, KDE Plasma, packages, and services; edits remain interactive.
Wi-Fi uses the existing mock backend, including known, secured, open, and enterprise
networks. Installation advances through simulated phases and supports cancellation.

**The `desktop-mock` feature alone mocks only Wi-Fi; always pass `--preview` for
UI review on the development host.**

`--preview` is accepted only by a `desktop-mock` build and conflicts with `--config`,
`--secrets`, `--silent`, and `--demo`. It replaces system probes, storage enumeration,
package searches, installation, and reboot. No installation or storage commands are
started. Logs are still written to `/tmp`.

Capture all scenes at five display configurations, starting with Full HD at 100%:

```sh
uv run slint-ui/scripts/review.py --output /tmp/azfs-ui-review
```

The output contains PNG screenshots, JSON element trees, process logs, and an
`index.html` gallery. Open the images and inspect them; successful capture does not
establish that a layout is correct. Use `--scenes disk review` or
`--sizes 1920x1080@1` to review only the primary configuration.

Run repeatable interaction checks at Full HD with 100% and 200% scale and at 1366×768:

```sh
uv run slint-ui/scripts/interactions.py --output /tmp/azfs-ui-interactions
```

These cover editing, focus recovery, keyboard scrolling, Wi-Fi credentials,
forget/reconnect/disconnect, enterprise errors, pool inspection, installation
progress, completion, and cancellation. All input values are disposable fixtures.

Review the remaining editor decisions and their error/empty states at the same three configurations:

```sh
uv run slint-ui/scripts/design_review.py --output /tmp/azfs-design-review
```

This adds account validation without losing entered values, administrator controls,
unsaved-account feedback, timezone city search, locale/keyboard filtering,
optional packages, display manager and GPU choices, Wayland/console profiles,
service removal, and repository/AUR package selection. These scripts drive real
callbacks and save individual screenshots; their assertions supplement visual
inspection rather than scoring the design. The review findings and boundaries
are recorded in [Design review](DESIGN_REVIEW.md).

Exercise the storage decision flow, including search, disabled partitions,
cancel/confirm, keyboard focus, encryption, swap, pool selection, and Review:

```sh
uv run slint-ui/scripts/storage_review.py --size 1920x1080 --output /tmp/azfs-storage-review
uv run slint-ui/scripts/storage_review.py --size 1920x1080 --scale 1.5 \
  --output /tmp/azfs-storage-review-scaled
uv run slint-ui/scripts/storage_review.py --keyboard-only --size 1920x1080 --scale 2 \
  --output /tmp/azfs-storage-keyboard
AZFS_PREVIEW_STORAGE=many uv run slint-ui/scripts/storage_review.py \
  --fixture many --output /tmp/azfs-storage-many
```

`AZFS_PREVIEW_STORAGE` accepts `empty` (no disks), `many` (20 partitions per disk),
and `missing` (unknown filesystems and labels). Pass the matching `--fixture`
argument to the storage script: the flag selects assertions, and the environment
variable selects inventory data. These overrides only affect preview fixtures.
The storage script defaults to 1366×768; specify Full HD for the primary review.
Storage discovery in the production picker is read-only; choosing an existing
pool does not import it. Space and dataset details for unimported pools remain
unknown until installation imports them.

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

Slint is built from the `feat/linuxkms-integration` branch of the fork, with the
exact revision recorded in `Cargo.lock`. This combines the independent Skia
software, cursor damage, pointer input, and console isolation changes.

When started from a VT, the installer enables tap-to-click for its GUI child.
Set `SLINT_LIBINPUT_TAP_TO_CLICK=0` to disable it. Explicit values are preserved
when returning from the post-install shell.

Libinput uses neutral pointer acceleration by default. Override it with
`SLINT_LIBINPUT_ACCEL_SPEED=0.3` (finite values from -1 to 1). Test the resulting
feel on the target mouse or touchpad; headless rendering cannot validate it.

For the post-install chroot shell, automatic cleanup, completion-screen return,
and disposable VM test fixture, see [Post-install shell](POST_INSTALL_SHELL.md).
The `shell` interaction flow checks the simulated completion/return states.
The `logs` flow verifies that completion opens at the latest output, scrolling
back preserves earlier lines, and **Latest output** resumes following. Both
flows check that the action panel stays below the log and inside the window.

The `--preview alongside` scene exercises graphical disk resizing. See
[Alongside development and verification](../docs/dual-boot-development.md) for
fixtures, runtime tool requirements and test boundaries.
