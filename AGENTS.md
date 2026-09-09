# Repository instructions

## Start here

Read [Developer guide](docs/development.md) for environments, checks, release
artifacts and USB updates. For Slint work, also read
[Slint coding and visual review](docs/slint-ui-review.md) and the
[GUI README](slint-ui/README.md). [Documentation index](docs/README.md) links the
remaining guides. Keep shared rules in this file; `CLAUDE.md` points here.

`archinstall_zfs` is a Rust workspace: `core/` owns installer operations and
validation, `tui/` the terminal frontend, `slint-ui/` the GUI, `xtask/` build/test
harnesses, and `gen_iso/profile/` versioned live-image inputs. Prefer existing
patterns over new abstractions; do not patch only generated profiles or artifacts.

## Development and checks

- Use `cargo add` or the relevant package-manager command for dependencies;
  inspect manifest and lockfile changes. Prefer Rust edition 2024.
- Run Python with `uv run`, add Python dependencies with `uv add` if needed.
- Use tracked process/session tools for previews and VMs. Stop the process you
  started, not all processes matching a name. Do not launch untracked shell jobs.
- Keep blocking work off the UI thread and follow existing cancellation,
  weak-handle and event-loop update patterns.

Before committing, run these unless the change clearly does not require them:

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Useful targeted checks:

```sh
cargo test -p archinstall-zfs-core prepare --locked
cargo test -p archinstall-zfs-slint --locked
cargo check -p archinstall-zfs-slint --no-default-features --features desktop-mock --locked
just test-live-update
```

Documentation-only edits need accurate commands and working links. UI changes
need runtime checks in addition to Rust checks. Boot/input/cleanup changes need
the relevant VM or hardware test; a passing unit test does not prove that path.
For disposable install tests, build with `just cargo-build`, then use
`just test-install-encrypted-pool --tmpfs --timeout 1800`.

## Git workflow

- Work from a feature branch off `main`; preserve unrelated local changes.
- Commit completed, validated logical steps separately with clear messages.
- Delete local branches only when merged and unrelated to active work.
- Never mention assistants or AI tools in commit messages, trailers, branch
  names, PR titles or PR descriptions. Do not add generated-by/session trailers.
- PR descriptions: a short paragraph describing what and why, then bullets.
  Add a Breaking changes section only for breaking changes, and Testing only for
  manual/hardware checks not covered by CI.
- Do not post GitHub/GitLab review replies or comments. Draft replies in chat for
  the user to edit and post.

## Safe UI development

The GUI features select these networking/rendering paths:

| Feature | Backend | Wi-Fi |
| --- | --- | --- |
| `linuxkms` (default) | LinuxKMS, software Skia | iwd |
| `desktop` | winit | NetworkManager |
| `desktop-mock` | winit | Deterministic mock |

**`desktop-mock` alone mocks only Wi-Fi. Always pass `--preview` for UI review on
the development host.** Preview also replaces storage/system probes, package
search, installation and reboot, while retaining real components and editing
callbacks. Use disposable fixture credentials.

```sh
SLINT_EMIT_DEBUG_INFO=1 cargo build -p archinstall-zfs-slint \
  --no-default-features --features desktop-mock,slint/mcp --locked
SLINT_BACKEND=headless SLINT_MCP_PORT=9315 target/debug/azfs \
  --preview welcome --preview-size 1920x1080 --ui-scale 1
```

Emit debug info at build time for MCP element lookup. Rebuild and restart after
edits. Keep MCP/mock features out of production artifacts; use `just cargo-build`
for the LinuxKMS release binary. Read the runtime and `slint-build` pins in
`slint-ui/Cargo.toml` and `Cargo.lock` before changing the Slint dependency.

## UI review requirements

- Review **design, interaction and rendering** separately. Correct drawing does
  not prove good hierarchy, alignment, action placement or information density.
- Use 1920×1080 at 100% as the primary baseline, plus compact and scaled checks
  from the [review guide](docs/slint-ui-review.md). Do not use 800×600 as the main
  design reference or resize a screenshot to simulate another viewport.
- Capture the real Slint preview. Inspect individual full-size screenshots;
  galleries/contact sheets are overviews, not sufficient evidence for details.
- Record a coverage matrix of pages, modes, dialogs, data states, scale and
  interactions. Exercise keyboard navigation, focus return, validation, errors,
  empty/loading states and long/missing/many-item fixtures where applicable.
- Extend existing `slint-ui/scripts/` flows and `slint-ui/src/preview.rs` fixtures
  when needed. Do not bypass the callback being tested by directly forcing its
  final state. `storage_review.py --fixture` needs the matching
  `AZFS_PREVIEW_STORAGE` environment value to select fixture data.
- Use shared controls and `slint-ui/ui/styling.slint` tokens, layouts instead of manual
  positioning, bounded scrolling bodies and fixed action footers. Wrap an entire
  conditional layout in `if` so an empty layout does not consume space.
- Revisit consumers when a shared component changes. Do not call the entire UI
  reviewed after opening each page once or merely passing capture assertions.
- State exactly what was checked. Headless previews do not establish physical
  touchpad feel, KMS cursor damage, VT keyboard isolation or real chroot cleanup.

## Live image and physical media

Follow [Live binary updates](gen_iso/LIVE_UPDATE.md) for `/azfs-update/azfs` on
Ventoy. The service copies a trusted release binary into the live writable root
before consoles start; it does not launch the GUI. A base ISO with that service
is required, and kernel/ZFS/service changes still require a new base ISO.

Before writing, identify physical media by current model, serial and mount source.
Preserve unrelated files, verify copied data, and unmount/eject only after writes
and any VM using the drive finish. Never infer physical boot success from QEMU.
Follow [Post-install shell](slint-ui/POST_INSTALL_SHELL.md) for VT/chroot ownership
and cleanup; simulated shell screenshots do not validate that lifecycle.
