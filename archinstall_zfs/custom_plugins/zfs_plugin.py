from archinstall import SysCommand


def add_archzfs_repo(target_path: str = "/") -> None:
    """Add archzfs repository to pacman.conf"""
    pacman_conf = f"{target_path.rstrip('/')}/etc/pacman.conf"

    with open(pacman_conf, "a") as f:
        f.write("\n[archzfs]\n")
        f.write("Server = https://archzfs.com/$repo/$arch\n")
        f.write("SigLevel = Optional TrustAll\n")

    SysCommand('pacman -Sy')


class ZfsPlugin:
    def on_pacstrap(self, packages):
        # Add ZFS packages to initial pacstrap
        packages.extend(['zfs-dkms', 'zfs-utils'])
        return packages

    def on_install(self, installation):
        add_archzfs_repo(installation.target)
        return False

    def on_mkinitcpio(self, installation):
        # Find the index of 'filesystems' hook
        filesystems_index = installation._hooks.index('filesystems')
        # Insert 'zfs' right before it
        installation._hooks.insert(filesystems_index, 'zfs')
        return False