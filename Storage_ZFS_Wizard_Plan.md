## Storage & ZFS Wizard – Implementation Plan

### Objectives
- Replace four separate entries with one gated wizard: Installation Mode, Disk Configuration, Swap, and ZFS Configuration become a single “Storage & ZFS (Wizard)”.
- Keep “Init System” as a separate top‑level menu item.
- Ensure ZFS tools and kernel modules are available before launching the wizard/menu (hard-fail if not).
- For Existing Pool, perform an early, ephemeral import during the wizard to detect encryption and prompt/verify the passphrase immediately (no AUTO mode).

### Scope (what changes, at a glance)
- Add a new wizard flow in `archinstall_zfs/menu/global_config.py` that orchestrates the four areas in order with gating.
- Pre-initialize ZFS in `archinstall_zfs/main.py` before launching the menu; if it fails, exit.
- Add helpers in `archinstall_zfs/zfs/__init__.py` to detect pool encryption and to verify a passphrase using a temporary early import.
- Optional improvement: in `archinstall_zfs/zfs/__init__.py`, change `load_key()` to use `-L file://…` to avoid re-prompting if a key file was written.

### Early ZFS initialization (before menu)
- Where: `archinstall_zfs/main.py` in `main()` after internet/UEFI checks and before building/launching the menu.
- Do: call `ensure_reflector_finished_and_stopped()` and then `initialize_zfs()` to ensure `zpool`/`zfs` are available and the `zfs` module is loaded.
- If initialization fails: print a clear error and exit before launching the wizard/menu.
- Idempotent: `initialize_zfs()` returns early if tools/module are already present; safe to call again later (we keep the later call as a no-op or remove it).
- Mirror interaction: archive pinning is restored in `ZFSInitializer.run()` finally; user mirror configuration remains functional.

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
     - Pool selection: list importable pools via `zpool import` (with Refresh + “Enter manually”). Tools are guaranteed available.
     - Immediately detect encryption and verify passphrase:
       - Early import: `zpool import -fN <pool>`; check `zfs get -H encryption <pool>`.
       - If encrypted: prompt for passphrase; verify using `zfs load-key -L file://<tmpkey> <pool>`; on success, `zfs unload-key <pool>` (best‑effort) and `zpool export <pool>`; store mode=POOL and password.
       - If not encrypted: ask whether to encrypt new base dataset; if yes, prompt for password and store mode=DATASET; otherwise mode=NONE. Always `zpool export <pool>` at the end.
     - Dataset prefix: alphanumeric; validate that `<pool>/<prefix>` doesn’t already exist via `zfs list` (block on conflict).
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
- Keep `ZFSEncryptionMode` as `{NONE, POOL, DATASET}` (no AUTO).
- Update `GlobalConfig.validate_for_install()`:
  - Require `zfs_encryption_password` when `zfs_encryption_mode in {POOL, DATASET}`.
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

### Install-time behavior (explicit encryption mode from wizard)
- In `archinstall_zfs/main.py` (within `perform_installation()` mapping):
  - Pass `.with_encryption(mode=POOL|DATASET|NONE, password=<stored_or_None>)` directly from the wizard.
- In `archinstall_zfs/zfs/__init__.py`:
  - `ZFSEncryption` will be constructed with `preselected_mode` and `preselected_password`, so it skips interactive prompts.
  - `encryption_handler.setup()` writes the key file if a password was provided.
  - `ZFSPool.import_pool()` will call `load_key()` if a password exists.

### Optional improvement (Phase 2)
- Update `ZFSPool.load_key()` to avoid interactive prompts by using the key file when available:
  - Replace `zfs load-key {pool}` with `zfs load-key -L file://{self.paths.key_file} {pool}`.
  - This uses the key written by `ZFSEncryption.setup()` so users are not prompted twice.

- ### Validation & messages
- Dataset prefix validation: run `zfs list <pool>/<prefix>`; if it exists, show error and block until a different prefix is chosen.
- Pool discovery:
  - If no pools found: “No importable pools detected. Ensure the pool is exported in the other OS, then use Refresh.”
- Gating message examples:
  - “Select an EFI partition before configuring ZFS (Existing Pool). Press Enter to select the EFI partition now.”
  - “Select a ZFS partition before configuring ZFS (New Pool). Press Enter to select the ZFS partition now.”

### Edge cases
- Early ZFS initialization failure: print a clear error and exit before the menu; the wizard is not started.
- Wrong passphrase at wizard time (Existing Pool): loop-prompt until verified (or cancel), using early import + `zfs load-key -L file://<tmpkey>`; always export the pool afterward.
- Switching modes mid-wizard: clear incompatible fields to avoid stale state (e.g., `zfs_partition_by_id`).

### Testing plan
- Manual (on live ISO):
  - Existing Pool (encrypted): wizard prompts immediately for passphrase after pool selection; verification succeeds; install proceeds without further pass prompts.
  - Existing Pool (unencrypted): wizard optionally asks to encrypt base dataset and collects password if chosen.
  - New Pool: normal flow; dataset creation works; encryption password collected in wizard.
  - Full Disk: normal flow; ZRAM and ZSWAP modes behave correctly.
  - Gating: ZFS step cannot be accessed prematurely; previews update correctly.
  - Pool discovery: works with tools; refresh finds pools after exporting from other OS.
- Automated:
  - Unit-test `GlobalConfig.validate_for_install()` for NONE/POOL/DATASET combinations.
  - Unit-test `_discover_importable_pools()` parser with sample `zpool import` output.
  - Unit-test `_mode_change_reset()` clears incompatible fields.
  - Unit-test `_validate_dataset_prefix_available()` logic with mocked command outcomes.

### Implementation order
1) Pre-initialize ZFS before the menu: in `main()`, call `ensure_reflector_finished_and_stopped()` and `initialize_zfs()` before `ask_user_questions()`; exit on failure.
2) Implement helpers in `archinstall_zfs/zfs/__init__.py`:
   - `detect_pool_encryption(pool_name: str) -> bool`
   - `verify_pool_passphrase(pool_name: str, password: str) -> bool`
3) Implement the wizard in `GlobalConfigMenu` and replace the four entries with one “Storage & ZFS (Wizard)”.
4) Wire Existing Pool step to call detection + verification helpers; store `zfs_encryption_mode` and `zfs_encryption_password` accordingly; add prefix validation.
5) Keep Init System separate and unchanged; add concise preview for the wizard item.
6) Manual test all three modes; adjust messages and gating as needed.
7) Optional Phase 2: switch `load_key()` to `-L file://…` to avoid second prompt.

### Acceptance criteria
- The four entries are replaced by one “Storage & ZFS (Wizard)” item, and “Init System” remains separate.
- The wizard gates ZFS specifics until disk/EFI/swap prerequisites are set for the chosen mode.
- Existing Pool performs early, ephemeral import during the wizard; encrypted pools prompt immediately and verify passphrase; unencrypted pools offer dataset encryption.
- Pool discovery and prefix validation work as described.
- Installation completes successfully for Full Disk, New Pool, and Existing Pool (encrypted and unencrypted), including swap modes.

### Non-goals
- Changing bootloader behavior or dataset layout logic beyond the wizard/UI and AUTO integration.
- Changing initramfs handlers or package selection logic.


