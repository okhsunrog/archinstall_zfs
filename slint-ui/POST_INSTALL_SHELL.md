# Post-install shell

On the completed-install screen, **Open installed system shell** closes the
current KMS GUI, enters the installed system in the text console, and launches a
new completion GUI after the shell exits and cleanup succeeds. **Quit** still
returns directly to the live shell; **Reboot** remains separate.

Before the shell prompt, a prominent console banner asks the user not to reboot
or power off from chroot: type `exit`, wait for automatic unmount/export and the
completion screen, then choose **Reboot** there. This is guidance, not a command
restriction in the root maintenance shell.

The graphical child never invokes chroot itself. Its parent remains outside
Slint and Tokio and owns this sequence:

1. Receive the non-secret installation identity over a private Unix socket.
2. Wait for the graphical child to terminate; restore the VT and terminal modes.
3. Import the target pool by GUID, without mounting other boot environments.
4. Mount only the installer's root, home, root-home and VM datasets plus the
   recorded EFI PARTUUID under a fresh directory in `/run`.
5. Run `arch-chroot ... /bin/bash --login` through a transient systemd service
   with a PTY. The service owns the shell's descendants; background jobs are
   stopped when the session finishes. Interrupting the PTY client also triggers
   an explicit service stop before cleanup.
6. Unmount the session tree, export its pool and remove the temporary directory.
7. Launch a fresh GUI showing the completion state and the existing installation
   log. It does not load the original config/secrets or register an install
   callback. The UI scale is preserved.

Encrypted targets prompt through `zfs load-key -L prompt`. No encryption or user
password is included in the resume message. A pool already imported by someone
else is refused rather than taken over. Dataset mountpoint properties are not
modified. The current feature is supported on the Linux KMS ISO path; desktop
builds do not claim terminal handoff support.

## Ownership and failure handling

`core::zfs_cleanup::OwnedMounts` contains the expected root dataset, its mount
location and an optional pool owned by the operation. Both normal installation
cleanup and the maintenance session use it. It checks the mount source before
recursive unmount and retains pool ownership when a cleanup command fails so
cleanup can be retried. It never invokes `zfs unmount -a`, forced unmount or
forced export. The installation pipeline still preserves pre-existing pools.

`core::installed_system::InstalledSystem` captures pool GUID, dataset prefix and
EFI PARTUUID before the successful installation exports its pool. Failure to
capture this optional identity disables the shell action without invalidating
an otherwise successful installation.

`slint-ui/src/console_session.rs` owns child processes, the bounded socket
protocol, signals and VT restoration. `installed_shell.rs` owns shell-session
preparation and interaction. `completion.rs` contains only the non-secret result
and its presentation.

Preparation/shell errors return to the completion screen after successful
cleanup; they do not turn a completed installation into a failed installation.
If cleanup is blocked, the console reports the path and error and offers retry
(or reopening the shell while the target is still mounted). No new GUI or
Reboot action is offered while cleanup remains incomplete. If stdin closes,
the supervisor reports the retained resources and exits with an error.

The parent survives a GUI child failure and restores terminal input. Killing
the parent itself with SIGKILL, a kernel panic or loss of power cannot guarantee
resource cleanup. This is a maintenance root shell, not a sandbox against
intentional host modifications.

## Verification

Normal Rust checks:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p archinstall-zfs-slint --no-default-features --features desktop-mock --all-targets -- -D warnings
```

For UI inspection, build with `SLINT_EMIT_DEBUG_INFO=1` and
`--no-default-features --features desktop-mock,slint/mcp`, then render
`--preview done`. Clicking the shell action in preview only updates a simulated
result; no mounting, shell, service or reboot is started.

The ignored `console_session::tests::vm_supervisor_round_trip` test runs the
**real supervisor** on an active VT in a disposable VM. Set `AZFS_TEST_BINARY`
to the newly built KMS GUI and `AZFS_TEST_TARGET` to the fixture's JSON identity.
The production binary has no test-target or resume-file command-line option.

Build both artifacts with `SLINT_EMIT_DEBUG_INFO=1` and `--features slint/mcp`;
use the test executable printed by `cargo test --no-run`. Set
`SLINT_MCP_PORT=9315` for GUI interaction and use actual console key events for
password entry and shell commands. A fixture is provided in
`scripts/shell-vm-fixture.sh`; it refuses disks without its dedicated test serial.

Assert all of these, not only GUI screenshots:

- The initial GUI PID terminates before the shell starts.
- Commands write into the target dataset, not the live root; EFI is mounted.
- After `exit`, a different GUI PID shows completion, the session directory is
  gone, the pool is exported and its systemd service/background jobs are gone.
- An unrelated process holding the mount blocks cleanup and return to the GUI;
  releasing it and retrying completes cleanup.
- Encrypted targets prompt without echo, reopen after export, and clean up when
  unlocking is cancelled.
- Killing the shell/client restores terminal modes and still attempts cleanup.
- Final Quit returns keyboard input to the original live shell.

The fixture exercises real ZFS, EFI, chroot, systemd PTY and KMS handoff. It does
not replace a full distribution installation test or physical-GPU validation.
