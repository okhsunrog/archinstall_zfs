//! Prepare and verify a preserved filesystem only inside the disposable ISO VM.
use crate::qemu::QemuVm;
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

pub fn prepare(vm: &QemuVm, config: &Path) -> Result<(), String> {
    let output = vm.ssh_run(r#"set -euo pipefail
[ "$(readlink -f /dev/disk/by-id/virtio-archzfs-test-disk)" = /dev/vda ]
sgdisk --zap-all --new=1:2048:+500M --typecode=1:ef00 --new=2:0:+50G --typecode=2:8300 --new=3:0:+1G --typecode=3:8300 /dev/vda
udevadm settle
mkfs.fat -F32 /dev/vda1
mkfs.ext4 -F /dev/vda2
mkdir -p /run/preserved
mount /dev/vda2 /run/preserved
printf 'preserved filesystem payload\n' > /run/preserved/KEEP.txt
umount /run/preserved
# Write through a normal mount: the live medium need not carry mtools or a
# usable pacman keyring for the fixture.
mount /dev/vda1 /run/preserved
mkdir -p /run/preserved/EFI/FOREIGN
printf 'foreign EFI payload\n' > /run/preserved/EFI/FOREIGN/KEEP.EFI
umount /run/preserved
"#).map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Alongside fixture preparation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let output = vm
        .ssh_run("sfdisk --json /dev/vda")
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Cannot inspect VM fixture GPT".into());
    }
    let layout: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(config).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let swap_bytes = if matches!(
        config["swap_mode"].as_str(),
        Some("zswap_partition" | "zswap_partition_encrypted")
    ) {
        8_u64 * 1024 * 1024 * 1024
    } else {
        0
    };
    config["installation_mode"] = "alongside".into();
    config["alongside"] = serde_json::json!({ "before": layout["partitiontable"], "source": {"Shrink":{"partition":2}}, "efi":{"Reuse":{"partition":1}}, "allocation_bytes":32_u64*1024*1024*1024 + swap_bytes, "swap_bytes": swap_bytes });
    let path = std::env::temp_dir().join(format!("archzfs-alongside-{}.json", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(&serde_json::to_vec(&config).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        vm.scp_to(&path, "/root/config.json");
        Ok(())
    })();
    let _ = fs::remove_file(path);
    result
}

pub fn verify(vm: &QemuVm) -> Result<(), String> {
    let output = vm
        .ssh_run(
            r#"set -euo pipefail
[ "$(readlink -f /dev/disk/by-id/virtio-archzfs-test-disk)" = /dev/vda ]
e2fsck -fn /dev/vda2
mount -o ro /dev/vda2 /run/preserved
value=$(cat /run/preserved/KEEP.txt)
umount /run/preserved
[ "$value" = 'preserved filesystem payload' ]
# The installer leaves the ESP mounted under the target; FAT cannot be
# mounted a second time, so read it where it is.
esp=$(findmnt -n -o TARGET --source /dev/vda1 | head -n 1)
if [ -z "$esp" ]; then mount -o ro /dev/vda1 /run/preserved; esp=/run/preserved; fi
value=$(cat "$esp/EFI/FOREIGN/KEEP.EFI")
cp "$esp/EFI/zbm/vmlinuz.EFI" /run/installed-zbm.EFI
[ "$esp" != /run/preserved ] || umount /run/preserved
[ "$value" = 'foreign EFI payload' ]
[ "$(od -An -tx1 -N2 /run/installed-zbm.EFI | tr -d ' \n')" = 4d5a ]
"#,
        )
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Preserved filesystem/ESP verification failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
