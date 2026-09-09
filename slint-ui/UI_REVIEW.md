# UI and LinuxKMS review — 2026-09-09

The original LinuxKMS build reproduced the reported console input leak in an
UEFI QEMU guest: text entered while the GUI was running became a shell command
after the GUI closed. A disposable marker-file command was used, never a real
credential.

The updated backend disables VT keyboard translation while libinput owns input.
The installer supervisor also recovers the VT after abnormal GUI termination.
The same guest checks cover normal exit, SIGKILL of the GUI child, and SIGTERM of
the supervisor. No marker command reached the shell; UTF-8 keyboard mode was
restored, and a separate command entered after exit still worked.

The Slint branch `feat/renderer-skia-software-linuxkms` was rebased onto upstream
`12acf1a25ad09d5384c14fc7330d4040790e729e`. Its resulting revision is
`d6b44e120`. Cursor damage is now registered before Skia determines the partial
rendering clip. A real software-Skia pixel regression test compares incremental
frames against full repaint during movement, edge clipping, fractional positions,
and hiding. It fails with the old damage ordering and passes with the fix.

The installer changes address:

- Icon centering, welcome status rows, and network indicator alignment.
- A compact connection-success view with Done as the primary action and a
  consistently positioned Close button. Disconnect is in network management.
- Cancellation and stale async results during network scanning, disconnect, and
  forgetting saved networks. Mock forget now removes the saved profile.
- Consistent disk model, capacity, device-node, serial, and persistent-path rows.
- Wizard content width, sidebar spacing, footer alignment, focus restoration,
  keyboard scrolling, and wrapped validation messages.
- Package-search activation, pool action alignment, and installation progress
  positioned above the log.
- Password-strength scoring moved off the UI thread, with debouncing and stale
  result checks after editing, clearing, and reopening dialogs.

`scripts/review.py` captures the actual Slint components for all preview scenes
at 800×600 and 1280×800 with scale 1, and 1920×1080 with scales 1.5 and 2.
Window metadata is saved and the actual scale is asserted: the headless backend
requires an explicit scale-change event, unlike desktop backends.

`scripts/interactions.py` drives editing, filtering, keyboard navigation, all
network transitions, pool import/export, installation completion/cancellation,
and invalid-configuration blocking. It saves screenshots of the intermediate
states for visual inspection. The preview replaces hardware and installation
operations while retaining the real editing and validation logic.

Run `just check` for the repository's formatting, clippy, tests, feature checks,
and dependency audit. The lockfile upgrades h2 to 0.4.16, resolving the advisory
that failed main's audit job. Audit still reports non-fatal third-party
maintenance/yank warnings; they are not suppressed.
The final test run also exposed two AUR cache tests racing through a shared
environment variable. They now pass explicit cache paths without mutating the
process environment; the full workspace tests pass with parallel execution.

The VM does not reproduce the physical Synaptics touchpad. Pointer acceleration
now starts at libinput's neutral speed and supports an environment override;
the resulting feel and physical display-driver behavior still require testing
on the laptop. These checks do not constitute a complete disk installation test.
