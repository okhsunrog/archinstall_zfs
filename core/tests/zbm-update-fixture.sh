#!/usr/bin/bash
# Requires root. Only mounts and modifies an image created in this fixture.
set -euo pipefail
[[ $EUID == 0 ]] || { echo 'Run this fixture as root' >&2; exit 1; }
helper=$(realpath "$(dirname "$0")/../assets/azfs-install-zbm")
work=$(mktemp -d)
cleanup() { if mountpoint -q "$work/esp"; then umount "$work/esp"; fi; rm -rf -- "$work"; }
trap cleanup EXIT
mkdir "$work/esp" "$work/stage"
truncate -s 64M "$work/esp.img"
mkfs.fat -F32 "$work/esp.img" >/dev/null
mount -o loop "$work/esp.img" "$work/esp"
esp=$work/esp
stage=$work/stage
mkdir -p "$esp/EFI/BOOT"
printf 'Foreign bootloader\n' > "$esp/EFI/BOOT/BOOTX64.EFI"
cp "$esp/EFI/BOOT/BOOTX64.EFI" "$work/foreign"
make_image() {
    truncate -s "$2" "$1"
    printf MZ | dd of="$1" conv=notrunc status=none
    printf '\200\000\000\000' | dd of="$1" bs=1 seek=60 conv=notrunc status=none
    printf 'PE\000\000\144\206' | dd of="$1" bs=1 seek=128 conv=notrunc status=none
    printf '\013\002' | dd of="$1" bs=1 seek=152 conv=notrunc status=none
    printf '\012\000' | dd of="$1" bs=1 seek=220 conv=notrunc status=none
}
make_image "$stage/vmlinuz.EFI" 8M
bash "$helper" "$stage" "$esp"
cmp "$stage/vmlinuz.EFI" "$esp/EFI/zbm/vmlinuz.EFI"
cmp "$work/foreign" "$esp/EFI/BOOT/BOOTX64.EFI"
[[ ! -e $esp/EFI/zbm/vmlinuz-backup.EFI ]]
cp "$stage/vmlinuz.EFI" "$work/old"
# Occupy enough space that old and new cannot coexist on the ESP.
dd if=/dev/zero of="$esp/filler" bs=1M count=49 status=none
make_image "$stage/vmlinuz.EFI" 10M
bash "$helper" "$stage" "$esp"
cmp "$stage/vmlinuz.EFI" "$esp/EFI/zbm/vmlinuz.EFI"
cmp "$work/old" "$stage/previous-installed.EFI"
cmp "$work/foreign" "$esp/EFI/BOOT/BOOTX64.EFI"
cp "$stage/vmlinuz.EFI" "$work/current"
make_image "$stage/vmlinuz.EFI" 70M
if bash "$helper" "$stage" "$esp"; then echo 'Oversized image accepted' >&2; exit 1; fi
cmp "$work/current" "$esp/EFI/zbm/vmlinuz.EFI"
printf 'invalid image' > "$stage/vmlinuz.EFI"
if bash "$helper" "$stage" "$esp"; then echo 'Invalid image accepted' >&2; exit 1; fi
cmp "$work/current" "$esp/EFI/zbm/vmlinuz.EFI"
# Reclaim only a verified owned fallback when it would block the main update.
rm "$esp/filler" "$esp/EFI/BOOT/BOOTX64.EFI"
cp "$work/current" "$esp/EFI/BOOT/BOOTX64.EFI"
dd if=/dev/zero of="$esp/filler" bs=1M count=40 status=none
make_image "$stage/vmlinuz.EFI" 18M
bash "$helper" "$stage" "$esp"
cmp "$stage/vmlinuz.EFI" "$esp/EFI/zbm/vmlinuz.EFI"
[[ ! -e $esp/EFI/BOOT/BOOTX64.EFI ]]
# MZ alone is not enough: reject a non-EFI PE before replacing anything.
printf '\000\000' | dd of="$stage/vmlinuz.EFI" bs=1 seek=220 conv=notrunc status=none
if bash "$helper" "$stage" "$esp"; then echo 'Non-EFI PE accepted' >&2; exit 1; fi
[[ $(stat -c %s "$esp/EFI/zbm/vmlinuz.EFI") == 18874368 ]]
printf 'PASS: FAT publication, single-image replacement, root recovery, foreign fallback preservation, owned fallback reclamation, invalid and oversized image rejection\n'
