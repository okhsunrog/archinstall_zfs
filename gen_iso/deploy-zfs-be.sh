#!/usr/bin/env bash

set -Eeuo pipefail

app_name="${0##*/}"
dataset=""
source_root=""
boot_source=""
kernel="linux-lts"
mount_dir="/mnt/archzfs-be"
snapshot="fresh"
key_file="/etc/zfs/zroot.key"
dry_run=0
mounted=0
created_parent=0

usage() {
    cat <<EOF
Usage: ${app_name} --dataset POOL/BE/root --source-root DIR [options]

Deploy a completed mkarchiso airootfs staging tree as a writable
ZFSBootMenu boot environment.

Required:
  --dataset DATASET      New root dataset, ending in /root
  --source-root DIR      mkarchiso workdir/<arch>/airootfs directory

Options:
  --boot-source DIR      Directory containing vmlinuz-<kernel>
                         (auto-detected from the mkarchiso workdir)
  --kernel PACKAGE       Kernel package name (default: linux-lts)
  --mount-dir DIR        Temporary deployment mount (default: /mnt/archzfs-be)
  --snapshot NAME        Final snapshot name (default: fresh)
  --key-file FILE        Key copied into encrypted-root initramfs
                         (default: /etc/zfs/zroot.key)
  --dry-run              Validate and print the plan without changing ZFS
  -h, --help             Show this help

The target dataset and its parent must not already exist. The pool bootfs
property is never changed.
EOF
}

die() {
    printf '[%s] ERROR: %s\n' "${app_name}" "$*" >&2
    exit 1
}

info() {
    printf '[%s] %s\n' "${app_name}" "$*"
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

cleanup() {
    if (( mounted )) && mountpoint -q -- "${mount_dir}"; then
        umount -- "${mount_dir}" || true
    fi
}

on_error() {
    local line="$1"
    if (( created_parent )); then
        printf '[%s] ERROR: deployment failed at line %s; partial dataset retained: %s\n' \
            "${app_name}" "${line}" "${dataset%/root}" >&2
        printf '[%s] Remove it explicitly before retrying: zfs destroy -r %q\n' \
            "${app_name}" "${dataset%/root}" >&2
    fi
}

trap cleanup EXIT
trap 'on_error "$LINENO"' ERR

while (( $# )); do
    case "$1" in
        --dataset)
            (( $# >= 2 )) || die "--dataset requires a value"
            dataset="$2"
            shift 2
            ;;
        --source-root)
            (( $# >= 2 )) || die "--source-root requires a value"
            source_root="$2"
            shift 2
            ;;
        --boot-source)
            (( $# >= 2 )) || die "--boot-source requires a value"
            boot_source="$2"
            shift 2
            ;;
        --kernel)
            (( $# >= 2 )) || die "--kernel requires a value"
            kernel="$2"
            shift 2
            ;;
        --mount-dir)
            (( $# >= 2 )) || die "--mount-dir requires a value"
            mount_dir="$2"
            shift 2
            ;;
        --snapshot)
            (( $# >= 2 )) || die "--snapshot requires a value"
            snapshot="$2"
            shift 2
            ;;
        --key-file)
            (( $# >= 2 )) || die "--key-file requires a value"
            key_file="$2"
            shift 2
            ;;
        --dry-run)
            dry_run=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

(( EUID == 0 )) || die "run this script as root (for example, with sudo)"

for command in arch-chroot cat chmod find findmnt grep install ln mount mountpoint \
    realpath rm rsync stat sync umount zfs zpool; do
    require_command "${command}"
done

[[ -n "${dataset}" ]] || die "--dataset is required"
[[ -n "${source_root}" ]] || die "--source-root is required"
[[ "${dataset}" =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]*(/[A-Za-z0-9][A-Za-z0-9_.:-]*){2,}$ ]] \
    || die "invalid dataset name: ${dataset}"
[[ "${dataset##*/}" == "root" ]] || die "target dataset must end in /root"
[[ "${kernel}" =~ ^[A-Za-z0-9][A-Za-z0-9@._+-]*$ ]] || die "invalid kernel package name"
[[ "${snapshot}" =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]*$ ]] || die "invalid snapshot name"

source_root="$(realpath -e -- "${source_root}")"
[[ -d "${source_root}" ]] || die "source root is not a directory: ${source_root}"
[[ -x "${source_root}/usr/local/bin/azfs" ]] || die "source root does not contain executable azfs"
[[ -x "${source_root}/usr/bin/mkinitcpio" ]] || die "source root does not contain mkinitcpio"
[[ -x "${source_root}/usr/bin/pacman-key" ]] || die "source root does not contain pacman-key"
[[ -x "${source_root}/usr/bin/zfs" ]] || die "source root does not contain ZFS tools"
[[ -f /etc/hostid ]] || die "host has no /etc/hostid"
[[ "$(stat -c '%s' -- /etc/hostid)" == "4" ]] || die "host /etc/hostid is not exactly four bytes"

# A recursive chown of the mkarchiso workdir destroys the rootfs ownership
# information. Refuse it instead of producing an unbootable or insecure BE.
source_owner="$(stat -c '%u' -- "${source_root}/usr/bin/env")"
[[ "${source_owner}" == "0" ]] \
    || die "source root ownership is not preserved (usr/bin/env UID is ${source_owner}, expected 0)"

work_dir="$(realpath -m -- "${source_root}/../..")"
if [[ -z "${boot_source}" ]]; then
    for candidate in \
        "${work_dir}/x86_64/boot" \
        "${work_dir}/iso/arch/boot/x86_64"; do
        if [[ -f "${candidate}/vmlinuz-${kernel}" ]]; then
            boot_source="${candidate}"
            break
        fi
    done
fi
[[ -n "${boot_source}" ]] || die "could not auto-detect mkarchiso boot artifacts"
boot_source="$(realpath -e -- "${boot_source}")"
[[ -f "${boot_source}/vmlinuz-${kernel}" ]] \
    || die "kernel not found: ${boot_source}/vmlinuz-${kernel}"

mapfile -t module_dirs < <(find "${source_root}/usr/lib/modules" -mindepth 1 -maxdepth 1 -type d -print)
(( ${#module_dirs[@]} == 1 )) \
    || die "expected exactly one kernel module directory, found ${#module_dirs[@]}"
kernel_release="${module_dirs[0]##*/}"
[[ -f "${module_dirs[0]}/pkgbase" ]] || die "kernel module directory has no pkgbase marker"
[[ "$(<"${module_dirs[0]}/pkgbase")" == "${kernel}" ]] \
    || die "kernel module pkgbase does not match --kernel ${kernel}"
find "${module_dirs[0]}" -type f -name 'zfs.ko*' -print -quit | grep -q . \
    || die "ZFS kernel module is missing for ${kernel_release}"

parent="${dataset%/root}"
pool="${dataset%%/*}"
zpool list -H -o name -- "${pool}" >/dev/null 2>&1 || die "pool is not imported: ${pool}"

current_root="$(findmnt -n -o SOURCE /)"
[[ "${current_root}" != "${dataset}" ]] || die "refusing to replace the running root dataset"
[[ "${current_root}" != "${parent}"/* ]] || die "target parent contains the running root dataset"
if zfs list -H -o name -- "${parent}" >/dev/null 2>&1; then
    die "target parent already exists: ${parent}"
fi
if zfs list -H -o name -- "${dataset}" >/dev/null 2>&1; then
    die "target dataset already exists: ${dataset}"
fi

commandline=""
if [[ "${current_root}" == "${pool}/"* ]]; then
    commandline="$(zfs get -H -o value org.zfsbootmenu:commandline -- "${current_root}")"
    [[ "${commandline}" != "-" ]] || commandline=""
fi
if [[ " ${commandline} " != *" archinstall_zfs.demo=1 "* ]]; then
    commandline="${commandline:+${commandline} }archinstall_zfs.demo=1"
fi

encryption="$(zfs get -H -o value encryption -- "${pool}")"
key_target=""
if [[ "${encryption}" != "off" ]]; then
    key_location="$(zfs get -H -o value keylocation -- "${pool}")"
    case "${key_location}" in
        file:///*)
            [[ -f "${key_file}" ]] \
                || die "encrypted pool requires readable --key-file: ${key_file}"
            key_target="$(realpath -m -- "${key_location#file://}")"
            [[ "${key_target}" == /* && "${key_target}" != "/" ]] \
                || die "unsafe ZFS keylocation: ${key_location}"
            # The key is copied into this BE, so it must not depend on another
            # BE remaining available as ZFSBootMenu's key source.
            keysource="${dataset}"
            ;;
        prompt)
            keysource=""
            ;;
        *)
            die "unsupported encrypted-pool keylocation: ${key_location}"
            ;;
    esac
else
    keysource=""
fi

mount_dir="$(realpath -m -- "${mount_dir}")"
[[ "${mount_dir}" != "/" ]] || die "refusing to use / as deployment mount"
if mountpoint -q -- "${mount_dir}"; then
    die "deployment mount is already in use: ${mount_dir}"
fi
if [[ -d "${mount_dir}" ]] && find "${mount_dir}" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
    die "deployment mount is not empty: ${mount_dir}"
fi

info "source root: ${source_root}"
info "kernel: ${kernel} (${kernel_release})"
info "target dataset: ${dataset}"
info "temporary mount: ${mount_dir}"
info "ZFSBootMenu command line: ${commandline}"
info "ZFSBootMenu key source: ${keysource:-not set}"

if (( dry_run )); then
    info "dry run completed; no changes made"
    exit 0
fi

install -d -m 0755 -- "${mount_dir}"

zfs create -u \
    -o mountpoint=none \
    -o canmount=off \
    -o overlay=off \
    -o compression=lz4 \
    -- "${parent}"
created_parent=1

create_options=(
    -u
    -o mountpoint=/
    -o canmount=noauto
    -o overlay=off
    -o "org.zfsbootmenu:commandline=${commandline}"
)
if [[ -n "${keysource}" ]]; then
    create_options+=(-o "org.zfsbootmenu:keysource=${keysource}")
fi
zfs create "${create_options[@]}" -- "${dataset}"

mount -t zfs -o zfsutil -- "${dataset}" "${mount_dir}"
mounted=1

info "copying mkarchiso rootfs into the dataset"
rsync -aHAX --numeric-ids --delete -- "${source_root}/" "${mount_dir}/"

install -D -m 0644 -- "${boot_source}/vmlinuz-${kernel}" "${mount_dir}/boot/vmlinuz-${kernel}"
install -D -m 0644 -- /etc/hostid "${mount_dir}/etc/hostid"
rm -f -- "${mount_dir}/etc/zfs/zpool.cache"

initramfs_files=(/etc/hostid)
if [[ -n "${key_target}" ]]; then
    install -D -m 0000 -- "${key_file}" "${mount_dir}${key_target}"
    initramfs_files+=("${key_target}")
fi

# Replace the live-media initramfs preset with a normal ZFS-root preset.
rm -f -- "${mount_dir}/etc/mkinitcpio.conf.d/archiso.conf"
if [[ "${kernel}" != "linux" ]]; then
    rm -f -- "${mount_dir}/etc/mkinitcpio.d/linux.preset"
fi
install -d -m 0755 -- "${mount_dir}/etc/mkinitcpio.d" "${mount_dir}/etc/mkinitcpio.conf.d"
cat >"${mount_dir}/etc/mkinitcpio.d/${kernel}.preset" <<EOF
# Generated for the archinstall_zfs ZFSBootMenu demo environment.
ALL_kver="/boot/vmlinuz-${kernel}"
PRESETS=('default')
default_image="/boot/initramfs-${kernel}.img"
EOF

{
    printf 'FILES=('
    printf '%q ' "${initramfs_files[@]}"
    printf ')\n'
    printf '%s\n' 'HOOKS=(base udev autodetect microcode modconf kms keyboard keymap consolefont block zfs filesystems)'
} >"${mount_dir}/etc/mkinitcpio.conf.d/zfs-root.conf"
chmod 0644 -- "${mount_dir}/etc/mkinitcpio.d/${kernel}.preset" \
    "${mount_dir}/etc/mkinitcpio.conf.d/zfs-root.conf"

# The live ISO keeps pacman's keyring in tmpfs. A normal writable BE needs a
# persistent keyring of its own; do not copy the host's private local key.
rm -rf -- "${mount_dir}/etc/pacman.d/gnupg"
install -d -m 0755 -- "${mount_dir}/etc/pacman.d/gnupg"
rm -f -- \
    "${mount_dir}/etc/systemd/system/etc-pacman.d-gnupg.mount" \
    "${mount_dir}/etc/systemd/system/pacman-init.service" \
    "${mount_dir}/etc/systemd/system/zfs-dkms-autoinstall.service" \
    "${mount_dir}/etc/systemd/system/multi-user.target.wants/pacman-init.service"

# Keep the hardware-oriented live profile, but remove behavior that only
# makes sense for an ephemeral installation medium.
rm -f -- \
    "${mount_dir}/etc/systemd/system/multi-user.target.wants/reflector.service" \
    "${mount_dir}/etc/systemd/system/multi-user.target.wants/sshd.service" \
    "${mount_dir}/etc/systemd/journald.conf.d/volatile-storage.conf"
install -D -m 0644 /dev/null "${mount_dir}/etc/cloud/cloud-init.disabled"
ln -sfn /dev/null "${mount_dir}/etc/systemd/system/zfs-mount.service"
printf '%s\n' 'archzfs-demo' >"${mount_dir}/etc/hostname"

cat >"${mount_dir}/usr/local/bin/azfs-demo" <<'EOF'
#!/usr/bin/env bash
exec /usr/local/bin/azfs --demo "$@"
EOF
chmod 0755 -- "${mount_dir}/usr/local/bin/azfs-demo"

cat >>"${mount_dir}/etc/motd" <<'EOF'

Safe bare-metal UI test environment:
  azfs       LinuxKMS UI; safe mode is enforced by the kernel command line
  azfs-demo  LinuxKMS UI with safe mode explicitly enabled
EOF

install -D -m 0644 /dev/stdin "${mount_dir}/etc/archinstall-zfs/demo-be" <<EOF
dataset=${dataset}
kernel=${kernel}
kernel_release=${kernel_release}
source_root=${source_root}
EOF

info "initializing a persistent pacman keyring"
arch-chroot "${mount_dir}" /usr/bin/pacman-key --init
arch-chroot "${mount_dir}" /usr/bin/pacman-key --populate archlinux

info "generating normal ZFS-root initramfs"
arch-chroot "${mount_dir}" /usr/bin/mkinitcpio -p "${kernel}"

initramfs="${mount_dir}/boot/initramfs-${kernel}.img"
[[ -s "${initramfs}" ]] || die "mkinitcpio did not create ${initramfs}"
if find "${initramfs}" -perm /0077 -print -quit | grep -q .; then
    die "initramfs is readable by non-root users"
fi
initramfs_listing="$(arch-chroot "${mount_dir}" /usr/bin/lsinitcpio "/boot/initramfs-${kernel}.img")"
grep -Eq '/zfs\.ko(\.|$)' <<<"${initramfs_listing}" \
    || die "generated initramfs does not contain zfs.ko"
if [[ -n "${key_target}" ]]; then
    grep -qx "${key_target#/}" <<<"${initramfs_listing}" \
        || die "generated initramfs does not contain the pool key"
fi

[[ "$(stat -c '%u:%g' -- "${mount_dir}/usr/bin/env")" == "0:0" ]] \
    || die "deployed rootfs ownership verification failed"
[[ -x "${mount_dir}/usr/local/bin/azfs" ]] || die "deployed azfs binary is not executable"

zfs snapshot -- "${dataset}@${snapshot}"
sync -f -- "${mount_dir}"
umount -- "${mount_dir}"
mounted=0

[[ "$(zfs get -H -o value mounted -- "${dataset}")" == "no" ]] \
    || die "target dataset remained mounted"

info "created ${dataset}@${snapshot}"
info "ZFSBootMenu will discover the BE from its /boot kernel and mountpoint=/ property"
info "pool bootfs was not changed"
