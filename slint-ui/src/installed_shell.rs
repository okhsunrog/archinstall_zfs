//! A temporary, explicitly owned mount tree for post-install maintenance.
//! Never changes dataset mountpoint properties or unmounts unrelated pools.

use archinstall_zfs_core::boot_environment::BootEnvironment;
use archinstall_zfs_core::installed_system::InstalledSystem;
use archinstall_zfs_core::system::cmd::{CommandRunner, RealRunner, check_exit};
use color_eyre::eyre::{Result, bail, ensure};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Session<'a> {
    runner: &'a dyn CommandRunner,
    target: &'a InstalledSystem,
    directory: PathBuf,
    imported: bool,
    mounted: bool,
}

impl Session<'_> {
    fn command(&self, program: &str, args: &[&str]) -> Result<String> {
        let output = self.runner.run(program, args)?;
        check_exit(&output, program)?;
        Ok(output.stdout.trim().into())
    }

    fn prepare(&mut self) -> Result<()> {
        self.target.validate()?;
        let pools = self.command("zpool", &["list", "-H", "-o", "name,guid"])?;
        ensure!(
            !pools.lines().any(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                fields.first() == Some(&self.target.pool_name.as_str())
                    || fields.get(1) == Some(&self.target.pool_guid.as_str())
            }),
            "The installed pool is already imported. Close its other users and export it before opening this session."
        );
        let dir = self.directory.to_string_lossy();
        self.command(
            "zpool",
            &[
                "import",
                "-N",
                "-R",
                &dir,
                "-o",
                "cachefile=none",
                &self.target.pool_guid,
            ],
        )?;
        self.imported = true;
        let guid = self.command(
            "zpool",
            &["get", "-H", "-o", "value", "guid", &self.target.pool_name],
        )?;
        ensure!(
            guid == self.target.pool_guid,
            "Imported pool identity changed"
        );

        let be = BootEnvironment::new(&self.target.pool_name, &self.target.dataset_prefix);
        let encryption_root = self.command(
            "zfs",
            &["get", "-H", "-o", "value", "encryptionroot", &be.root()],
        )?;
        if encryption_root != "-" {
            let key = self.command(
                "zfs",
                &["get", "-H", "-o", "value", "keystatus", &be.root()],
            )?;
            if key != "available" {
                println!("Unlock the installed system. The passphrase is not saved.");
                let status = crate::console_session::wait_command(Command::new("zfs").args([
                    "load-key",
                    "-L",
                    "prompt",
                    &encryption_root,
                ]))?;
                ensure!(status.success(), "Could not unlock the installed system");
            }
        }

        // These are the datasets the installer creates/mounts, not every BE in the pool.
        for dataset in archinstall_zfs_core::dataset_layout::default_datasets() {
            let path = dataset
                .properties
                .iter()
                .find(|(name, _)| name == "mountpoint")
                .unwrap()
                .1
                .trim_start_matches('/');
            let mount = self.directory.join(path);
            self.safe_mount_directory(&mount)?;
            let name = be.child(&dataset.name);
            let actual = self.command("zfs", &["get", "-H", "-o", "value", "mountpoint", &name])?;
            ensure!(
                Path::new(&actual) == mount,
                "Unexpected mountpoint for {name}: {actual}"
            );
            self.command("zfs", &["mount", &name])?;
            self.mounted = true;
        }
        let efi = self.directory.join("boot/efi");
        self.safe_mount_directory(&efi)?;
        self.command(
            "mount",
            &[
                "-t",
                "vfat",
                &format!("/dev/disk/by-partuuid/{}", self.target.efi_partuuid),
                &efi.to_string_lossy(),
            ],
        )?;
        ensure!(
            self.directory.join("bin/bash").exists(),
            "Installed shell /bin/bash is missing"
        );
        Ok(())
    }

    fn safe_mount_directory(&self, path: &Path) -> Result<()> {
        // Refuse symlinks before creating directories inside an installed root.
        let relative = path.strip_prefix(&self.directory)?;
        let mut current = self.directory.clone();
        for part in relative.components() {
            current.push(part);
            if let Ok(metadata) = std::fs::symlink_metadata(&current) {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "Unsafe mount directory: {}",
                    current.display()
                );
            } else {
                std::fs::create_dir(&current)?;
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        let be = BootEnvironment::new(&self.target.pool_name, &self.target.dataset_prefix);
        let mut mounts = archinstall_zfs_core::zfs_cleanup::OwnedMounts {
            root: self.directory.clone(),
            root_dataset: be.root(),
            pool_to_export: self.imported.then(|| self.target.pool_name.clone()),
        };
        mounts.cleanup(self.runner)?;
        self.mounted = false;
        self.imported = false;
        std::fs::remove_dir(&self.directory)?;
        Ok(())
    }

    fn shell(&self) -> Result<()> {
        let unit = format!("azfs-shell-{}.service", std::process::id());
        println!("\n================================================================");
        println!("  INSTALLED SYSTEM — MAINTENANCE SHELL");
        println!(
            "  Target: {}/{}",
            self.target.pool_name, self.target.dataset_prefix
        );
        println!("----------------------------------------------------------------");
        println!("  DO NOT REBOOT OR POWER OFF FROM THIS SHELL.");
        println!("  Type exit to return to the installer, then choose Reboot there.");
        println!("  After exit, the installer will unmount filesystems and export");
        println!("  the pool automatically. Wait for the completion screen.");
        println!("  Background jobs started here will be stopped when you exit.");
        println!("================================================================\n");
        // systemd owns the entire process tree, including double-forked shell jobs.
        // Its PTY gives bash proper terminal semantics and foreground job control.
        let result = crate::console_session::wait_command(Command::new("systemd-run").args([
            "--quiet",
            "--pty",
            "--wait",
            "--collect",
            "--service-type=exec",
            "--unit",
            &unit,
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=10s",
            "--",
            "arch-chroot",
            &self.directory.to_string_lossy(),
            "/bin/bash",
            "--login",
        ]));
        // If the PTY client was interrupted, the service may still be running.
        // Stop the whole cgroup before attempting to unmount the session.
        let stopped = self.runner.run("systemctl", &["stop", &unit])?;
        if !stopped.success() {
            let state =
                self.command("systemctl", &["show", "-p", "LoadState", "--value", &unit])?;
            ensure!(
                state == "not-found",
                "Could not stop maintenance session {unit}"
            );
        }
        let status = result?;
        ensure!(status.success(), "Shell session finished with {status}");
        Ok(())
    }
}

pub fn run(target: &InstalledSystem) -> Result<String> {
    let directory = tempfile::Builder::new()
        .prefix("azfs-shell-")
        .tempdir_in("/run")?
        .keep();
    let mut session = Session {
        runner: &RealRunner,
        target,
        directory,
        imported: false,
        mounted: false,
    };
    let result = session.prepare().and_then(|()| session.shell());
    println!("\nCleaning up the installed-system session...");
    while let Err(error) = session.cleanup() {
        eprintln!(
            "Cleanup incomplete: {error}\nMount directory: {}\nResolve any processes holding this pool from another console, then press Enter to retry. Type shell to reopen it when still mounted.",
            session.directory.display()
        );
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer)? == 0 {
            bail!(
                "Cleanup incomplete; retained {}. Original error: {error}",
                session.directory.display()
            );
        }
        if answer.trim() == "shell"
            && session.mounted
            && let Err(error) = session.shell()
        {
            eprintln!("{error}");
        }
    }
    match result {
        Ok(()) => Ok(
            "Shell closed. Session mounts were released; the installed pool was exported.".into(),
        ),
        Err(error) => Ok(format!(
            "Installation is complete. Maintenance session: {error}. Session cleanup completed."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archinstall_zfs_core::system::cmd::CmdOutput;
    use std::sync::Mutex;

    struct ExistingPool(Mutex<Vec<String>>);
    impl CommandRunner for ExistingPool {
        fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput> {
            self.0
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            match (program, args.first().copied()) {
                ("zpool", Some("list")) => Ok(CmdOutput {
                    stdout: "pool\t123\n".into(),
                    stderr: "".into(),
                    exit_code: 0,
                }),
                ("findmnt", _) => Ok(CmdOutput {
                    stdout: "".into(),
                    stderr: "".into(),
                    exit_code: 1,
                }),
                _ => panic!("unexpected resource mutation: {program} {args:?}"),
            }
        }
        fn run_with_stdin(&self, _: &str, _: &[&str], _: &[u8]) -> Result<CmdOutput> {
            panic!("unexpected stdin command")
        }
    }

    #[test]
    fn preparation_refuses_existing_pool_without_exporting_it() {
        let runner = ExistingPool(Mutex::new(vec![]));
        let target = InstalledSystem {
            pool_name: "pool".into(),
            pool_guid: "123".into(),
            dataset_prefix: "arch0".into(),
            efi_partuuid: "abcd".into(),
        };
        let directory = tempfile::tempdir().unwrap().keep();
        let mut session = Session {
            runner: &runner,
            target: &target,
            directory,
            imported: false,
            mounted: false,
        };
        assert!(session.prepare().is_err());
        session.cleanup().unwrap();
        assert_eq!(runner.0.lock().unwrap().len(), 2);
    }

    #[test]
    fn refuses_symlink_mount_directories_without_writing_through_them() {
        let runner = ExistingPool(Mutex::new(vec![]));
        let target = InstalledSystem {
            pool_name: "pool".into(),
            pool_guid: "123".into(),
            dataset_prefix: "arch0".into(),
            efi_partuuid: "abcd".into(),
        };
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("boot")).unwrap();
        let session = Session {
            runner: &runner,
            target: &target,
            directory: root.path().into(),
            imported: false,
            mounted: false,
        };
        assert!(
            session
                .safe_mount_directory(&root.path().join("boot/efi"))
                .is_err()
        );
        assert!(!outside.path().join("efi").exists());
        assert!(runner.0.lock().unwrap().is_empty());
    }
}
