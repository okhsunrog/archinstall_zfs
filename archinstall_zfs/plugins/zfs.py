from archinstall import SysCommand

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