#!/usr/bin/env bash
# shellcheck disable=SC2034

iso_name="archzfs"
iso_label="ARCH_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="Arch Linux <https://archlinux.org>"
iso_application="Arch Linux baseline"
iso_version="testing"
install_dir="arch"
buildmodes=('iso')
bootmodes=('bios.syslinux.eltorito'
           'uefi-x64.grub.eltorito')
arch="x86_64"
pacman_conf="pacman.conf"
airootfs_image_type="erofs"
airootfs_image_tool_options=('-zlz4')             # FAST compression
bootstrap_tarball_compression=('cat') 
file_permissions=(
  ["/etc/shadow"]="0:0:400"
)
