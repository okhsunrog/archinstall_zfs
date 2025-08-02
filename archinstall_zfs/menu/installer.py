from enum import Enum
from typing import Any, cast

from archinstall.lib.global_menu import GlobalMenu
from archinstall.tui import EditMenu, MenuItem, MenuItemGroup, SelectMenu


class InitSystem(Enum):
    DRACUT = "dracut"
    MKINITCPIO = "mkinitcpio"


class InstallerMenu(GlobalMenu):
    def __init__(self, data_store: dict[str, Any]):
        super().__init__(data_store)

        # Disable options that conflict with ZFS installation
        self.set_enabled("disk_config", False)
        self.set_enabled("disk_encryption", False)
        self.set_enabled("swap", False)
        self.set_enabled("bootloader", False)
        self.set_enabled("uki", False)
        self.set_enabled("kernels", False)
        self.set_enabled("parallel downloads", False)
        self.set_enabled("additional-repositories", False)

        # Add ZFS-specific options
        zfs_options = [
            MenuItem(
                text="Dataset Prefix",
                value="arch0",
                action=lambda x: self._select_dataset_prefix(x),
                preview_action=self._prev_dataset_prefix,
                key="dataset_prefix",
            ),
            MenuItem(
                text="Init System",
                value=InitSystem.DRACUT,
                action=lambda x: self._select_init_system(x),
                preview_action=self._prev_init_system,
                key="init_system",
            ),
        ]

        # Insert ZFS options at the beginning of the menu
        self._item_group.items[0:0] = zfs_options

    def _select_dataset_prefix(self, preset: str) -> str:
        return cast(str, EditMenu("Dataset Prefix", header="Enter prefix for ZFS datasets", default_text=preset).input().text())

    def _select_init_system(self, _preset: InitSystem) -> InitSystem:
        menu = SelectMenu(MenuItemGroup([MenuItem("Dracut", InitSystem.DRACUT), MenuItem("Mkinitcpio", InitSystem.MKINITCPIO)]), header="Select init system")
        return cast(InitSystem, menu.run().item().value)

    def _prev_dataset_prefix(self, item: MenuItem) -> str | None:
        if item.value:
            return f"Dataset prefix: {item.value}"
        return None

    def _prev_init_system(self, item: MenuItem) -> str | None:
        if item.value:
            return f"Init system: {item.value.value}"
        return None
