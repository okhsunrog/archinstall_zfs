# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Overview

`archinstall_zfs` is an Arch Linux installer focused on ZFS root setups. It has:

- `core/`: shared installer logic, config, ZFS/disk/network/system helpers.
- `tui/`: ratatui terminal installer.
- `slint-ui/`: graphical installer built with Slint.
- `xtask/`: development and QEMU install-test harnesses.
- `gen_iso/`: Arch ISO profiles and build assets.

The workspace is Rust. Keep changes scoped and prefer existing local patterns over new abstractions.

## Common Commands

Use these before committing unless the change clearly does not require the full set:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Useful targeted checks:

```sh
cargo test -p archinstall-zfs-core prepare
cargo test -p archinstall-zfs-slint
cargo check -p archinstall-zfs-slint --no-default-features --features desktop-mock
```

Real install tests require a built `xtask` and installer binary plus QEMU/KVM:

```sh
cargo build --release --bin azfs-tui --bin xtask
just test-install-encrypted-pool --tmpfs --timeout 1800
```

## Git Workflow

- Work from a feature branch off `main`.
- Do not revert unrelated local changes.
- Delete local branches only when they are merged and unrelated to active work.
- Prefer focused commits with clear messages.

## Wi-Fi Backends

`slint-ui` chooses the core Wi-Fi backend by feature:

- `linuxkms` (default): installer ISO path, Slint Linux KMS backend, `wifi-iwd`.
- `desktop`: local desktop development path, winit backend, `wifi-nm`.
- `desktop-mock`: deterministic canned Wi-Fi data, winit backend, `wifi-mock`.

For UI iteration, prefer `desktop-mock` so tests do not depend on host Wi-Fi hardware, iwd, NetworkManager, or real credentials.

## Slint UI Development Loop

When touching `.slint` files, do not rely on compile success alone. Render and interact with the UI yourself.

Recommended agentic loop:

1. Build the app with Slint MCP support and debug info:

   ```sh
   SLINT_EMIT_DEBUG_INFO=1 cargo build \
     -p archinstall-zfs-slint \
     --no-default-features \
     --features desktop-mock,slint/mcp
   ```

   `SLINT_EMIT_DEBUG_INFO=1` must be present at build time. Setting it only when running the already-built binary is not enough for MCP element-tree and id lookup.

2. Run the app headlessly with MCP:

   ```sh
   SLINT_MCP_PORT=9315 SLINT_BACKEND=headless target/debug/azfs
   ```

   The app should log:

   ```text
   Slint MCP server listening on http://127.0.0.1:9315/mcp
   ```

3. Use MCP over HTTP to inspect and drive the UI:

   ```sh
   curl -s -X POST http://127.0.0.1:9315/mcp \
     -H 'Content-Type: application/json' \
     -H 'Accept: application/json, text/event-stream' \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_windows","arguments":{}}}'
   ```

   Useful MCP tools:

   - `list_windows`
   - `get_window_properties`
   - `get_element_tree`
   - `find_elements_by_id`
   - `query_element_descendants`
   - `click_element`
   - `set_element_value`
   - `dispatch_key_event`
   - `take_screenshot`

4. Save screenshots and inspect them visually:

   ```sh
   curl -s -X POST http://127.0.0.1:9315/mcp \
     -H 'Content-Type: application/json' \
     -H 'Accept: application/json, text/event-stream' \
     -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"take_screenshot","arguments":{"windowHandle":{"index":"1","generation":"1"},"imageMimeType":"image/png"}}}' \
     > /tmp/azfs-screenshot.json

   jq -r '.result.content[] | select(.type=="image") | .data' /tmp/azfs-screenshot.json \
     | base64 -d > /tmp/azfs-screenshot.png
   ```

   Then inspect with the local image viewer tool. Do not ask the human to verify basic layout and interaction issues that the agent can verify with screenshots and MCP.

5. Drive realistic flows end to end:

   - Open the Wi-Fi popup from the welcome connectivity pill.
   - Wait for mock scan results.
   - Select a secured unknown network and verify password entry.
   - Set a passphrase with `set_element_value`.
   - Click `Connect` and wait through connecting/verifying/connected states.
   - Test enterprise-network error state.
   - Test `Back`, `Rescan`, `Disconnect`, and known-network `Forget`.
   - Take screenshots for the important states and inspect for clipping, overlap, off-center icons, missing buttons, and truncated text.

6. After visual and interaction checks, run normal Rust checks:

   ```sh
   cargo fmt --all --check
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   ```

## Slint Layout Notes

- Use layouts (`VerticalLayout`, `HorizontalLayout`, `GridLayout`) instead of manual positions, except for overlays.
- A layout with no visible children can still affect sizing if it exists unconditionally. For phase-specific footers, wrap the whole layout in an `if` so it is not allocated in phases that do not need it.
- Scrollable content should fill the allocated body area. Avoid binding a `ScrollView` preferred height to its content height when the parent is meant to constrain it.
- Set `clip: true` on body containers where phase-specific content must not draw outside its allocated area.
- For status icons inside `VerticalLayout`, wrap the icon in a centered `HorizontalLayout` if visual centering matters.
- Prefer existing shared components such as `StyledButton`, `StyledTextInput`, `Icon`, and `SignalBars`.
- Keep UI state in Slint globals and async/business logic in Rust controllers.

## Slint MCP Pitfalls

- `slint/mcp` is a Cargo feature. Add it only on the command line for debug runs.
- `SLINT_MCP_PORT` starts the MCP server at runtime.
- `SLINT_EMIT_DEBUG_INFO=1` is needed at build time for element ids and tree traversal.
- If MCP returns errors about `ElementHandle API requires debug info`, rebuild with `SLINT_EMIT_DEBUG_INFO=1`.
- Window handles and element handles have the same shape but are not interchangeable.
- Headless screenshots can work even when element-tree lookup is broken; do not mistake screenshot success for full MCP readiness.

## Slint References

The local Slint checkout contains agent docs and deeper MCP documentation:

- `/home/okhsunrog/code/rust/slint/ai-plugins/skills/slint/SKILL.md`
- `/home/okhsunrog/code/rust/slint/ai-plugins/skills/slint/reference/debugging-and-mcp.md`
- `/home/okhsunrog/code/rust/slint/ai-plugins/skills/slint/reference/polish.md`
- `/home/okhsunrog/code/rust/slint/docs/development/mcp-server.md`
- `/home/okhsunrog/code/rust/slint/docs/astro/src/content/docs/guide/tooling/ai-coding-assistants.mdx`

Read the relevant file before doing non-trivial Slint work.
