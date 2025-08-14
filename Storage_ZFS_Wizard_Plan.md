## Storage & ZFS Wizard – Implementation Plan

### Objectives
- Replace four separate entries with one gated wizard: Installation Mode, Disk Configuration, Swap, and ZFS Configuration become a single “Storage & ZFS (Wizard)”.
- Keep “Init System” as a separate top‑level menu item.
- For Existing Pool, defer encryption prompting to install-time via an AUTO path that detects pool encryption and prompts when ZFS tools are guaranteed available.

### Scope (what changes, at a glance)
- Add a new wizard flow in `archinstall_zfs/menu/global_config.py` that orchestrates the four areas in order with gating.
- Update `archinstall_zfs/menu/models.py` to include `ZFSEncryptionMode.AUTO` and adjust validation.
- Update `archinstall_zfs/main.py` mapping to pass `mode=None,password=None` when in Existing Pool + AUTO to trigger runtime detection.
- Optional improvement: in `archinstall_zfs/zfs/__init__.py`, change `load_key()` to use `-L file://…` to avoid re-prompting if a key file was written.

### High-level UX
- The main menu shows one entry: “Storage & ZFS (Wizard)” with a concise preview line.
- The wizard is mode-aware and gated:
  - You cannot configure ZFS specifics until the required disk/EFI/swap selections are complete for the chosen mode.
  - Each step provides friendly hints and can jump to the missing prerequisite.
- “Init System” remains a separate top-level item.

### Wizard flow (mode-aware)
1) Installation Mode
   - Full Disk | New Pool | Existing Pool
   - On change, clear incompatible selections (e.g., clear `zfs_partition_by_id` when switching away from New Pool).

2) Disks/Partitions
   - Full Disk: select `disk_by_id`.
   - New Pool: select `disk_by_id`, `efi_partition_by_id`, `zfs_partition_by_id`.
   - Existing Pool: select `efi_partition_by_id` only (no whole-disk selection).

3) Swap
   - Modes: None | ZRAM | ZSWAP + partition | ZSWAP + encrypted partition.
   - If Full Disk + ZSWAP: prompt for `swap_partition_size` (e.g., “16G”).
   - If non–Full Disk + ZSWAP: select `swap_partition_by_id`.

4) ZFS specifics (gated by steps 2–3)
   - Existing Pool:
     - Pool selection: list importable pools via `zpool import` (with Refresh + “Enter manually”). If tools unavailable, show a hint and allow manual entry.
     - Dataset prefix: alphanumeric; best-effort validation that `<pool>/<prefix>` doesn’t already exist (`zfs list`), with graceful fallback.
     - Encryption: store `AUTO` (no password collected in wizard). At install-time, the code will detect pool encryption and prompt if needed.
   - New Pool:
     - Pool name (alphanumeric), dataset prefix (alphanumeric).
     - Encryption: Pool | Dataset | None. If Pool/Dataset selected, prompt for passphrase now.

5) Summary & Confirm
   - Compact preview of: disks/partitions, swap, pool/prefix, datasets to be created, encryption mode/handling, EFI install intent.

### Gating rules
- ZFS specifics step is disabled until the required selections for the chosen mode are made:
  - Full Disk: requires `disk_by_id` (and `swap_partition_size` if ZSWAP mode).
  - New Pool: requires `disk_by_id`, `efi_partition_by_id`, `zfs_partition_by_id` (and `swap_partition_by_id` if ZSWAP modes).
  - Existing Pool: requires `efi_partition_by_id` (and `swap_partition_by_id` if ZSWAP modes).
- The wizard shows a short explanation and a one-key jump to the missing prerequisite step.

### Data model changes (`archinstall_zfs/menu/models.py`)
- Add enum value: `class ZFSEncryptionMode(Enum): AUTO = "auto"` (in addition to NONE, POOL, DATASET).
- Default behavior:
  - When the user selects Existing Pool in the wizard, set `zfs_encryption_mode = AUTO` by default.
  - For other modes, default can remain `NONE` unless changed in the wizard.
- Update `GlobalConfig.validate_for_install()`:
  - Require `zfs_encryption_password` only when `zfs_encryption_mode in {POOL, DATASET}`.
  - Do not require a password for `AUTO` or `NONE`.
  - Keep existing disk/EFI/partition requirements per mode.

### Menu/UI changes (`archinstall_zfs/menu/global_config.py`)
- Replace the following individual entries with a single entry named “Storage & ZFS (Wizard)”: Installation Mode, Disk Configuration, Swap, ZFS Configuration.
- Keep “Init System” as a separate top-level entry.
- Provide a concise preview for the wizard line, e.g.: `Mode: existing_pool; EFI: /dev/disk/by-id/...; Pool: tank; Prefix: arch0; Swap: zram`.
- New methods to implement:
  - `run_storage_wizard()` – orchestrates the wizard steps and maintains gating.
  - `_wizard_step_mode()` – choose mode, clear incompatible fields if changed.
  - `_wizard_step_disks()` – select disks/partitions based on mode.
  - `_wizard_step_swap()` – select swap mode and size/partition.
  - `_wizard_step_zfs()` – mode-aware ZFS settings as described.
  - `_wizard_step_summary()` – final review/confirm.
  - Helpers:
    - `_discover_importable_pools()` – parse `zpool import` output into selectable names; include Refresh and manual entry options; handle missing tools gracefully.
    - `_validate_dataset_prefix_available(pool, prefix)` – attempt `zfs list` for `<pool>/<prefix>`; warn and accept if tools unavailable.
    - `_mode_change_reset()` – clear incompatible fields on mode change.

### Install-time behavior (AUTO encryption path)
- In `archinstall_zfs/main.py` (within `perform_installation()` mapping):
  - If `installation_mode == EXISTING_POOL` and wizard stored `zfs_encryption_mode == AUTO`, pass `.with_encryption(mode=None, password=None)` into `ZFSManagerBuilder`.
- In `archinstall_zfs/zfs/__init__.py` (current behavior already suitable):
  - `ZFSEncryption` sees `preselected_mode is None` and `is_new_pool == False` → runs `_setup_existing_pool_encryption()`:
    - Imports pool (no mounts) and checks `zfs get encryption`.
    - If encrypted: prompts for passphrase then sets mode=POOL.
    - If not encrypted: asks whether to encrypt the base dataset (DATASET) or not.
  - `ZFSManager.setup_for_installation()` calls `encryption_handler.setup()` to write the key file if a password was provided, then imports the pool to target; if a password exists, `load_key()` is called.

### Optional improvement (Phase 2)
- Update `ZFSPool.load_key()` to avoid interactive prompts by using the key file when available:
  - Replace `zfs load-key {pool}` with `zfs load-key -L file://{self.paths.key_file} {pool}`.
  - This uses the key written by `ZFSEncryption.setup()` so users are not prompted twice.

### Validation & messages
- Dataset prefix validation: try `zfs list <pool>/<prefix>`; if success (exists), show error; if command fails due to missing tools, accept and warn that conflicts will be detected during installation.
- Pool discovery:
  - If `zpool` unavailable, show: “ZFS tools not available on the live system; manual pool entry only. Tools will be initialized during installation.”
  - If no pools found: “No importable pools detected. Ensure the pool is exported in the other OS, then use Refresh.”
- Gating message examples:
  - “Select an EFI partition before configuring ZFS (Existing Pool). Press Enter to select the EFI partition now.”
  - “Select a ZFS partition before configuring ZFS (New Pool). Press Enter to select the ZFS partition now.”

### Edge cases
- Missing ZFS tools at wizard time: wizard still usable; we defer encryption prompting to install-time.
- Wrong passphrase at install-time (AUTO path): user is prompted again by `_get_password()` until correct; failing cancels the run and surfaces an error.
- Switching modes mid-wizard: clear incompatible fields to avoid stale state (e.g., `zfs_partition_by_id`).

### Testing plan
- Manual (on live ISO):
  - Existing Pool (encrypted): ensure no prompts appear in wizard; during installation, ZFS is initialized, user is prompted once for passphrase, and import proceeds.
  - Existing Pool (unencrypted): ensure wizard asks about encrypting the base dataset during install-time detection.
  - New Pool: normal flow; dataset creation works; encryption password collected in wizard.
  - Full Disk: normal flow; ZRAM and ZSWAP modes behave correctly.
  - Gating: ZFS step cannot be accessed prematurely; previews update correctly.
  - Pool discovery: works with tools; graceful manual fallback without tools.
- Automated:
  - Unit-test `GlobalConfig.validate_for_install()` with `AUTO` mode and other permutations.
  - Unit-test `_discover_importable_pools()` parser with sample `zpool import` output.
  - Unit-test `_mode_change_reset()` clears incompatible fields.
  - Unit-test `_validate_dataset_prefix_available()` logic with mocked command outcomes.

### Implementation order
1) Add `AUTO` to `ZFSEncryptionMode` and adjust `validate_for_install()` (models.py).
2) Update `perform_installation()` mapping to pass `mode=None,password=None` for Existing Pool + AUTO (main.py).
3) Implement the wizard in `GlobalConfigMenu` and replace the four entries with one “Storage & ZFS (Wizard)”.
4) Implement helpers for pool discovery, prefix validation, and gating; add concise preview for the wizard item; keep Init System separate and unchanged.
5) Manual test all three modes; adjust messages and gating as needed.
6) Optional Phase 2: switch `load_key()` to `-L file://…` to avoid second prompt.

### Acceptance criteria
- The four entries are replaced by one “Storage & ZFS (Wizard)” item, and “Init System” remains separate.
- The wizard gates ZFS specifics until disk/EFI/swap prerequisites are set for the chosen mode.
- Existing Pool stores `AUTO` encryption mode and defers prompting until install-time; encrypted pools prompt exactly once.
- Pool discovery and prefix validation work (with graceful fallbacks when tools are missing).
- Installation completes successfully for Full Disk, New Pool, and Existing Pool (encrypted and unencrypted), including swap modes.

### Non-goals
- Changing bootloader behavior or dataset layout logic beyond the wizard/UI and AUTO integration.
- Changing initramfs handlers or package selection logic.


