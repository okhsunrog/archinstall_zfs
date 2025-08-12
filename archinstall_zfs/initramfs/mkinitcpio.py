from __future__ import annotations

from pathlib import Path

from archinstall.lib.general import SysCommand

from .base import InitramfsHandler


class MkinitcpioInitramfsHandler(InitramfsHandler):
    def __init__(self, target: Path, encryption_enabled: bool = False):
        super().__init__(target, encryption_enabled)

    def install_packages(self) -> list[str]:
        return ["mkinitcpio"]

    def configure(self) -> None:
        # Ensure MODULES+=(zfs) in mkinitcpio.conf on target
        conf_path = self.target / "etc/mkinitcpio.conf"
        content = conf_path.read_text() if conf_path.exists() else ""

        if "MODULES=(" in content:
            if "zfs" not in content:
                content = content.replace("MODULES=(", "MODULES=(zfs ")
        else:
            content += "\nMODULES=(zfs)\n"

        conf_path.parent.mkdir(parents=True, exist_ok=True)
        conf_path.write_text(content)

    def setup_hooks(self) -> None:
        # Rely on standard mkinitcpio hooks; no special encryption hooks for native ZFS encryption
        return None

    def generate_initramfs(self, _: str) -> bool:
        try:
            # Use mkinitcpio -P to build all present kernels in the chroot
            SysCommand(f"arch-chroot {self.target} mkinitcpio -P")
            return True
        except Exception:
            return False
