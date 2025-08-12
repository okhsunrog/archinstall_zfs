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
        conf_path = self.target / "etc/mkinitcpio.conf"
        conf_path.parent.mkdir(parents=True, exist_ok=True)
        content = conf_path.read_text() if conf_path.exists() else ""

        # Ensure MODULES contains zfs
        if "MODULES=(" in content:
            if "zfs" not in content:
                content = content.replace("MODULES=(", "MODULES=(zfs ")
        else:
            content += "MODULES=(zfs)\n"

        # Ensure HOOKS exist and include zfs before filesystems
        if "HOOKS=(" not in content:
            hooks = "HOOKS=(base udev autodetect modconf kms keyboard keymap consolefont block zfs filesystems fsck)"
            content += hooks + "\n"
        else:
            updated_lines: list[str] = []
            for hooks_line in content.splitlines():
                new_line = hooks_line
                if hooks_line.strip().startswith("HOOKS=(") and " zfs " not in f" {hooks_line} ":
                    if "filesystems" in hooks_line:
                        new_line = hooks_line.replace("filesystems", "zfs filesystems")
                    elif hooks_line.rstrip().endswith(")"):
                        new_line = hooks_line[:-1] + " zfs)"
                updated_lines.append(new_line)
            content = "\n".join(updated_lines) + "\n"

        # Set COMPRESSION=cat to avoid double-compression on ZFS, with a comment
        if "COMPRESSION=" not in content:
            content += '# ZFS datasets are already compressed, use uncompressed initramfs to avoid double compression\nCOMPRESSION="cat"\n'
        else:
            # Replace existing COMPRESSION line
            lines = []
            replaced = False
            for line in content.splitlines():
                if line.strip().startswith("COMPRESSION="):
                    lines.append('COMPRESSION="cat"')
                    replaced = True
                else:
                    lines.append(line)
            content = "\n".join(lines) + ("\n" if not content.endswith("\n") else "")
            if not replaced:
                content += 'COMPRESSION="cat"\n'

        # If encryption is enabled, include the key file
        if self.encryption_enabled:
            if "FILES=(" in content:
                if "/etc/zfs/zroot.key" not in content:
                    content = content.replace("FILES=(", "FILES=(/etc/zfs/zroot.key ")
            else:
                content += "FILES=(/etc/zfs/zroot.key)\n"

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
