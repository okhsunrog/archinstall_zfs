## Swap support plan

Goal: Add swap support with two safe options — ZRAM and a traditional swap partition — while explicitly not supporting swap on ZFS zvols per Arch Wiki and OpenZFS guidance.

### Scope
- Modes: None, ZRAM only, ZSWAP + swap partition, ZSWAP + encrypted swap partition
- Default: None
- No zvol swap, no swapfile on ZFS
- No hibernation/resume support

### User‑facing changes
- New installer menu section: "Swap"
  - Options:
    - None
    - ZRAM only (disables zswap)
    - ZSWAP + swap partition
    - ZSWAP + encrypted swap partition (dm-crypt with random key each boot)
  - If ZRAM: optional advanced settings later (uses sensible defaults)
  - If swap partition:
    - Full‑disk: ask for swap size (e.g. 16G)
    - New/Existing pool: prompt to select an existing partition by-id for swap

### Configuration model
- Add `SwapMode` enum: `none | zram | zswap_partition | zswap_partition_encrypted`
- Extend `GlobalConfig`:
  - `swap_mode: SwapMode = none`
  - `swap_partition_size: str | None = None` (e.g. "16G"; used when mode is a partition mode in full‑disk)
  - `swap_partition_by_id: str | None = None` (used in non‑full‑disk ZSWAP modes)
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

2) ZSWAP + swap partition
   - Enable zswap (`zswap.enabled=1` kernel parameter via ZFSBootMenu dataset property)
   - Full‑disk:
     - Partition layout: EFI (p1), ZFS (p2), Swap (p3)
     - Create ZFS as p2 using the disk minus requested swap tail: `sgdisk -n 2:0:-<size> -t 2:bf00`
     - Create Swap as p3 at the end of disk: `sgdisk -n 3:0:0 -t 3:8200`
     - Format swap (`mkswap` p3); do not activate it on the live ISO
   - New/Existing pool:
     - Use selected partition by-id; format with `mkswap`
   - fstab: rely on `genfstab` to include the swap UUID line (we already filter only ZFS lines); if missing, append `UUID=<uuid> none swap defaults 0 0`
   - Hibernation: out of scope

3) ZSWAP + encrypted swap partition
   - Enable zswap (`zswap.enabled=1` kernel parameter via ZFSBootMenu dataset property)
   - Full‑disk:
     - Partition layout: EFI (p1), ZFS (p2), Swap (p3)
     - Do not format underlying p3 directly
   - New/Existing pool:
     - Use selected partition by-id; do not format it directly
   - Add `/etc/crypttab` entry (use PARTUUID for stability), example:
     - `cryptswap PARTUUID=<swap-partuuid> /dev/urandom swap,cipher=aes-xts-plain64,size=256`
   - Add `/etc/fstab` entry referencing the mapped device:
     - `/dev/mapper/cryptswap none swap defaults 0 0`
   - Let systemd-cryptsetup generator create `systemd-cryptsetup@cryptswap.service` which will `systemd-makefs swap /dev/mapper/cryptswap` on first boot
   - Hibernation: not supported with random key each boot

4) Existing/New pool modes
   - No partitioning changes by installer
   - If a ZSWAP mode is selected, user picks an existing partition by-id for swap (unencrypted or encrypted)
   - ZRAM remains available as a diskless option

### Files/code to touch
- `archinstall_zfs/menu/models.py`
  - Add `SwapMode` enum and new fields in `GlobalConfig`
- `archinstall_zfs/menu/global_config.py`
  - Add "Swap" menu item and sub‑dialog to pick mode; prompt for size (full‑disk ZSWAP modes) or pick partition by-id (non‑full‑disk ZSWAP modes)
  - Validation: full‑disk+ZSWAP require `swap_partition_size`; non‑full‑disk+ZSWAP require `swap_partition_by_id`
- `archinstall_zfs/disk/__init__.py`
  - Update full‑disk `create_partitions()` to create p1=EFI, p2=ZFS (to -<swap_size>), p3=Swap (tail)
  - Add helper to `mkswap` the new swap partition and record its by‑id path
- `archinstall_zfs/main.py`
  - When `swap_mode == zram`: install `zram-generator`, write `zram-generator.conf`
  - When `swap_mode` is a ZSWAP mode:
    - Full‑disk: format p3 if unencrypted; for encrypted, write `crypttab` (PARTUUID) and `fstab` for `/dev/mapper/cryptswap`
    - Non‑full‑disk: operate on the selected partition similarly
    - Ensure zswap kernel parameter matches the selected mode
- `archinstall_zfs/zfs/__init__.py`
  - Set `org.zfsbootmenu:commandline` to include `zswap.enabled=0` for ZRAM or `zswap.enabled=1` for ZSWAP modes

### Safety and constraints
- No zvol swap, per Arch Wiki and OpenZFS warnings
- Toggle zswap via kernel parameter per selected mode: disable for ZRAM, enable for ZSWAP modes
- For non‑full‑disk ZSWAP modes, require selecting an existing partition; the installer does not create/resize partitions in these modes

### Acceptance criteria
- Selecting ZRAM results in `zram-generator` installed and an effective `/proc/swaps` entry after first boot
- Selecting ZSWAP + swap partition (full‑disk): disk ends with p1=EFI, p2=ZFS, p3=Linux swap; `/etc/fstab` contains a valid swap UUID line
- Selecting ZSWAP + encrypted swap partition: `/etc/crypttab` (PARTUUID + /dev/urandom + swap options) and `/etc/fstab` for `/dev/mapper/cryptswap` are present; swap activates on boot
- Selecting None: no zram config file installed, no swap partition created; fstab has no swap line
- CI/lint/tests pass

### Future work (post‑v1)
- Allow selecting an existing swap partition for new/existing pool modes
- Hibernation support (resume kernel arg and initramfs integration)
- Advanced ZRAM tuning (multiple devices, bounds, NUMA)

### References
- ArchWiki — Zram: https://wiki.archlinux.org/title/Zram
- ArchWiki — Zswap: https://wiki.archlinux.org/title/Zswap


