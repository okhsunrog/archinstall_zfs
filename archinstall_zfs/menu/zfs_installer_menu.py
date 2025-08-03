"""
ZFS Installer Menu - Custom menu system using composition over inheritance.

This provides a clean separation between archinstall library functions and
ZFS-specific configuration, allowing for better maintainability and version independence.
"""

import sys
from enum import Enum
from typing import Any

from archinstall.lib.args import ArchConfig
from archinstall.lib.authentication.authentication_menu import AuthenticationMenu
from archinstall.lib.interactions.general_conf import (
    ask_additional_packages_to_install,
    ask_for_a_timezone,
    ask_hostname,
    ask_ntp,
)
from archinstall.lib.interactions.network_menu import ask_to_configure_network
from archinstall.lib.locale.locale_menu import LocaleMenu
from archinstall.lib.mirrors import MirrorMenu
from archinstall.lib.translationhandler import tr
from archinstall.tui import EditMenu, MenuItem, MenuItemGroup, SelectMenu, Tui
from archinstall.tui.result import ResultType


class InitSystem(Enum):
    DRACUT = "dracut"
    MKINITCPIO = "mkinitcpio"


class ZFSEncryptionMode(Enum):
    NONE = "No encryption"
    POOL = "Encrypt entire pool"
    DATASET = "Encrypt base dataset only"


class ZFSInstallerMenu:
    """
    Custom installer menu using composition.

    This approach gives us full control over the menu structure and
    avoids compatibility issues with different archinstall versions.
    """

    def __init__(self, arch_config: ArchConfig):
        self.config = arch_config

        # ZFS-specific configuration
        self.dataset_prefix: str = "arch0"
        self.init_system: InitSystem = InitSystem.DRACUT
        self.zfs_encryption_mode: ZFSEncryptionMode = ZFSEncryptionMode.NONE
        self.zfs_encryption_password: str | None = None

    def run(self) -> None:
        """Run the main installer menu loop."""
        with Tui():
            while True:
                choice = self._show_main_menu()

                if choice == "install":
                    if self._validate_config():
                        break
                    continue
                if choice == "save":
                    self._save_config()
                elif choice == "abort":
                    sys.exit(1)
                elif choice:
                    self._handle_menu_choice(choice)

    def _show_main_menu(self) -> str | None:
        """Display the main configuration menu."""
        menu_items = [
            # Standard archinstall options (using their functions directly)
            MenuItem(text=tr("Locale configuration"), action=lambda _: self._configure_locale(), preview_action=lambda _: self._preview_locale(), key="locale"),
            MenuItem(
                text=tr("Mirror configuration"), action=lambda _: self._configure_mirrors(), preview_action=lambda _: self._preview_mirrors(), key="mirrors"
            ),
            MenuItem(
                text=tr("Network configuration"), action=lambda _: self._configure_network(), preview_action=lambda _: self._preview_network(), key="network"
            ),
            MenuItem(
                text=tr("Hostname"), action=lambda _: self._configure_hostname(), preview_action=lambda _: f"Hostname: {self.config.hostname}", key="hostname"
            ),
            MenuItem(text=tr("Authentication"), action=lambda _: self._configure_authentication(), preview_action=lambda _: self._preview_auth(), key="auth"),
            MenuItem(
                text=tr("Timezone"), action=lambda _: self._configure_timezone(), preview_action=lambda _: f"Timezone: {self.config.timezone}", key="timezone"
            ),
            MenuItem(
                text=tr("NTP (time sync)"),
                action=lambda _: self._configure_ntp(),
                preview_action=lambda _: f"NTP: {'Enabled' if self.config.ntp else 'Disabled'}",
                key="ntp",
            ),
            MenuItem(
                text=tr("Additional packages"), action=lambda _: self._configure_packages(), preview_action=lambda _: self._preview_packages(), key="packages"
            ),
            # Separator
            MenuItem(text=""),
            # ZFS-specific options
            MenuItem(
                text="ZFS Dataset Prefix",
                action=lambda _: self._configure_dataset_prefix(),
                preview_action=lambda _: f"Dataset prefix: {self.dataset_prefix}",
                key="zfs_prefix",
            ),
            MenuItem(
                text="ZFS Encryption",
                action=lambda _: self._configure_zfs_encryption(),
                preview_action=lambda _: self._preview_zfs_encryption(),
                key="zfs_encryption",
            ),
            MenuItem(
                text="Init System",
                action=lambda _: self._configure_init_system(),
                preview_action=lambda _: f"Init system: {self.init_system.value}",
                key="init_system",
            ),
            # Separator
            MenuItem(text=""),
            # Actions
            MenuItem(text=tr("Save configuration"), key="save"),
            MenuItem(text=tr("Install"), key="install"),
            MenuItem(text=tr("Abort"), key="abort"),
        ]

        menu = SelectMenu(MenuItemGroup(menu_items), header="Arch Linux ZFS Installation Configuration")

        result = menu.run()
        return result.item().key if result.item() else None

    def _handle_menu_choice(self, choice: str) -> None:
        """Handle a menu choice by calling the appropriate configuration method."""
        for item in self._show_main_menu.__code__.co_consts:
            if hasattr(item, "key") and item.key == choice:
                if item.action:
                    item.action(None)
                break

    # Standard archinstall configuration methods
    def _configure_locale(self) -> None:
        locale_menu = LocaleMenu()
        self.config.locale_config = locale_menu.run()

    def _configure_mirrors(self) -> None:
        mirror_menu = MirrorMenu()
        self.config.mirror_config = mirror_menu.run()

    def _configure_network(self) -> None:
        self.config.network_config = ask_to_configure_network()

    def _configure_hostname(self) -> None:
        hostname = ask_hostname()
        if hostname:
            self.config.hostname = hostname

    def _configure_authentication(self) -> None:
        auth_menu = AuthenticationMenu()
        self.config.auth_config = auth_menu.run()

    def _configure_timezone(self) -> None:
        timezone = ask_for_a_timezone()
        if timezone:
            self.config.timezone = timezone

    def _configure_ntp(self) -> None:
        self.config.ntp = ask_ntp(self.config.ntp)

    def _configure_packages(self) -> None:
        packages = ask_additional_packages_to_install()
        if packages:
            self.config.packages = packages

    # ZFS-specific configuration methods
    def _configure_dataset_prefix(self) -> None:
        result = EditMenu("ZFS Dataset Prefix", header="Enter prefix for ZFS datasets", default_text=self.dataset_prefix).input()

        if result.text():
            self.dataset_prefix = result.text()

    def _configure_zfs_encryption(self) -> None:
        encryption_menu = SelectMenu(
            MenuItemGroup(
                [
                    MenuItem(ZFSEncryptionMode.NONE.value, ZFSEncryptionMode.NONE),
                    MenuItem(ZFSEncryptionMode.POOL.value, ZFSEncryptionMode.POOL),
                    MenuItem(ZFSEncryptionMode.DATASET.value, ZFSEncryptionMode.DATASET),
                ]
            ),
            header="Select ZFS encryption mode",
        )

        result = encryption_menu.run()
        if result.type_ != ResultType.Skip:
            self.zfs_encryption_mode = result.item().value

            if self.zfs_encryption_mode != ZFSEncryptionMode.NONE:
                self._get_encryption_password()

    def _get_encryption_password(self) -> None:
        """Get encryption password for ZFS."""
        while True:
            password_result = EditMenu("ZFS Encryption Password", header="Enter password for ZFS encryption", hide_input=True).input()

            if not password_result.text():
                continue

            verify_result = EditMenu("Verify Password", header="Enter password again", hide_input=True).input()

            if password_result.text() == verify_result.text():
                self.zfs_encryption_password = password_result.text()
                break

    def _configure_init_system(self) -> None:
        init_menu = SelectMenu(
            MenuItemGroup([MenuItem("Dracut", InitSystem.DRACUT), MenuItem("Mkinitcpio", InitSystem.MKINITCPIO)]), header="Select init system"
        )

        result = init_menu.run()
        if result.type_ != ResultType.Skip:
            self.init_system = result.item().value

    # Preview methods
    def _preview_locale(self) -> str | None:
        if self.config.locale_config:
            return f"Locale: {self.config.locale_config.sys_lang}"
        return "Locale: Not configured"

    def _preview_mirrors(self) -> str | None:
        if self.config.mirror_config:
            return f"Mirrors: {len(self.config.mirror_config.regions)} regions"
        return "Mirrors: Not configured"

    def _preview_network(self) -> str | None:
        if self.config.network_config:
            return f"Network: {self.config.network_config.type.value}"
        return "Network: Not configured"

    def _preview_auth(self) -> str | None:
        if self.config.auth_config:
            user_count = len(self.config.auth_config.users) if self.config.auth_config.users else 0
            return f"Users: {user_count}, Root: {'Set' if self.config.auth_config.root_enc_password else 'Not set'}"
        return "Authentication: Not configured"

    def _preview_packages(self) -> str | None:
        if self.config.packages:
            return f"Additional packages: {len(self.config.packages)} selected"
        return "Additional packages: None"

    def _preview_zfs_encryption(self) -> str | None:
        mode_text = self.zfs_encryption_mode.value
        if self.zfs_encryption_mode != ZFSEncryptionMode.NONE:
            password_status = "Set" if self.zfs_encryption_password else "Not set"
            return f"Encryption: {mode_text}, Password: {password_status}"
        return f"Encryption: {mode_text}"

    def _validate_config(self) -> bool:
        """Validate that required configuration is set."""
        errors = []

        if not self.config.locale_config:
            errors.append("Locale configuration is required")

        if not self.config.auth_config or (not self.config.auth_config.users and not self.config.auth_config.root_enc_password):
            errors.append("Authentication configuration is required")

        if self.zfs_encryption_mode != ZFSEncryptionMode.NONE and not self.zfs_encryption_password:
            errors.append("ZFS encryption password is required when encryption is enabled")

        if errors:
            error_text = "Configuration errors:\n" + "\n".join(f"• {error}" for error in errors)
            SelectMenu(MenuItemGroup([MenuItem("OK", None)]), header=error_text).run()
            return False

        return True

    def _save_config(self) -> None:
        """Save configuration to file."""
        # This would implement config saving
        pass

    def get_zfs_config(self) -> dict[str, Any]:
        """Get ZFS-specific configuration."""
        return {
            "dataset_prefix": self.dataset_prefix,
            "init_system": self.init_system,
            "encryption_mode": self.zfs_encryption_mode,
            "encryption_password": self.zfs_encryption_password,
        }
