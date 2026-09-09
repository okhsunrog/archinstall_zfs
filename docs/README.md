# Documentation

| Document | Purpose |
| --- | --- |
| [Developer guide](development.md) | Build environments, checks, release/USB workflow and validation boundaries |
| [Alongside installation development](dual-boot-development.md) | Storage safety contract, EFI policy and disposable resize tests; integration in progress |
| [Slint coding and visual review](slint-ui-review.md) | Practical design, interaction, rendering and implementation procedure |
| [GUI development](../slint-ui/README.md) | Preview scenes, fixtures and executable review commands |
| [Live binary updates](../gen_iso/LIVE_UPDATE.md) | Update the installer on Ventoy without rebuilding its base ISO |
| [Post-install shell](../slint-ui/POST_INSTALL_SHELL.md) | VT handoff, chroot, owned cleanup and completion GUI restart |
| [Boot debugging](debugging-boot.md) | Inspect a disposable installed system or USB boot in QEMU |
| [ZFS root installation guide](zfs-root-install-guide.md) | Manual installation and the installer's system setup |
| [Installation benchmark results](install-bench-results.md) | Recorded measurements, not a performance guarantee |
| [Design review, 2026-09-09](../slint-ui/DESIGN_REVIEW.md) | Dated design findings and coverage |
| [Earlier UI/KMS review](../slint-ui/UI_REVIEW.md) | Historical rendering/input investigation |

Coding-agent instructions live in [AGENTS.md](../AGENTS.md).
Current procedures belong in the guides; dated results describe the tested
revision and do not establish that later changes have been rechecked.
