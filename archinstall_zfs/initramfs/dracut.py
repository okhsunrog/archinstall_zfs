from pathlib import Path

from archinstall.lib.general import SysCommand

from .base import InitramfsHandler


class DracutInitramfsHandler(InitramfsHandler):
    def __init__(self, target: Path, encryption_enabled: bool = False):
        super().__init__(target, encryption_enabled)
        self.scripts_dir = Path(target) / "usr/local/bin"
        self.hooks_dir = Path(target) / "etc/pacman.d/hooks"
        self.conf_dir = Path(target) / "etc/dracut.conf.d"

    def _write_dracut_conf(self) -> None:
        lines = [
            'hostonly="yes"',
            'hostonly_cmdline="no"',
            'fscks="no"',
            'early_microcode="yes"',
            'compress="zstd"',
        ]

        if self.encryption_enabled:
            lines.append('install_items+=" /etc/zfs/zroot.key "')

        (self.conf_dir / "zfs.conf").write_text("\n".join(lines) + "\n")

    def configure(self) -> None:
        self._create_directories()
        self._write_dracut_conf()
        self._create_scripts()
        self._create_hooks()

    def _create_directories(self) -> None:
        self.scripts_dir.mkdir(parents=True, exist_ok=True)
        self.hooks_dir.mkdir(parents=True, exist_ok=True)
        self.conf_dir.mkdir(parents=True, exist_ok=True)

    def _create_scripts(self) -> None:
        dracut_install_script = """#!/usr/bin/env bash
args=('--force' '--no-hostonly-cmdline')
while read -r line; do
    if [[ "$line" == 'usr/lib/modules/'+([^/])'/pkgbase' ]]; then
        read -r pkgbase < "/${line}"
        kver="${line#'usr/lib/modules/'}"
        kver="${kver%'/pkgbase'}"
        install -Dm0644 "/${line%'/pkgbase'}/vmlinuz" "/boot/vmlinuz-${pkgbase}"
        dracut "${args[@]}" "/boot/initramfs-${pkgbase}.img" --kver "$kver"
    fi
done"""

        dracut_remove_script = """#!/usr/bin/env bash
while read -r line; do
    if [[ "$line" == 'usr/lib/modules/'+([^/])'/pkgbase' ]]; then
        read -r pkgbase < "/${line}"
        rm -f "/boot/vmlinuz-${pkgbase}" "/boot/initramfs-${pkgbase}.img"
    fi
done"""

        (self.scripts_dir / "dracut-install.sh").write_text(dracut_install_script)
        (self.scripts_dir / "dracut-remove.sh").write_text(dracut_remove_script)

        SysCommand(f"chmod +x {self.scripts_dir}/dracut-install.sh {self.scripts_dir}/dracut-remove.sh")

    def _create_hooks(self) -> None:
        install_hook = """[Trigger]
Type = Path
Operation = Install
Operation = Upgrade
Target = usr/lib/modules/*/pkgbase

[Action]
Description = Updating linux initcpios (with dracut!)...
When = PostTransaction
Exec = /usr/local/bin/dracut-install.sh
Depends = dracut
NeedsTargets"""

        remove_hook = """[Trigger]
Type = Path
Operation = Remove
Target = usr/lib/modules/*/pkgbase

[Action]
Description = Removing linux initcpios...
When = PreTransaction
Exec = /usr/local/bin/dracut-remove.sh
NeedsTargets"""

        (self.hooks_dir / "90-dracut-install.hook").write_text(install_hook)
        (self.hooks_dir / "60-dracut-remove.hook").write_text(remove_hook)

    # InitramfsHandler API
    def install_packages(self) -> list[str]:
        return ["dracut"]

    def setup_hooks(self) -> None:
        # Hooks are created as part of configure()
        return None

    def generate_initramfs(self, _: str) -> bool:
        # Generate initramfs inside the chroot for the latest installed kernel
        # Compute version and pkgbase inside the chroot shell to avoid host expansion
        try:
            cmd = (
                f"arch-chroot {self.target} bash -lc "
                f"'kver=$(ls -1 /usr/lib/modules | sort | tail -n1); "
                f"pkgbase=$(cat /usr/lib/modules/$kver/pkgbase 2>/dev/null || echo linux); "
                f"install -Dm0644 /usr/lib/modules/$kver/vmlinuz /boot/vmlinuz-$pkgbase; "
                f"dracut --force /boot/initramfs-$pkgbase.img --kver $kver'"
            )
            SysCommand(cmd)
            return True
        except Exception:
            return False
