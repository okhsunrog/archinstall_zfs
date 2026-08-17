#!/usr/bin/env bash

set -Eeuo pipefail

app_name="${0##*/}"
dataset=""
binary_dir="target/release"
mount_dir="/mnt/archzfs-be"
snapshot=""
force=0
dry_run=0
mounted=0

# The binaries this tooling puts into a demo boot environment, and where they
# live inside it.
binaries=(azfs azfs-tui)
install_dir="/usr/local/bin"

usage() {
    cat <<EOF
Usage: ${app_name} --dataset POOL/BE/root [options]

Replace the installer binaries inside a boot environment that already exists,
without rebuilding it. Recreating a demo BE means a full mkarchiso run and a
fresh dataset; replacing the two binaries is what actually changes between one
test and the next.

Only the binaries are touched. The rest of the environment is deliberately left
alone: deploy-zfs-be.sh customises a great deal after it copies the rootfs —
initramfs presets, hostname, keyring, the removal of live-medium units — and
copying the build overlay back over an existing environment would undo that.

Required:
  --dataset DATASET      Boot environment root dataset, as deployed

Options:
  --binary-dir DIR       Where the built binaries are (default: ${binary_dir})
  --mount-dir DIR        Temporary mount (default: ${mount_dir})
  --snapshot NAME        Snapshot the environment before changing it
  --force                Update even if the dataset carries no deployment marker
  --dry-run              Report what would change and exit
  -h, --help             Show this help

Refuses to touch the running system, and refuses a dataset that is mounted
somewhere already.
EOF
}

die() {
    printf '[%s] ERROR: %s\n' "${app_name}" "$*" >&2
    exit 1
}

info() {
    printf '[%s] %s\n' "${app_name}" "$*"
}

cleanup() {
    if (( mounted )) && mountpoint -q -- "${mount_dir}"; then
        umount -- "${mount_dir}" || true
    fi
}
trap cleanup EXIT

while (( $# )); do
    case "$1" in
        --dataset)
            (( $# >= 2 )) || die "--dataset requires a value"
            dataset="$2"
            shift 2
            ;;
        --binary-dir)
            (( $# >= 2 )) || die "--binary-dir requires a value"
            binary_dir="$2"
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
        --force)
            force=1
            shift
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

[[ -n "${dataset}" ]] || { usage >&2; die "--dataset is required"; }
[[ -n "${snapshot}" && ! "${snapshot}" =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]*$ ]] &&
    die "invalid snapshot name: ${snapshot}"

for command in findmnt install mount mountpoint sync umount zfs; do
    command -v "${command}" >/dev/null 2>&1 || die "required command not found: ${command}"
done

(( EUID == 0 )) || die "must run as root (mounting a dataset needs it)"

# ── What is being updated ────────────────────────────────────────────────

zfs list -H -o name -- "${dataset}" >/dev/null 2>&1 ||
    die "no such dataset: ${dataset} (deploy it first with zfs-be-deploy)"

running_root="$(findmnt -no SOURCE / || true)"
[[ "${running_root}" == "${dataset}" ]] &&
    die "${dataset} is the system you are running; boot another environment to update it"

existing_mount="$(zfs get -H -o value mounted -- "${dataset}")"
[[ "${existing_mount}" == "yes" ]] &&
    die "${dataset} is mounted at $(zfs get -H -o value mountpoint -- "${dataset}"); unmount it first"

# ── What is being installed ──────────────────────────────────────────────

declare -a sources=()
for binary in "${binaries[@]}"; do
    path="${binary_dir}/${binary}"
    [[ -f "${path}" ]] || die "not built: ${path} (run 'just cargo-build' first)"
    [[ -x "${path}" ]] || die "not executable: ${path}"
    sources+=("${path}")
done

info "updating ${dataset}"
for path in "${sources[@]}"; do
    info "  ${path} -> ${install_dir}/$(basename -- "${path}") ($(stat -c %y -- "${path}" | cut -d. -f1))"
done

if (( dry_run )); then
    info "dry run: nothing was changed"
    exit 0
fi

# ── Update ───────────────────────────────────────────────────────────────

if mountpoint -q -- "${mount_dir}"; then
    die "mount point is already in use: ${mount_dir}"
fi
mkdir -p -- "${mount_dir}"

mount -t zfs -o zfsutil -- "${dataset}" "${mount_dir}"
mounted=1

marker="${mount_dir}/etc/archinstall-zfs/demo-be"
if [[ ! -f "${marker}" ]]; then
    (( force )) || die "${dataset} carries no deployment marker (${marker#"${mount_dir}"}); \
it may not be a demo environment. Pass --force to update it anyway."
    info "no deployment marker; continuing because --force was given"
fi

if [[ -n "${snapshot}" ]]; then
    # Before the write, so the snapshot is of the environment as it was.
    sync
    zfs snapshot -- "${dataset}@${snapshot}"
    info "snapshotted ${dataset}@${snapshot}"
fi

for path in "${sources[@]}"; do
    install -D -m 0755 -o root -g root -- \
        "${path}" "${mount_dir}${install_dir}/$(basename -- "${path}")"
done

sync
umount -- "${mount_dir}"
mounted=0

info "updated ${#sources[@]} binaries in ${dataset}"
info "reboot into the environment to run them"
