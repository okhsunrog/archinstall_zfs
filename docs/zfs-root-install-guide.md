# Installing a Linux distribution on ZFS root, the `archinstall_zfs` way

This document describes, end to end, the installation procedure that
[`archinstall_zfs`](../README.md) automates: a UEFI-only, ZFS-on-root system whose
boot loader is [ZFSBootMenu](https://zfsbootmenu.org/) and whose dataset layout is
built for **multiple boot environments from day one**.

It is written as a *manual* guide — every automated step is given as the shell
command a human would type — so it doubles as documentation of the installer's
design decisions and as a recipe you can follow by hand.

**Primary target: Arch Linux.** Everything distro-specific is called out, and
[§17](#17-porting-to-other-distributions) maps each Arch-specific step onto Debian/Ubuntu,
Gentoo, Void, Alpine and Fedora. The ZFS layer itself — pool properties, dataset
layout, hostid, `zfs-list.cache`, boot environments — is identical everywhere.

---

## Table of contents

1. [Design decisions at a glance](#1-design-decisions-at-a-glance)
2. [Prerequisites](#2-prerequisites)
3. [Step 0 — ZFS on the live system](#3-step-0--zfs-on-the-live-system)
4. [Step 1 — Partitioning](#4-step-1--partitioning)
5. [Step 2 — The hostid](#5-step-2--the-hostid-do-this-before-creating-the-pool)
6. [Step 3 — Creating the pool](#6-step-3--creating-the-pool)
7. [Step 4 — Dataset layout](#7-step-4--dataset-layout)
8. [Step 5 — Encryption](#8-step-5--encryption)
9. [Step 6 — Export, re-import, mount](#9-step-6--export-re-import-mount)
10. [Step 7 — Base system](#10-step-7--base-system)
11. [Step 8 — ZFS inside the target](#11-step-8--zfs-inside-the-target)
12. [Step 9 — initramfs](#12-step-9--initramfs)
13. [Step 10 — ZFSBootMenu](#13-step-10--zfsbootmenu)
14. [Step 11 — fstab, services, `zfs-list.cache`](#14-step-11--fstab-services-and-zfs-listcache)
15. [Step 12 — Swap, TRIM, snapshots, teardown](#15-step-12--swap-trim-snapshots-and-teardown)
16. [Managing a machine with multiple boot environments](#16-managing-a-machine-with-multiple-boot-environments)
17. [Porting to other distributions](#17-porting-to-other-distributions)
18. [Troubleshooting](#18-troubleshooting)
19. [References](#19-references)

---

## 1. Design decisions at a glance

| Decision | Choice | Why |
|---|---|---|
| Firmware | **UEFI only** | ZFSBootMenu ships as a UEFI executable; BIOS/GRUB-on-ZFS forces `compatibility=grub2` and a second `bpool`. Refusing BIOS removes the whole boot-pool problem. |
| Boot loader | **ZFSBootMenu**, built locally | It is a Linux kernel + initramfs that imports the pool, so it understands native encryption, snapshots and boot environments. Building it on the target guarantees its ZFS module matches the pool's feature flags. |
| `/boot` | **Inside the root dataset**, no separate partition | A boot environment must be *one* filesystem containing its own kernel and initramfs, otherwise snapshot rollback desynchronises kernel and modules. |
| ESP | 500 MiB, mounted at `/boot/efi` | Holds only the ZBM bundle(s). It is deliberately *not* `/boot`. |
| Pool layout | one pool, one vdev, `-O mountpoint=none` | Simple, and every dataset states its own mountpoint. |
| Dataset layout | `pool/<prefix>/root` + `pool/<prefix>/data/*` | `<prefix>` (default `arch0`) is the boot environment. Multiple prefixes coexist in one pool. See [§16](#16-managing-a-machine-with-multiple-boot-environments). |
| Dataset creation | always `zfs create -u` | Never auto-mount at creation time; a single explicit ordered mount pass follows. |
| `overlay` | **`off`** on the BE hierarchy | A ZFS mount over a non-empty directory silently hides files. `overlay=off` turns a latent corruption into a loud mount failure. |
| `canmount` | `noauto` on every `.../root` | Two BEs both claiming `/` must never both mount. |
| hostid | fixed **`0x00bab10c`** via `zgenhostid -f` | Deterministic across live ISO, ZBM and installed system, so `zpool import` never trips the "pool was last used by another system" guard. |
| `cachefile` | **`none`**, `zfs-import-scan.service` | No stale `zpool.cache` to go out of sync with a moved disk. |
| Mount units | `zfs-mount-generator` + a **BE-aware** ZED cache hook | Only the *running* BE's datasets get systemd mount units — otherwise BE #2's `/home` would be mounted while BE #1 is booted. |
| initramfs | **dracut** by default, mkinitcpio supported | dracut's ZFS module works with a systemd-based initramfs; the archzfs mkinitcpio `zfs` hook is busybox/udev-only. |
| initramfs compression | `cat` (none) | The dataset is already `lz4`/`zstd`-compressed; compressing twice costs boot time and saves nothing. |
| Encryption | native ZFS, key file at `/etc/zfs/zroot.key`, mode `000` | Same key material for `zpool create`, `zfs load-key` and the initramfs; ZBM prompts for the passphrase before the kernel loads. |
| TRIM | NVMe → `autotrim=on`; SATA SSD → `zfs-trim-weekly@pool.timer`; HDD → nothing | `fstrim.timer` is a VFS-level tool and is a no-op on ZFS. |
| Swap | **none by default**; zram, plain partition or encrypted partition on request; **never a zvol** | Swapping to a zvol can deadlock the ARC under memory pressure. |
| Device naming | `/dev/disk/by-id/…` (or `by-path`) | `/dev/sdX` reorders between boots and causes sporadic import failures. |

---

## 2. Prerequisites

* A **UEFI** machine (`/sys/firmware/efi` must exist). The installer hard-fails otherwise.
* A live medium that can load the ZFS kernel module. Options:
  * a custom archiso with `zfs-dkms`/`zfs-utils` baked in (see [Arch wiki: Install Arch Linux on ZFS § Create a custom ISO](https://wiki.archlinux.org/title/Install_Arch_Linux_on_ZFS)), or
  * the stock ISO plus an on-the-fly `archzfs` install (§3), or
  * Alpine Linux Extended, which ships ZFS out of the box and is what the OpenZFS handbook recommends.
* Network connectivity.
* Secure Boot **disabled** — out-of-tree ZFS modules will not load otherwise.

> **Licensing note.** ZFS is CDDL, the kernel is GPLv2, so no distribution ships a
> prebuilt in-tree module. Arch uses the unofficial `archzfs` repository, Debian
> uses DKMS from `contrib`, Gentoo builds from source. Redistributing an ISO that
> bundles a linked `zfs.ko` is legally contested — build such images locally, do
> not publish them.

---

## 3. Step 0 — ZFS on the live system

You need `zpool`/`zfs` **and** a loaded `zfs.ko` before you can touch a disk.

```bash
# 1. Stop the mirror-ranking service from fighting you for the pacman DB lock.
systemctl stop reflector.service reflector.timer

# 2. Refresh mirrors if the medium is old (a stale baked-in mirrorlist makes
#    every download crawl).  Skip if the list is fresher than ~24h.
reflector --latest 20 --protocol https --sort rate --save /etc/pacman.d/mirrorlist

# 3. Already have ZFS?  Then skip the rest.
lsmod | grep -q zfs && command -v zpool && exit 0

# 4. Add the archzfs repository.
cat >> /etc/pacman.conf <<'EOF'

[archzfs]
SigLevel = Never
Server = https://github.com/archzfs/archzfs/releases/download/experimental
EOF

# 5. The live root is a tmpfs overlay; give it room for the DKMS build.
mount -o remount,size=50% /run/archiso/cowspace

# 6. Install.  Precompiled first, DKMS as the fallback.
pacman -Sy --noconfirm zfs-utils zfs-linux-lts \
  || pacman -S --noconfirm zfs-dkms zfs-utils linux-lts-headers

modprobe zfs
```

### Kernel choice and the precompiled/DKMS decision

This is the single most common way to end up with an unbootable ZFS system, so the
installer treats it as a first-class validation step.

| Mode | Packages | Constraint |
|---|---|---|
| **Precompiled** (default) | `zfs-utils` + `zfs-<kernel>` (e.g. `zfs-linux-lts`) | The package's embedded kernel version must match the installed kernel *exactly*. `archzfs` versions look like `2.3.3_6.12.41.1-1` — the second half is the kernel. |
| **DKMS** | `zfs-utils` + `zfs-dkms` + `<kernel>-headers` | The kernel must fall inside the `META` file's `Linux-Maximum` range of that OpenZFS release. Builds at install time (slow) but survives kernel bumps. |

`archinstall_zfs` queries both the local ALPM sync databases and, as a fallback,
downloads and parses `archzfs.db` directly, then compares versions before the
install starts (`core/src/kernel/scanner.rs`). It prefers precompiled and falls
back to DKMS. Manually, the equivalent check is:

```bash
pacman -Si linux-lts zfs-linux-lts | grep -E '^(Name|Version)'
```

> **`linux-lts` is strongly recommended.** Mainline and `zen` regularly land kernel
> releases that OpenZFS does not yet support; on those days a `pacman -Syu` leaves
> you with a kernel that has no ZFS module and therefore no root filesystem. The
> LTS series moves slowly enough that archzfs is always ready.

---

## 4. Step 1 — Partitioning

GPT, three partitions at most:

| # | Size | Type | Purpose |
|---|---|---|---|
| 1 | 500 MiB | `ef00` (EFI System) | ESP — ZFSBootMenu bundles only |
| 2 | rest | `bf00` (Solaris Root) | the ZFS vdev |
| 3 | optional, **at the end of the disk** | `8200` (Linux swap) | swap partition, if chosen |

```bash
DISK=/dev/disk/by-id/nvme-Samsung_SSD_990_PRO_2TB_S6Z1NJ0X123456

# Wipe old signatures: first and last 34 sectors, then sgdisk's own zap.
dd if=/dev/zero of="$DISK" bs=512 count=34 conv=notrunc
SECTORS=$(blockdev --getsz "$DISK")
dd if=/dev/zero of="$DISK" bs=512 count=34 seek=$((SECTORS - 34)) conv=notrunc
sgdisk --zap-all "$DISK"

sgdisk -o "$DISK"                                          # fresh GPT
sgdisk -n 1:0:+500M -t 1:ef00 -c 1:EFI  "$DISK"

# With swap: carve it off the tail *first*, then let ZFS take what's left.
sgdisk -n 3:-8G:0  -t 3:8200 -c 3:swap "$DISK"
sgdisk -n 2:0:0    -t 2:bf00 -c 2:ZFS  "$DISK"
# Without swap, just:  sgdisk -n 2:0:0 -t 2:bf00 -c 2:ZFS "$DISK"

partprobe "$DISK"; udevadm settle
mkfs.fat -I -F32 "${DISK}-part1"
```

Notes on the choices:

* **Zeroing both ends by hand.** `sgdisk --zap-all` clears the GPT headers but not
  a leftover ZFS label or LVM/mdadm superblock in the same region; the two `dd`
  passes make the disk unambiguously blank so `zpool create` does not need
  guesswork.
* **Swap at the *end*.** Placing swap last means the ZFS partition can be grown
  later by deleting swap and re-creating partition 2 — the reverse is impossible.
* **Partition path suffix.** `by-id` paths use `-part1`; raw nodes ending in a
  digit (`nvme0n1`, `mmcblk0`) use `p1`; everything else (`sda`) just appends `1`.
  `core/src/disk/partition.rs::partition_path` encodes exactly this rule.
* **Always use `by-id`.** Both the Arch wiki and the OpenZFS handbook are blunt
  about this: creating a pool on `/dev/sdX` leads to sporadic import failures when
  the kernel enumerates disks in a different order. `by-path` is accepted as an
  alternative; `archinstall_zfs` refuses anything that is not a `by-id`/`by-path`
  link or a recognised `/dev/{sd,vd,xvd,nvme,mmcblk}*` node.
* **ESP size.** 500 MiB is generous for two ZBM bundles (~30 MiB each). It is *not*
  `/boot`: kernels and initramfs images live inside the root dataset. This is the
  key structural difference from the OpenZFS/Debian handbook layout, which puts
  `/boot` on a separate `bpool` so GRUB can read it.

---

## 5. Step 2 — The hostid (do this *before* creating the pool)

The hostid identifies the **machine**, not the operating system installed on it.
ZFS stamps the importing host's hostid into the pool label; if it later differs,
`zpool import` reports `pool was previously in use from another system` and refuses
without `-f`, which at boot means an initramfs emergency shell. (The same identifier
backs the `multihost`/MMP property, whose documentation requires that *each host* set
a unique hostid.)

There are three different environments that will import this pool — the live ISO,
ZFSBootMenu, and the installed system — so the installer pins all three to one
constant value:

```bash
zgenhostid -f 0x00bab10c
```

`0x00bab10c` ("bablloc") is the value used by the ZFSBootMenu guides, and picking a
constant rather than the libc-derived `hostid` output means the live medium and the
installed system agree by construction.

Three places must carry it, and all three are handled later in this guide:

1. `/etc/hostid` on the live system — now (`zgenhostid`, above).
2. `/etc/hostid` in the target, and **inside the initramfs** — the initramfs
   mounts root before `/etc` exists, so the file must be baked in
   (`install_items`/`FILES`, §12).
3. The kernel command line of the booted BE, via
   `org.zfsbootmenu:commandline=spl.spl_hostid=0x00bab10c …` (§13).

> ZFSBootMenu additionally has `zbm.set_hostid` (on by default), which passes
> *ZBM's own* hostid to the kernel it boots. If ZBM booted with hostid `00000000`
> it will hand `spl.spl_hostid=00000000` to your kernel and the import fails —
> which is why the ZBM bundle's own command line is also pinned (§13).

Also pre-create the `zfs-list.cache` file and start ZED **now**, so the cache is
populated as datasets are created:

```bash
mkdir -p /etc/zfs/zfs-list.cache
: > /etc/zfs/zfs-list.cache/zroot          # named after the pool
systemctl enable --now zfs-zed.service
```

---

## 6. Step 3 — Creating the pool

```bash
POOL=zroot
VDEV="${DISK}-part2"

zpool create -f \
    -o ashift=12 \
    -O acltype=posixacl \
    -O relatime=on \
    -O xattr=sa \
    -O dnodesize=auto \
    -O normalization=formD \
    -O devices=off \
    -O compression=lz4 \
    -O mountpoint=none \
    -R /mnt \
    "$POOL" "$VDEV"

zpool set cachefile=none "$POOL"
```

Property by property:

| Option | Value | Rationale |
|---|---|---|
| `-o ashift=12` | 4 KiB sectors | Modern drives are 4 Kn or 512e-with-4K-internals. `ashift=9` on such a drive is a permanent, unfixable performance hit; `ashift=12` on a true 512-byte drive costs only a little space. When in doubt, 12. |
| `-O acltype=posixacl` | POSIX ACLs | **Mandatory** — `systemd-journald` fails at boot without ACL support on `/var/log/journal`. |
| `-O xattr=sa` | system-attribute xattrs | Stores xattrs in the dnode instead of a hidden directory; large I/O reduction, and required in practice for `posixacl` to perform sanely. |
| `-O relatime=on` | — | `atime=off` breaks some mailers; `relatime` is the standard compromise. |
| `-O dnodesize=auto` | — | Needed for `xattr=sa` to use larger dnodes efficiently. |
| `-O normalization=formD` | UTF-8 NFD | Filename comparison normalisation; must be set at creation, cannot be changed later. Implies `utf8only=on`. |
| `-O devices=off` | — | Device nodes on the pool cannot be opened. A small hardening win; `/dev` is a devtmpfs anyway. |
| `-O compression=lz4` | or `zstd`, `zstd-5`, `zstd-10` | Free space and, on most workloads, free speed. `lz4` is the safe default; `zstd` trades CPU for ratio. |
| `-O mountpoint=none` | — | The pool root is a container. Each dataset declares its own mountpoint. Prevents accidentally storing data in the pool root, which can never be moved into a BE afterwards. |
| `-R /mnt` | altroot | Every mountpoint is prefixed with `/mnt` for the duration of the install. **Also implies `cachefile=none`** for this import. |
| `-f` | — | The disk was just zapped; force past any residue. |
| `cachefile=none` (after) | — | See below. |

### Why no `autotrim` at creation time

`autotrim` is deliberately *not* in the `zpool create` line. It is set afterwards,
and only for NVMe — see [§15](#trim). Setting it blindly hurts SATA SSDs.

### Why `cachefile=none` instead of `/etc/zfs/zpool.cache`

The classic Arch/OpenZFS recipe writes a `zpool.cache` and enables
`zfs-import-cache.service`. That file is a snapshot of vdev paths; if a disk is
moved to another port or a USB enclosure, the cache is wrong and the pool does not
import. `archinstall_zfs` instead sets `cachefile=none` and enables
`zfs-import-scan.service`, which discovers pools by scanning `/dev` at boot. It is
marginally slower and dramatically more robust — and since the *root* pool is
imported by ZFSBootMenu, not by systemd, the scan service only has to handle any
additional pools.

---

## 7. Step 4 — Dataset layout

```
zroot                          mountpoint=none          (pool root, container)
└── zroot/arch0                mountpoint=none          ← the boot environment
    │                          compression=…, overlay=off
    ├── zroot/arch0/root       mountpoint=/  canmount=noauto     ← THE BE dataset
    ├── zroot/arch0/data       mountpoint=none                    (container)
    │   ├── zroot/arch0/data/home   mountpoint=/home
    │   └── zroot/arch0/data/root   mountpoint=/root
    └── zroot/arch0/vm         mountpoint=/vm
```

```bash
PREFIX=arch0

zfs create -u -o mountpoint=none -o compression=lz4 -o overlay=off  "$POOL/$PREFIX"

zfs create -u -o mountpoint=/     -o canmount=noauto  "$POOL/$PREFIX/root"
zfs create -u -o mountpoint=none                      "$POOL/$PREFIX/data"
zfs create -u -o mountpoint=/home                     "$POOL/$PREFIX/data/home"
zfs create -u -o mountpoint=/root                     "$POOL/$PREFIX/data/root"
zfs create -u -o mountpoint=/vm                       "$POOL/$PREFIX/vm"
```

Creation order matters: datasets are sorted by depth so parents exist first, and
missing intermediate containers (`data`) are auto-created with `mountpoint=none`
(`core/src/dataset_layout.rs`).

**Only `root` is structural — the rest is a suggestion.** What has to be true is that
each boot environment owns exactly one dataset with `mountpoint=/` and
`canmount=noauto`, holding its own `/boot`. Everything else is yours to shape: add
`.../docker` for `/var/lib/docker`, `.../portage/distfiles` for a Gentoo BE, split
`/var/log` out, rename `data/home` to whatever you like, or drop `vm` entirely.
Nothing downstream — the ZED cache hook, ZFSBootMenu, the mount ordering — cares
about the names or the number of data datasets; they all work off the BE root and
the dataset hierarchy beneath it.

### `zfs create -u` — never mount at creation time

> `-u` — *Do not mount the newly created file system.*
> — [`zfs-create(8)`](https://openzfs.github.io/openzfs-docs/man/master/8/zfs-create.8.html)

Without `-u`, `zfs create -o mountpoint=/home …` mounts the dataset **immediately**,
in creation order, which is the wrong order for a root filesystem tree:

* `/mnt/home` would be mounted before `/mnt` (the root dataset) exists, so it would
  land on the live system's tmpfs and then be shadowed the moment root is mounted.
* With encryption, a dataset can be created before its key is loaded; the automatic
  mount then fails mid-run and leaves the tree half-built.
* Mount failures during creation abort the run at an awkward point, with some
  datasets created and some not.

Creating everything unmounted and then doing **one explicit ordered mount pass**
(§9) makes the whole phase restartable and its failures legible. `zfskit` maps
`CreateOptions::no_mount()` onto exactly this flag, so every dataset the installer
creates is `zfs create -u …`.

### `overlay=off` — turn silent data-hiding into a loud error

> `overlay` — *Allow mounting on a busy directory or a directory which already contains files or directories.*
> — [`zfsprops(7)`](https://openzfs.github.io/openzfs-docs/man/master/7/zfsprops.7.html)

OpenZFS defaults to `overlay=on`. That means `zfs mount zroot/arch0/data/home` onto
a `/home` that already has files **succeeds**, and those files vanish from view —
still on disk, still consuming space, invisible and un-backed-up. On a fresh
install this happens whenever a package's `post_install` created content under a
mountpoint before the dataset was mounted.

Setting `overlay=off` on the BE container (inherited by every child) converts that
into:

```
cannot mount '/home': directory is not empty
```

…which the installer treats as fatal and refuses to continue past
(`dataset_layout::mount_datasets_ordered` propagates it). A loud failure you can
fix beats a silent one you discover months later.

### `canmount=noauto` on `.../root`

This is the property that makes multiple boot environments possible. Every BE's
root dataset declares `mountpoint=/`. If they were `canmount=on`, `zfs mount -a`
(and the ZFS import services) would try to mount *all of them* on `/` and the
result is undefined. `noauto` means "only ever mounted explicitly" — by the
initramfs for the BE you actually booted, and by nothing else.

`ZFSBootMenu` discovers BEs precisely by looking for filesystems with
`mountpoint=/` (or `mountpoint=legacy` plus `org.zfsbootmenu:active=on`) that
contain a kernel/initramfs pair in `/boot`.

### Why data datasets sit *inside* the BE prefix

`zroot/arch0/data/home` rather than a pool-level `zroot/data/home`. This is a
deliberate departure from the ZFSBootMenu guides, which put `/home` outside the BE.

* **Inside**: each BE gets its own `/home`, so a rollback of the BE is total and two
  BEs cannot corrupt each other's dotfiles. Cost: `/home` is not shared, and
  `zfs rollback` of the *root* dataset does not roll back `/home` (they are separate
  datasets — see the ZFSBootMenu caveat about single-filesystem environments).
* **Outside** (`zroot/data/home`, sibling of `zroot/arch0`): sharing works, but every
  BE sees it and you must be certain the ZED cache hook classifies it as a *shared*
  dataset rather than another BE's.

The BE-aware ZED hook (§14) supports both: it keeps datasets under the running BE
*plus* datasets that belong to no BE at all. If you want a shared `/home`, create it
at `zroot/data/home` instead and it will be mounted by every BE.

---

## 8. Step 5 — Encryption

ZFS native encryption, `aes-256-gcm`, keyed by a passphrase stored in a file. Two
modes:

| Mode | Encryption root | Effect |
|---|---|---|
| **Pool** | `zroot` | Everything in the pool, including future BEs, is encrypted and inherits the key. |
| **Dataset** | `zroot/<prefix>` | Only this BE is encrypted. Other BEs / shared datasets stay plaintext. |

```bash
# The key file.  Same content is used by zpool create, zfs load-key and the initramfs.
mkdir -p /etc/zfs
printf '%s' 'correct horse battery staple' > /etc/zfs/zroot.key
chmod 000 /etc/zfs/zroot.key
```

`chmod 000` is intentional: root bypasses the mode bits, so ZFS can still read it,
while no other process — and no accidental `cat` from a non-root shell — can.

**Pool-level** — add to the `zpool create` line in §6:

```bash
    -O encryption=aes-256-gcm \
    -O keyformat=passphrase \
    -O keylocation=file:///etc/zfs/zroot.key \
```

**Dataset-level** — add the same three `-o` properties to the
`zfs create -u … "$POOL/$PREFIX"` line in §7 instead.

`keyformat=passphrase` (rather than `raw`/`hex`) is what lets ZFSBootMenu prompt
you interactively at boot: ZBM tries `keylocation` first, and falls back to a
passphrase prompt when the file is not reachable from the pre-boot environment.

### Ordering constraints you must respect

1. The key file must exist **before** `zpool create` / `zfs create`, because
   `keylocation=file://…` is read at that moment.
2. The key must be **loaded** before any encrypted dataset is mounted:
   ```bash
   zfs load-key -L file:///etc/zfs/zroot.key zroot          # pool mode
   zfs load-key -L file:///etc/zfs/zroot.key zroot/arch0    # dataset mode
   ```
   `-L` overrides the stored `keylocation` for this one call — necessary after a
   re-import under an altroot, where the recorded path may not resolve.
3. In *existing pool* mode (adding a BE to an encrypted pool), the **pool key must
   be loaded before creating the new base dataset**, otherwise the child cannot
   inherit the wrapping key.
4. The key file must be copied into the target **before the initramfs is generated**
   (§12) — `archinstall_zfs` runs `prepare_encryption_key()` between the ZFS-package
   phase and the initramfs phase for exactly this reason.

---

## 9. Step 6 — Export, re-import, mount

```bash
sync
zfs umount -a
zpool export "$POOL"

zpool import -N -R /mnt "$POOL"
zfs load-key -L file:///etc/zfs/zroot.key "$POOL"      # if encrypted

zfs mount "$POOL/$PREFIX/root"        # canmount=noauto → must be explicit, and first
zfs mount "$POOL/$PREFIX/vm"          # then children, shallowest first
zfs mount "$POOL/$PREFIX/data/home"
zfs mount "$POOL/$PREFIX/data/root"

mkdir -p /mnt/boot/efi
mount "${DISK}-part1" /mnt/boot/efi
```

The export/import round trip is not ceremony. Both the Arch wiki and the ZFSBootMenu
guides insist on it:

* it proves the pool can be imported cleanly by hostid (the exact operation the
  initramfs will perform at every boot), and
* it clears the "in use by another system" flag, so the first real boot does not
  need `zpool import -f`.

`-N` imports without mounting anything, so the explicit ordered mount below is the
only thing that mounts — root first (because `canmount=noauto`), then children
sorted by depth, so no child is ever mounted before its parent's mountpoint exists.
Combined with `overlay=off`, any ordering mistake fails loudly.

The container datasets (`$POOL/$PREFIX` and `$POOL/$PREFIX/data`) are never mounted:
they carry `mountpoint=none` and exist only to hold properties for their children to
inherit. Only the four datasets above are mounted.

> If the export fails with "pool is busy", the usual culprit is an `arch-chroot`
> that left `/proc`, `/sys` or `/dev` bind-mounted inside the tree. `findmnt -R /mnt`
> shows them.

---

## 10. Step 7 — Base system

```bash
pacstrap -K /mnt \
    base base-devel \
    linux-lts \
    linux-firmware linux-firmware-marvell sof-firmware \
    dracut \
    intel-ucode          # or amd-ucode, per /proc/cpuinfo vendor
```

* **Microcode** is detected from the CPU vendor and installed unconditionally;
  dracut's `early_microcode=yes` (§12) prepends it to the initramfs so it is
  applied before the kernel finishes booting.
* **`dracut`, not `mkinitcpio`**, is installed here when `init_system = dracut`
  (the default). Both are supported and the choice propagates to the ZBM config
  and to the `rootprefix` property.
* **No `pacstrap` in the real installer.** `archinstall_zfs` drives `libalpm`
  directly (`core/src/system/alpm_pacman.rs`) with its own async parallel
  downloader, which is why it has no runtime dependency on `pacman`/`pacstrap`
  binaries. The resulting on-disk state is identical.

Then the usual `arch-chroot` configuration: `hostname`, `locale.gen` +
`locale.conf`, `vconsole.conf`, `/etc/localtime` symlink, `systemd-timesyncd`,
mirrorlist, users, `sudoers`.

---

## 11. Step 8 — ZFS inside the target

The target needs the *same* ZFS userland and a module built for *its* kernel:

```bash
# Add archzfs to /mnt/etc/pacman.conf, init the keyring, import + locally sign keys
arch-chroot /mnt pacman-key --init
arch-chroot /mnt pacman-key --populate archlinux
for KEY in 3A9917BF0DED5C13F69AC68FABEC0A1208037BE9 \
           DDF7DB817396A49B2A2723F7403BD972F75D9D76; do
  arch-chroot /mnt pacman-key --keyserver hkps://keyserver.ubuntu.com -r "$KEY"
  arch-chroot /mnt pacman-key --lsign-key "$KEY"
done

arch-chroot /mnt pacman -Sy --noconfirm zfs-utils zfs-linux-lts
#   …or, for DKMS:   zfs-utils zfs-dkms linux-lts-headers
```

The installer tries several keyservers in turn and downgrades a total failure to a
warning, because the repo block is written with `SigLevel = Never` anyway — the
keys are a best-effort integrity improvement, not a hard requirement.

---

## 12. Step 9 — initramfs

The initramfs is the piece that actually imports the pool and mounts root. It must
contain: the ZFS module, the ZFS userland, `/etc/hostid`, and — if encrypted — the
key file.

### dracut (default)

`/mnt/etc/dracut.conf.d/zfs.conf`:

```ini
hostonly="yes"
hostonly_cmdline="no"
fscks="no"
early_microcode="yes"
# ZFS datasets are already compressed, use uncompressed initramfs
# to avoid double compression
compress="cat"
omit_dracutmodules+=" network btrfs brltty plymouth "
install_items+=" /etc/zfs/zroot.key "     # encrypted installs only
```

* `hostonly=yes` keeps the image small (only this machine's drivers).
* `hostonly_cmdline=no` stops dracut from baking a `root=` into the image — ZBM
  supplies it, and a baked-in one would conflict.
* `compress=cat` — the image lands on an already-compressed dataset; compressing it
  again just adds decompression latency at every boot.
* `fscks=no`, `omit_dracutmodules` — nothing here needs fsck, a network stack, or a
  splash screen.

Generation, inside the chroot — note that it must key off the **installed** kernel,
not `uname -r` (which is the live ISO's kernel):

```bash
arch-chroot /mnt bash -c '
  kver=$(ls -1 /usr/lib/modules | sort | tail -n1)
  pkgbase=$(cat /usr/lib/modules/$kver/pkgbase 2>/dev/null || echo linux)
  install -Dm0644 /usr/lib/modules/$kver/vmlinuz /boot/vmlinuz-$pkgbase
  dracut --force /boot/initramfs-$pkgbase.img --kver $kver
'
```

Arch's `dracut` package ships no pacman hooks, so the installer writes them —
`/etc/pacman.d/hooks/90-dracut-install.hook` and `60-dracut-remove.hook`, driving
`/usr/local/bin/dracut-{install,remove}.sh` — triggered on
`usr/lib/modules/*/pkgbase`. Without these, a kernel upgrade would install new
modules and leave a stale initramfs: an unbootable system on next reboot.

### mkinitcpio (alternative)

`/mnt/etc/mkinitcpio.conf`:

```bash
MODULES=(zfs)
HOOKS=(base udev autodetect microcode modconf kms keyboard keymap consolefont block zfs filesystems)
COMPRESSION="cat"
FILES=(/etc/zfs/zroot.key)      # encrypted installs only
```

Two non-obvious edits, both automated in
`core/src/installer/initramfs/mkinitcpio.rs`:

1. **`zfs` goes immediately before `filesystems`.**
2. **`systemd` → `udev`, `sd-vconsole` → `keymap`.** The `zfs` hook shipped by
   `zfs-utils` is a legacy busybox/udev hook and is **not** compatible with a
   systemd-based initramfs; leaving `systemd` in `HOOKS` produces an image that
   locks out the root filesystem. (The Arch wiki carries the same warning. The AUR
   package `mkinitcpio-sd-zfs` provides a systemd-compatible hook, but it is
   lightly tested, especially with encryption — which is precisely why dracut is
   the default here.)

If you use encryption *and* a non-US keyboard, put `keyboard` before `autodetect`
so the passphrase prompt has your layout.

```bash
arch-chroot /mnt mkinitcpio -P
```

---

## 13. Step 10 — ZFSBootMenu

ZFSBootMenu is not a traditional boot loader. It is a small Linux kernel plus a
ZFS-aware initramfs, packaged as one UEFI executable. The firmware runs it, it
imports your pool, presents the boot environments, and `kexec`s into the one you
choose. Because it is Linux+ZFS, it understands native encryption, snapshots,
`zfs rollback`, and can drop you to a recovery shell with full pool access.

### Build it on the target, not download it

`archinstall_zfs` installs `zfsbootmenu` from the **AUR** into the chroot and runs
`generate-zbm` there, rather than downloading the prebuilt EFI from
`get.zfsbootmenu.org`. The prebuilt image carries whatever OpenZFS version its
release was built against; building locally guarantees ZBM's ZFS matches the pool's
enabled feature flags and the module in your initramfs.

`/mnt/etc/zfsbootmenu/config.yaml`:

```yaml
Global:
  ManageImages: true
  BootMountPoint: /boot/efi
  DracutConfDir: /etc/zfsbootmenu/dracut.conf.d
  InitCPIOConfig: /etc/zfsbootmenu/mkinitcpio.conf
  InitCPIO: false            # true when the target uses mkinitcpio
Components:
  Enabled: false             # no separate kernel+initramfs pair…
EFI:
  ImageDir: /boot/efi/EFI/zbm
  Versions: false            # …one bundle, overwritten in place
  Enabled: true              # …a single bundled UEFI executable
Kernel:
  CommandLine: zbm.import_policy=hostid zbm.timeout=10 ro quiet loglevel=0
```

* `EFI.Enabled: true` + `Components.Enabled: false` → one self-contained
  `vmlinuz.EFI`. The command line is *baked into the executable*, which matters
  because some firmware silently discards the `-u` load-options of an
  `efibootmgr` entry.
* `Versions: false` → `generate-zbm` overwrites in place and renames the previous
  image to `vmlinuz-backup.EFI`, giving you exactly one known-good fallback rather
  than an ESP slowly filling with versioned images.
* `zbm.import_policy=hostid` → if the pool's recorded hostid does not match, ZBM
  adopts it rather than refusing. This is the forgiving-but-safe middle ground
  between `strict` and `force`.
* `zbm.timeout=10` → show the menu with a 10-second countdown, then boot `bootfs`.
  **This only works if the pool's `bootfs` property is set** — without it ZBM waits
  for input forever.

Build and install the bundle:

```bash
arch-chroot /mnt generate-zbm
install -Dm0644 /mnt/boot/efi/EFI/zbm/vmlinuz.EFI \
                /mnt/boot/efi/EFI/BOOT/BOOTX64.EFI
```

The copy to `EFI/BOOT/BOOTX64.EFI` is the removable-media fallback path. If NVRAM
is cleared, the board is replaced, or the disk is moved to another machine, the
firmware still finds a boot loader.

A pacman hook keeps it fresh — `/mnt/etc/pacman.d/hooks/95-zfsbootmenu.hook`:

```ini
[Trigger]
Type = Path
Operation = Install
Operation = Upgrade
Target = usr/lib/modules/*/pkgbase
Target = usr/lib/modules/*/extramodules/zfs.ko*

[Trigger]
Type = Package
Operation = Install
Operation = Upgrade
Target = zfsbootmenu
Target = zfs-utils

[Action]
Description = Regenerating ZFSBootMenu...
When = PostTransaction
Exec = /usr/bin/generate-zbm
Depends = zfsbootmenu
```

Two triggers, because there are two independent reasons the bundle goes stale. The
path trigger fires when the kernel or the ZFS module changes (`pkgbase`, `zfs.ko*`).
The package trigger covers a new `zfsbootmenu` or `zfs-utils` release, which changes
neither of those paths — without it the ESP would keep an old ZBM until some
unrelated kernel update happened to rebuild it.

### Firmware boot entries

```bash
efibootmgr -c -d "$DISK" -p 1 -L "ZFSBootMenu"          -l '\EFI\zbm\vmlinuz.EFI'
efibootmgr -c -d "$DISK" -p 1 -L "ZFSBootMenu (Backup)" -l '\EFI\zbm\vmlinuz-backup.EFI'
```

No `-u` load options: the command line is already embedded in the bundle.

### The ZFS properties that drive ZBM

```bash
zfs set org.zfsbootmenu:commandline='spl.spl_hostid=0x00bab10c zswap.enabled=0 rw' \
        "$POOL/$PREFIX/root"
zfs set org.zfsbootmenu:rootprefix='root=ZFS='  "$POOL/$PREFIX/root"   # dracut
#                                    'zfs='                            # mkinitcpio
zpool set bootfs="$POOL/$PREFIX/root" "$POOL"
```

| Property | Value | Notes |
|---|---|---|
| `org.zfsbootmenu:commandline` | `spl.spl_hostid=… zswap.enabled=… rw` | **Must not contain `root=`.** ZBM composes `root=` itself from `rootprefix` + the selected dataset; a second `root=` breaks the boot. |
| `org.zfsbootmenu:rootprefix` | `root=ZFS=` / `zfs=` | dracut's ZFS module parses `root=ZFS=pool/ds`; the mkinitcpio `zfs` hook parses `zfs=pool/ds`. Getting this wrong drops you into an emergency shell. ZBM's own default is per-distro-guess (`zfs=` for Arch), so setting it explicitly removes the guesswork. |
| `bootfs` (pool property) | `pool/prefix/root` | Which BE auto-boots after `zbm.timeout`. |
| `org.zfsbootmenu:keysource` | a dataset | Optional: tells ZBM which filesystem to look in for key files before prompting. Useful when each BE carries its own copy of the key. |

`rw` is on the command line because the dracut/mkinitcpio ZFS hooks mount root
read-only unless told otherwise, and `zbm` itself is told `ro` — the two are
independent.

---

## 14. Step 11 — fstab, services and `zfs-list.cache`

### fstab

ZFS mounts its own datasets; they must **not** be in `fstab` as well, or systemd and
ZFS race each other. But the ESP does need an entry, and the root dataset is listed
explicitly for tooling that reads `fstab` to answer "what is `/`?".

```bash
genfstab -U /mnt | grep -vE 'zfs|zroot' > /mnt/etc/fstab
cat >> /mnt/etc/fstab <<EOF

# ZFS root dataset
$POOL/$PREFIX/root	/	zfs	defaults	0	0
EOF
```

The ESP line gets two edits (`core/src/installer/fstab.rs`):

* **`nofail`** — a missing or corrupt ESP must not block boot into a system that is
  already running from ZFS.
* **passno `0`** — there is no `fsck.vfat` worth running at boot, and a non-zero
  passno makes systemd generate an fsck dependency that can fail the mount.

### ZFS services

```bash
for unit in zfs.target zfs-import.target zfs-volumes.target \
            zfs-import-scan.service zfs-zed.service; do
  systemctl --root=/mnt enable "$unit"
done
```

Note what is **absent**: `zfs-import-cache.service` (we chose `cachefile=none`) and
`zfs-mount.service` (mounting is handled by `zfs-mount-generator`).

### `zfs-list.cache` and the boot-environment-aware ZED hook

`zfs-mount-generator(8)` is a systemd generator that runs *before* the pool is
imported. It cannot ask ZFS what datasets exist, so it reads a plain-text cache at
`/etc/zfs/zfs-list.cache/<pool>` — essentially `zfs list -Ho name,mountpoint,canmount,…`
— and emits real `.mount` units from it. This is what gets `/var/log` and `/home`
ordered correctly relative to the rest of the boot, rather than bulk-mounted after
the fact by `zfs-mount.service`.

The cache is maintained by ZED's `history_event-zfs-list-cacher.sh` zedlet, which
rewrites it on every ZFS history event.

**The stock zedlet is wrong for a multi-BE machine.** It dumps *every* dataset in
the pool, so a system booted into `arch0` would get mount units for `arch1`'s
`/home` and `/root` too — two BEs' data fighting over the same mountpoints.

#### The replacement hook

`archinstall_zfs` ships its own zedlet
([`assets/history_event-zfs-list-cacher.sh`](../assets/history_event-zfs-list-cacher.sh)).
Despite the `.sh` name — ZED requires that suffix to recognise a zedlet — it is a
**Python 3 script**; ZED simply `exec`s it, so the target must have a `python3` in
`PATH` (it does, `base` pulls it in transitively, but a minimal non-Arch target may
not). Its logic, in order:

1. **Gate on the event.** Exit unless `ZEVENT_SUBCLASS == history_event` and
   `ZEVENT_POOL` is set. Then exit unless `/etc/zfs/zfs-list.cache/<pool>` exists
   *and is writable* — this is why §5 pre-creates that file before the pool is
   built. No file, no cache updates, silently.
2. **Serialise.** Take an exclusive `flock(2)` on `/run/zfs-list.cache@<pool>.lock`,
   so two ZED invocations racing on the same pool cannot interleave. The lock is on
   a dedicated file rather than the cache itself, because the cache is replaced by
   `rename(2)` in step 7 and a lock held on it would end up pinning an unlinked
   inode.
3. **Find the running BE.** Locate the ZFS dataset mounted on `/`, trying
   `/proc/mounts`, then `mount`, then `zfs mount` in turn (three fallbacks because
   the hook also runs in odd contexts — early boot, chroots — where any one of them
   can be unavailable). Bail out if none matches. The BE is the *parent* of that
   dataset: `zroot/arch0/root` → `zroot/arch0`.
4. **Enumerate everything.** `zfs list -H -t filesystem -r -o <20 properties> <pool>`
   — `name`, `mountpoint`, `canmount`, `atime`, `relatime`, `devices`, `exec`,
   `readonly`, `setuid`, `nbmand`, `encroot`, `keylocation`, and the eight
   `org.openzfs.systemd:*` properties `zfs-mount-generator(8)` understands. Volumes
   are excluded (`-t filesystem`), which is correct — a zvol has no mount unit.
   `encroot` and `keylocation` are what let the generator emit key-load
   dependencies for encrypted datasets. If `zfs list` fails, returns nothing, or
   cannot be run at all, the hook stops here and leaves the existing cache alone —
   overwriting it with nothing would strip every mount unit on the next boot.
5. **Identify all BEs.** A second, small `zfs list -o name,mountpoint,org.zfsbootmenu:active`
   — separate because the cache's column layout is fixed by `zfs-mount-generator`
   and cannot simply gain a column. A filesystem counts as a boot environment when
   its `mountpoint` is `/`, or when it is `legacy` with `org.zfsbootmenu:active=on`;
   its parent goes into the BE set. On the layout in §7 that yields
   `{zroot/arch0, zroot/arch1, …}`. Environments hidden from ZBM's menu with
   `org.zfsbootmenu:active=off` still count here — hiding a BE does not make it safe
   to mount its `/home` underneath a different one.
6. **Filter.** A dataset is written to the cache if **any** of:
   - it is the running BE or sits below it (`zroot/arch0/…` — the BE's own hierarchy), **or**
   - it has no `/` in its name (the pool root itself), **or**
   - it sits below no BE in the set (a **shared** dataset such as a pool-level
     `zroot/data/home`, which every BE should mount).

   Datasets belonging to some *other* BE match none of the three and are dropped.
   That is the whole point. Membership is compared on dataset-name component
   boundaries, so `zroot/arch10` is not mistaken for part of `zroot/arch1`.
7. **Write, only on change.** Compare the rendered rows against the current cache
   and stop if they are identical — ZFS history events are frequent and most change
   nothing. Otherwise write a temporary file *in the cache directory*, `fsync` it,
   and `rename(2)` it over the cache. The temp file shares the directory because
   `rename` is only atomic within one filesystem, and `/run` is not the same one as
   `/etc`. Readers therefore see either the old cache or the new one, never a
   half-written file.

It is installed at `/etc/zfs/zed.d/history_event-zfs-list-cacher.sh`, mode `0755`,
and marked **immutable** (`chattr +i`) so a `zfs-utils` upgrade cannot silently
restore the stock version and reintroduce cross-BE mounts. The installer clears the
flag with `chattr -i` before overwriting, so re-running it is idempotent.

Do not place a boot environment directly under the pool root (a `zroot/root` with
`mountpoint=/`): the BE would then be the pool itself, every dataset would count as
part of it, and nothing could ever be classified as shared. The `pool/<prefix>/root`
layout of §7 avoids this by construction.

The hook is quiet by default. To trace what it decides — the running BE it detected,
the BE set, how many datasets it kept — set `DEBUG = True` at the top of the script
and read `/tmp/zed_debug.log`.

Finally, the cache generated during installation contains altroot-prefixed paths
(`/mnt/home`), so it is rewritten on the way into the target — `/mnt` → `/`,
`/mnt/home` → `/home`. The OpenZFS handbook does the same with
`sed -Ei "s|/mnt/?|/|" /etc/zfs/zfs-list.cache/*`.

```bash
cp /etc/hostid /mnt/etc/hostid
sed -E 's|^([^\t]*)\t/mnt(/?)|\1\t/|' /etc/zfs/zfs-list.cache/$POOL \
    > /mnt/etc/zfs/zfs-list.cache/$POOL
```

---

## 15. Step 12 — Swap, TRIM, snapshots and teardown

### Swap

Three supported shapes, and one that is deliberately unsupported. The default is
**no swap at all** (`SwapMode::None`) — you opt in to one of the following.

**zram.** No disk involvement, no interaction with ZFS at all:

```ini
# /mnt/etc/systemd/zram-generator.conf
[zram0]
zram-size = min(ram / 2, 4096)
compression-algorithm = zstd
```

**Swap partition.**

```bash
mkswap "${DISK}-part3"
printf '\n# Swap\n%s\tnone\tswap\tdefaults\t0\t0\n' "${DISK}-part3" >> /mnt/etc/fstab
```

**Encrypted swap partition** — random key every boot, so hibernation is impossible
but no key management is needed:

```bash
# /mnt/etc/crypttab
cryptswap	/dev/disk/by-id/…-part3	/dev/urandom	swap,cipher=aes-xts-plain64,size=256
# /mnt/etc/fstab
/dev/mapper/cryptswap	none	swap	defaults	0	0
```

When a swap partition is in use, the BE command line gets `zswap.enabled=1`
(compressed swap cache in front of the disk); with zram or no swap it is
`zswap.enabled=0`, because zswap in front of zram is pure overhead.

> **Never swap to a zvol.** The OpenZFS handbook is explicit: *"On systems with
> extremely high memory pressure, using a zvol for swap can result in lockup,
> regardless of how much swap is still available."* Writing out swap requires ZFS
> to allocate memory, which is exactly what is unavailable. `archinstall_zfs` does
> not offer the option.

<a id="trim"></a>
### TRIM

ZFS has two TRIM mechanisms and the correct one depends on the hardware:

| Storage | Strategy | Command |
|---|---|---|
| NVMe | continuous | `zpool set autotrim=on "$POOL"` |
| SATA/SAS SSD | periodic | `systemctl --root=/mnt enable zfs-trim-weekly@$POOL.timer` |
| HDD | none | — |

`autotrim=on` issues TRIM as blocks are freed. NVMe's deep command queues absorb
this for free. On SATA, TRIM commands *block* the bus, so continuous TRIM produces
latency spikes under load on consumer drives — hence the weekly one-shot
`zpool trim` instead.

**`fstrim.timer` is never enabled.** `fstrim` is a VFS-level tool with no knowledge
of ZFS's block allocator; on a ZFS-only system it silently does nothing on every
run. Detection is by device path: `nvme*` → NVMe, otherwise
`/sys/class/block/<dev>/queue/rotational`.

TRIM is configured **only when the installer created the pool** (full-disk and
new-pool modes). Adding a boot environment to an *existing* pool leaves `autotrim`
and any trim timer exactly as they were — the pool's owner already made that call,
and silently changing a property on a pool you did not create would be rude.

### An initial snapshot

Cheap, and the thing you will want the first time an update goes wrong:

```bash
zfs snapshot -r "$POOL/$PREFIX@fresh-install"
```

### Teardown

```bash
sync
umount /mnt/boot/efi

zfs umount -a           || true
zfs umount "$POOL/$PREFIX/root" || true
zfs umount -af          || true
zfs umount -f "$POOL/$PREFIX/root" || true

zpool export "$POOL" || zpool export -f "$POOL"
```

The escalation ladder (bulk → explicit → bulk+force → explicit+force, each followed
by a `sync(2)`) exists because a chroot or a lingering process routinely holds one
mount just long enough to make the first attempt fail.

> **Do not skip the export.** An un-exported pool still carries the live system's
> ownership stamp; the initramfs will refuse to import it and you will meet the
> emergency shell on the very first boot.

---

## 16. Managing a machine with multiple boot environments

This is the payoff of the whole layout. A single pool holds several complete,
independently bootable systems:

```
zroot
├── zroot/arch0/root      ← Arch, stable, mountpoint=/  canmount=noauto
├── zroot/arch1/root      ← Arch, testing a new kernel
├── zroot/void0/root      ← a different distro entirely
└── zroot/data/…          ← optional pool-level shared data, belongs to no BE
```

ZFSBootMenu lists all of them at boot. You pick one; the others stay untouched and
unmounted.

### The rules that make it safe

1. **Exactly one dataset per BE is the BE.** `mountpoint=/`, `canmount=noauto`,
   containing its own `/boot/vmlinuz-*` and `/boot/initramfs-*`. ZFSBootMenu is
   explicit that a boot environment should be *one* filesystem, so that a snapshot
   of it is a consistent view of system state.
2. **`canmount=noauto` is not optional.** Two datasets with `mountpoint=/` and
   `canmount=on` will both be mounted by `zfs mount -a`, non-deterministically.
3. **`overlay=off` everywhere in the BE hierarchy.** It is the guard rail that
   catches "I mounted BE #2's `/home` on top of BE #1's populated `/home`".
4. **The ZED cache hook must be BE-aware** (§14) — otherwise the running BE gets
   systemd mount units for other BEs' datasets.
5. **All BEs on a machine share one hostid value — but each needs its own copy of
   the file.** The hostid identifies the *machine*, not the OS instance: it is what
   `zpool import` compares against the pool label, and a mismatch means
   `pool was previously in use from another system` and an import that needs `-f`.
   Since your BEs are the same physical machine and never run at once, they must all
   report `0x00bab10c`. What is per-BE is the *copy*: every BE needs `/etc/hostid`
   present and baked into its own initramfs, because root is mounted before `/etc`
   exists. `gen_iso/deploy-zfs-be.sh` does exactly that — it copies the host's
   `/etc/hostid` into the new BE and rebuilds that BE's initramfs.
6. **Leave `bootfs` alone unless you mean it.** `bootfs` selects the *default* BE
   after the ZBM countdown. Creating a new BE should not change it — you want the
   known-good one to remain the automatic choice until the new one has proven itself.

### Creating a new boot environment from scratch

Same shape as §7, different prefix. Run this from a live medium, or from a running
BE (it does not touch the running one):

```bash
POOL=zroot ; NEW=arch1

# Encrypted pool?  Load the key first, so the child inherits it.
zfs load-key -L file:///etc/zfs/zroot.key "$POOL" 2>/dev/null || true

zfs create -u -o mountpoint=none -o canmount=off -o overlay=off  "$POOL/$NEW"
zfs create -u -o mountpoint=/ -o canmount=noauto -o overlay=off \
           -o org.zfsbootmenu:commandline='spl.spl_hostid=0x00bab10c zswap.enabled=0 rw' \
           -o org.zfsbootmenu:rootprefix='root=ZFS=' \
           "$POOL/$NEW/root"

mkdir -p /mnt/newbe
mount -t zfs -o zfsutil "$POOL/$NEW/root" /mnt/newbe
# …pacstrap / debootstrap / rsync a rootfs in, chroot, build the initramfs…
umount /mnt/newbe
```

`-o zfsutil` on `mount -t zfs` is what tells the ZFS mount helper to honour the
dataset's properties (rather than treating it as a legacy mount).

Note the **`zfs create -u`** again: without it, `-o mountpoint=/` would make ZFS try
to mount the new BE **on top of your running root**, right now. That is the single
most dangerous thing you can do on a multi-BE machine, and `-u` is what prevents it.

### Cloning an existing BE (the usual workflow)

Far more common than building from scratch — this is how you take a safe snapshot
before a risky upgrade:

```bash
POOL=zroot ; SRC=arch0 ; NEW=arch1

zfs snapshot -r "$POOL/$SRC@pre-upgrade"

zfs create -u -o mountpoint=none -o canmount=off -o overlay=off "$POOL/$NEW"
zfs clone -o mountpoint=/ -o canmount=noauto \
          "$POOL/$SRC/root@pre-upgrade" "$POOL/$NEW/root"

# The clone inherits the source's ZBM properties; adjust if needed.
zfs set org.zfsbootmenu:commandline='spl.spl_hostid=0x00bab10c zswap.enabled=0 rw' \
        "$POOL/$NEW/root"
```

Reboot, pick `arch1` in ZFSBootMenu, upgrade there. If it works, promote it and make
it the default; if not, reboot into `arch0` and destroy it.

```bash
zfs promote "$POOL/$NEW/root"          # break the clone dependency on arch0
zpool set bootfs="$POOL/$NEW/root" "$POOL"
```

ZFSBootMenu can also do the snapshot-clone-and-boot dance interactively from its
own menu, without a running system — which is the whole point of it being a
ZFS-aware pre-boot environment.

### `zfs set -u` — editing another BE's mountpoint without mounting it

> `-u` — *Update mountpoint, sharenfs, sharesmb property but do not mount or share the dataset.*
> — [`zfs-set(8)`](https://openzfs.github.io/openzfs-docs/man/master/8/zfs-set.8.html)

Normally, changing `mountpoint` makes ZFS **unmount the dataset and remount it at
the new location immediately**. On a multi-BE machine that behaviour is a foot-gun:

```bash
# DANGEROUS on a running system: ZFS acts on this the moment you press Enter.
zfs set mountpoint=/mnt/foo zroot/arch1/root
```

If `zroot/arch1/root` happened to be mounted, ZFS moves it. If you then set it back
to `/` and its `canmount` is not `noauto`, ZFS will try to mount it **over your
running root**.

`-u` decouples the property change from the mount action:

```bash
# Safe: change the property, mount state is untouched.
zfs set -u mountpoint=/mnt/foo zroot/arch1/root

# Now mount it deliberately, where and when you want:
mkdir -p /mnt/foo
zfs mount zroot/arch1/root

# …inspect / repair / rsync …

zfs umount zroot/arch1/root
zfs set -u mountpoint=/ zroot/arch1/root      # restore, still no mount attempt
```

Use `zfs set -u` whenever you:

* temporarily relocate another BE's datasets to inspect or repair them from the
  running system;
* restore `mountpoint=/` afterwards — **this is the important one**, because
  without `-u` you are asking ZFS to mount that dataset on `/` right now;
* fix up altroot-prefixed mountpoints left behind by an interrupted install;
* script BE manipulation, where the mount step should be explicit and checked
  rather than an implicit side effect of a property write.

The same reasoning applies to `zfs create -u` and to `zpool import -N` — this
codebase never lets ZFS decide *when* to mount something. Property changes and
mount operations are always separate, explicit, ordered steps.

### Inspecting a BE from a live medium

```bash
zpool import -N -R /mnt zroot
zfs load-key -L file:///etc/zfs/zroot.key zroot      # if encrypted
zfs mount zroot/arch1/root
zfs mount zroot/arch1/data/home
mount /dev/disk/by-id/…-part1 /mnt/boot/efi
# …
umount /mnt/boot/efi
zfs umount -a
zpool export zroot
```

Note the `-R /mnt` altroot: it prefixes every mountpoint for this import only,
without touching the stored `mountpoint` properties. It is the right tool when you
just want to *look*; `zfs set -u` is the right tool when you need the change to
persist.

For read-only forensics, `zpool import -N -o readonly=on -o cachefile=none` cannot
modify the pool at all — this is what the installer's demo mode uses.

### Inspecting another BE from a *running* system

Same idea, but the pool is already imported, so `-R` is not available and the
mountpoints have to be moved with `zfs set -u`:

```bash
# Retarget. -u keeps every one of these a pure property write.
zfs set -u mountpoint=/mnt/other        zroot/arch1/root
zfs set -u mountpoint=/mnt/other/home   zroot/arch1/data/home

# Mount explicitly, root first.
zfs mount zroot/arch1/root
zfs mount zroot/arch1/data/home
# … inspect …
zfs unmount zroot/arch1/data/home
zfs unmount zroot/arch1/root

# Restore. -u matters *most* here: data/home is canmount=on, so a plain
# `zfs set mountpoint=/home` would mount it straight over the running /home.
zfs set -u mountpoint=/home  zroot/arch1/data/home
zfs set -u mountpoint=/      zroot/arch1/root
```

Two things to expect while the BE is retargeted:

* **It stops looking like a boot environment.** Nothing under it has `mountpoint=/`
  any more, so the ZED hook reclassifies its datasets as *shared* and writes them
  into the running BE's `zfs-list.cache`. Restoring `mountpoint=/` and letting one
  more ZFS event fire puts the cache right again — verify with
  `cut -f1 /etc/zfs/zfs-list.cache/<pool>` before rebooting.
* **`overlay=off` is your seatbelt.** If a restore step is fumbled and something
  tries to mount the other BE's root on the live `/`, ZFS refuses with
  `cannot mount '/': directory is not empty` instead of shadowing the running
  system.

### Destroying a boot environment

```bash
# Confirm it is not the one you are running, and not the default.
findmnt -n -o SOURCE /
zpool get bootfs zroot

zfs destroy -r zroot/arch1
```

If it was a clone whose origin snapshot still lives in another BE, `zfs destroy`
will tell you; either `zfs promote` the survivor first, or destroy the origin
snapshot afterwards.

### A safety checklist before rebooting into a new BE

```bash
BE=zroot/arch1/root
zfs get -o property,value mountpoint,canmount,overlay,encryption,keystatus "$BE"
zfs get -o property,value org.zfsbootmenu:commandline,org.zfsbootmenu:rootprefix "$BE"
zpool get bootfs zroot
ls -l /mnt/be/boot/vmlinuz-* /mnt/be/boot/initramfs-*     # kernel + initramfs present
od -A n -t x1 /mnt/be/etc/hostid                          # 0c b1 ba 00  (LE 00bab10c)
lsinitcpio /mnt/be/boot/initramfs-linux-lts.img | grep -E '/zfs\.ko'
```

Expect: `mountpoint=/`, `canmount=noauto`, `overlay=off`, a command line **without**
`root=`, a `rootprefix` matching the initramfs generator, a kernel/initramfs pair,
the right hostid, and `zfs.ko` inside the image.

---

## 17. Porting to other distributions

The ZFS layer — §§4–9, §14, §16 — is distro-independent. Only these change:

| Step | Arch | Debian / Ubuntu | Gentoo | Void | Alpine | Fedora |
|---|---|---|---|---|---|---|
| ZFS packages | `archzfs` repo: `zfs-utils` + `zfs-<kernel>` or `zfs-dkms` | `contrib`: `zfsutils-linux`, `zfs-dkms` (backports recommended) | `sys-fs/zfs`, `sys-fs/zfs-kmod` | `zfs` (main repo) | `zfs`, `zfs-lts` | `zfs-fedora` repo, DKMS/kABI |
| Bootstrap into target | `pacstrap -K /mnt` | `debootstrap` | `stage3` tarball + `emerge` | `xbps-install -SR … -r /mnt` | `apk --root /mnt` | `dnf --installroot` |
| initramfs | dracut *or* mkinitcpio | `initramfs-tools` *or* dracut | dracut *or* genkernel | dracut | `mkinitfs` | dracut |
| `rootprefix` | `zfs=` (mkinitcpio) / `root=ZFS=` (dracut) | `root=ZFS=` | `root=ZFS=` | `root=zfs:` | `root=ZFS=` | `root=zfs:` |
| Extra datasets flagged to initramfs | — | `ZFS_INITRD_ADDITIONAL_DATASETS` in `/etc/default/zfs` | — | — | — | — |
| fstab generator | `genfstab -U /mnt` | hand-written / `blkid` | hand-written | hand-written | hand-written | hand-written |
| Service manager | systemd | systemd | systemd *or* OpenRC (`rc-update add zfs-import boot`) | runit (`ln -s /etc/sv/zed /var/service/`) | OpenRC | systemd |
| Post-upgrade hook for ZBM + initramfs | pacman hooks | `dpkg` triggers / `kernel-postinst.d` | portage hooks | `xbps` `kernel.d` hooks | `apk` triggers | `kernel-install` |

Constant everywhere:

* `zgenhostid -f`, and `/etc/hostid` inside the initramfs;
* `canmount=noauto` + `mountpoint=/` per boot environment;
* `overlay=off` on the BE hierarchy;
* `zfs create -u` / `zfs set -u` to keep mounting explicit;
* `zpool import -N -R /mnt` during installation and the export/import round trip;
* `/etc/zfs/zfs-list.cache/<pool>` + a BE-aware ZED zedlet;
* the ZFSBootMenu properties, `bootfs`, and the `EFI/BOOT/BOOTX64.EFI` fallback.

The one genuinely distro-shaped decision is **`rootprefix`**, because it depends on
which initramfs generator parses the command line, not on the distro name. Get it
from the initramfs you actually built, not from the distro's reputation.

---

## 18. Troubleshooting

See [`docs/debugging-boot.md`](debugging-boot.md) for the full QEMU-based workflow.
The common failures, in the order you meet them:

| Symptom | Cause | Fix |
|---|---|---|
| ZBM lists BEs but never auto-boots | `bootfs` unset, or `zbm.timeout` missing from ZBM's own command line | `zpool set bootfs=pool/prefix/root pool` |
| ZBM header shows `spl_hostid=00000000` | ZBM imported with its default hostid | Ensure `/etc/hostid` is in ZBM's initramfs; pin `spl_hostid=0x00bab10c` on ZBM's command line |
| Kernel drops to dracut emergency shell | hostid mismatch, or `root=` duplicated, or ZFS missing from the initramfs, or the initramfs was built for the *live* kernel | Check `zfs get org.zfsbootmenu:commandline` contains **no** `root=`; check `rootprefix`; `lsinitcpio`/`lsinitrd` for `zfs.ko`; rebuild with `--kver` from `/usr/lib/modules` |
| `cannot import 'pool': no such pool available` in the initramfs | stale `zpool.cache`, or hostid mismatch | `zpool set cachefile=none pool`, remove `/etc/zfs/zpool.cache`, regenerate the initramfs |
| `cannot mount '/home': directory is not empty` | `overlay=off` doing its job | Something wrote into the mountpoint before the dataset was mounted. Inspect and clear it — do **not** set `overlay=on` |
| `zpool export` says the pool is busy | chroot left `/proc`, `/sys`, `/dev` bind-mounted | `findmnt -R /mnt`, then `umount` them |
| Another BE's `/home` is mounted | stock ZED zedlet restored by a package upgrade | Inspect `/etc/zfs/zfs-list.cache/<pool>`; reinstall the BE-aware `history_event-zfs-list-cacher.sh` and `chattr +i` it (set `DEBUG = True` in it to trace its decisions) |
| Boots to a login prompt but the network is dead | ISO network configs matched interface names that differ on the installed kernel | Use `Name=en*` wildcards in `/etc/systemd/network/*.network` |

---

## 19. References

**Arch Linux**
- [Install Arch Linux on ZFS](https://wiki.archlinux.org/title/Install_Arch_Linux_on_ZFS) — pool options, mkinitcpio `systemd`→`udev` warning, hostid, custom ISO
- [ZFS](https://wiki.archlinux.org/title/ZFS) — general administration, `zfs-mount-generator`, swap
- [Installation guide](https://wiki.archlinux.org/title/Installation_guide), [dracut](https://wiki.archlinux.org/title/Dracut), [mkinitcpio](https://wiki.archlinux.org/title/Mkinitcpio)
- [Persistent block device naming](https://wiki.archlinux.org/title/Persistent_block_device_naming), [EFI system partition](https://wiki.archlinux.org/title/EFI_system_partition)
- [Unofficial user repositories § ArchZFS](https://wiki.archlinux.org/title/Unofficial_user_repositories#archzfs)

**OpenZFS**
- [Arch Linux Root on ZFS](https://openzfs.github.io/openzfs-docs/Getting%20Started/Arch%20Linux/Root%20on%20ZFS.html) — note its explicit warning that its layout is *not* ZBM-compatible
- [Debian Bookworm Root on ZFS](https://openzfs.github.io/openzfs-docs/Getting%20Started/Debian/Debian%20Bookworm%20Root%20on%20ZFS.html) — the canonical `bpool`/`rpool`, `zfs-list.cache`, altroot-`sed` recipe
- [`zfsprops(7)`](https://openzfs.github.io/openzfs-docs/man/master/7/zfsprops.7.html) — `overlay`, `canmount`, `mountpoint`, `xattr`, `acltype`, `dnodesize`, `normalization`, encryption properties
- [`zfs-create(8)`](https://openzfs.github.io/openzfs-docs/man/master/8/zfs-create.8.html) — the `-u` flag
- [`zfs-set(8)`](https://openzfs.github.io/openzfs-docs/man/master/8/zfs-set.8.html) — the `-u` flag
- [`zfs-mount-generator(8)`](https://openzfs.github.io/openzfs-docs/man/master/8/zfs-mount-generator.8.html) — `zfs-list.cache`, `org.openzfs.systemd:*`
- [`zpool-trim(8)`](https://openzfs.github.io/openzfs-docs/man/master/8/zpool-trim.8.html)
- [Hardware / NVMe low-level formatting](https://openzfs.github.io/openzfs-docs/Performance%20and%20Tuning/Hardware.html)

**ZFSBootMenu**
- [Overview](https://zfsbootmenu.org/)
- [Boot environments and you](https://docs.zfsbootmenu.org/en/latest/general/bootenvs-and-you.html)
- [UEFI booting](https://docs.zfsbootmenu.org/en/latest/general/uefi-booting.html)
- [`zfsbootmenu(7)`](https://docs.zfsbootmenu.org/en/latest/man/zfsbootmenu.7.html) — every `org.zfsbootmenu:*` property and `zbm.*` parameter
- [Void Linux (UEFI) guide](https://docs.zfsbootmenu.org/en/latest/guides/void-linux/uefi.html) — the reference procedure most of this document's boot half descends from
- [Ubuntu (UEFI)](https://docs.zfsbootmenu.org/en/latest/guides/ubuntu/uefi.html), [Alpine (UEFI)](https://docs.zfsbootmenu.org/en/latest/guides/alpine/uefi.html), [Fedora (UEFI)](https://docs.zfsbootmenu.org/en/v3.0.x/guides/fedora/uefi.html)

**Other distributions**
- [Gentoo wiki: ZFS](https://wiki.gentoo.org/wiki/ZFS), [ZFS/ZFSBootMenu](https://wiki.gentoo.org/wiki/ZFS/ZFSBootMenu), [rootfs on ZFS](https://wiki.gentoo.org/wiki/ZFS/rootfs)
- [Debian wiki: ZFS](https://wiki.debian.org/ZFS) — DKMS from `contrib`, backports, CDDL/GPL background

**This repository**
- `core/src/prepare.rs` — pool + dataset creation, encryption modes, `overlay=off`
- `core/src/dataset_layout.rs` — the layout, `zfs create -u`, ordered mounting
- `core/src/bootmenu.rs` — ZBM config, properties, `generate-zbm`, EFI entries
- `core/src/zfs_target_files.rs` — hostid, `zfs-list.cache` rewriting, the ZED hook
- `core/src/installer/initramfs/` — dracut and mkinitcpio configuration
- `assets/history_event-zfs-list-cacher.sh` — the BE-aware ZED zedlet
- `gen_iso/deploy-zfs-be.sh` — a worked example of deploying an additional BE
- `docs/debugging-boot.md` — QEMU boot debugging
