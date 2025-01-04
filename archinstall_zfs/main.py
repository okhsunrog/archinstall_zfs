# Standard library imports
from pathlib import Path
import socket
from typing import Literal
import os

import archinstall
from archinstall import SysInfo, debug, info, error, SysCommand, Installer, GlobalMenu, ConfigurationOutput
from archinstall.lib.disk import DiskLayoutConfiguration, DiskLayoutType
from archinstall.lib.exceptions import SysCallError
from archinstall.lib.profile import profile_handler
from archinstall.tui.curses_menu import SelectMenu, MenuItemGroup, EditMenu, Tui
from archinstall.tui.menu_item import MenuItem
from archinstall.lib.storage import storage
from archinstall.lib.plugins import plugins

from archinstall_zfs.storage.zfs_init import initialize_zfs, add_archzfs_repo
from archinstall_zfs.storage.disk import DiskManager, DiskManagerBuilder
from archinstall_zfs.storage.zfs import ZFSManager, ZFSManagerBuilder

InstallMode = Literal["full_disk", "new_pool", "existing_pool"]

class ZfsPlugin:
    def on_install(self, installation):
        add_archzfs_repo(installation.target, installation)
        return False

    def on_mkinitcpio(self, installation):
        # Find the index of 'filesystems' hook
        filesystems_index = installation._hooks.index('filesystems')
        # Insert 'zfs' right before it
        installation._hooks.insert(filesystems_index, 'zfs')
        return False


plugins['zfs'] = ZfsPlugin()


def check_internet() -> bool:
    debug("Checking internet connection")
    try:
        socket.create_connection(("archlinux.org", 80))
        info("Internet connection available")
        return True
    except OSError as e:
        error(f"No internet connection: {str(e)}")
        return False

def get_installation_mode() -> InstallMode:
    debug("Displaying installation mode selection menu")
    modes = [
        MenuItem("Full disk - Format and create new ZFS pool", "full_disk"),
        MenuItem("Partition - Create new ZFS pool on existing partition", "new_pool"),
        MenuItem(
            "Existing pool - Install alongside existing ZFS system", "existing_pool"
        ),
    ]

    menu = SelectMenu(
        MenuItemGroup(modes),
        header="Select Installation Mode\n\nWarning: Make sure you have backups!",
    )

    selected = menu.run().item().value
    info(f"Selected installation mode: {selected}")
    return selected


def prepare_installation() -> tuple[ZFSManager, DiskManager]:
    with Tui():
        mode = get_installation_mode()

        disk_builder = DiskManagerBuilder()
        zfs_builder = ZFSManagerBuilder()

        # Configure ZFS base settings
        zfs_builder.with_dataset_prefix(
            EditMenu(
                "Dataset Prefix",
                header="Enter prefix for ZFS datasets",
                default_text="arch0"
            ).input().text()
        ).with_mountpoint(Path("/mnt"))

        # Handle different installation modes
        if mode == "full_disk":
            disk_manager, zfs_partition = disk_builder.select_disk().destroying_build()
            zfs_builder.select_pool_name().setup_encryption()
            zfs = zfs_builder.new_pool(zfs_partition).build()

        elif mode == "new_pool":
            disk_manager = disk_builder.select_disk().select_efi_partition().build()
            zfs_partition = disk_manager.select_zfs_partition()
            zfs_builder.select_pool_name().setup_encryption()
            zfs = zfs_builder.new_pool(zfs_partition).build()

        else:  # existing_pool
            disk_manager = disk_builder.select_disk().select_efi_partition().build()
            zfs = zfs_builder.select_existing_pool().build()

    return zfs, disk_manager


def perform_installation(disk_manager: DiskManager, zfs_manager: ZFSManager) -> bool:
    try:
        # !TODO: use single mountpoint from single place across the whole installer
        mountpoint = Path("/mnt")
        archinstall.arguments['disk_config'] = DiskLayoutConfiguration(DiskLayoutType.Pre_mount, mountpoint=mountpoint)

        # ZFS setup
        zfs_manager.prepare()
        zfs_manager.setup_for_installation(Path("/mnt"))

        # Mount EFI partition
        disk_manager.mount_efi_partition(Path("/mnt"))

        ask_user_questions()

        config = ConfigurationOutput(archinstall.arguments)
        config.write_debug()
        config.save()

        if archinstall.arguments.get('dry_run'):
            exit(0)

        if not archinstall.arguments.get('silent'):
            with Tui():
                if not config.confirm_config():
                    debug('Installation aborted')
                    return False

        # Perform actual installation
        info('Starting installation...')

        with Installer(
                mountpoint,
                disk_config=archinstall.arguments['disk_config'],
                disk_encryption=None,
                kernels=['linux-lts'],
                base_packages=['base', 'base-devel', 'linux-firmware', 'linux-lts-headers', 'zfs-dkms', 'zfs-utils'],
        ) as installation:

            installation.sanity_check()

            if mirror_config := archinstall.arguments.get('mirror_config', None):
                installation.set_mirrors(mirror_config, on_target=False)

            installation.minimal_installation(
                hostname=archinstall.arguments.get('hostname', 'archzfs'),
                locale_config=archinstall.arguments['locale_config']
            )

            if mirror_config := archinstall.arguments.get('mirror_config', None):
                installation.set_mirrors(mirror_config, on_target=True)

            if users := archinstall.arguments.get('!users', []):
                installation.create_users(users)

            if root_pw := archinstall.arguments.get('!root-password', ''):
                installation.user_set_pw('root', root_pw)

            if profile_config := archinstall.arguments.get('profile_config', None):
                profile_handler.install_profile_config(installation, profile_config)

            if packages := archinstall.arguments.get('packages', []):
                installation.add_additional_packages(packages)

            installation.genfstab()
            zfs_manager.copy_misc_files()

        return True
    except Exception as e:
        error(f"Installation failed: {str(e)}")
        return False


def ask_user_questions() -> None:
    """Get user input for installation configuration"""
    with Tui():
        global_menu = GlobalMenu(data_store=archinstall.arguments)
        global_menu.disable_all()

        # Keep essential options enabled
        global_menu.set_enabled('archinstall-language', True)
        global_menu.set_enabled('locale_config', True)
        global_menu.set_enabled('mirror_config', True)
        global_menu.set_enabled('timezone', True)
        global_menu.set_enabled('!root-password', True)
        global_menu.set_enabled('!users', True)
        global_menu.set_enabled('profile_config', True)
        global_menu.set_enabled('audio_config', True)
        global_menu.set_enabled('network_config', True)
        global_menu.set_enabled('packages', True)

        global_menu.set_enabled('save_config', True)
        global_menu.set_enabled('install', True)
        global_menu.set_enabled('abort', True)

        global_menu.run()

def check_zfs_module() -> bool:
    debug("Checking ZFS kernel module")
    try:
        SysCommand("modprobe zfs")
        info("ZFS module loaded successfully")
        return True
    except SysCallError:
        return False

def main() -> bool:
    storage['LOG_PATH'] = Path(os.path.expanduser('~'))
    storage['LOG_FILE'] = Path('archinstall.log')
    storage['LOG_LEVEL'] = 'DEBUG'

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
        error(f"Installation failed: {str(e)}")
        debug(f"Full error details: {repr(e)}")
        return False


if __name__ == '__main__':
    main()