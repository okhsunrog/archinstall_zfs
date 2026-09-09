# Slint coding and visual review

This is the working procedure for developers and coding agents changing the
installer UI. It covers design judgment, interaction, and rendering separately.
Use the actual Slint application and deterministic data, not a recreated HTML
page or a generated image that merely resembles the interface.

## 1. Establish the scope and design question

Read the touched view, its shared controls, its Rust controller and the relevant
state/model. Inspect the Slint versions and git revision in
`slint-ui/Cargo.toml` and `Cargo.lock`; use documentation matching that checkout.
If a local Slint checkout is available, its `ai-plugins/skills/slint/` references
on layout, polish, events, interop and MCP are useful. The project workflow must
remain usable without a developer-specific absolute path to that checkout.

Before changing spacing, state what the user must understand and do on the
screen. Ask whether the primary action is clear, whether the information order
supports the decision, and whether the layout still makes sense in other modes.
An interface can render perfectly while presenting the wrong interaction.

Make a small coverage matrix before a broad review:

| Screen/dialog | State and fixture | Display/scale | Interaction | Capture | Finding/status |
| --- | --- | --- | --- | --- | --- |
| Disk / picker | New pool, many partitions | 1920×1080 / 1 | Search, select, cancel, confirm | PNG + tree | Pending |
| Welcome / Wi-Fi | Offline, then connected | 1366×768 / 1 | Connect, Done, reopen settings | PNG + tree | Pending |

Only mark rows reviewed after inspecting their images and performing the listed
actions. Enumerate the screens and states checked in the final report. Do not
say “all UI checked” because every wizard page was opened once.

## 2. Build a safe, representative preview

All commands below run from the repository root:

```sh
SLINT_EMIT_DEBUG_INFO=1 cargo build -p archinstall-zfs-slint \
  --no-default-features --features desktop-mock,slint/mcp --locked
```

Use **both** `desktop-mock` and `--preview`. The feature by itself only mocks
Wi-Fi; preview replaces hardware/installation operations while retaining the
real UI, editing controllers and validation. Never use actual passwords or
private identifiers as review fixtures.

For manual MCP exploration:

```sh
SLINT_BACKEND=headless SLINT_MCP_PORT=9315 target/debug/azfs \
  --preview new-pool --preview-size 1920x1080 --ui-scale 1
```

Omit `SLINT_BACKEND=headless` to use a desktop window. Use your tool's tracked
process/session mechanism for a long-running preview and stop that exact process
afterwards. The capture scripts manage their own child processes and ports.
Do not start untracked shell jobs or kill every `azfs`/QEMU process by name.

Debug info must be emitted **at build time**. Setting that variable only when
starting an old executable does not enable element lookup. Rebuild and restart
after source changes; a running process does not pick up the new binary.

## 3. Use the agreed display matrix

| Configuration | Purpose |
| --- | --- |
| 1920×1080, scale 1 | Primary composition, typography and screenshot baseline |
| 1366×768, scale 1 | Smaller laptop viewport, density and scrolling |
| 1280×800, scale 1 | Additional compact/aspect-ratio check |
| 1920×1080, scale 1.5 | Fractional scaling, alignment and readable controls |
| 1920×1080, scale 2 | Large UI, wrapping, footer and dialog constraints |

Treat 800×600 captures in older reports as historical stress checks, not the
current design baseline. A screenshot's physical dimensions and UI scale both
matter: 1920×1080 at scale 2 has roughly 960×540 logical pixels. Do not resize an
existing PNG and call that a new display test. `Preview.ready()` checks the
actual window size and scale reported by MCP.

## 4. Capture pages, then exercise transitions

```sh
# Every canned scene at the five display configurations above.
uv run python slint-ui/scripts/review.py --output target/ui-review/pages

# Standard editing, Wi-Fi, install/cancel, shell, logs and validation flows.
uv run python slint-ui/scripts/interactions.py --output target/ui-review/flows

# Extended account, regional and desktop/package editor states.
uv run python slint-ui/scripts/design_review.py --output target/ui-review/editors

# Storage selection across modes and adjacent ZFS/Review screens.
uv run python slint-ui/scripts/storage_review.py --size 1920x1080 \
  --output target/ui-review/storage
uv run python slint-ui/scripts/storage_review.py --keyboard-only \
  --size 1920x1080 --scale 2 --output target/ui-review/storage-keyboard
AZFS_PREVIEW_STORAGE=many uv run python slint-ui/scripts/storage_review.py \
  --fixture many --size 1920x1080 --output target/ui-review/storage-many
```

For an isolated change, start with the affected cases instead of blindly
rerunning every scenario:

```sh
uv run python slint-ui/scripts/review.py --scenes welcome offline \
  --sizes 1920x1080@1 1366x768@1 1920x1080@2 --output target/ui-review/network
uv run python slint-ui/scripts/interactions.py --flows wifi \
  --output target/ui-review/network-flows
```

The page script defaults to five configurations; interaction/editor scripts
default to three. `storage_review.py` defaults to **1366×768**, so specify
`--size 1920x1080` for the primary review. Its `--fixture` flag selects assertions,
while `AZFS_PREVIEW_STORAGE` selects data: use matching values (`empty`, `many`,
or `missing`). Do not assume the flag alone changes the inventory.

The [GUI README](../slint-ui/README.md) lists scenes and additional flows.
Scripts emit individual PNGs, element trees, window metadata and process logs.
`review.py` also writes an HTML gallery. Passing assertions is evidence of the
asserted behavior, not an automated design score.

## 5. Inspect full-size images, not just galleries

Open the actual PNG with the available image viewer. Use a gallery/contact
sheet to find inconsistent pages, then inspect each relevant image individually.
At thumbnail size, clipped text, uneven baselines, small hit targets and missing
icon strokes can disappear. Zoom or inspect a crop when necessary; keep the
original full-screen capture as evidence of composition. Crops and overviews do
not replace it.

Review three distinct aspects:

- **Design:** grouping, reading order, primary action, density, typography,
  alignment, spacing, color meaning and consistency with adjacent screens.
- **Interaction:** selection/commit/cancel, focus, keyboard scrolling, validation,
  loading, errors, recovery, disabled explanations and preserved user input.
- **Rendering:** clipping, overlap, ellipsis, icon alignment, scaling and redraw.

In particular, apply these project decisions:

- Keep comparable values and metadata groups on consistent columns/edges.
  Badges must not float according to the model name's length or distribute
  themselves across spare row width. Size/type/transport are secondary metadata.
- Use compact storage assignment summaries with Choose/Change opening a bounded,
  searchable picker. Do not repeat the entire partition inventory under every
  assignment. Check Full Disk, New Pool and Existing Pool, and their effects on
  ZFS and Review. Preserve enough device identity to avoid an ambiguous choice;
  explain why a partition is unavailable.
- Give actions visible button treatment. Quit should look clickable; a small
  bare word is insufficient. Keep related actions together with a clear primary
  and secondary order. Use explicit verbs instead of relying on a chevron.
- With Wi-Fi hardware present, **Wi-Fi & network** with its icon remains available
  before and after connection. It is prominent offline and secondary once the
  user can proceed. Connection success leads to **Done**; disconnect belongs in
  connection management. A VM without Wi-Fi hardware will not show this button.
- Keep installation logs above the lower action card. The card holds progress,
  status and buttons; it stays available while logs scroll. Inspect completion,
  failure, cancelling/cancelled, and return from the installed-system shell.
- Use consistent heading/body/secondary sizes and restrained rounding. Prefer
  the existing 44 logical-pixel height for prominent actions; compact row actions
  can use the established 40-pixel pattern. Do not shrink text/buttons to make
  an overfull screen fit. Allow the appropriate body to scroll.
- Keep labels visible when inputs contain text. Validation should explain the
  problem near the field and preserve entered values. Ordinary settings should
  not look like success notifications merely because they have a value.

Include ready, empty, loading, success, error, disabled, selected and focused
states where they exist. Exercise long labels/serials/paths, missing metadata,
many items, search with no results, and switching modes after making selections.

## 6. Drive the real UI with MCP

Use `list_windows`, then `get_window_properties` to obtain the current window
and root element handles. Inspect `get_element_tree` or look up ids. Window and
element handles have similar shapes but are not interchangeable; do not paste a
handle from an earlier run into a new process.

Use `click_element`, `set_element_value` and `dispatch_key_event` to perform real
actions. Prefer stable accessible roles/labels or ids over absolute coordinates.
Wait for a meaningful state, not an arbitrary long sleep. Check that element
trees are not truncated and inspect geometry as well as visible text.

The shared `Preview` helper in `slint-ui/scripts/review.py` already implements
this HTTP workflow and process cleanup. Extend existing flows for regressions
rather than creating an independent automation framework. Where direct API calls
are needed, discover the tools/schemas supported by the pinned Slint build.

Test Tab/Shift+Tab, arrows, Enter and Escape as applicable. Opening a dialog must
put focus somewhere useful; closing it must restore focus to the triggering
control. Scroll a focused control into view. Verify selection cancellation does
not silently commit a tentative choice. Distinguish row selection from confirming
an action, and errors from simply having no results.

## 7. Write Slint code to support those decisions

Use the project's existing controls (`StyledButton`, `StyledTextInput`, `Icon`,
`SignalBars`, `SetupRow`, `ConfigItem`) and the palette/spacing/type/radius tokens
in `slint-ui/ui/styling.slint`. Read the component API before adding another control or
hardcoding a new visual convention. Standard widgets remain useful, but replacing
the shared controls wholesale is not required to fix one page.

- Use `HorizontalLayout`, `VerticalLayout` and `GridLayout` for normal placement.
  Reserve manual coordinates for overlays/custom drawing. Padding belongs on a
  layout, not a `Text` element.
- Distinguish fill, preferred size and minimum size. Use stretch factors for
  flexible space, and bound metadata/button groups that should keep their size.
  Avoid making a row's alignment depend on a long label's implicit width.
- Wrap an entire phase-specific layout in `if`. An always-present empty layout
  can still consume height even when all its children are hidden.
- Constrain a scrollable body to the available area. Do not bind its preferred
  height to the whole content when the parent needs to limit it. Keep footer
  actions outside that scrolling body, and clip at the body boundary.
- Center icons within their allocated layout cell; a horizontal wrapper can be
  necessary inside a vertical layout. Reuse `Icon` and its tint/size behavior.
- Use meaningful property directions (`in`, `out`, `in-out`, private). A binding
  reacts to dependencies; imperative assignment can change that relationship.
  Use two-way bindings only when both sides really own edits. Keep bindings pure.
- Give custom controls correct accessible roles, labels, enabled/selected state,
  focus behavior and keyboard activation. New icons must not remove action text
  where that text explains the action.
- Keep presentation state in Slint globals/adapters and business operations in
  Rust controllers/core. Do not put disk or network operations in layout code.
  Run blocking work away from the UI thread and return updates through Slint's
  event-loop mechanism. Follow existing weak-handle and cancellation patterns.
- Guard async searches, scans and validation against stale responses after a
  newer request, clearing a field, switching modes or closing a dialog. Invalid
  submissions should retain the user's input for correction.

When a bug only appears with real data, extend `slint-ui/src/preview.rs` and its
fixtures to represent that shape. Drive the same component/controller paths;
do not fix only the preview or manually force final globals to bypass the
interaction being tested. Keep all simulated operations inside preview mode.

## 8. Keep hardware verification separate

Headless previews cannot validate LinuxKMS partial redraw, physical cursor
damage, touchpad acceleration, VT keyboard isolation, real Wi-Fi timing or a
post-install chroot. Use the default LinuxKMS build for those. The
[developer guide](development.md) and
[post-install shell guide](../slint-ui/POST_INSTALL_SHELL.md) describe the paths.

For VT isolation, use disposable marker input, never real credentials. Test that
typing in the GUI does not become a shell command after normal exit or a GUI
child crash, and that the console works after restoration. Mouse/touchpad speed
must be judged on the target device; a static screenshot cannot establish it.

For backend changes, test incremental redraw against full repaint where possible,
then check cursor movement, clipping at edges, hiding/showing and scale changes
on KMS. Do not change renderer internals to compensate for an application layout
mistake without evidence that the backend is responsible.

## 9. Finish with evidence and a bounded claim

After implementation, run the appropriate Rust checks from
[AGENTS.md](../AGENTS.md), review the changed diff, and save the screenshots.
For shared controls, revisit their affected consumers and state variants.

Report the source revision/features, tested display sizes/scales, interactions,
findings fixed, artifact paths, and what remains untested. Keep dated reports such
as [Design review](../slint-ui/DESIGN_REVIEW.md) distinct from this living workflow.
Do not recycle an old screenshot as evidence of a new build. Documentation-only
edits do not constitute another visual review of the application.
