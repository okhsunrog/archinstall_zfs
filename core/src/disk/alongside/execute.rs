use super::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

/// Apply an explicitly confirmed plan. The caller must run this off the UI
/// thread, and must not cancel it between filesystem shrink and GPT update.
/// This operation never mounts or formats an existing partition.
///
/// `budget` must describe the boot image being installed. `recovery_dir` must
/// be a new directory on the live filesystem, outside the disk being changed.
/// A partial failure is reported, never automatically rolled back or retried.
pub fn execute(
    runner: &dyn CommandRunner,
    request: &Request,
    budget: BootSpace,
    recovery_dir: &Path,
) -> Result<crate::prepare::PreparedPartitions> {
    ensure!(
        crate::system::sysinfo::has_uefi(),
        "Restart the installer in UEFI mode before installing"
    );
    check_tools(&[
        ("sfdisk", "util-linux"),
        ("sgdisk", "gptfdisk"),
        ("lsblk", "util-linux"),
        ("blockdev", "util-linux"),
    ])?;
    let disk = &request.before.device;
    let mut handle = OpenOptions::new().read(true).write(true).open(disk)?;
    ensure!(
        handle.metadata()?.file_type().is_block_device(),
        "Installation requires a block device"
    );
    handle
        .try_lock()
        .wrap_err("Another process holds the disk lock")?;
    let mut mbr = [0; 512];
    handle.read_exact(&mut mbr)?;
    validate_protective_mbr(&mbr)?;
    let (current, filesystems) = inspect(runner, disk)?;
    ensure!(
        current == request.before,
        "The disk layout changed after selection. Refresh and review a new plan; nothing has been written"
    );
    let plan = request.plan()?;
    let space = inspect_efi(
        runner,
        &current,
        request.efi.existing_partition(),
        &filesystems,
    )?;
    request.efi.validate_space(&space, budget)?;
    if plan.efi_start.is_some() {
        check_tools(&[("mkfs.fat", "dosfstools")])?;
        ensure!(
            budget.required_bytes()? < ESP_BYTES - 8 * MIB,
            "The boot image and selected copies do not fit the proposed additional ESP"
        );
    }
    let filesystem = if let Some((part, sectors)) = &plan.shrink {
        let fs = filesystems
            .iter()
            .find(|f| f.path == part.node)
            .and_then(|f| f.fstype.clone())
            .ok_or_else(|| eyre!("Cannot identify the filesystem to shrink"))?;
        let bytes = sectors * current.sectorsize;
        let minimum = minimum_size(runner, part, &fs)?;
        ensure!(
            bytes > MIB && bytes - MIB >= minimum,
            "The selected size is below the current safe minimum; refresh and allocate less space to Linux"
        );
        if fs == "ntfs" {
            probe::run(
                runner,
                "ntfsresize",
                &[
                    "--no-action",
                    "--no-progress-bar",
                    "--size",
                    &(bytes - MIB).to_string(),
                    &part.node.to_string_lossy(),
                ],
            )?;
        }
        Some(fs)
    } else {
        None
    };
    let validation = probe::run(runner, "sgdisk", &["--verify", &disk.to_string_lossy()])?;
    ensure!(
        validation.contains("No problems found."),
        "GPT verification requires attention: {validation}"
    );
    // This is the last read-only gate. Keep backups outside the target and
    // retain them even when a later command fails.
    std::fs::create_dir(recovery_dir)
        .wrap_err("Create a fresh recovery directory on the live filesystem")?;
    let dump = probe::run(runner, "sfdisk", &["--dump", &disk.to_string_lossy()])?;
    let mut backup = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(recovery_dir.join("partition-table.sfdisk"))?;
    backup.write_all(dump.as_bytes())?;
    backup.sync_all()?;
    File::open(recovery_dir)?.sync_all()?;
    tracing::info!(path = %recovery_dir.display(), "Saved partition table before resizing; this is not a filesystem backup");
    let prepared = apply(runner, request, &plan, filesystem.as_deref()).wrap_err_with(|| format!("Storage operation stopped. Some changes may already have completed. Do not retry or restore GPT blindly. Original partition table: {}", recovery_dir.join("partition-table.sfdisk").display()))?;
    // Persistent aliases are created by udev, which may be waiting for the
    // disk lock. Release it before waiting for them.
    drop(handle);
    for path in std::iter::once(&prepared.efi)
        .chain(prepared.zfs.iter())
        .chain(prepared.swap.iter())
    {
        crate::disk::partition::wait_for_path(path)?;
    }
    Ok(prepared)
}

/// Prefer a persistent `/dev/disk/by-id` alias for paths that are written
/// into the installed system, as the full-disk layout does. The kernel node
/// is only a fallback: it can change across boots.
fn stable_disk_path(disk: &Path) -> PathBuf {
    let Ok(canonical) = disk.canonicalize() else {
        return disk.to_path_buf();
    };
    let Ok(entries) = std::fs::read_dir("/dev/disk/by-id") else {
        return disk.to_path_buf();
    };
    let mut aliases: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.canonicalize().ok().as_deref() == Some(&canonical))
        .collect();
    aliases.sort();
    // Model and serial names read better than WWN or EUI identifiers, but
    // any persistent alias is preferable to the kernel node.
    let generic = |p: &PathBuf| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        name.starts_with("wwn-") || name.starts_with("nvme-eui.")
    };
    aliases
        .iter()
        .find(|p| !generic(p))
        .or_else(|| aliases.first())
        .cloned()
        .unwrap_or_else(|| {
            tracing::warn!(disk = %disk.display(), "No persistent alias for the disk; using the kernel node");
            disk.to_path_buf()
        })
}

/// The partitioning tools exit successfully even when the kernel refuses to
/// re-read the table (`EBUSY` while another process holds a partition node).
/// The GPT on disk is complete at that point; ask for a rescan before giving
/// up rather than reporting a consistent disk as damaged.
fn sync_kernel_partitions(runner: &dyn CommandRunner, disk: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 1..=5 {
        match inspect(runner, disk) {
            Ok(_) => return Ok(()),
            Err(error) => last = Some(error),
        }
        tracing::warn!(
            attempt,
            "Kernel partition table is stale; requesting a rescan"
        );
        let out = runner.run("blockdev", &["--rereadpt", &disk.to_string_lossy()])?;
        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "Partition table rescan was refused");
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(last.expect("at least one attempt")).wrap_err(
        "The kernel still reports the previous partition table. The GPT on disk is complete; close programs using this disk (or reboot the live system) and retry the plan after refreshing",
    )
}

fn validate_protective_mbr(mbr: &[u8; 512]) -> Result<()> {
    ensure!(
        mbr[510..] == [0x55, 0xaa],
        "Missing protective MBR signature"
    );
    let types: Vec<_> = (0..4)
        .map(|i| mbr[446 + i * 16 + 4])
        .filter(|t| *t != 0)
        .collect();
    ensure!(
        types == [0xee],
        "Hybrid or non-protective MBR is not supported; no disk changes were made"
    );
    Ok(())
}

fn apply(
    runner: &dyn CommandRunner,
    request: &Request,
    plan: &Plan,
    fs: Option<&str>,
) -> Result<crate::prepare::PreparedPartitions> {
    let disk = &request.before.device;
    let dev = disk.to_string_lossy();
    let mut expected = request.before.clone();
    if let Some((part, sectors)) = &plan.shrink {
        let bytes = sectors * expected.sectorsize;
        let node = part.node.to_string_lossy();
        tracing::info!(partition = %node, new_bytes = bytes, "Shrinking filesystem before partition boundary");
        match fs {
            Some("ntfs") => {
                let out = runner.run_with_stdin(
                    "env",
                    &[
                        "LC_ALL=C",
                        "ntfsresize",
                        "--no-progress-bar",
                        "--size",
                        &(bytes - MIB).to_string(),
                        &node,
                    ],
                    b"y\n",
                )?;
                check_exit(&out, "Shrink NTFS")?;
                // ntfsresize deliberately leaves NTFS dirty for Windows to
                // check. Do not use --force or clear that flag to probe it.
                let mut boot = [0; 512];
                File::open(&part.node)?.read_exact(&mut boot)?;
                ensure!(
                    &boot[3..11] == b"NTFS    ",
                    "NTFS boot sector cannot be verified after shrink"
                );
                let sector = u16::from_le_bytes(boot[11..13].try_into()?);
                let count = u64::from_le_bytes(boot[40..48].try_into()?);
                ensure!(
                    matches!(sector, 512 | 4096)
                        && count > 0
                        && count
                            .checked_add(1)
                            .and_then(|n| n.checked_mul(u64::from(sector)))
                            .is_some_and(|n| n <= bytes),
                    "NTFS still exceeds the planned partition boundary"
                );
            }
            Some("ext4") => {
                // Read-only checking has already passed. Persist a clean
                // check before resize2fs; never force the resizer itself.
                probe::run(runner, "e2fsck", &["-f", "-p", &node])?;
                probe::run(
                    runner,
                    "resize2fs",
                    &[&node, &format!("{}K", (bytes - MIB) / 1024)],
                )?;
                probe::run(runner, "e2fsck", &["-f", "-n", &node])?;
                let (block, count, _) = probe::ext_size(runner, &node)?;
                ensure!(
                    count.checked_mul(block).is_some_and(|n| n <= bytes),
                    "ext4 still exceeds the planned partition boundary"
                );
            }
            _ => bail!("Unsupported filesystem; partition table was not modified"),
        }
        let number = part.number(disk)?;
        let out = runner.run_with_stdin(
            "env",
            &[
                "LC_ALL=C",
                "LOCK_BLOCK_DEVICE=0",
                "sfdisk",
                "--wipe",
                "never",
                "--wipe-partitions",
                "never",
                "-N",
                &number.to_string(),
                &dev,
            ],
            format!("size={sectors}\n").as_bytes(),
        )?;
        check_exit(&out, "Update the resized partition boundary")?;
        expected
            .partitions
            .iter_mut()
            .find(|p| p.node == part.node)
            .ok_or_else(|| eyre!("Partition missing from expected layout"))?
            .size = *sectors;
        ensure!(
            Layout::read(runner, disk)? == expected,
            "GPT verification failed after shrink; no new partitions will be formatted"
        );
    }
    let mut args = Vec::new();
    if let Some(start) = plan.efi_start {
        args.extend([
            format!("--new={}:{}:{}", plan.efi_number, start, plan.zfs_start - 1),
            format!("--typecode={}:ef00", plan.efi_number),
        ]);
    }
    args.extend([
        format!(
            "--new={}:{}:{}",
            plan.zfs_number,
            plan.zfs_start,
            plan.swap_start.unwrap_or(plan.end) - 1
        ),
        format!("--typecode={}:bf00", plan.zfs_number),
    ]);
    if let (Some(number), Some(start)) = (plan.swap_number, plan.swap_start) {
        args.extend([
            format!("--new={number}:{start}:{}", plan.end - 1),
            format!("--typecode={number}:8200"),
        ]);
    }
    args.push(dev.to_string());
    probe::run(
        runner,
        "sgdisk",
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    // Do not wait for udev while holding the whole-disk lock: its workers may
    // be waiting for that very lock. Kernel geometry and devtmpfs nodes are
    // verified below; udev can populate aliases after this operation returns.
    let after = Layout::read(runner, disk)?;
    verify_created(&expected, &after, plan)?;
    // Compare kernel-reported sizes before a node is handed to mkfs or ZFS.
    sync_kernel_partitions(runner, disk)?;
    use super::super::partition::partition_path;
    if plan.efi_start.is_some() {
        // The kernel node exists now; persistent aliases appear only after
        // udev runs, which must not be awaited under the disk lock.
        crate::disk::partition::format_efi(runner, &partition_path(disk, plan.efi_number))?;
    }
    let stable = stable_disk_path(disk);
    Ok(crate::prepare::PreparedPartitions {
        efi: partition_path(&stable, plan.efi_number),
        zfs: Some(partition_path(&stable, plan.zfs_number)),
        swap: plan.swap_number.map(|n| partition_path(&stable, n)),
    })
}

fn verify_created(before: &Layout, after: &Layout, plan: &Plan) -> Result<()> {
    let mut retained = after.clone();
    let zfs = retained
        .partitions
        .iter()
        .find(|p| p.number(&before.device).ok() == Some(plan.zfs_number))
        .ok_or_else(|| eyre!("New ZFS partition is missing"))?;
    ensure!(
        zfs.start == plan.zfs_start
            && zfs.end()? == plan.swap_start.unwrap_or(plan.end)
            && zfs
                .kind
                .eq_ignore_ascii_case("6A85CF4D-1DD2-11B2-99A6-080020736631"),
        "New ZFS partition differs from plan"
    );
    if let Some(start) = plan.efi_start {
        let efi = retained
            .partitions
            .iter()
            .find(|p| p.number(&before.device).ok() == Some(plan.efi_number))
            .ok_or_else(|| eyre!("New EFI partition is missing"))?;
        ensure!(
            efi.start == start
                && efi.end()? == plan.zfs_start
                && efi.kind.eq_ignore_ascii_case(EFI_TYPE),
            "New EFI partition differs from plan"
        );
    }
    if let (Some(number), Some(start)) = (plan.swap_number, plan.swap_start) {
        let swap = retained
            .partitions
            .iter()
            .find(|p| p.number(&before.device).ok() == Some(number))
            .ok_or_else(|| eyre!("New swap partition is missing"))?;
        ensure!(
            swap.start == start
                && swap.end()? == plan.end
                && swap
                    .kind
                    .eq_ignore_ascii_case("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F"),
            "New swap partition differs from plan"
        );
    }
    retained.partitions.retain(|p| {
        p.number(&before.device).ok() != Some(plan.zfs_number)
            && !(plan.swap_number.is_some() && p.number(&before.device).ok() == plan.swap_number)
            && !(plan.efi_start.is_some() && p.number(&before.device).ok() == Some(plan.efi_number))
    });
    ensure!(
        &retained == before,
        "An existing partition or GPT identity changed unexpectedly; refusing to format"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hybrid_mbr_is_not_accepted_as_gpt() {
        let mut mbr = [0; 512];
        mbr[510..].copy_from_slice(&[0x55, 0xaa]);
        mbr[450] = 0xee;
        assert!(validate_protective_mbr(&mbr).is_ok());
        mbr[466] = 7;
        assert!(validate_protective_mbr(&mbr).is_err());
    }
}
