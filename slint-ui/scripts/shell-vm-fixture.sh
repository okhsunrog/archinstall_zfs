#!/bin/bash
# Run ONLY in a disposable VM. The named test disk is overwritten.
# QEMU: -drive file=test.qcow2,if=none,id=test,format=qcow2
#       -device virtio-blk-pci,drive=test,serial=azfs-shell-test-disk
# Usage: bash shell-vm-fixture.sh [/dev/vda] [plain|encrypted]
set -euo pipefail
disk=${1:-/dev/vda}
mode=${2:-plain}
[[ $mode == plain || $mode == encrypted ]]
[[ $(lsblk -dn -o SERIAL "$disk") == azfs-shell-test-disk ]] || {
    echo 'Refusing to overwrite a disk without the dedicated test serial.' >&2
    exit 1
}
[[ $disk == /dev/vd[a-z] ]]
modprobe zfs
if zpool list shelltest >/dev/null 2>&1; then
    echo 'Fixture pool already exists; use a fresh disposable VM.' >&2
    exit 1
fi
sgdisk --zap-all "$disk"
sgdisk -n 1:1M:+128M -t 1:ef00 -n 2:0:0 -t 2:bf00 "$disk"
udevadm settle
mkfs.vfat "${disk}1"
mkdir -p /run/azfs-fixture
zpool create -f -R /run/azfs-fixture -O mountpoint=none shelltest "${disk}2"
if [[ $mode == encrypted ]]; then
    # Disposable test credential, never a real user's secret.
    printf 'vm-test-passphrase\n' > /run/azfs-fixture-key
    zfs create -u -o mountpoint=none -o encryption=aes-256-gcm \
        -o keyformat=passphrase -o keylocation=file:///run/azfs-fixture-key shelltest/arch0
else
    zfs create -u -o mountpoint=none shelltest/arch0
fi
zfs create -u -o mountpoint=/ -o canmount=noauto shelltest/arch0/root
zfs create -u -o mountpoint=none shelltest/arch0/data
zfs create -u -o mountpoint=/home shelltest/arch0/data/home
zfs create -u -o mountpoint=/root shelltest/arch0/data/root
zfs create -u -o mountpoint=/vm shelltest/arch0/vm
zfs mount shelltest/arch0/root
root=/run/azfs-fixture
mkdir -p "$root"/{usr/bin,usr/lib,etc,boot/efi,proc,sys,dev,run,tmp}
ln -s usr/bin "$root/bin"
ln -s usr/lib "$root/lib"
ln -s usr/lib "$root/lib64"
for binary in bash touch cat sleep id ls; do
    cp "/usr/bin/$binary" "$root/usr/bin/"
    ldd "/usr/bin/$binary" | awk '/=> \// {print $3} /^\s*\/lib/ {print $1}' |
        while read -r library; do cp -L "$library" "$root/usr/lib/"; done
done
cp -L /lib64/ld-linux-x86-64.so.2 "$root/usr/lib/"
cp /etc/group /etc/nsswitch.conf "$root/etc/"
printf 'root:x:0:0:root:/root:/bin/bash\n' > "$root/etc/passwd"
printf '{"pool_name":"shelltest","pool_guid":"%s","dataset_prefix":"arch0","efi_partuuid":"%s"}\n' \
    "$(zpool get -H -o value guid shelltest)" \
    "$(blkid -s PARTUUID -o value "${disk}1")" > /root/azfs-shell-target.json
zpool export shelltest
rm -f /run/azfs-fixture-key
rmdir /run/azfs-fixture
printf 'Fixture ready: /root/azfs-shell-target.json (%s)\n' "$mode"
