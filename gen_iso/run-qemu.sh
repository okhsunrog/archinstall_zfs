#!/usr/bin/env bash
#
# Copyright (C) 2020 David Runge <dvzrv@archlinux.org>
# Heavily modified for archinstall_zfs
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A simple script to run an archiso image or a disk image using qemu. The image
# can be booted using BIOS or UEFI.
#
# Requirements:
# - qemu
# - edk2-ovmf (when UEFI booting)

set -eu

print_help() {
    local usagetext
    IFS='' read -r -d '' usagetext <<EOF || true
Usage:
    run-qemu.sh [options]

Options:
    -a              Set accessibility support using brltty
    -b              Set boot type to 'BIOS' (UEFI is default)
    -h              Print help
    -i [image]      ISO image to boot into
    -D [image]      Disk image (*.qcow2) to use
    -s              Use Secure Boot (only relevant when using UEFI)
    -u              Set boot type to 'UEFI' (default)
    -v              Use VNC display (instead of default SDL)
    -S              Use serial console exclusively (no GUI)
    -c [image]      Attach an additional optical disc image (e.g. for cloud-init)
    -U [file]       Path to UEFI variables file (e.g. my_vars.fd)
    -C [file]       Path to UEFI code file (e.g. OVMF_CODE.4m.fd)
    -m [memory]     Set VM memory in MB (default: 4096)
    -p [cores]      Set VM CPU core count (default: 2)
EOF
    printf '%s' "${usagetext}"
}

cleanup_working_dir() {
    if [[ -d "${working_dir}" ]]; then
        rm -rf -- "${working_dir}"
    fi
}

find_ovmf_file() {
    local file_to_find=$1
    local paths=(
        "/usr/share/edk2/x64/${file_to_find}"
        "/usr/share/edk2-ovmf/x64/${file_to_find}"
        "/usr/share/OVMF/${file_to_find}"
    )
    for path in "${paths[@]}"; do
        if [[ -f "${path}" ]]; then
            echo "${path}"
            return
        fi
    done
}

copy_ovmf_vars() {
    local ovmf_vars_template
    ovmf_vars_template=$(find_ovmf_file "OVMF_VARS.4m.fd")
    if [[ -z "${ovmf_vars_template}" ]]; then
        ovmf_vars_template=$(find_ovmf_file "OVMF_VARS.fd")
    fi

    if [[ ! -f "${ovmf_vars_template}" ]]; then
        printf 'ERROR: %s\n' "OVMF_VARS.fd not found. Install edk2-ovmf."
        exit 1
    fi
    cp -av -- "${ovmf_vars_template}" "${working_dir}/OVMF_VARS.fd"
    uefi_vars_file="${working_dir}/OVMF_VARS.fd"
}

check_images() {
    if [[ -z "$iso_image" ]] && [[ -z "$disk_image" ]]; then
        printf 'ERROR: %s\n' "At least an ISO image (-i) or a disk image (-D) must be specified."
        print_help
        exit 1
    fi
    if [[ -n "$iso_image" && ! -f "$iso_image" ]]; then
        printf 'ERROR: %s\n' "ISO image file (${iso_image}) does not exist."
        exit 1
    fi
    if [[ -n "$disk_image" && ! -f "$disk_image" ]]; then
        printf 'ERROR: %s\n' "Disk image file (${disk_image}) does not exist."
        exit 1
    fi
}

run_image() {
    local boot_order='d'
    if [[ -z "${iso_image}" ]] && [[ -n "${disk_image}" ]]; then
        boot_order='c'
    fi

    if [[ "$boot_type" == 'uefi' ]]; then
        # Handle UEFI Code file
        if [[ -z "${uefi_code_file}" ]]; then
            if [[ "${secure_boot}" == 'on' ]]; then
                uefi_code_file=$(find_ovmf_file "OVMF_CODE.secboot.4m.fd")
            else
                uefi_code_file=$(find_ovmf_file "OVMF_CODE.4m.fd")
            fi
        fi

        if [[ ! -f "${uefi_code_file}" ]]; then
            printf 'ERROR: %s\n' "OVMF code file not found. Install edk2-ovmf and/or specify path with -C."
            exit 1
        fi

        # Handle UEFI Vars file
        if [[ -z "${uefi_vars_file}" ]]; then
            copy_ovmf_vars
        elif [[ ! -f "${uefi_vars_file}" ]]; then
            printf 'ERROR: %s\n' "UEFI vars file (${uefi_vars_file}) does not exist."
            exit 1
        fi

        qemu_options+=(
            '-drive' "if=pflash,format=raw,unit=0,file=${uefi_code_file},read-only=on"
            '-drive' "if=pflash,format=raw,unit=1,file=${uefi_vars_file}"
        )
        if [[ "${uefi_code_file}" == *".secboot."* ]]; then
             qemu_options+=('-global' "driver=cfi.pflash01,property=secure,value=on")
        fi
    fi

    if [[ "${accessibility}" == 'on' ]]; then
        qemu_options+=(
            '-chardev' 'braille,id=brltty'
            '-device' 'usb-braille,id=usbbrl,chardev=brltty'
        )
    fi

    if [[ -n "${iso_image}" ]]; then
         qemu_options+=('-cdrom' "${iso_image}")
    fi

    if [[ -n "${disk_image}" ]]; then
        qemu_options+=('-drive' "file=${disk_image},format=qcow2,if=virtio")
    fi

    if [[ -n "${oddimage}" ]]; then
        qemu_options+=(
            '-device' 'ide-cd,drive=cdrom1'
            '-drive' "id=cdrom1,if=none,format=raw,media=cdrom,read-only=on,file=${oddimage}"
        )
    fi

    if [[ "${serial_console}" == "on" ]]; then
        display="none -nographic"
    fi

    qemu-system-x86_64 \
        -enable-kvm \
        -cpu host \
        -m "${memory}" -smp "${cpu_cores}" \
        -boot "order=${boot_order},menu=on,reboot-timeout=5000" \
        -name "archinstall-zfs-vm,process=archinstall-zfs-vm" \
        -vga virtio \
        -display ${display} \
        -audiodev pa,id=snd0 \
        -device ich9-intel-hda -device hda-output,audiodev=snd0 \
        -net nic -net user,hostfwd=tcp::2222-:22 \
        -machine type=q35,smm=on,accel=kvm,usb=on,pcspk-audiodev=snd0 \
        -global ICH9-LPC.disable_s3=1 \
        -serial stdio \
        -no-reboot \
        "${qemu_options[@]}"
}

iso_image=''
disk_image=''
oddimage=''
accessibility='off'
boot_type='uefi'
secure_boot='off'
display='sdl'
serial_console='off'
uefi_vars_file=''
uefi_code_file=''
memory=4096
cpu_cores=2
qemu_options=()
working_dir="$(mktemp -dt run-qemu.XXXXXXXXXX)"
trap cleanup_working_dir EXIT

if (( ${#} == 0 )); then
    print_help
    exit 1
fi

while getopts 'abhi:sD:uvSc:U:C:m:p:' flag; do
    case "$flag" in
        a)
            accessibility='on'
            ;;
        b)
            boot_type='bios'
            ;;
        h)
            print_help
            exit 0
            ;;
        i)
            iso_image="$OPTARG"
            ;;
        s)
            secure_boot='on'
            ;;
        D)
            disk_image="$OPTARG"
            ;;
        u)
            boot_type='uefi'
            ;;
        v)
            display='vnc=0.0.0.0:0'
            ;;
        S)
            serial_console='on'
            ;;
        c)
            oddimage="$OPTARG"
            ;;
        U)
            uefi_vars_file="$OPTARG"
            ;;
        C)
            uefi_code_file="$OPTARG"
            ;;
        m)
            memory="$OPTARG"
            ;;
        p)
            cpu_cores="$OPTARG"
            ;;
        *)
            print_help
            exit 1
            ;;
    esac
done

if [[ "${serial_console}" == 'on' && "${display}" != "sdl" ]]; then
    printf "ERROR: -S (serial) and -v (VNC) are mutually exclusive.\n" >&2
    exit 1
fi

check_images
run_image
