## Swap support plan

Goal: Add swap support with two safe options — ZRAM and a traditional swap partition — while explicitly not supporting swap on ZFS zvols per Arch Wiki and OpenZFS guidance.

### Scope (v1)
- Modes: None, ZRAM only, ZSWAP + swap partition, ZSWAP + encrypted swap partition
- Default: None
- No zvol swap, no swapfile on ZFS
- No hibernation/resume support in v1 (documented limitation)

### User‑facing changes
- New installer menu section: "Swap"
  - Options:
    - None
    - ZRAM only (disables zswap)
    - ZSWAP + swap partition
    - ZSWAP + encrypted swap partition (dm-crypt with random key each boot)
  - If ZRAM: optional advanced settings later (v1 uses sensible defaults)
  - If swap partition: only supported in Full‑disk mode for v1; asks for size (e.g. 0.5G, 8G). For other modes, show info message that only ZRAM is supported in v1

### Configuration model
- Add `SwapMode` enum: `none | zram | zswap_partition | zswap_partition_encrypted`
- Extend `GlobalConfig`:
  - `swap_mode: SwapMode = none`
  - `swap_partition_size: str | None = None` (e.g. "8G"; used when mode is a partition mode in full‑disk)
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

2) ZSWAP + swap partition (Full‑disk install only in v1)
   - Enable zswap (`zswap.enabled=1` kernel parameter via ZFSBootMenu dataset property)
   - Partition layout: EFI (p1), Swap (p2), ZFS (p3)
   - Create swap of requested size (e.g. `sgdisk -n 2:0:+<size> -t 2:8200`)
   - Create ZFS partition from the remainder (`-n 3:0:0 -t 3:bf00`)
   - Format swap (`mkswap`); do not activate it on the live ISO
   - fstab: rely on `genfstab` to include the swap UUID line (we already filter only ZFS lines); if missing, append `UUID=<uuid> none swap defaults 0 0`
   - Hibernation: out of scope in v1

3) ZSWAP + encrypted swap partition (Full‑disk install only in v1)
   - Enable zswap (`zswap.enabled=1` kernel parameter via ZFSBootMenu dataset property)
   - Partition layout: EFI (p1), Swap (p2), ZFS (p3)
   - Do not format underlying p2 directly
   - Add `/etc/crypttab` entry (use PARTUUID for stability), example:
     - `cryptswap PARTUUID=<p2-partuuid> /dev/urandom swap,cipher=aes-xts-plain64,size=256`
   - Add `/etc/fstab` entry referencing the mapped device:
     - `/dev/mapper/cryptswap none swap defaults 0 0`
   - Let systemd-cryptsetup generator create `systemd-cryptsetup@cryptswap.service` which will `systemd-makefs swap /dev/mapper/cryptswap` on first boot
   - Hibernation: not supported with random key each boot

4) Existing/New pool modes in v1
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
- Toggle zswap via kernel parameter per selected mode: disable for ZRAM, enable for ZSWAP modes
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


