#!/usr/bin/env bash
# Helpers shared by deploy-zfs-be.sh and update-zfs-be.sh. Sourced, not run:
# the caller sets app_name, mount_dir and mounted before sourcing, and keeps
# its own set -Eeuo pipefail.
# shellcheck disable=SC2154  # app_name, mount_dir and mounted come from the caller

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

# Option parsing: option $1 takes a value, so the remaining argument count $2
# must cover both the option and its value.
need_value() {
    (( $2 >= 2 )) || die "$1 requires a value"
}

cleanup() {
    if (( mounted )) && mountpoint -q -- "${mount_dir}"; then
        umount -- "${mount_dir}" || true
    fi
}
trap cleanup EXIT

mount_be() {
    mount -t zfs -o zfsutil -- "$1" "${mount_dir}"
    mounted=1
}

umount_be() {
    umount -- "${mount_dir}"
    mounted=0
}
