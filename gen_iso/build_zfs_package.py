#!/usr/bin/env python3
# Deprecated: local precompiled ZFS packaging flow removed.
# This file is intentionally minimal to prevent accidental use.
# Use DKMS flow and public ArchZFS repo only (zfs-dkms + zfs-utils).

def main() -> None:
    raise SystemExit("gen_iso/build_zfs_package.py is deprecated and disabled. Use DKMS flow with public ArchZFS repo.")

if __name__ == "__main__":
    main()
