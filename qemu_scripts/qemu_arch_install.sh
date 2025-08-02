#!/bin/bash

# Default ISO location
DEFAULT_ISO=~/tmp_zfs/archlinux-2024.12.01-x86_64.iso
ISO_IMAGE=${1:-$DEFAULT_ISO}

qemu-system-x86_64 -enable-kvm -m 4G -cpu host -smp 2,sockets=1,cores=2,threads=1 -boot d -cdrom "$ISO_IMAGE" -drive file=arch.qcow2,format=qcow2 -net nic -net user,hostfwd=tcp::2222-:22 -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2-ovmf/x64/OVMF_CODE.4m.fd -drive if=pflash,format=raw,file=./my_vars.fd

