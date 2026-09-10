//! Destructive integration test restricted to a loop device created here.
//! Run the built example as root; it takes a filesystem name, never a disk path.
use archinstall_zfs_core::{
    disk::alongside::*,
    system::cmd::{CommandRunner, RealRunner, check_exit},
};
use color_eyre::eyre::{Result, ensure};
use std::{fs::File, path::PathBuf, process::Command};

struct LoopDevice(String);
impl Drop for LoopDevice {
    fn drop(&mut self) {
        let _ = Command::new("losetup").args(["--detach", &self.0]).status();
    }
}
fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = RealRunner.run(program, args)?;
    check_exit(&output, program)?;
    Ok(output.stdout)
}
fn main() -> Result<()> {
    color_eyre::install()?;
    let fs = std::env::args().nth(1).unwrap_or_else(|| "ext4".into());
    let mode = std::env::args().nth(2).unwrap_or_else(|| "shrink".into());
    ensure!(
        matches!(mode.as_str(), "shrink" | "free" | "new-esp" | "swap"),
        "Mode must be shrink, free or new-esp"
    );
    ensure!(
        matches!(fs.as_str(), "ext4" | "ntfs"),
        "Pass ext4 or ntfs, not a device path"
    );
    let dir = tempfile::tempdir()?;
    let image = dir.path().join("disk.img");
    File::create(&image)?.set_len(120 * GIB)?;
    let loop_path = run(
        "losetup",
        &["--find", "--show", "--partscan", image.to_str().unwrap()],
    )?
    .trim()
    .to_owned();
    let device = LoopDevice(loop_path);
    ensure!(device.0.starts_with("/dev/loop"), "Unexpected loop device");
    run(
        "sgdisk",
        &[
            "--clear",
            "--new=1:0:+500M",
            "--typecode=1:ef00",
            "--change-name=1:Existing EFI",
            "--new=2:0:+70G",
            if fs == "ntfs" {
                "--typecode=2:0700"
            } else {
                "--typecode=2:8300"
            },
            "--change-name=2:Existing OS",
            "--new=3:0:+1G",
            "--typecode=3:2700",
            "--change-name=3:Recovery",
            &device.0,
        ],
    )?;
    run("udevadm", &["settle"])?;
    let efi = format!("{}p1", device.0);
    let data = format!("{}p2", device.0);
    run("mkfs.fat", &["-F32", &efi])?;
    let sentinel = dir.path().join("sentinel.txt");
    let contents = "Retained operating system data\n".repeat(4096);
    std::fs::write(&sentinel, &contents)?;
    run(
        "mcopy",
        &["-i", &efi, sentinel.to_str().unwrap(), "::KEEP.TXT"],
    )?;
    if mode == "new-esp" {
        let filler = dir.path().join("filler.bin");
        File::create(&filler)?.set_len(430 * MIB)?;
        run(
            "mcopy",
            &["-i", &efi, filler.to_str().unwrap(), "::FILLER.BIN"],
        )?;
    }
    if fs == "ntfs" {
        run("mkntfs", &["--quick", &data])?;
        run(
            "ntfscp",
            &[&data, sentinel.to_str().unwrap(), "/sentinel.txt"],
        )?;
    } else {
        run("mkfs.ext4", &["-q", &data])?;
        run(
            "debugfs",
            &[
                "-w",
                "-R",
                &format!("write {} /sentinel.txt", sentinel.display()),
                &data,
            ],
        )?;
    }
    let (before, filesystems) = inspect(&RealRunner, &PathBuf::from(&device.0))?;
    let minimum = minimum_size(&RealRunner, &before.partitions[1], &fs)?;
    eprintln!("{fs}: minimum with headroom {} MiB", minimum / MIB);
    let request = Request {
        before: before.clone(),
        source: if mode == "free" {
            let (start, end) = *before.free_extents()?.last().unwrap();
            SpaceSource::Unallocated { start, end }
        } else {
            SpaceSource::Shrink { partition: 2 }
        },
        efi: if mode == "new-esp" {
            EfiChoice::CreateAfterInsufficientSpace {
                existing_partition: 1,
            }
        } else {
            EfiChoice::Reuse { partition: 1 }
        },
        swap_bytes: if mode == "swap" { 8 * GIB } else { 0 },
        allocation_bytes: if mode == "new-esp" {
            33 * GIB
        } else if mode == "swap" {
            40 * GIB
        } else {
            32 * GIB
        },
    };
    let budget = BootSpace::default();
    request
        .efi
        .validate_space(&inspect_efi(&RealRunner, &before, 1, &filesystems)?, budget)?;
    let prepared = execute(&RealRunner, &request, budget, &dir.path().join("recovery"))?;
    ensure!(
        (prepared.efi == std::path::Path::new(&efi)) == (mode != "new-esp"),
        "EFI selection did not follow the explicit plan"
    );
    let actual = if fs == "ntfs" {
        // ntfsresize schedules Windows' consistency check. Read the test
        // payload despite that flag; do not clear it or force a resize.
        ensure!(
            mode == "free" || RealRunner.run("ntfsresize", &["--info", &data])?.exit_code != 0,
            "NTFS check-required flag was unexpectedly cleared"
        );
        run("ntfscat", &["--force", &data, "/sentinel.txt"])?
    } else {
        run("debugfs", &["-R", "cat /sentinel.txt", &data])?
    };
    ensure!(actual == contents, "Retained filesystem content changed");
    ensure!(
        run("mtype", &["-i", &efi, "::KEEP.TXT"])? == contents,
        "Existing ESP content changed"
    );
    let after = Layout::read(&RealRunner, &PathBuf::from(&device.0))?;
    ensure!(
        after.partitions.len()
            == before.partitions.len()
                + if matches!(mode.as_str(), "new-esp" | "swap") {
                    2
                } else {
                    1
                },
        "Unexpected partition count"
    );
    if mode == "swap" {
        let swap = prepared.swap.as_ref().expect("planned swap node");
        ensure!(
            after
                .partitions
                .iter()
                .any(|p| &p.node == swap && p.size * after.sectorsize == 8 * GIB),
            "Swap size differs from plan"
        );
    }
    if mode == "free" {
        ensure!(
            before.partitions[1] == after.partitions[1],
            "Using free space changed the existing data partition"
        );
    }
    ensure!(
        before.partitions[0] == after.partitions[0] && before.partitions[2] == after.partitions[2],
        "Existing ESP or recovery partition changed"
    );
    ensure!(
        execute(&RealRunner, &request, budget, &dir.path().join("retry")).is_err(),
        "Stale plan was accepted"
    );
    eprintln!(
        "PASS: {fs} {mode}, retained payload, ESP content, recovery geometry, new ZFS partition and stale-plan rejection"
    );
    Ok(())
}
