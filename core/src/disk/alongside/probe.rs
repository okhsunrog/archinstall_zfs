use super::*;
use crate::system::cmd::CmdOutput;

#[derive(Debug, Clone, Deserialize)]
pub struct Filesystem {
    pub path: PathBuf,
    #[serde(rename = "type")]
    kind: String,
    pub size: u64,
    pub ro: bool,
    #[serde(default)]
    pub fstype: Option<String>,
    pub mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Filesystem>,
}

/// Read-only inspection. Refuse active disks rather than unmounting anything
/// the live system or another application may own.
pub fn inspect(runner: &dyn CommandRunner, disk: &Path) -> Result<(Layout, Vec<Filesystem>)> {
    let layout = Layout::read(runner, disk)?;
    let out = runner.run(
        "lsblk",
        &[
            "--json",
            "--tree",
            "--bytes",
            "--paths",
            "--output",
            "PATH,TYPE,SIZE,RO,FSTYPE,MOUNTPOINTS",
            &layout.device.to_string_lossy(),
        ],
    )?;
    check_exit(&out, "Inspect disk usage")?;
    #[derive(Deserialize)]
    struct Devices {
        blockdevices: Vec<Filesystem>,
    }
    let mut devices: Devices = serde_json::from_str(&out.stdout)?;
    ensure!(devices.blockdevices.len() == 1, "Expected one whole disk");
    let d = devices.blockdevices.remove(0);
    ensure!(
        d.path == layout.device && matches!(d.kind.as_str(), "disk" | "loop"),
        "Select a whole disk, not a mapped device"
    );
    ensure!(
        (layout.lastlba + 1) * layout.sectorsize <= d.size,
        "GPT exceeds the device capacity"
    );
    for node in std::iter::once(&d).chain(d.children.iter()) {
        ensure!(
            !node.ro
                && node
                    .mountpoints
                    .iter()
                    .all(|p| p.as_deref().is_none_or(str::is_empty)),
            "{} is read-only or in use. Unmount its filesystems and disable its swap before proceeding.",
            node.path.display()
        );
        if node.path != d.path {
            ensure!(
                node.kind == "part" && node.children.is_empty(),
                "{} has active mappings; close them in the existing system first",
                node.path.display()
            );
            ensure!(
                node.fstype.as_deref() != Some("zfs_member"),
                "A disk containing ZFS members cannot be repartitioned by this workflow"
            );
        }
    }
    ensure!(
        d.children.len() == layout.partitions.len(),
        "Kernel partition list does not match GPT; refresh the device before proceeding"
    );
    for p in &layout.partitions {
        ensure!(
            d.children
                .iter()
                .any(|c| c.path == p.node && c.size == p.size * layout.sectorsize),
            "Kernel partition size differs from GPT for {}",
            p.node.display()
        );
    }
    Ok((layout, d.children))
}

/// The argv for `env LC_ALL=C [env...] program args...`: every filesystem
/// tool is run with a C locale so its output can be parsed.
fn env_argv<'a>(env: &[&'a str], program: &'a str, args: &[&'a str]) -> Vec<&'a str> {
    let mut argv = Vec::with_capacity(env.len() + args.len() + 2);
    argv.push("LC_ALL=C");
    argv.extend_from_slice(env);
    argv.push(program);
    argv.extend_from_slice(args);
    argv
}

pub(super) fn run(runner: &dyn CommandRunner, program: &str, args: &[&str]) -> Result<String> {
    let out = runner.run("env", &env_argv(&[], program, args))?;
    check_exit(&out, program)?;
    Ok(out.stdout)
}

/// Like [`run`], with extra `NAME=value` pairs and input on stdin. The exit
/// status is left to the caller, whose context message names the operation.
pub(super) fn run_with_stdin(
    runner: &dyn CommandRunner,
    env: &[&str],
    program: &str,
    args: &[&str],
    stdin: &[u8],
) -> Result<CmdOutput> {
    runner.run_with_stdin("env", &env_argv(env, program, args), stdin)
}

fn number_after(text: &str, key: &str) -> Result<u64> {
    let value = text
        .lines()
        .find_map(|line| line.trim().strip_prefix(key))
        .and_then(|s| s.split_whitespace().next())
        .ok_or_else(|| eyre!("Cannot interpret filesystem tool output: missing {key}"))?;
    Ok(value.parse()?)
}

pub(super) fn ext_size(runner: &dyn CommandRunner, device: &str) -> Result<(u64, u64, u64)> {
    let header = run(runner, "dumpe2fs", &["-h", device])?;
    let block = number_after(&header, "Block size:")?;
    let count = number_after(&header, "Block count:")?;
    let free = number_after(&header, "Free blocks:")?;
    ensure!(
        matches!(block, 1024 | 2048 | 4096) && free <= count,
        "Unsupported ext4 block size or invalid accounting"
    );
    Ok((block, count, free))
}

/// Returns a conservative lower bound. Passing this check does not bypass the
/// resizer's own checks. Failed or unparseable probes never enable shrinking.
/// What a filesystem occupies and the smallest size it may be shrunk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShrinkLimits {
    /// Bytes the filesystem needs for its data and metadata, as reported by
    /// the resizer. This is the basis for "free space" in the planner.
    pub used_bytes: u64,
    /// The hard lower bound for resizing: `used_bytes` plus a safety margin
    /// against the resizer's estimate.
    pub minimum_bytes: u64,
}

/// The smallest size the filesystem may safely be shrunk to. See
/// [`shrink_limits`] for the numbers behind it.
pub fn minimum_size(runner: &dyn CommandRunner, part: &Partition, fs: &str) -> Result<u64> {
    Ok(shrink_limits(runner, part, fs)?.minimum_bytes)
}

pub fn shrink_limits(
    runner: &dyn CommandRunner,
    part: &Partition,
    fs: &str,
) -> Result<ShrinkLimits> {
    check_tools(required_tools(fs)?)?;
    ensure!(
        part.attrs.is_empty(),
        "Partition attributes require manual review before resizing"
    );
    let dev = part.node.to_string_lossy();
    let raw = match fs {
        "ntfs" => {
            ensure!(
                part.kind.eq_ignore_ascii_case(BASIC_TYPE),
                "Only ordinary NTFS data partitions can be resized; recovery and system partitions are preserved"
            );
            run(runner, "ntfs-3g.probe", &["--readwrite", &dev]).wrap_err("NTFS is not safely writable. Shut down Windows fully, disable Fast Startup/hibernation, and check the filesystem in Windows")?;
            let info = run(runner, "ntfsresize", &["--info", "--no-progress-bar", &dev])?;
            number_after(&info, "You might resize at ")?
        }
        "ext4" => {
            ensure!(
                part.kind.eq_ignore_ascii_case(LINUX_TYPE),
                "Only ordinary Linux filesystem partitions can be resized"
            );
            run(runner, "e2fsck", &["-f", "-n", &dev]).wrap_err("ext4 needs filesystem maintenance; check it from the existing system before resizing")?;
            let (block, count, free) = ext_size(runner, &dev)?;
            let info = run(runner, "resize2fs", &["-P", &dev])?;
            let min = number_after(&info, "Estimated minimum size of the filesystem:")?;
            min.max(count - free)
                .checked_mul(block)
                .ok_or_else(|| eyre!("Filesystem size overflow"))?
        }
        _ => unreachable!("required_tools rejected an unsupported filesystem"),
    };
    // The resizer's minimum is an estimate (notably resize2fs with small
    // blocks); keep a margin above it. Working space for the retained system
    // is a planning default, not a resize limit: see `shrink_defaults`.
    let minimum_bytes = raw
        .checked_add((raw / 10).max(GIB))
        .ok_or_else(|| eyre!("Minimum size overflow"))?;
    Ok(ShrinkLimits {
        used_bytes: raw,
        minimum_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn tools_run_under_env_with_a_c_locale_and_extra_variables_after_it() {
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: "No problems found.\n".into(),
                ..Default::default()
            },
            CannedResponse::default(),
        ]);

        let verified = run(&runner, "sgdisk", &["--verify", "/dev/sda"]).unwrap();
        assert_eq!(verified, "No problems found.\n");
        let out = run_with_stdin(
            &runner,
            &["LOCK_BLOCK_DEVICE=0"],
            "sfdisk",
            &["--wipe", "never", "-N", "3", "/dev/sda"],
            b"size=2048\n",
        )
        .unwrap();
        assert!(out.success());

        let calls = runner.calls();
        assert_eq!(calls[0].program, "env");
        assert_eq!(
            calls[0].args,
            ["LC_ALL=C", "sgdisk", "--verify", "/dev/sda"]
        );
        assert_eq!(calls[1].program, "env");
        assert_eq!(
            calls[1].args,
            [
                "LC_ALL=C",
                "LOCK_BLOCK_DEVICE=0",
                "sfdisk",
                "--wipe",
                "never",
                "-N",
                "3",
                "/dev/sda"
            ]
        );
    }

    #[test]
    fn a_failing_tool_is_reported_by_name() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 1,
            stderr: "bad superblock".into(),
            ..Default::default()
        }]);
        let err = run(&runner, "e2fsck", &["-f", "-n", "/dev/sda2"]).unwrap_err();
        assert!(err.to_string().contains("e2fsck"), "{err}");
        assert!(err.to_string().contains("bad superblock"), "{err}");
    }

    #[test]
    fn exact_bytes_are_required_and_localized_or_missing_output_fails() {
        assert_eq!(
            number_after(
                "You might resize at 123456 bytes or 1 MB",
                "You might resize at "
            )
            .unwrap(),
            123456
        );
        assert!(number_after("Minsize: unknown", "You might resize at ").is_err());
    }
}
