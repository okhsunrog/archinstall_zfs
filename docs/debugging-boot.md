# Debugging Boot Issues in QEMU

Guide for diagnosing why an installed system doesn't boot after `archinstall-zfs-rs` installation.

## Quick Diagnosis Workflow

1. Boot from disk with VNC + QEMU monitor
2. Take a screenshot to see where it's stuck
3. If ZFSBootMenu shows: check hostid, commandline, bootfs
4. If kernel drops to emergency shell: check initramfs, root= parameter
5. If login prompt shows but no SSH: check network, sshd

## Boot with VNC + QEMU Monitor

The serial console is useless for ZFSBootMenu (it renders to the framebuffer, not serial). Use VNC + the QEMU monitor for screenshots:

```bash
OVMF_CODE=$(find /usr/share/edk2 /usr/share/edk2-ovmf -name "OVMF_CODE*.4m.fd" ! -name "*secboot*" -print -quit)
DISK="gen_iso/arch.qcow2"
VARS="gen_iso/my_vars.fd"

qemu-system-x86_64 -enable-kvm -cpu host -m 4096 -smp 2 \
    -boot "order=c" \
    -display vnc=127.0.0.1:0 \
    -monitor unix:/tmp/qemu-mon.sock,server,nowait \
    -net nic -net user,hostfwd=tcp::2223-:22 \
    -machine type=q35,smm=on,accel=kvm,usb=on \
    -global ICH9-LPC.disable_s3=1 \
    -drive "if=pflash,format=raw,unit=0,file=$OVMF_CODE,read-only=on" \
    -drive "if=pflash,format=raw,unit=1,file=$VARS" \
    -drive "file=$DISK,format=qcow2,if=none,id=disk0" \
    -device "virtio-blk-pci,drive=disk0,serial=archzfs-test-disk" \
    -daemonize
```

Take a screenshot after ~30 seconds:
```bash
echo "screendump /tmp/qemu-screen.ppm" | socat - UNIX-CONNECT:/tmp/qemu-mon.sock
magick /tmp/qemu-screen.ppm /tmp/qemu-screen.png
```

Send Enter key to ZFSBootMenu (if stuck at menu):
```bash
echo "sendkey ret" | socat - UNIX-CONNECT:/tmp/qemu-mon.sock
```

Kill the VM:
```bash
kill $(pgrep -f qemu-system)
```

## UEFI Vars and Boot Order

The UEFI vars file (`my_vars.fd`) stores EFI boot entries. After installation, `efibootmgr` writes ZFSBootMenu entries into it. If you reset the vars file (for clean boot testing), the efibootmgr entries are lost.

The fallback bootloader at `EFI/BOOT/BOOTX64.EFI` on the EFI partition handles this — UEFI finds it automatically when no specific boot entry exists. The installer copies ZFSBootMenu there for this reason.

If the disk doesn't boot after resetting UEFI vars, verify the fallback exists:
```bash
# Boot the ISO, import the pool, mount the EFI partition
mount /dev/disk/by-id/virtio-archzfs-test-disk-part1 /tmp/efi
find /tmp/efi -type f
# Should show:
# /tmp/efi/EFI/ZBM/VMLINUZ.EFI
# /tmp/efi/EFI/ZBM/RECOVERY.EFI
# /tmp/efi/EFI/BOOT/BOOTX64.EFI
```

## Symptom: ZFSBootMenu Shows Menu but Doesn't Auto-Boot

Screenshot shows ZFSBootMenu menu with boot environments listed but no countdown.

Causes:
- `bootfs` pool property not set (required for auto-boot)
- `zbm.timeout` not passed on ZBM's kernel cmdline

Check from the ISO:
```bash
zpool import -N testpool
zpool get bootfs testpool
# Should show: bootfs    testpool/arch0/root
```

Fix:
```bash
zpool set bootfs=testpool/arch0/root testpool
```

## Symptom: ZFSBootMenu Shows `spl_hostid=00000000`

ZBM header displays the hostid it used to import the pool. If it shows `00000000`, ZBM is using its default (embedded) hostid, not the one from the installed system.

This matters because ZBM's `zbm.set_hostid` (enabled by default) passes its own hostid to the booted kernel. If ZBM imported with hostid 0, the kernel gets `spl.spl_hostid=00000000`, which may not match the pool's hostid.

The pool was created with `zgenhostid -f 0x00bab10c`, so it expects hostid `00bab10c`.

Fix: pass `spl_hostid=0x00bab10c` on ZBM's own kernel cmdline via `efibootmgr -u`:
```bash
efibootmgr -c -d /dev/disk/by-id/virtio-archzfs-test-disk-part1 \
    -L "ZFSBootMenu" \
    -l "\\EFI\\ZBM\\VMLINUZ.EFI" \
    -u "spl_hostid=0x00bab10c zbm.timeout=10"
```

Note: the `org.zfsbootmenu:commandline` ZFS property also contains `spl.spl_hostid=0x00bab10c`, but this is passed to the **booted OS kernel**, not to ZBM itself. Both are needed.

## Symptom: Kernel Drops to Dracut Emergency Shell

Screenshot shows:
```
Generating "/run/initramfs/rdsosreport.txt"
Entering emergency shell. Exit the shell to continue.
```

This means dracut started but couldn't mount the ZFS root. Common causes:

### Cause 1: Hostid mismatch
The kernel's `spl.spl_hostid` doesn't match the pool's hostid. ZFS refuses to import.

Verify from the ISO:
```bash
zpool import -N testpool
zpool get all testpool | grep hostid
# Check what hostid the pool was created with

# Check what's in the installed system
zfs mount testpool/arch0/root
od -A n -t x1 /mnt/etc/hostid | tr -d ' \n'
# Should be: 0cb1ba00 (little-endian for 00bab10c)
```

### Cause 2: `root=` parameter wrong or duplicated
The `org.zfsbootmenu:commandline` must NOT contain `root=`. ZBM adds it automatically using the `org.zfsbootmenu:rootprefix` property.

Check:
```bash
zfs get org.zfsbootmenu:commandline testpool/arch0/root
# Should contain: spl.spl_hostid=0x00bab10c zswap.enabled=0 rw
# Should NOT contain: root=ZFS= or root=zfs:

zfs get org.zfsbootmenu:rootprefix testpool/arch0/root
# For dracut: root=ZFS=
# For mkinitcpio: zfs=
```

### Cause 3: Initramfs missing ZFS module
The dracut initramfs wasn't built with ZFS support.

Check:
```bash
zfs mount testpool/arch0/root
cat /mnt/etc/dracut.conf.d/zfs.conf
# Should contain: hostonly="yes"
ls /mnt/boot/
# Should contain: vmlinuz-linux-lts, initramfs-linux-lts.img
```

### Cause 4: Wrong kernel version in initramfs
The dracut `generate` command used `$(uname -r)` (ISO kernel) instead of the installed kernel version.

Check:
```bash
ls /mnt/usr/lib/modules/
# Should show the installed kernel version, e.g., 6.18.20-1-lts
ls /mnt/boot/
# vmlinuz-linux-lts and initramfs-linux-lts.img should exist
```

The correct generation command (inside chroot):
```bash
kver=$(ls -1 /usr/lib/modules | sort | tail -n1)
pkgbase=$(cat /usr/lib/modules/$kver/pkgbase 2>/dev/null || echo linux)
install -Dm0644 /usr/lib/modules/$kver/vmlinuz /boot/vmlinuz-$pkgbase
dracut --force /boot/initramfs-$pkgbase.img --kver $kver
```

## Symptom: Login Prompt Shows but No SSH

Screenshot shows `archzfs-test login:` but SSH connection refused or times out.

### Cause 1: sshd not enabled or not installed
```bash
# From ISO, mount and check
zpool import -R /mnt testpool
zfs mount testpool/arch0/root
ls /mnt/etc/systemd/system/multi-user.target.wants/ | grep ssh
cat /mnt/etc/ssh/sshd_config.d/10-root-login.conf
# Should show: PermitRootLogin yes
```

### Cause 2: Network not configured
The installed system uses systemd-networkd with configs copied from the ISO. Interface names may differ between ISO and installed kernel.

Check if network configs use wildcard matching:
```bash
cat /mnt/etc/systemd/network/*.network
# Should match on Name=en* or similar wildcard, not specific interface names
```

### Cause 3: SSH host keys not generated
OpenSSH needs host keys generated on first boot. The `sshdgenkeys.service` should handle this automatically, but check:
```bash
ls /mnt/etc/ssh/ssh_host_*
# If empty, keys haven't been generated yet
```

## Inspecting the Installed System from the ISO

Boot the testing ISO, then:
```bash
# Import and mount
zpool import -R /mnt testpool
zfs mount testpool/arch0/root
zfs mount -a

# Mount EFI
mount /dev/disk/by-id/virtio-archzfs-test-disk-part1 /mnt/boot/efi

# Now inspect anything under /mnt/
cat /mnt/etc/fstab
cat /mnt/etc/hostname
ls /mnt/boot/
find /mnt/boot/efi/EFI -type f
cat /mnt/etc/dracut.conf.d/zfs.conf
zfs get all testpool/arch0/root | grep zfsbootmenu

# Clean up
umount /mnt/boot/efi
zfs umount -a
zpool export testpool
```

## Useful ZFS Property Checks

```bash
# All ZBM-related properties
zfs get all testpool/arch0/root | grep zfsbootmenu

# Pool bootfs (needed for auto-boot)
zpool get bootfs testpool

# Dataset layout
zfs list -o name,mountpoint,canmount

# Encryption status
zfs get encryption,keystatus testpool/arch0/root
```
