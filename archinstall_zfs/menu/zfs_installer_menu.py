"""
ZFS Installer Menu - Custom menu system using composition over inheritance.

This provides a clean separation between archinstall library functions and
ZFS-specific configuration, allowing for better maintainability and version independence.
"""

import re
import sys
from pathlib import Path
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
from archinstall.lib.models.locale import LocaleConfiguration
from archinstall.lib.translationhandler import tr
from archinstall.tui import EditMenu, MenuItem, MenuItemGroup, SelectMenu, Tui
from archinstall.tui.result import ResultType

from archinstall_zfs.initramfs.base import InitramfsHandler
from archinstall_zfs.initramfs.dracut import DracutInitramfsHandler
from archinstall_zfs.initramfs.mkinitcpio import MkinitcpioInitramfsHandler
from archinstall_zfs.menu.models import GlobalConfig, InitSystem, InstallationMode, ZFSEncryptionMode, ZFSModuleMode


class GlobalConfigMenu:
    """
    Custom installer menu using composition.

    This approach gives us full control over the menu structure and
    avoids compatibility issues with different archinstall versions.
    """

    def __init__(self, arch_config: ArchConfig):
        self.config = arch_config
        self.cfg = GlobalConfig()

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

    def _get_menu_items(self) -> list[MenuItem]:
        """Get the list of menu items for the main configuration menu."""
        return [
            # Standard archinstall options (using their functions directly)
            MenuItem(text=tr("Locale configuration"), preview_action=self._preview_locale, key="locale"),
            MenuItem(text=tr("Mirror configuration"), preview_action=self._preview_mirrors, key="mirrors"),
            MenuItem(text=tr("Network configuration"), preview_action=self._preview_network, key="network"),
            MenuItem(text=tr("Hostname"), preview_action=lambda _: f"Hostname: {self.config.hostname}", key="hostname"),
            MenuItem(text=tr("Authentication"), preview_action=self._preview_auth, key="auth"),
            MenuItem(text=tr("Timezone"), preview_action=lambda _: f"Timezone: {self.config.timezone}", key="timezone"),
            MenuItem(
                text=tr("NTP (time sync)"),
                preview_action=lambda _: f"NTP: {'Enabled' if self.config.ntp else 'Disabled'}",
                key="ntp",
            ),
            MenuItem(text=tr("Additional packages"), preview_action=self._preview_packages, key="packages"),
            # Separator
            MenuItem(text=""),
            # ZFS-specific options
            MenuItem(
                text="ZFS Configuration",
                preview_action=self._preview_zfs_configuration,
                key="zfs_config",
            ),
            MenuItem(
                text="Installation Mode",
                preview_action=self._preview_installation_mode,
                key="install_mode",
            ),
            MenuItem(
                text="Disk Configuration",
                preview_action=self._preview_disk_configuration,
                key="disk_config",
            ),
            MenuItem(
                text="ZFS Pool Name",
                preview_action=lambda _: f"Pool: {self.cfg.pool_name or 'Not set'}",
                key="pool_name",
            ),
            MenuItem(
                text="ZFS Encryption",
                preview_action=self._preview_zfs_encryption,
                key="zfs_encryption",
            ),
            MenuItem(
                text="Init System",
                preview_action=lambda _: f"Init system: {self.cfg.init_system.value}",
                key="init_system",
            ),
            MenuItem(
                text="ZFS Modules Source",
                preview_action=lambda _: f"ZFS modules: {self.cfg.zfs_module_mode.value}",
                key="zfs_modules",
            ),
            # Separator
            MenuItem(text=""),
            # Actions
            MenuItem(text=tr("Save configuration"), key="save"),
            MenuItem(text=tr("Install"), key="install"),
            MenuItem(text=tr("Abort"), key="abort"),
        ]

    def _show_main_menu(self) -> str | None:
        """Display the main configuration menu."""
        menu_items = self._get_menu_items()
        menu = SelectMenu(MenuItemGroup(menu_items), header="Arch Linux ZFS Installation Configuration")

        result = menu.run()
        return result.item().key if result.item() else None

    def _handle_menu_choice(self, choice: str) -> None:
        """Handle a menu choice by calling the appropriate configuration method."""
        handlers: dict[str, Any] = {
            "locale": self._configure_locale,
            "mirrors": self._configure_mirrors,
            "network": self._configure_network,
            "hostname": self._configure_hostname,
            "auth": self._configure_authentication,
            "timezone": self._configure_timezone,
            "ntp": self._configure_ntp,
            "packages": self._configure_packages,
            "zfs_config": self._configure_zfs_configuration,
            "install_mode": self._configure_installation_mode,
            "disk_config": self._configure_disk_configuration,
            "pool_name": self._configure_pool_name,
            "init_system": self._configure_init_system,
        }
        handler = handlers.get(choice)
        if handler:
            handler()

    # Standard archinstall configuration methods
    def _configure_locale(self, *_: Any) -> None:
        # Use existing locale config or create default
        current_config = self.config.locale_config or LocaleConfiguration.default()
        locale_menu = LocaleMenu(current_config)
        self.config.locale_config = locale_menu.run()

    def _configure_mirrors(self, *_: Any) -> None:
        # Use existing mirror config if available
        mirror_menu = MirrorMenu(self.config.mirror_config)
        self.config.mirror_config = mirror_menu.run()

    def _configure_network(self, *_: Any) -> None:
        self.config.network_config = ask_to_configure_network(self.config.network_config)

    def _configure_hostname(self, *_: Any) -> None:
        hostname = ask_hostname(self.config.hostname)
        if hostname:
            self.config.hostname = hostname

    def _configure_authentication(self, *_: Any) -> None:
        # Use existing auth config if available
        auth_menu = AuthenticationMenu(self.config.auth_config)
        self.config.auth_config = auth_menu.run()

    def _configure_timezone(self, *_: Any) -> None:
        timezone = ask_for_a_timezone(self.config.timezone)
        if timezone:
            self.config.timezone = timezone

    def _configure_ntp(self, *_: Any) -> None:
        ntp_result = ask_ntp(self.config.ntp)
        if ntp_result is not None:
            self.config.ntp = ntp_result

    def _configure_packages(self, *_: Any) -> None:
        packages = ask_additional_packages_to_install(self.config.packages)
        if packages is not None:
            self.config.packages = packages

    # ZFS-specific configuration methods
    def _configure_installation_mode(self, *_: Any) -> None:
        mode_menu = SelectMenu(
            MenuItemGroup(
                [
                    MenuItem("Full Disk Installation", InstallationMode.FULL_DISK),
                    MenuItem("New ZFS Pool", InstallationMode.NEW_POOL),
                    MenuItem("Existing ZFS Pool", InstallationMode.EXISTING_POOL),
                ]
            ),
            header="Select installation mode",
        )
        result = mode_menu.run()
        if result.type_ != ResultType.Skip and result.item():
            self.cfg.installation_mode = result.item().value
            # Clear disk-related fields when switching modes to avoid stale state
            self.cfg.disk_by_id = None
            self.cfg.efi_partition_by_id = None
            self.cfg.zfs_partition_by_id = None

    def _configure_disk_by_id(self) -> bool:
        items = self._list_by_id_disks_menu_items()
        if not items:
            SelectMenu(MenuItemGroup([MenuItem("OK", None)]), header="No /dev/disk/by-id entries found").run()
            return False
        choice = SelectMenu(MenuItemGroup(items), header="Select target disk (/dev/disk/by-id)").run()
        if choice.item():
            self.cfg.disk_by_id = str(choice.item().value)
            return True
        return False

    def _configure_efi_partition_by_id(self) -> bool:
        parts = self._list_by_id_partitions_menu_items()
        if not parts:
            SelectMenu(MenuItemGroup([MenuItem("OK", None)]), header="No partitions found under /dev/disk/by-id").run()
            return False
        choice = SelectMenu(MenuItemGroup(parts), header="Select EFI partition (/dev/disk/by-id)").run()
        if choice.item():
            self.cfg.efi_partition_by_id = str(choice.item().value)
            return True
        return False

    def _configure_zfs_partition_by_id(self) -> bool:
        parts = self._list_by_id_partitions_menu_items()
        if not parts:
            SelectMenu(MenuItemGroup([MenuItem("OK", None)]), header="No partitions found under /dev/disk/by-id").run()
            return False
        choice = SelectMenu(MenuItemGroup(parts), header="Select ZFS partition (/dev/disk/by-id)").run()
        if choice.item():
            self.cfg.zfs_partition_by_id = str(choice.item().value)
            return True
        return False

    def _configure_disk_configuration(self, *_: Any) -> None:
        """Guided flow that asks for disk + partitions depending on install mode."""
        mode = self.cfg.installation_mode
        if not mode:
            SelectMenu(MenuItemGroup([MenuItem("OK", None)]), header="Select installation mode first").run()
            return

        # For full disk, only disk selection is needed (partitions will be created)
        if mode is InstallationMode.FULL_DISK:
            if not self._configure_disk_by_id():
                return
            # Partitions will be derived during full-disk partitioning
            return

        # For new/existing pool, we need an EFI partition and optionally ZFS partition (new pool)
        if not self._configure_disk_by_id():
            return
        if not self._configure_efi_partition_by_id():
            return
        if mode is InstallationMode.NEW_POOL:
            self._configure_zfs_partition_by_id()

    def _configure_pool_name(self, *_: Any) -> None:
        result = EditMenu(
            "ZFS Pool Name",
            header="Enter ZFS pool name (used for new_pool/existing_pool)",
            default_text=self.cfg.pool_name or "zroot",
        ).input()
        if result.text():
            self.cfg.pool_name = result.text()

    def _configure_dataset_prefix(self, *_: Any) -> None:
        result = EditMenu(
            "ZFS Dataset Prefix",
            header="Enter prefix for ZFS datasets",
            default_text=self.cfg.dataset_prefix,
        ).input()

        if result.text():
            self.cfg.dataset_prefix = result.text()

    def _configure_zfs_encryption(self, *_: Any) -> None:
        encryption_menu = SelectMenu(
            MenuItemGroup(
                [
                    MenuItem("No encryption", ZFSEncryptionMode.NONE),
                    MenuItem("Encrypt entire pool", ZFSEncryptionMode.POOL),
                    MenuItem("Encrypt base dataset only", ZFSEncryptionMode.DATASET),
                ]
            ),
            header="Select ZFS encryption mode",
        )

        result = encryption_menu.run()
        if result.type_ != ResultType.Skip:
            selected = result.item().value if result.item() else None
            if selected is not None:
                self.cfg.zfs_encryption_mode = selected

            if self.cfg.zfs_encryption_mode != ZFSEncryptionMode.NONE:
                self._get_encryption_password()

    def _configure_zfs_configuration(self, *_: Any) -> None:
        """Grouped flow for ZFS settings: dataset prefix, encryption, module mode."""
        while True:
            summary = self._preview_zfs_configuration(None) or ""
            menu = SelectMenu(
                MenuItemGroup(
                    [
                        MenuItem("Dataset Prefix", "prefix"),
                        MenuItem("Encryption", "encryption"),
                        MenuItem("Modules Source", "modules"),
                        MenuItem("Done", "done"),
                    ]
                ),
                header=f"ZFS Configuration\n{summary}",
            )
            choice = menu.run().item().value
            if choice == "prefix":
                self._configure_dataset_prefix()
            elif choice == "encryption":
                self._configure_zfs_encryption()
            elif choice == "modules":
                self._configure_zfs_modules()
            else:
                break

    def _get_encryption_password(self) -> None:
        """Get encryption password for ZFS."""
        while True:
            password_result = EditMenu("ZFS Encryption Password", header="Enter password for ZFS encryption", hide_input=True).input()

            if not password_result.text():
                continue

            verify_result = EditMenu("Verify Password", header="Enter password again", hide_input=True).input()

            if password_result.text() == verify_result.text():
                self.cfg.zfs_encryption_password = password_result.text()
                break

    def _configure_init_system(self, *_: Any) -> None:
        init_menu = SelectMenu(
            MenuItemGroup([MenuItem("Dracut", InitSystem.DRACUT), MenuItem("Mkinitcpio", InitSystem.MKINITCPIO)]), header="Select init system"
        )

        result = init_menu.run()
        if result.type_ != ResultType.Skip:
            selected = result.item().value if result.item() else None
            if selected is not None:
                self.cfg.init_system = selected

    def _configure_zfs_modules(self, *_: Any) -> None:
        mode_menu = SelectMenu(
            MenuItemGroup(
                [
                    MenuItem("Precompiled (preferred)", ZFSModuleMode.PRECOMPILED),
                    MenuItem("DKMS", ZFSModuleMode.DKMS),
                ]
            ),
            header="Select ZFS modules source",
        )
        result = mode_menu.run()
        if result.type_ != ResultType.Skip:
            selected = result.item().value if result.item() else None
            if selected is not None:
                self.cfg.zfs_module_mode = selected

    # Preview methods
    def _preview_locale(self, *_: Any) -> str | None:
        if self.config.locale_config:
            return f"Locale: {self.config.locale_config.sys_lang}"
        return "Locale: Not configured"

    def _preview_mirrors(self, *_: Any) -> str | None:
        if self.config.mirror_config:
            return "Mirrors: Configured"
        return "Mirrors: Not configured"

    def _preview_network(self, *_: Any) -> str | None:
        if self.config.network_config:
            return f"Network: {self.config.network_config.type.value}"
        return "Network: Not configured"

    def _preview_auth(self, *_: Any) -> str | None:
        if self.config.auth_config:
            user_count = len(self.config.auth_config.users) if self.config.auth_config.users else 0
            return f"Users: {user_count}, Root: {'Set' if self.config.auth_config.root_enc_password else 'Not set'}"
        return "Authentication: Not configured"

    def _preview_packages(self, *_: Any) -> str | None:
        if self.config.packages:
            return f"Additional packages: {len(self.config.packages)} selected"
        return "Additional packages: None"

    def _preview_zfs_encryption(self, *_: Any) -> str | None:
        mode_text = self.cfg.zfs_encryption_mode.value
        if self.cfg.zfs_encryption_mode != ZFSEncryptionMode.NONE:
            password_status = "Set" if self.cfg.zfs_encryption_password else "Not set"
            return f"Encryption: {mode_text}, Password: {password_status}"
        return f"Encryption: {mode_text}"

    def _preview_zfs_configuration(self, *_: Any) -> str | None:
        enc = self._preview_zfs_encryption(None) or ""
        return f"Dataset prefix: {self.cfg.dataset_prefix}\n{enc}\nModules: {self.cfg.zfs_module_mode.value}"

    def _preview_installation_mode(self, *_: Any) -> str | None:
        if not self.cfg.installation_mode:
            return "Install mode: Not set"
        return f"Install mode: {self.cfg.installation_mode.value}"

    def _preview_disk_configuration(self, *_: Any) -> str:
        mode = self.cfg.installation_mode
        if not mode:
            return "Disk: (mode not set)"
        if mode is InstallationMode.FULL_DISK:
            return f"Disk: {self.cfg.disk_by_id or 'Not set'} (full disk)"
        if mode is InstallationMode.NEW_POOL:
            return (
                f"Disk: {self.cfg.disk_by_id or 'Not set'}; EFI: {self.cfg.efi_partition_by_id or 'Not set'}; ZFS: {self.cfg.zfs_partition_by_id or 'Not set'}"
            )
        # The only remaining valid enum value is EXISTING_POOL
        return f"Disk: {self.cfg.disk_by_id or 'Not set'}; EFI: {self.cfg.efi_partition_by_id or 'Not set'}"

    # --- by-id listing helpers for global menu ---
    @staticmethod
    def _by_id_dir() -> Path:
        return Path("/dev/disk/by-id")

    @classmethod
    def _list_by_id_disks(cls) -> list[Path]:
        base = cls._by_id_dir()
        if not base.exists():
            return []
        entries = []
        for p in base.iterdir():
            if not p.is_symlink():
                continue
            # Filter out partitions (names ending with -part<digits>)
            if re.search(r"-part\d+$", p.name):
                continue
            entries.append(p)
        return sorted(entries)

    @classmethod
    def _list_by_id_partitions(cls) -> list[Path]:
        base = cls._by_id_dir()
        if not base.exists():
            return []
        parts = []
        for p in base.iterdir():
            if not p.is_symlink():
                continue
            if re.search(r"-part\d+$", p.name):
                parts.append(p)
        return sorted(parts)

    def _list_by_id_disks_menu_items(self) -> list[MenuItem]:
        return [MenuItem(str(p), p) for p in self._list_by_id_disks()]

    def _list_by_id_partitions_menu_items(self) -> list[MenuItem]:
        # If a disk is selected, prefer partitions of that disk only
        if self.cfg.disk_by_id:
            base_path = Path(self.cfg.disk_by_id)
            name_prefix = base_path.name + "-part"
            candidates = [p for p in self._list_by_id_partitions() if p.name.startswith(name_prefix)]
        else:
            candidates = self._list_by_id_partitions()
        return [MenuItem(str(p), p) for p in candidates]

    def _validate_config(self) -> bool:
        """Validate that required configuration is set."""
        errors = []

        if not self.config.locale_config:
            errors.append("Locale configuration is required")

        if not self.config.auth_config or (not self.config.auth_config.users and not self.config.auth_config.root_enc_password):
            errors.append("Authentication configuration is required")

        # Validate ZFS-specific and top-level install prerequisites via the config model
        errors.extend(self.cfg.validate_for_install())

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
        return self.cfg.to_json()

    # Factory for initramfs handler
    def create_initramfs_handler(self, target: Path, encryption_enabled: bool = False) -> InitramfsHandler:
        if self.cfg.init_system == InitSystem.DRACUT:
            return DracutInitramfsHandler(target, encryption_enabled)
        return MkinitcpioInitramfsHandler(target, encryption_enabled)

    # Serialization for combined configuration file
    def to_json(self) -> dict[str, Any]:
        return self.cfg.to_json()

    def apply_json(self, data: dict[str, Any]) -> None:
        if not data:
            return
        # Use model parsing; keep existing values if keys are absent
        new_cfg = GlobalConfig.from_json(data)
        # Merge field-by-field to avoid clobbering password unintentionally
        for field in new_cfg.model_fields:
            value = getattr(new_cfg, field)
            if value is not None:
                setattr(self.cfg, field, value)
