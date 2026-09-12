//! The directory the new system is assembled in.
//!
//! ZFS refuses to mount a dataset over a directory with anything in it, even
//! one empty subdirectory. An earlier run that was cancelled, or a stray
//! `mkdir` under `/mnt`, therefore has to be dealt with before the disk is
//! touched: the failure used to come after the resize and the pool creation,
//! where it was expensive, instead of before, where it is free.

use std::path::Path;

use color_eyre::eyre::{Context, Result, bail, ensure};

use crate::system::cmd::{CommandRunner, check_exit};

/// Make `target` an empty directory: unmount what an earlier run left under
/// it, remove empty directories, and refuse to go on while files remain.
pub fn ensure_empty(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    ensure!(
        target.is_absolute() && target != Path::new("/"),
        "refusing to prepare {} as an installation target",
        target.display()
    );
    std::fs::create_dir_all(target)
        .wrap_err_with(|| format!("cannot create {}", target.display()))?;
    unmount_under(runner, target)?;
    prune_empty_dirs(target);
    let leftovers = entries(target)?;
    if !leftovers.is_empty() {
        bail!(
            "The target directory {} is not empty: {}. Move or delete its contents, then start the installation again.",
            target.display(),
            leftovers.join(", ")
        );
    }
    tracing::info!(target = %target.display(), "target directory is empty");
    Ok(())
}

/// Unmount every filesystem mounted at or under `target`, deepest first.
pub fn unmount_under(runner: &dyn CommandRunner, target: &Path) -> Result<()> {
    let listing = runner.run(
        "findmnt",
        &["--noheadings", "--raw", "--output", "TARGET,SOURCE"],
    )?;
    check_exit(&listing, "list mounted filesystems")?;
    let mut mounts: Vec<(&str, &str)> = listing
        .stdout
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(path, _)| is_at_or_under(Path::new(path), target))
        .collect();
    mounts.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
    for (path, source) in mounts {
        tracing::info!(path, source, "unmounting what an earlier run left mounted");
        let output = runner.run("umount", &["--recursive", path])?;
        if !output.success() && !output.stderr.contains("not mounted") {
            check_exit(&output, "unmount an earlier run's filesystem")?;
        }
    }
    Ok(())
}

fn is_at_or_under(path: &Path, target: &Path) -> bool {
    path == target || path.starts_with(target)
}

/// Remove the empty directories under `dir`, deepest first, and report
/// whether `dir` itself is empty afterwards. Files and symlinks are never
/// touched, so what a user put there survives.
pub fn prune_empty_dirs(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut empty = true;
    for entry in entries.flatten() {
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_dir && prune_empty_dirs(&entry.path()) {
            let _ = std::fs::remove_dir(entry.path());
        } else {
            empty = false;
        }
    }
    empty
}

/// The first few names in `dir`, for an error message.
fn entries(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .wrap_err_with(|| format!("cannot read {}", dir.display()))?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if names.len() > 5 {
        let more = names.len() - 5;
        names.truncate(5);
        names.push(format!("and {more} more"));
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    fn no_mounts() -> RecordingRunner {
        RecordingRunner::new(vec![CannedResponse {
            stdout: "/ /dev/root\n/boot /dev/sda1\n".into(),
            ..Default::default()
        }])
    }

    #[test]
    fn empty_directories_are_removed_and_the_target_accepted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("boot/efi")).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        ensure_empty(&no_mounts(), dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn files_are_kept_and_named_in_the_refusal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        std::fs::write(dir.path().join("keep/../notes.txt"), b"x").ok();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let error = ensure_empty(&no_mounts(), dir.path()).unwrap_err();
        assert!(error.to_string().contains("notes.txt"), "{error}");
        assert!(dir.path().join("notes.txt").exists());
        assert!(!dir.path().join("empty").exists());
    }

    #[test]
    fn mounts_under_the_target_are_unmounted_deepest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: format!(
                    "/ /dev/root\n{root} pool/root\n{root}/boot/efi /dev/sda1\n{root}-other /dev/sdb1\n"
                ),
                ..Default::default()
            },
            CannedResponse::default(),
            CannedResponse::default(),
        ]);
        ensure_empty(&runner, dir.path()).unwrap();
        let umounts: Vec<Vec<String>> = runner
            .calls()
            .into_iter()
            .filter(|call| call.program == "umount")
            .map(|call| call.args)
            .collect();
        assert_eq!(
            umounts,
            vec![
                vec!["--recursive".to_string(), format!("{root}/boot/efi")],
                vec!["--recursive".to_string(), root.clone()],
            ]
        );
    }

    #[test]
    fn the_filesystem_root_is_never_a_target() {
        assert!(ensure_empty(&no_mounts(), Path::new("/")).is_err());
    }
}
