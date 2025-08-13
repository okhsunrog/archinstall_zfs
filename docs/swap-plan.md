## Swap support plan

Goal: Add swap support with two safe options — ZRAM and a traditional swap partition — while explicitly not supporting swap on ZFS zvols per Arch Wiki and OpenZFS guidance.

### Scope (v1)
- Modes: None, ZRAM, Swap partition
- Default: None
- No zvol swap, no swapfile on ZFS
- No hibernation/resume support in v1 (documented limitation)

### User‑facing changes
- New installer menu section: "Swap"
  - Options: None, ZRAM, Swap partition
  - If ZRAM: optional advanced settings later (v1 uses sensible defaults)
  - If Swap partition: only supported in Full‑disk mode for v1; asks for size (e.g. 0.5G, 8G). For other modes, show info message that only ZRAM is supported in v1

### Configuration model
- Add `SwapMode` enum: `none | zram | partition`
- Extend `GlobalConfig`:
  - `swap_mode: SwapMode = none`
  - `swap_partition_size: str | None = None` (e.g. "8G"; used only when mode == partition in full‑disk)
  - `zram_size_expr: str | None = "min(ram / 2, 4096)"` (default based on ArchWiki guidance)
  - `zram_fraction: float | None = None` (if set, takes precedence over size expression)

### Installer behavior
1) ZRAM mode
   - Ensure package `zram-generator` is installed
   - Write `/etc/systemd/zram-generator.conf` in target with defaults:
     - `[zram0]`
     - `zram-size = min(ram / 2, 4096)` (or `zram-fraction = X` if fraction is set)
     - `compression-algorithm = zstd`
     - `swap-priority = 100`
   - Keep zswap disabled to avoid intercepting zram (kernel parameter `zswap.enabled=0` already present)
   - No initramfs changes needed

2) Swap partition mode (Full‑disk install only in v1)
   - Partition layout: EFI (p1), Swap (p2), ZFS (p3)
   - Create swap of requested size (e.g. `sgdisk -n 2:0:+<size> -t 2:8200`)
   - Create ZFS partition from the remainder (`-n 3:0:0 -t 3:bf00`)
   - Format swap (`mkswap`); do not activate it on the live ISO
   - fstab: rely on `genfstab` to include the swap UUID line (we already filter only ZFS lines); if missing, append `UUID=<uuid> none swap defaults 0 0`
   - Hibernation: out of scope in v1 (no `resume=` kernel arg or initramfs resume hooks)

3) Existing/New pool modes in v1
   - Do not create swap partitions
   - Suggest ZRAM for these modes; potential future enhancement: allow selecting an existing swap partition by‑id

### Files/code to touch
- `archinstall_zfs/menu/models.py`
  - Add `SwapMode` enum and new fields in `GlobalConfig`
- `archinstall_zfs/menu/global_config.py`
  - Add "Swap" menu item and sub‑dialog to pick mode and, when applicable, partition size
  - Validation: partition size required in full‑disk + partition mode; for non full‑disk modes block partition selection
- `archinstall_zfs/disk/__init__.py`
  - Update full‑disk `create_partitions()` to optionally insert swap as p2 and move ZFS to p3
  - Add helper to `mkswap` the new partition and record its by‑id path
- `archinstall_zfs/main.py`
  - When `swap_mode == zram`: install `zram-generator`, write `zram-generator.conf`
  - When `swap_mode == partition`: ensure swap formatting done; after `genfstab`, verify swap entry exists (append if needed)

### Safety and constraints
- No zvol swap, per Arch Wiki and OpenZFS warnings
- Keep `zswap.enabled=0` (already set via ZFSBootMenu command line); ZRAM path does not use zswap
- If user selects partition mode outside of full‑disk, show message and prevent proceeding (v1)

### Acceptance criteria
- Selecting ZRAM results in `zram-generator` installed and an effective `/proc/swaps` entry after first boot
- Selecting Swap partition (full‑disk): disk ends with p1=EFI, p2=Linux swap, p3=ZFS; `/etc/fstab` contains a valid swap UUID line
- Selecting None: no zram config file installed, no swap partition created; fstab has no swap line
- CI/lint/tests pass

### Future work (post‑v1)
- Allow selecting an existing swap partition for new/existing pool modes
- Hibernation support (resume kernel arg and initramfs integration)
- Advanced ZRAM tuning (multiple devices, bounds, NUMA)

### References
- ArchWiki — Zram: https://wiki.archlinux.org/title/Zram
- ArchWiki — Zswap: https://wiki.archlinux.org/title/Zswap


