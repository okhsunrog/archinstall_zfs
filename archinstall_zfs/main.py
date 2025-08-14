# Standard library imports
import contextlib
import socket
from pathlib import Path
from shutil import copy2
from typing import cast

from archinstall import SysInfo, debug, error, info
from archinstall.lib.applications.application_handler import application_handler
from archinstall.lib.args import ArchConfig, Arguments, arch_config_handler
from archinstall.lib.configuration import ConfigurationOutput
from archinstall.lib.general import SysCommand
from archinstall.lib.installer import accessibility_tools_in_use, run_custom_user_commands
from archinstall.lib.models.device import DiskLayoutConfiguration, DiskLayoutType
from archinstall.lib.models.users import User
from archinstall.lib.profile.profiles_handler import profile_handler
from archinstall.tui.curses_menu import MenuItemGroup, SelectMenu, Tui
from archinstall.tui.menu_item import MenuItem

from archinstall_zfs.config_io import load_combined_configuration, save_combined_configuration
from archinstall_zfs.disk import DiskManagerBuilder
from archinstall_zfs.installer import ZFSInstaller
from archinstall_zfs.menu import GlobalConfigMenu
from archinstall_zfs.menu.models import InstallationMode, SwapMode, ZFSEncryptionMode, ZFSModuleMode
from archinstall_zfs.zfs import ZFS_SERVICES, EncryptionMode, ZFSManagerBuilder
from archinstall_zfs.zfs.kmod_setup import add_archzfs_repo, initialize_zfs


def check_internet() -> bool:
    debug("Checking internet connection")
    try:
        # Use a direct IP and short timeout to avoid DNS-related hangs
        socket.create_connection(("1.1.1.1", 53), timeout=3.0)
        info("Internet connection available")
        return True
    except OSError as e:
        error(f"No internet connection: {e!s}")
        return False


def perform_installation(installer_menu: GlobalConfigMenu, arch_config: ArchConfig) -> bool:
    try:
        mountpoint = Path("/mnt")

        # Ensure disk_config mountpoint matches the ZFS target
        if not arch_config.disk_config:
            arch_config.disk_config = DiskLayoutConfiguration(DiskLayoutType.Pre_mount, mountpoint=mountpoint)

        # Run confirmation before any destructive operations

        config = ConfigurationOutput(arch_config)
        config.write_debug()
        # Merge ZFS config into the same user_configuration.json
        save_combined_configuration(config, config._default_save_path, installer_menu.to_json())

        with Tui():
            if not config.confirm_config():
                debug("Installation aborted")
                return False

        # Perform actual installation
        info("Starting installation...")

        # Build managers and perform disk/ZFS preparation now (after confirm)
        # Installation mode comes from the menu's config (use enum directly)
        assert installer_menu.cfg.installation_mode is not None
        mode = installer_menu.cfg.installation_mode
        disk_builder = DiskManagerBuilder()
        zfs_builder = ZFSManagerBuilder()

        # Map encryption selection
        if installer_menu.cfg.zfs_encryption_mode is ZFSEncryptionMode.POOL:
            selected_mode: EncryptionMode | None = EncryptionMode.POOL
        elif installer_menu.cfg.zfs_encryption_mode is ZFSEncryptionMode.DATASET:
            selected_mode = EncryptionMode.DATASET
        else:
            selected_mode = EncryptionMode.NONE

        zfs_builder.with_dataset_prefix(installer_menu.cfg.dataset_prefix).with_mountpoint(mountpoint).with_init_system(
            installer_menu.cfg.init_system.value
        ).with_encryption(selected_mode, installer_menu.cfg.zfs_encryption_password)

        # Configure disk builder strictly from global menu
        if installer_menu.cfg.disk_by_id:
            disk_builder.with_selected_disk(Path(installer_menu.cfg.disk_by_id))
        # new_pool/existing_pool require EFI
        if mode is not InstallationMode.FULL_DISK and installer_menu.cfg.efi_partition_by_id:
            disk_builder.with_efi_partition(Path(installer_menu.cfg.efi_partition_by_id))

        # Configure optional swap tail for full-disk ZSWAP modes
        if (
            installer_menu.cfg.installation_mode is not None
            and installer_menu.cfg.installation_mode.value == "full_disk"
            and installer_menu.cfg.swap_mode in {SwapMode.ZSWAP_PARTITION, SwapMode.ZSWAP_PARTITION_ENCRYPTED}
            and installer_menu.cfg.swap_partition_size
        ):
            disk_builder.with_swap_size(installer_menu.cfg.swap_partition_size)

        # Build disk manager and zfs manager depending on mode
        if mode is InstallationMode.FULL_DISK:
            disk_manager, zfs_partition = disk_builder.destroying_build()
            zfs_manager = (
                zfs_builder.with_mountpoint(mountpoint)
                .with_dataset_prefix(installer_menu.cfg.dataset_prefix)
                .with_encryption(selected_mode, installer_menu.cfg.zfs_encryption_password)
                .set_new_pool(zfs_partition, cast(str, installer_menu.cfg.pool_name))
                .build()
            )
        elif mode is InstallationMode.NEW_POOL:
            disk_manager = disk_builder.build()
            zfs_partition = Path(cast(str, installer_menu.cfg.zfs_partition_by_id))
            zfs_manager = (
                zfs_builder.with_mountpoint(mountpoint)
                .with_dataset_prefix(installer_menu.cfg.dataset_prefix)
                .with_encryption(selected_mode, installer_menu.cfg.zfs_encryption_password)
                .set_new_pool(zfs_partition, cast(str, installer_menu.cfg.pool_name))
                .build()
            )
        else:  # existing_pool
            disk_manager = disk_builder.build()
            zfs_manager = (
                zfs_builder.with_mountpoint(mountpoint)
                .with_dataset_prefix(installer_menu.cfg.dataset_prefix)
                .with_encryption(selected_mode, installer_menu.cfg.zfs_encryption_password)
                .set_existing_pool(cast(str, installer_menu.cfg.pool_name))
                .build()
            )

        # ZFS setup
        zfs_manager.setup_for_installation()

        # Mount EFI partition
        disk_manager.mount_efi_partition(mountpoint)

        # Create initramfs handler based on menu selection
        initramfs_handler = installer_menu.create_initramfs_handler(mountpoint, bool(zfs_manager.encryption_handler.password))

        # NEW: Use enhanced kernel registry for package management
        from archinstall_zfs.kernel import get_kernel_registry, EnhancedZFSInstaller
        
        selected_kernels: list[str] = arch_config.kernels if arch_config.kernels else ["linux-lts"]
        primary_kernel = selected_kernels[0]  # Use first kernel as primary
        
        registry = get_kernel_registry()
        zfs_installer = EnhancedZFSInstaller(registry)
        
        # Validate installation plan before proceeding
        validation_errors = zfs_installer.validate_installation_plan(
            primary_kernel,
            installer_menu.cfg.zfs_module_mode
        )
        if validation_errors:
            for error_msg in validation_errors:
                info(f"Validation warning: {error_msg}")

        # ZFSInstaller will use its own default base packages optimized for ZFS
        disk_cfg = arch_config.disk_config or DiskLayoutConfiguration(DiskLayoutType.Pre_mount, mountpoint=mountpoint)
        with ZFSInstaller(
            mountpoint,
            disk_config=disk_cfg,
            initramfs_handler=initramfs_handler,
            kernels=selected_kernels,
        ) as installation:
            installation.sanity_check()

            if arch_config.mirror_config:
                installation.set_mirrors(arch_config.mirror_config, on_target=False)

            installation.minimal_installation(
                hostname=arch_config.hostname,
                locale_config=arch_config.locale_config,
                mkinitcpio=False,  # Defer initramfs until ZFS packages are installed
            )

            if arch_config.mirror_config:
                installation.set_mirrors(arch_config.mirror_config, on_target=True)

            # Ensure archzfs repos are available both on the host (used by pacstrap)
            # and in the target (for the installed system's pacman.conf)
            add_archzfs_repo()
            # Ensure the target has refreshed keyring and synced DBs before package install
            add_archzfs_repo(installation.target, installation)

            # NEW: Install ZFS packages using enhanced system with proper fallback
            result = zfs_installer.install_with_fallback(
                primary_kernel,
                installer_menu.cfg.zfs_module_mode,
                installation
            )
            
            if not result.success:
                raise RuntimeError(f"ZFS package installation failed: {result.get_summary()}")
            
            # Log what was actually installed
            info(result.get_summary())
            if result.fallback_occurred and result.actual_mode:
                info(f"Note: Fallback occurred from {result.requested_mode.value} to {result.actual_mode.value}")

            # Ensure initramfs is generated once the right modules are present
            installation.regenerate_initramfs()

            # If user selected to copy the current ISO network configuration
            # Perform a copy of the config
            if arch_config.network_config:
                arch_config.network_config.install_network_config(installation, arch_config.profile_config)

            if arch_config.auth_config and arch_config.auth_config.users:
                installation.create_users(arch_config.auth_config.users)

            # Set root password if provided
            if arch_config.auth_config and arch_config.auth_config.root_enc_password:
                root_user = User("root", arch_config.auth_config.root_enc_password, False)
                installation.set_user_password(root_user)

            # Install applications (audio, bluetooth) via the official handler
            if arch_config.app_config:
                # Pass users if we created any (for per-user PipeWire enablement)
                users = arch_config.auth_config.users if (arch_config.auth_config and arch_config.auth_config.users) else None
                application_handler.install_applications(installation, arch_config.app_config, users)

            # Install selected profile(s) and run their post-install hooks
            if arch_config.profile_config:
                profile_handler.install_profile_config(installation, arch_config.profile_config)

            if arch_config.packages:
                installation.add_additional_packages(arch_config.packages)

            if arch_config.timezone:
                installation.set_timezone(arch_config.timezone)

            if arch_config.ntp:
                installation.activate_time_synchronization()

            # Enable accessibility services if used on the live ISO
            if accessibility_tools_in_use():
                installation.enable_espeakup()

            # Run any custom post-install commands if provided
            if arch_config.custom_commands:
                run_custom_user_commands(arch_config.custom_commands, installation)

            installation.enable_service(ZFS_SERVICES)

            # Configure zswap kernel parameter based on selected swap mode
            # - ZRAM: zswap must be disabled
            # - ZSWAP partition modes: zswap must be enabled
            if installer_menu.cfg.swap_mode == SwapMode.ZRAM:
                zfs_manager.set_zswap_enabled(False)
            elif installer_menu.cfg.swap_mode in {SwapMode.ZSWAP_PARTITION, SwapMode.ZSWAP_PARTITION_ENCRYPTED}:
                zfs_manager.set_zswap_enabled(True)
            else:
                zfs_manager.set_zswap_enabled(False)

            # Generate base fstab before making swap-specific additions
            zfs_manager.genfstab()

            # Swap configuration on target
            if installer_menu.cfg.swap_mode == SwapMode.ZRAM:
                # zram-generator
                installation.add_additional_packages(["zram-generator"])
                zram_conf = installation.target / "etc/systemd/zram-generator.conf"
                zram_conf.parent.mkdir(parents=True, exist_ok=True)
                lines = ["[zram0]"]
                if installer_menu.cfg.zram_fraction is not None:
                    lines.append(f"zram-fraction = {installer_menu.cfg.zram_fraction}")
                else:
                    lines.append(f"zram-size = {installer_menu.cfg.zram_size_expr or 'min(ram / 2, 4096)'}")
                lines.append("compression-algorithm = zstd")
                lines.append("swap-priority = 100")
                zram_conf.write_text("\n".join(lines) + "\n")
            elif installer_menu.cfg.swap_mode in {SwapMode.ZSWAP_PARTITION, SwapMode.ZSWAP_PARTITION_ENCRYPTED}:
                # Unencrypted: format and rely on genfstab; Encrypted: write crypttab+fstab entries
                # Determine swap partition path
                if installer_menu.cfg.installation_mode is not None and installer_menu.cfg.installation_mode is InstallationMode.FULL_DISK:
                    # Full-disk path is part3 if swap tail requested
                    dm = disk_manager.config
                    swap_part = dm.swap_partition if dm.swap_partition else None
                else:
                    swap_part = Path(installer_menu.cfg.swap_partition_by_id) if installer_menu.cfg.swap_partition_by_id else None

                if swap_part is not None:
                    fstab = installation.target / "etc/fstab"
                    if installer_menu.cfg.swap_mode == SwapMode.ZSWAP_PARTITION:
                        with contextlib.suppress(Exception):
                            SysCommand(f"mkswap {swap_part}")
                        # Append explicit swap UUID line, since genfstab won't include inactive swap
                        with contextlib.suppress(Exception):
                            uuid = SysCommand(f"blkid -s UUID -o value {swap_part}").decode().strip()
                            # Avoid duplicate if present
                            existing = fstab.read_text() if fstab.exists() else ""
                            if uuid and (f"UUID={uuid}" not in existing):
                                with open(fstab, "a") as f:
                                    f.write(f"UUID={uuid} none swap defaults 0 0\n")
                    else:
                        # Encrypted random-key dm-crypt: set up crypttab and fstab only
                        partuuid = SysCommand(f"blkid -s PARTUUID -o value {swap_part}").decode().strip()
                        crypttab = installation.target / "etc/crypttab"
                        crypttab_line = f"cryptswap PARTUUID={partuuid} /dev/urandom swap,cipher=aes-xts-plain64,size=256\n"
                        with open(crypttab, "a") as f:
                            f.write(crypttab_line)
                        with open(fstab, "a") as f:
                            f.write("/dev/mapper/cryptswap none swap defaults 0 0\n")

            zfs_manager.copy_misc_files()

            # Copy custom ZED hook, then make it immutable
            try:
                repo_asset = Path(__file__).resolve().parent.parent / "assets" / "zed" / "history_event-zfs-list-cacher.sh"
                host_path = Path("/etc/zfs/zed.d/history_event-zfs-list-cacher.sh")
                zed_src = repo_asset if repo_asset.exists() else host_path
                if zed_src.exists():
                    zed_dst_dir = installation.target / "etc" / "zfs" / "zed.d"
                    zed_dst_dir.mkdir(parents=True, exist_ok=True)
                    zed_dst = zed_dst_dir / zed_src.name
                    # Ensure destination is replaced cleanly even if identical path/device
                    with contextlib.suppress(Exception):
                        installation.arch_chroot("chattr -i /etc/zfs/zed.d/history_event-zfs-list-cacher.sh")
                    try:
                        if zed_dst.exists():
                            zed_dst.unlink(missing_ok=True)
                    except Exception:
                        # Fallback: remove inside chroot in case of attribute/permission issues
                        installation.arch_chroot("rm -f /etc/zfs/zed.d/history_event-zfs-list-cacher.sh")
                    copy2(zed_src, zed_dst)
                    installation.arch_chroot("chattr +i /etc/zfs/zed.d/history_event-zfs-list-cacher.sh")
                else:
                    debug(f"Custom ZED script not found at {repo_asset} or {host_path}, skipping copy")
            except Exception as e:
                error(f"Failed to install ZED history cacher hook: {e!s}")

            if disk_manager.config.efi_partition:
                zfs_manager.setup_bootloader(disk_manager.config.efi_partition)
            else:
                error("EFI partition not found, skipping bootloader setup")

            info("For post-installation tips, see https://wiki.archlinux.org/index.php/Installation_guide#Post-installation")

            # Simple replacement for ask_chroot functionality (always interactive here)
            with Tui():
                chroot_menu = SelectMenu(
                    MenuItemGroup([MenuItem("Yes", True), MenuItem("No", False)]),
                    header="Would you like to chroot into the newly created installation for post-installation configuration?",
                )
                chroot = chroot_menu.run().item().value
            if chroot:
                with contextlib.suppress(BaseException):
                    installation.drop_to_shell()

        disk_manager.finish(mountpoint)
        zfs_manager.finish()

        return True
    except Exception as e:
        error(f"Installation failed: {e!s}")
        return False


def ask_user_questions(arch_config: ArchConfig, zfs_data: dict | None = None, run_ui: bool = True) -> GlobalConfigMenu:
    """Ask user questions via ZFS installer menu and return it."""
    installer_menu = GlobalConfigMenu(arch_config)
    if zfs_data:
        installer_menu.apply_json(zfs_data)
    if run_ui:
        installer_menu.run()
    return installer_menu


def main() -> bool:
    # Removed direct storage logging config writes: keys are not part of the typed storage dict

    info("Starting ZFS installation")

    if not check_internet():
        error("Internet connection required")
        return False

    if not SysInfo.has_uefi():
        error("EFI boot mode required")
        return False

    # Pre-initialize ZFS (tools + kernel module) before launching the menu
    try:
        print("Preparing live system for ZFS support (this may take a minute)...")
        initialize_zfs()
    except Exception as e:
        error(f"Failed to initialize ZFS on live system: {e!s}")
        return False

    try:
        debug("Starting installation preparation")
        zfs_data: dict | None = None
        # If user provided a config, load it like guided installer does
        if arch_config_handler.args.config is not None:
            arch_config = arch_config_handler.config
            try:
                _, zfs_data = load_combined_configuration(arch_config_handler.args.config)
            except Exception:
                zfs_data = None
        else:
            # Build a default ArchConfig for interactive menu
            args = Arguments(mountpoint=Path("/mnt"), silent=False, dry_run=False)
            config_dict = {
                "disk_config": DiskLayoutConfiguration(DiskLayoutType.Pre_mount, mountpoint=Path("/mnt")).json(),
                "hostname": "archzfs",
                "locale_config": {},
                "mirror_config": {},
                "network_config": {},
                "profile_config": {},
                "auth_config": {},
                "app_config": {},
                "packages": [],
                "timezone": "UTC",
                "ntp": True,
                "swap": False,
            }
            arch_config = ArchConfig.from_config(config_dict, args)
        run_ui = not arch_config_handler.args.silent
        installer_menu = ask_user_questions(arch_config, zfs_data, run_ui=run_ui)

        # Preparation moved to perform_installation; proceed directly
        debug("Starting installation execution")
        success = perform_installation(installer_menu, arch_config)
        if not success:
            error("Installation execution failed")
            return False

        info("Installation completed successfully")
        return True
    except Exception as e:
        error(f"Installation failed: {e!s}")
        debug(f"Full error details: {e!r}")
        return False


if __name__ == "__main__":
    main()
