//! Installing a Debian target: debootstrap from the live medium, then apt
//! inside the target.
//!
//! apt runs in the chroot rather than being driven from the medium. The
//! target's own apt is always the version its packages expect, and its
//! hooks — DKMS builds, initramfs triggers — run the way they will on every
//! later upgrade. Progress comes from `APT::Status-Fd`, apt's machine-readable
//! status stream, pointed at standard output.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::eyre::{Context, Result, bail};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::async_download::PackageProgress;
use super::cmd::{CommandRunner, check_exit};
use crate::distro::Apt;

const SOURCES_PATH: &str = "etc/apt/sources.list.d/debian.sources";
/// debootstrap writes the release alone here; the deb822 file replaces it.
const LEGACY_SOURCES_PATH: &str = "etc/apt/sources.list";
const PREFERENCES_PATH: &str = "etc/apt/preferences.d/archinstall-zfs";

/// Options for every apt-get run. Configuration files a package ships are
/// kept as the installer wrote them, and nothing waits for an answer.
const APT_OPTIONS: &[&str] = &[
    "-y",
    "-q",
    "-o",
    "APT::Status-Fd=1",
    "-o",
    "Dpkg::Use-Pty=0",
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

/// Install the release into `target` with debootstrap, and point apt at
/// every suite the installation takes packages from.
pub fn bootstrap(
    runner: &dyn CommandRunner,
    target: &Path,
    apt: &Apt,
    cancel: &CancellationToken,
) -> Result<()> {
    let target_str = target.to_string_lossy();
    let components = format!("--components={}", apt.components.join(","));
    let keyring = format!("--keyring={}", apt.keyring);
    tracing::info!(
        suite = apt.suite,
        mirror = apt.mirror,
        "running debootstrap"
    );
    let output = runner.run_cancellable(
        "debootstrap",
        &[
            "--arch=amd64",
            &components,
            &keyring,
            apt.suite,
            &target_str,
            apt.mirror,
        ],
        cancel,
    )?;
    check_exit(&output, "debootstrap")?;

    write_sources(target, apt)
}

/// The deb822 sources and the pins, replacing what debootstrap left.
fn write_sources(target: &Path, apt: &Apt) -> Result<()> {
    let write = |path: &str, content: String| -> Result<()> {
        let path = target.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content).wrap_err_with(|| format!("cannot write {}", path.display()))
    };
    write(SOURCES_PATH, apt.sources())?;
    write(PREFERENCES_PATH, apt.preferences())?;
    match std::fs::remove_file(target.join(LEGACY_SOURCES_PATH)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).wrap_err("cannot remove the release-only sources.list"),
    }
    tracing::info!("apt sources and pins written");
    Ok(())
}

/// apt inside an installed target.
pub struct AptTarget {
    runner: Arc<dyn CommandRunner>,
    root: PathBuf,
}

impl AptTarget {
    pub fn new(runner: Arc<dyn CommandRunner>, root: &Path) -> Self {
        Self {
            runner,
            root: root.to_path_buf(),
        }
    }

    /// `apt-get <args>` in the target, handing each status line to
    /// `on_status`.
    fn apt_get(
        &self,
        args: &[&str],
        cancel: &CancellationToken,
        on_status: &mut dyn FnMut(Status<'_>),
    ) -> Result<super::cmd::CmdOutput> {
        let root = self.root.to_string_lossy();
        let mut full: Vec<&str> = vec![&root, "env", "DEBIAN_FRONTEND=noninteractive", "apt-get"];
        full.extend_from_slice(APT_OPTIONS);
        full.extend_from_slice(args);
        self.runner
            .run_streaming("arch-chroot", &full, cancel, &mut |line| {
                if let Some(status) = Status::parse(line) {
                    on_status(status);
                }
            })
    }

    pub fn update(&self, cancel: &CancellationToken) -> Result<()> {
        let output = self.apt_get(&["update"], cancel, &mut |_| {})?;
        check_exit(&output, "apt-get update")
    }

    /// Install `packages` in one transaction.
    pub fn install(
        &self,
        packages: &[&str],
        cancel: &CancellationToken,
        progress_tx: Option<&Arc<watch::Sender<PackageProgress>>>,
    ) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        if let Some(name) = packages.iter().find(|name| name.starts_with('-')) {
            bail!("'{name}' is not a package name");
        }
        // apt's status stream gives an overall percentage and a package name
        // but no count; a simulated run supplies the count the progress
        // display shows.
        let total = self.count_changes(packages, cancel)?;
        tracing::info!(count = packages.len(), total, "installing with apt");

        let mut install = vec!["install"];
        install.extend_from_slice(packages);
        let mut last_error = None;
        let output = self.apt_get(&install, cancel, &mut |status| match status {
            Status::Package {
                package, percent, ..
            } => {
                if let Some(tx) = progress_tx {
                    tx.send_replace(PackageProgress::Installing {
                        package: package.to_string(),
                        current: ((percent / 100.0) * total as f32).round() as usize,
                        total,
                        percent: percent as u32,
                    });
                }
            }
            Status::Download { message, .. } => {
                tracing::debug!(target: "apt.download", "{message}");
            }
            Status::Error { package, message } => {
                tracing::error!(package, "{message}");
                last_error = Some(format!("{package}: {message}"));
            }
        })?;
        if let Some(tx) = progress_tx {
            tx.send_replace(PackageProgress::Done);
        }
        if !output.success() {
            let detail = last_error.unwrap_or_else(|| output.stderr.trim().to_string());
            bail!(
                "apt-get install failed (exit {}): {detail}",
                output.exit_code
            );
        }
        Ok(())
    }

    /// How many packages the transaction would unpack.
    fn count_changes(&self, packages: &[&str], cancel: &CancellationToken) -> Result<usize> {
        let mut simulate = vec!["--simulate", "install"];
        simulate.extend_from_slice(packages);
        let output = self.apt_get(&simulate, cancel, &mut |_| {})?;
        check_exit(&output, "apt-get --simulate install")?;
        Ok(output
            .stdout
            .lines()
            .filter(|line| line.starts_with("Inst "))
            .count())
    }
}

/// One line of apt's status stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status<'a> {
    Download {
        percent: f32,
        message: &'a str,
    },
    Package {
        package: &'a str,
        percent: f32,
        message: &'a str,
    },
    Error {
        package: &'a str,
        message: &'a str,
    },
}

impl<'a> Status<'a> {
    /// Parse `kind:subject:percent:message`.
    ///
    /// The subject may itself contain colons — a package qualified by its
    /// architecture is `libc6:amd64` — and so may the message, so the
    /// percentage is found as the first field after the kind that reads as
    /// a number, rather than by position.
    pub fn parse(line: &'a str) -> Option<Self> {
        let (kind, rest) = line.split_once(':')?;
        if !matches!(kind, "dlstatus" | "pmstatus" | "pmerror") {
            return None;
        }
        let mut offset = 0;
        let (subject, percent, message) = loop {
            let colon = offset + rest[offset..].find(':')?;
            let after = &rest[colon + 1..];
            let (field, message) = after.split_once(':')?;
            if let Ok(percent) = field.parse::<f32>() {
                break (&rest[..colon], percent, message);
            }
            offset = colon + 1;
        };
        Some(match kind {
            "dlstatus" => Self::Download { percent, message },
            "pmstatus" => Self::Package {
                package: subject,
                percent,
                message,
            },
            _ => Self::Error {
                package: subject,
                message,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn package_status_is_parsed() {
        assert_eq!(
            Status::parse("pmstatus:zfs-dkms:45.4545:Configuring zfs-dkms (amd64)"),
            Some(Status::Package {
                package: "zfs-dkms",
                percent: 45.4545,
                message: "Configuring zfs-dkms (amd64)",
            })
        );
    }

    #[test]
    fn an_architecture_qualified_package_keeps_its_qualifier() {
        assert_eq!(
            Status::parse("pmstatus:libc6:amd64:10:Unpacking libc6:amd64"),
            Some(Status::Package {
                package: "libc6:amd64",
                percent: 10.0,
                message: "Unpacking libc6:amd64",
            })
        );
    }

    #[test]
    fn download_and_error_lines_are_told_apart() {
        assert_eq!(
            Status::parse("dlstatus:3:23.6:Retrieving file 3 of 12"),
            Some(Status::Download {
                percent: 23.6,
                message: "Retrieving file 3 of 12",
            })
        );
        assert_eq!(
            Status::parse("pmerror:/var/cache/apt/archives/x.deb:50:trying to overwrite"),
            Some(Status::Error {
                package: "/var/cache/apt/archives/x.deb",
                message: "trying to overwrite",
            })
        );
    }

    #[test]
    fn ordinary_output_is_not_status() {
        for line in [
            "Reading package lists...",
            "Setting up zfs-dkms (2.4.4-1~bpo13+1) ...",
            "pmstatus:no-percentage-here",
            "pmconffile:/etc/foo:'old' 'new'",
            "",
        ] {
            assert_eq!(Status::parse(line), None, "{line}");
        }
    }

    #[test]
    fn debootstrap_installs_the_release_with_its_components() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("etc/apt")).unwrap();
        std::fs::write(dir.path().join(LEGACY_SOURCES_PATH), "deb x trixie main\n").unwrap();
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        let apt = crate::distro::DEBIAN.apt().unwrap();

        bootstrap(&runner, dir.path(), apt, &CancellationToken::new()).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "debootstrap");
        assert_eq!(
            calls[0].args,
            [
                "--arch=amd64",
                "--components=main,contrib,non-free-firmware",
                "--keyring=/usr/share/keyrings/debian-archive-keyring.gpg",
                "trixie",
                &dir.path().to_string_lossy(),
                "http://deb.debian.org/debian",
            ]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(SOURCES_PATH)).unwrap(),
            apt.sources()
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(PREFERENCES_PATH)).unwrap(),
            apt.preferences()
        );
        assert!(
            !dir.path().join(LEGACY_SOURCES_PATH).exists(),
            "two source files would list the release twice"
        );
    }

    #[test]
    fn a_failed_debootstrap_writes_no_sources() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 1,
            stderr: "E: Couldn't download release".into(),
            ..Default::default()
        }]);
        let apt = crate::distro::DEBIAN.apt().unwrap();

        let error = bootstrap(&runner, dir.path(), apt, &CancellationToken::new()).unwrap_err();

        assert!(error.to_string().contains("debootstrap"), "{error}");
        assert!(!dir.path().join(SOURCES_PATH).exists());
    }

    #[test]
    fn install_runs_apt_get_in_the_target_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(RecordingRunner::new(vec![
            CannedResponse {
                stdout: "Inst a (1 trixie)\nInst b (2 trixie)\nConf a (1 trixie)\n".into(),
                ..Default::default()
            },
            CannedResponse {
                stdout: "pmstatus:a:50:Installing a\npmstatus:b:100:Installed b\n".into(),
                ..Default::default()
            },
        ]));
        let (tx, mut rx) = watch::channel(PackageProgress::default());
        let tx = Arc::new(tx);
        let apt = AptTarget::new(runner.clone(), dir.path());

        apt.install(&["a", "b"], &CancellationToken::new(), Some(&tx))
            .unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        for call in &calls {
            assert_eq!(call.program, "arch-chroot");
            assert_eq!(call.args[0], dir.path().to_string_lossy());
            assert_eq!(
                call.args[1..4],
                ["env", "DEBIAN_FRONTEND=noninteractive", "apt-get"]
            );
        }
        assert!(calls[0].args.contains(&"--simulate".to_string()));
        assert!(
            calls[1]
                .args
                .ends_with(&["install".into(), "a".into(), "b".into()])
        );
        assert!(matches!(*rx.borrow_and_update(), PackageProgress::Done));
    }

    #[test]
    fn a_failed_install_names_the_package_apt_blamed() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                stdout: "pmerror:zfs-dkms:80:installed zfs-dkms post-installation script \
                         subprocess returned error exit status 10\n"
                    .into(),
                exit_code: 100,
                ..Default::default()
            },
        ]));
        let apt = AptTarget::new(runner, dir.path());

        let error = apt
            .install(&["zfs-dkms"], &CancellationToken::new(), None)
            .unwrap_err()
            .to_string();

        assert!(error.contains("zfs-dkms: installed zfs-dkms"), "{error}");
    }

    #[test]
    fn an_option_is_not_taken_for_a_package() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(RecordingRunner::new(Vec::new()));
        let apt = AptTarget::new(runner.clone(), dir.path());

        assert!(
            apt.install(
                &["--allow-unauthenticated"],
                &CancellationToken::new(),
                None
            )
            .is_err()
        );
        assert!(runner.calls().is_empty());
    }
}
