#!/usr/bin/env bash

set -Eeuo pipefail

marker_name=".archinstall-zfs-be-workdir"
marker_text="archinstall_zfs ZFS BE staging workdir v1"

die() {
    printf '[reset-zfs-be-workdir.sh] ERROR: %s\n' "$*" >&2
    exit 1
}

(( EUID == 0 )) || die "run this script as root"
(( $# == 1 )) || die "usage: $0 WORKDIR"

work_dir="$(realpath -m -- "$1")"
[[ "${work_dir}" == /* ]] || die "workdir must be an absolute path"

case "${work_dir}" in
    /|/home|/root|/tmp|/var|/var/tmp)
        die "refusing unsafe workdir: ${work_dir}"
        ;;
esac

marker="${work_dir}/${marker_name}"
if [[ -e "${work_dir}" ]]; then
    [[ -f "${marker}" ]] || die "existing path is not a managed BE workdir: ${work_dir}"
    [[ "$(<"${marker}")" == "${marker_text}" ]] \
        || die "managed-workdir marker is invalid: ${marker}"

    mounts=()
    while IFS= read -r mounted_path; do
        if [[ "${mounted_path}" == "${work_dir}" || "${mounted_path}" == "${work_dir}/"* ]]; then
            mounts+=("${mounted_path}")
        fi
    done < <(findmnt -rn -o TARGET)
    if (( ${#mounts[@]} )); then
        printf '[reset-zfs-be-workdir.sh] ERROR: refusing to delete a workdir with active mounts:\n' >&2
        printf '  %s\n' "${mounts[@]}" >&2
        exit 1
    fi

    rm -rf --one-file-system -- "${work_dir}"
fi

install -d -m 0755 -- "${work_dir}"
printf '%s\n' "${marker_text}" >"${marker}"
