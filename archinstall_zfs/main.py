# Standard library imports
from pathlib import Path
import socket
from typing import Literal
import os

import archinstall
# Third-party imports
# import parted

# Local application imports
from archinstall import SysInfo, debug, info, error, Installer, ConfigurationOutput, GlobalMenu, SysCommand
from archinstall.lib import locale, plugins
from archinstall.tui.curses_menu import Tui, SelectMenu, MenuItemGroup, EditMenu
from archinstall.tui.menu_item import MenuItem
from archinstall.lib.storage import storage

from lib.interactions.general_conf import ask_chroot
from lib.models import NetworkConfiguration, AudioConfiguration
from lib.profile import profile_handler
from storage.disk import DiskManager
from storage.zfs import ZFSManager


InstallMode = Literal["full_disk", "new_pool", "existing_pool"]


class ZfsPlugin:
    def on_mirrors(self, mirror_config):
        # Add ZFS repo early in the process
        with open('/etc/pacman.conf', 'a') as fp:
            fp.write("\n[archzfs]\n")
            fp.write("SigLevel = Never\n")
            fp.write("Server = http://archzfs.com/$repo/x86_64\n")

        # Sync the new repo
        SysCommand('pacman -Sy')
        return mirror_config

    def on_pacstrap(self, packages):
        # Add ZFS packages to initial pacstrap
        packages.extend(['zfs-linux', 'zfs-utils'])
        return packages

    def on_install(self, installation):
        # Add repo to target system
        with open(f"{installation.target}/etc/pacman.conf", 'a') as fp:
            fp.write("\n[archzfs]\n")
            fp.write("SigLevel = Never\n")
            fp.write("Server = http://archzfs.com/$repo/x86_64\n")

        # Sync the new repo in target system
        installation.arch_chroot('pacman -Sy')
        return False

    def on_mkinitcpio(self, installation):
        installation._hooks.append('zfs')
        return False


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
    mode = get_installation_mode()

    disk_builder = DiskManagerBuilder()
    zfs_builder = ZFSManagerBuilder()

    zfs_builder.with_dataset_prefix(
        EditMenu(
            "Dataset Prefix",
            header="Enter prefix for ZFS datasets",
            default_text="arch0"
        ).input().text()
    ).with_mountpoint(Path("/mnt"))

    if mode == "full_disk":
        disk_manager, zfs_partition = disk_builder.select_disk().destroying_build()
        zfs = zfs_builder.new_pool(zfs_partition).build()

    elif mode == "new_pool":
        disk_manager = disk_builder.select_disk().select_efi_partition().build()
        zfs = zfs_builder.new_pool(disk_manager.select_zfs_partition()).build()

    else:  # existing_pool
        disk_manager = disk_builder.select_disk().select_efi_partition().build()
        zfs = zfs_builder.select_existing_pool().build()

    return zfs, disk_manager


def perform_installation(
        disk_manager: DiskManager,
        zfs_manager: ZFSManager
) -> bool:
    try:
        if mode != "existing_pool":
            zfs_manager.get_encryption_password()
            zfs_manager.create_pool(disk_manager.zfs_partition)
            zfs_manager.create_datasets()
            zfs_manager.export_pool()

        zfs_manager.import_pool(Path("/mnt"))
        disk_manager.mount_efi_partition(Path("/mnt"))

        if not zfs_manager.verify_mounts():
            raise RuntimeError("Mount verification failed")

        # Register ZFS plugin
        plugins['zfs'] = ZfsPlugin()

        # Start Arch Linux installation
        storage['MOUNT_POINT'] = Path('/mnt')

        # Configure installation parameters
        ask_user_questions()

        # Verify and save configuration
        config = ConfigurationOutput(archinstall.arguments)
        config.write_debug()
        config.save()

        if not config.confirm_config():
            raise RuntimeError("Installation configuration not confirmed")

        # Perform actual installation
        perform_installation_next(Path("/mnt"))

        return True
    except Exception as e:
        error(f"Installation failed: {str(e)}")
        return False


def ask_user_questions() -> None:
    """
    First, we'll ask the user for a bunch of user input.
    Not until we're satisfied with what we want to install
    will we continue with the actual installation steps.
    """

    with Tui():
        global_menu = GlobalMenu(data_store=archinstall.arguments)

        if not archinstall.arguments.get('advanced', False):
            global_menu.set_enabled('parallel downloads', False)

        global_menu.run()


def perform_installation_next(mountpoint: Path) -> None:
    import archinstall.lib.disk as disk
    """
    Performs the installation steps on a block device.
    Only requirement is that the block devices are
    formatted and setup prior to entering this function.
    """
    info('Starting installation...')
    disk_config: disk.DiskLayoutConfiguration = archinstall.arguments['disk_config']

    # Retrieve list of additional repositories and set boolean values appropriately
    enable_testing = 'testing' in archinstall.arguments.get('additional-repositories', [])
    enable_multilib = 'multilib' in archinstall.arguments.get('additional-repositories', [])
    locale_config: locale.LocaleConfiguration = archinstall.arguments['locale_config']

    with Installer(
            mountpoint,
            disk_config,
            disk_encryption=None,
            kernels=archinstall.arguments.get('kernels', ['linux'])
    ) as installation:

        installation.sanity_check()

        if mirror_config := archinstall.arguments.get('mirror_config', None):
            installation.set_mirrors(mirror_config, on_target=False)

        installation.minimal_installation(
            testing=enable_testing,
            multilib=enable_multilib,
            mkinitcpio=True,
            hostname=archinstall.arguments.get('hostname', 'archlinux'),
            locale_config=locale_config
        )

        if mirror_config := archinstall.arguments.get('mirror_config', None):
            installation.set_mirrors(mirror_config, on_target=True)

        if archinstall.arguments.get('swap'):
            installation.setup_swap('zram')

        # installation.add_additional_packages("dracut")

        # If user selected to copy the current ISO network configuration
        # Perform a copy of the config
        network_config: NetworkConfiguration | None = archinstall.arguments.get('network_config', None)

        if network_config:
            network_config.install_network_config(
                installation,
                archinstall.arguments.get('profile_config', None)
            )

        if users := archinstall.arguments.get('!users', None):
            installation.create_users(users)

        audio_config: AudioConfiguration | None = archinstall.arguments.get('audio_config', None)
        if audio_config:
            audio_config.install_audio_config(installation)
        else:
            info("No audio server will be installed")

        if archinstall.arguments.get('packages', None) and archinstall.arguments.get('packages', None)[0] != '':
            installation.add_additional_packages(archinstall.arguments.get('packages', None))

        if profile_config := archinstall.arguments.get('profile_config', None):
            profile_handler.install_profile_config(installation, profile_config)

        if timezone := archinstall.arguments.get('timezone', None):
            installation.set_timezone(timezone)

        if archinstall.arguments.get('ntp', False):
            installation.activate_time_synchronization()

        if archinstall.accessibility_tools_in_use():
            installation.enable_espeakup()

        if (root_pw := archinstall.arguments.get('!root-password', None)) and len(root_pw):
            installation.user_set_pw('root', root_pw)

        if profile_config := archinstall.arguments.get('profile_config', None):
            profile_config.profile.post_install(installation)

        # If the user provided a list of services to be enabled, pass the list to the enable_service function.
        # Note that while it's called enable_service, it can actually take a list of services and iterate it.
        if archinstall.arguments.get('services', None):
            installation.enable_service(archinstall.arguments.get('services', []))

        # If the user provided custom commands to be run post-installation, execute them now.
        if archinstall.arguments.get('custom-commands', None):
            archinstall.run_custom_user_commands(archinstall.arguments['custom-commands'], installation)

        installation.genfstab()

        info(
            "For post-installation tips, see https://wiki.archlinux.org/index.php/Installation_guide#Post-installation")

        if not archinstall.arguments.get('silent'):
            with Tui():
                chroot = ask_chroot()

            if chroot:
                try:
                    installation.drop_to_shell()
                except Exception:
                    pass

    debug(f"Disk states after installing:\n{disk.disk_layouts()}")


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

    try:
        with Tui():
            mode = get_installation_mode()
            zfs_manager, disk_manager = prepare_installation()
            perform_installation(disk_manager, zfs_manager)
    except Exception as e:
        error(f"Installation failed: {str(e)}")
