# Standard library imports
import contextlib
import os
import socket
import sys
from pathlib import Path
from typing import Any, Literal, cast

import archinstall
from archinstall import SysInfo, debug, error, info
from archinstall.lib.args import ArchConfig, Arguments
from archinstall.lib.configuration import ConfigurationOutput
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.general import SysCommand
from archinstall.lib.models import AudioConfiguration, NetworkConfiguration
from archinstall.lib.models.device import DiskLayoutConfiguration, DiskLayoutType
from archinstall.lib.storage import storage
from archinstall.tui.curses_menu import EditMenu, MenuItemGroup, SelectMenu, Tui
from archinstall.tui.menu_item import MenuItem

from archinstall_zfs.disk import DiskManager, DiskManagerBuilder
from archinstall_zfs.initramfs.dracut import DracutSetup
from archinstall_zfs.installer import ZFSInstaller
from archinstall_zfs.menu.zfs_installer_menu import ZFSInstallerMenu
from archinstall_zfs.zfs import ZFS_SERVICES, ZFSManager, ZFSManagerBuilder
from archinstall_zfs.zfs.kmod_setup import add_archzfs_repo, initialize_zfs

InstallMode = Literal["full_disk", "new_pool", "existing_pool"]


def check_internet() -> bool:
    debug("Checking internet connection")
    try:
        socket.create_connection(("archlinux.org", 80))
        info("Internet connection available")
        return True
    except OSError as e:
        error(f"No internet connection: {e!s}")
        return False


def get_installation_mode() -> InstallMode:
    menu = SelectMenu(
        MenuItemGroup(
            [
                MenuItem("Full Disk Installation", "full_disk"),
                MenuItem("New ZFS Pool", "new_pool"),
                MenuItem("Existing ZFS Pool", "existing_pool"),
            ]
        ),
        header="Select installation mode",
    )
    selected = menu.run().item().value
    info(f"Selected installation mode: {selected}")
    return cast(InstallMode, selected)


def prepare_installation() -> tuple[ZFSManager, DiskManager]:
    with Tui():
        mode = get_installation_mode()
        disk_builder = DiskManagerBuilder()
        zfs_builder = ZFSManagerBuilder()

        zfs_builder.with_dataset_prefix(
            EditMenu("Dataset Prefix", header="Enter prefix for ZFS datasets", default_text="arch0").input().text()
        ).with_mountpoint(Path("/mnt"))

        if mode != "full_disk":
            disk_manager = disk_builder.select_disk().select_efi_partition().build()

        match mode:
            case "full_disk":
                disk_manager, zfs_partition = disk_builder.select_disk().destroying_build()
                zfs = zfs_builder.new_pool(zfs_partition).build()
            case "new_pool":
                zfs_partition = disk_manager.select_zfs_partition()
                zfs = zfs_builder.new_pool(zfs_partition).build()
            case "existing_pool":
                zfs = zfs_builder.select_existing_pool().build()

    return zfs, disk_manager


def perform_installation(disk_manager: DiskManager, zfs_manager: ZFSManager) -> bool:
    try:
        mountpoint = zfs_manager.config.mountpoint

        # Create configuration for archinstall 3.0.9 compatibility
        config_dict = {
            "disk_config": DiskLayoutConfiguration(DiskLayoutType.Pre_mount, mountpoint=mountpoint).json(),
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
            "kernels": ["linux-lts"],
            "bootloader": "Systemd-boot",
            "swap": False,
        }

        # Create Arguments object for ArchConfig
        args = Arguments(
            mountpoint=mountpoint,
            silent=False,
            dry_run=False,
        )

        # Create ArchConfig object using from_config method
        arch_config = ArchConfig.from_config(config_dict, args)

        # ZFS setup
        zfs_manager.setup_for_installation()

        # Mount EFI partition
        disk_manager.mount_efi_partition(mountpoint)

        # Adding dracut configuration
        dracut = DracutSetup(str(mountpoint), encryption_enabled=bool(zfs_manager.encryption_handler.password))
        dracut.configure()
        zfs_config = ask_user_questions(arch_config)

        config = ConfigurationOutput(arch_config.safe_json())
        config.write_debug()
        config.save()

        if args.dry_run:
            sys.exit(0)

        if not args.silent:
            with Tui():
                if not config.confirm_config():
                    debug("Installation aborted")
                    return False

        # Perform actual installation
        info("Starting installation...")

        SECOND_STAGE = [
            "linux-lts",
            "linux-lts-headers",
            "linux-firmware",
            "zfs-dkms",
            "zfs-utils",
        ]

        # ZFSInstaller will use its own default base packages optimized for ZFS
        with ZFSInstaller(mountpoint, disk_config=arch_config.disk_config) as installation:
            installation.sanity_check()
            # No more dirty hack needed - ZFSInstaller handles base packages properly

            if arch_config.mirror_config:
                installation.set_mirrors(arch_config.mirror_config, on_target=False)

            installation.minimal_installation(
                testing=False,
                multilib=True,
                mkinitcpio=False,
                hostname=arch_config.hostname,
                locale_config=arch_config.locale_config,
            )

            if arch_config.mirror_config:
                installation.set_mirrors(arch_config.mirror_config, on_target=True)

            add_archzfs_repo(installation.target, installation)

            installation.add_additional_packages(SECOND_STAGE)

            # If user selected to copy the current ISO network configuration
            # Perform a copy of the config
            if arch_config.network_config:
                arch_config.network_config.install_network_config(installation, arch_config.profile_config)

            if arch_config.auth_config and arch_config.auth_config.users:
                installation.create_users(arch_config.auth_config.users)

            if arch_config.app_config:
                arch_config.app_config.install_audio_config(installation)
            else:
                info("No audio server will be installed")

            if arch_config.profile_config and hasattr(arch_config.profile_config, "profile") and hasattr(arch_config.profile_config.profile, "post_install"):
                # In archinstall 3.0.9, profile installation is handled differently
                # The profile should have a post_install method that we can call
                arch_config.profile_config.profile.post_install(installation)

            if arch_config.packages:
                installation.add_additional_packages(arch_config.packages)

            if arch_config.timezone:
                installation.set_timezone(arch_config.timezone)

            if arch_config.ntp:
                installation.activate_time_synchronization()

            if archinstall.accessibility_tools_in_use():
                installation.enable_espeakup()

            if arch_config.auth_config and arch_config.auth_config.root_enc_password:
                installation.user_set_pw("root", arch_config.auth_config.root_enc_password.plaintext)

            installation.enable_service(ZFS_SERVICES)

            zfs_manager.genfstab()
            zfs_manager.copy_misc_files()

            if disk_manager.config.efi_partition:
                zfs_manager.setup_bootloader(disk_manager.config.efi_partition)
            else:
                error("EFI partition not found, skipping bootloader setup")

            info("For post-installation tips, see https://wiki.archlinux.org/index.php/Installation_guide#Post-installation")

            if not args.silent:
                # Simple replacement for ask_chroot functionality

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


def ask_user_questions(arch_config: ArchConfig) -> dict[str, Any]:
    """Ask user questions and return ZFS-specific configuration."""
    installer_menu = ZFSInstallerMenu(arch_config)
    installer_menu.run()
    return installer_menu.get_zfs_config()


def check_zfs_module() -> bool:
    debug("Checking ZFS kernel module")
    try:
        SysCommand("modprobe zfs")
        info("ZFS module loaded successfully")
        return True
    except SysCallError:
        return False


def main() -> bool:
    storage["LOG_PATH"] = Path(os.path.expanduser("~"))
    storage["LOG_FILE"] = Path("archinstall.log")
    storage["LOG_LEVEL"] = "DEBUG"

    info("Starting ZFS installation")

    if not check_internet():
        error("Internet connection required")
        return False

    if not SysInfo.has_uefi():
        error("EFI boot mode required")
        return False

    initialize_zfs()

    try:
        debug("Starting installation preparation")
        zfs_manager, disk_manager = prepare_installation()
        debug("Installation preparation completed")

        debug("Starting installation execution")
        success = perform_installation(disk_manager, zfs_manager)
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
