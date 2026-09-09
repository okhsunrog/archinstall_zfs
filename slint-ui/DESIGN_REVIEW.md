# Installer design review — 2026-09-09

This is a dated result, not a claim that later changes have been reviewed.
For the current procedure, see [Slint coding and visual review](../docs/slint-ui-review.md).

This pass follows the storage redesign and reviews the remaining screens as
user interfaces: hierarchy, decision clarity, action placement, density, and
feedback. The primary reference is 1920×1080 at 100%. Secondary checks use
1366×768 at 100% and 1920×1080 at 200%. Screenshots come from the running Slint
application with deterministic preview fixtures, not image mockups.

## Findings and changes

| Area | Problem | Result |
| --- | --- | --- |
| System, Users, Desktop | Values drifted to the far side of rows; chevrons hid the editing action; ordinary values looked like success statuses. | Stable label/value columns, neutral values, explicit outlined Change/Manage buttons, and real checkboxes for toggles. |
| System | Repeated section labels and a long city list made regional setup harder to scan. | Base system and Language and region groups; city search and current timezone selection. |
| Users | Administrator rights were toggled by clicking an otherwise unlabelled account row. Invalid input silently disappeared. | Labelled administrator checkboxes, explicit Remove buttons, inline validation that preserves input, and explanations of account/password behavior. |
| Account form | At 200% scale Add user could scroll away while Done remained visible. | Both actions stay in the footer. Done reports an unfinished account instead of silently discarding it. |
| Desktop | Audio and seat access expanded into repeated radio rows, dominating the page. | Compact choices and separate environment, profile, sound, hardware, and software groups. Console only explicitly describes the profile without a graphical environment. |
| Packages | Selection and removal depended on clicking small result/chip text. Async searches could overwrite newer queries. | Separate result and selected-package lists, explicit Add/Remove controls, source labels, empty/error feedback, and stale-result rejection. |
| Services and optional packages | Actions, descriptions, and inputs lacked a consistent hierarchy. | Persistent field labels, readable descriptions, explicit service removal, and consistent dialog footers. |
| Selection and text dialogs | Small headings/actions, ambiguous OK, poor Escape/focus behavior, and truncated short lists. | Consistent headings and controls, Save/Cancel, bounded scrolling, keyboard focus recovery, and adequate short-list height. |
| Welcome | An offline user had no prominent action to configure networking; retry competed with the status row. | Network settings is prominent when offline. Status rows align; preparation/failure explanations sit below them. |
| Wi-Fi | Empty and unavailable states gave the same unhelpful message; error actions lacked a clear priority. | Distinct explanations, readable feedback, consistent password controls, and Back to networks as the primary error action. Successful connection retains Done as the primary action. |
| Installation | Reconsidered log/action balance against the accepted design. | Retained logs above the lower action card; checked progress, completion, failure, cancellation, and simulated return from the shell. |

## Coverage and reproduction

Build with `SLINT_EMIT_DEBUG_INFO=1`, `desktop-mock,slint/mcp`, then run the
commands in [README](README.md). The complementary checks are:

- `scripts/review.py`: page composition and ready/offline, missing UEFI, ZFS
  preparation/failure, empty/unavailable Wi-Fi, connection verification,
  no-internet, installation and cancellation states.
- `scripts/interactions.py`: editing, keyboard navigation/focus recovery,
  Wi-Fi password entry and connection management, pool inspection, simulated
  installation, completion, cancellation, shell return, log scrolling, and
  invalid-configuration blocking.
- `scripts/design_review.py`: account validation, duplicate names, administrator
  changes, pending input, removal/empty lists; distribution/timezone/locale/
  keyboard/download editors; optional packages, display manager and GPU lists;
  KDE, Sway and console-only page variants; empty services, repository/AUR
  searches, and adding/removing packages.
- `scripts/storage_review.py`: regression coverage of the earlier storage
  redesign after shared component and focus changes.

The scripts save individual PNGs and element trees for visual inspection.
Passing assertions alone is not a design verdict. Layout decisions were checked
on individual screenshots, including long text, errors, empty lists, open
menus, and 200% scale; small overview thumbnails are insufficient for this.

Validation completed for this pass: the 10 standard interaction flows at all
three configurations, the three extended editor flows at all three
configurations, and storage interaction at Full HD (mouse at 100%, keyboard at
200%). Formatting, workspace tests (314 passed, two ignored), and workspace
clippy with warnings denied passed. Visual artifacts from this local run are
under `target/ui-design-rest/index.html`; the scripts regenerate them after a
clean checkout.

## Boundaries

The scope covers all installer sections and the listed dialog/state families.
It is not an exhaustive combination of every distribution, desktop profile,
package inventory, text length, font, and display size. The graphical choices
were inspected with KDE, Sway, and console-only fixtures; this does not establish
that every desktop package set installs successfully.

Preview mode retains real UI callbacks and editing validation, but substitutes
hardware inventory, networking/package results, installation, and reboot. This
pass does not retest LinuxKMS cursor/input isolation on hardware, real Wi-Fi
timing, destructive installation, post-install chroot, or USB image writing.
The earlier [UI and LinuxKMS review](UI_REVIEW.md) is a historical record of those
separate checks, not evidence that they were repeated here.
