use std::fs;
use std::path::{Path, PathBuf};

use alpm::{Alpm, DownloadEvent, LogLevel, SigLevel, TransFlag};
use color_eyre::eyre::{Context, Result, bail, eyre};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::async_download::{DownloadProgress, DownloadTask};

/// Manages API filesystem mounts (proc, sys, dev, etc.) for a target chroot.
/// Mounts are unmounted in reverse order on drop.
/// This struct should outlive any `AlpmContext` that uses the target.
pub struct TargetMounts {
    mounts: Vec<PathBuf>,
}

impl TargetMounts {
    /// Prepare target directories and mount API filesystems.
    /// The mounts persist until this struct is dropped.
    pub fn setup(target: &Path) -> Result<Self> {
        prepare_target_dirs(target)?;
        let mounts = mount_api_filesystems(target)?;
        Ok(Self { mounts })
    }
}

impl Drop for TargetMounts {
    fn drop(&mut self) {
        for mount_point in self.mounts.iter().rev() {
            if let Err(e) = nix::mount::umount2(mount_point, nix::mount::MntFlags::MNT_DETACH) {
                tracing::warn!(
                    path = %mount_point.display(),
                    error = %e,
                    "failed to unmount API filesystem"
                );
            }
        }
    }
}

/// Wraps an alpm handle for host or target installs.
/// Does NOT own API filesystem mounts — use `TargetMounts` for that.
pub struct AlpmContext {
    handle: Alpm,
    root: PathBuf,
    is_target: bool,
}

impl AlpmContext {
    /// Create a context for installing packages on the HOST system.
    pub fn for_host(pacman_conf_path: &Path) -> Result<Self> {
        let conf =
            pacmanconf::Config::from_file(pacman_conf_path.to_str().unwrap_or("/etc/pacman.conf"))
                .wrap_err("failed to parse pacman.conf")?;

        let mut handle = Alpm::new(conf.root_dir.as_str(), conf.db_path.as_str())
            .map_err(|e| eyre!("failed to init alpm: {e}"))?;

        alpm_utils::configure_alpm(&mut handle, &conf)
            .map_err(|e| eyre!("failed to configure alpm: {e}"))?;

        let ctx = Self {
            handle,
            root: PathBuf::from(&conf.root_dir),
            is_target: false,
        };
        ctx.setup_callbacks();
        Ok(ctx)
    }

    /// Create a context for installing packages into a TARGET chroot.
    /// Requires that `TargetMounts::setup()` has already been called and
    /// the returned `TargetMounts` is kept alive for the duration.
    pub fn for_target(target: &Path, pacman_conf_path: &Path) -> Result<Self> {
        let conf =
            pacmanconf::Config::from_file(pacman_conf_path.to_str().unwrap_or("/etc/pacman.conf"))
                .wrap_err("failed to parse pacman.conf")?;

        let target_str = target.to_string_lossy();
        let db_path = format!("{}/var/lib/pacman", target_str);
        let cache_dir = format!("{}/var/cache/pacman/pkg/", target_str);

        let mut handle = Alpm::new(target_str.as_ref(), &db_path)
            .map_err(|e| eyre!("failed to init alpm for target: {e}"))?;

        // Configure from host config but with target paths
        alpm_utils::configure_alpm(&mut handle, &conf)
            .map_err(|e| eyre!("failed to configure alpm for target: {e}"))?;

        // Override cache dir to target
        handle
            .set_cachedirs([cache_dir.as_str()].iter())
            .map_err(|e| eyre!("failed to set cache dir: {e}"))?;

        // Ensure hook directories are set (relative to root, libalpm prepends root)
        handle
            .set_hookdirs(["/usr/share/libalpm/hooks/", "/etc/pacman.d/hooks/"].iter())
            .map_err(|e| eyre!("failed to set hook dirs: {e}"))?;

        let ctx = Self {
            handle,
            root: target.to_path_buf(),
            is_target: true,
        };
        ctx.setup_callbacks();
        Ok(ctx)
    }

    /// Sync all registered databases (equivalent to `pacman -Sy`).
    pub fn sync_databases(&mut self, force: bool) -> Result<()> {
        tracing::info!("syncing package databases");
        self.handle
            .syncdbs_mut()
            .update(force)
            .map_err(|e| eyre!("failed to sync databases: {e}"))?;
        Ok(())
    }

    /// Install packages by name (equivalent to `pacman -S --needed`).
    ///
    /// Downloads are performed asynchronously via reqwest (parallel, cancellable),
    /// then `trans_commit()` finds them in cache and only does the install phase.
    pub fn install_packages(
        &mut self,
        packages: &[&str],
        cancel: &CancellationToken,
        progress_tx: Option<std::sync::Arc<watch::Sender<DownloadProgress>>>,
    ) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        tracing::info!(?packages, "installing packages via alpm");

        self.handle
            .trans_init(TransFlag::NEEDED)
            .map_err(|e| eyre!("failed to init transaction: {e}"))?;

        // Find and add each package from sync databases
        for &pkg_name in packages {
            let pkg = self.find_package(pkg_name)?;
            self.handle
                .trans_add_pkg(pkg)
                .map_err(|e| eyre!("failed to add package '{pkg_name}': {e}"))?;
        }

        // Prepare (resolve deps, check conflicts)
        self.handle.trans_prepare().map_err(|e| {
            let msg = format!("transaction prepare failed: {e}");
            // TODO: extract PrepareData for detailed error messages
            eyre!(msg)
        })?;

        let count = self.handle.trans_add().len();
        if count == 0 {
            // All packages already installed (NEEDED flag skipped them)
            tracing::info!("all packages already up to date");
            self.handle
                .trans_release()
                .map_err(|e| eyre!("failed to release transaction: {e}"))?;
            return Ok(());
        }

        tracing::info!(count, "transaction prepared, downloading packages");

        // Extract download tasks from resolved packages
        let tasks: Vec<DownloadTask> = self
            .handle
            .trans_add()
            .iter()
            .filter_map(|pkg| {
                let db = pkg.db()?;
                Some(DownloadTask {
                    filename: pkg.filename()?.to_string(),
                    servers: db.servers().iter().map(|s| s.to_string()).collect(),
                    sha256: pkg.sha256sum().map(|s| s.to_string()),
                    size: pkg.size(),
                })
            })
            .collect();

        if !tasks.is_empty() {
            let cache_dir = PathBuf::from(
                self.handle
                    .cachedirs()
                    .first()
                    .unwrap_or("/var/cache/pacman/pkg/"),
            );

            // Async download via the existing tokio runtime
            let rt = tokio::runtime::Handle::current();
            rt.block_on(super::async_download::download_packages(
                tasks,
                cache_dir,
                5,
                cancel.clone(),
                progress_tx.clone(),
            ))?;
        }

        tracing::info!("installing packages");

        // Set up install progress callback. We hold a clone of the sender
        // so we can emit a `Done` event after the transaction completes —
        // without it the GUI's progress bar stays stuck on the last
        // "Installing N/N: pkg 100%" until the install thread exits and
        // the channel sender is dropped.
        let progress_tx_clone = progress_tx.clone();
        if let Some(tx) = progress_tx {
            self.handle
                .set_progress_cb(tx, |_kind, pkgname, percent, howmany, current, tx| {
                    tx.send_replace(super::async_download::PackageProgress::Installing {
                        package: pkgname.to_string(),
                        current,
                        total: howmany,
                        percent: percent as u32,
                    });
                });
        }

        // Commit — libalpm finds packages in cache, skips download phase.
        // Always emit `Done` afterwards (success or failure) so the GUI
        // bar gets dismissed before subsequent phases run.
        let commit_result = self.handle.trans_commit().map_err(|e| {
            let msg = format!("transaction commit failed: {e}");
            eyre!(msg)
        });

        if let Some(tx) = &progress_tx_clone {
            tx.send_replace(super::async_download::PackageProgress::Done);
        }

        commit_result?;

        self.handle
            .trans_release()
            .map_err(|e| eyre!("failed to release transaction: {e}"))?;

        tracing::info!("packages installed successfully");
        Ok(())
    }

    /// Dynamically register an additional repo (e.g., archzfs after config edit).
    pub fn register_repo(
        &mut self,
        name: &str,
        servers: &[&str],
        siglevel: SigLevel,
    ) -> Result<()> {
        let db = self
            .handle
            .register_syncdb_mut(name, siglevel)
            .map_err(|e| eyre!("failed to register repo '{name}': {e}"))?;
        for &server in servers {
            db.add_server(server)
                .map_err(|e| eyre!("failed to add server to '{name}': {e}"))?;
        }
        tracing::info!(name, "registered additional repo");
        Ok(())
    }

    /// Copy keyring, mirrorlist, and pacman.conf from host to target.
    /// Call this once after the first install transaction.
    pub fn finalize_target(&self) -> Result<()> {
        if !self.is_target {
            return Ok(());
        }

        // Copy GPG keyring (using cp -a to handle sockets and special files)
        let src_gpg = Path::new("/etc/pacman.d/gnupg");
        let dst_gpg = self.root.join("etc/pacman.d/gnupg");
        if src_gpg.exists() && !dst_gpg.exists() {
            fs::create_dir_all(&dst_gpg)?;
            // Use cp -a --no-preserve=ownership like pacstrap does.
            // fs_extra::dir::copy fails on socket files (S.keyboxd).
            let status = std::process::Command::new("cp")
                .args(["-a", "--no-preserve=ownership"])
                .arg(format!("{}/.", src_gpg.display()))
                .arg(&dst_gpg)
                .status()
                .wrap_err("failed to run cp for keyring")?;
            if !status.success() {
                return Err(eyre!("failed to copy keyring (cp exited with {status})"));
            }
            tracing::info!("copied GPG keyring to target");
        }

        // Copy mirrorlist
        let src_mirror = Path::new("/etc/pacman.d/mirrorlist");
        let dst_mirror = self.root.join("etc/pacman.d/mirrorlist");
        if src_mirror.exists() {
            fs::create_dir_all(dst_mirror.parent().unwrap())?;
            fs::copy(src_mirror, &dst_mirror).wrap_err("failed to copy mirrorlist")?;
            tracing::info!("copied mirrorlist to target");
        }

        // Copy pacman.conf, commenting out DownloadUser (the user may not
        // exist in the target chroot yet — same as pacstrap's sed workaround)
        let src_conf = Path::new("/etc/pacman.conf");
        let dst_conf = self.root.join("etc/pacman.conf");
        if src_conf.exists() {
            let content = fs::read_to_string(src_conf).wrap_err("failed to read pacman.conf")?;
            let patched: String = content
                .lines()
                .map(|line| {
                    if line.trim_start().starts_with("DownloadUser") {
                        format!("#{line}")
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&dst_conf, patched).wrap_err("failed to write target pacman.conf")?;
            tracing::info!("copied pacman.conf to target (DownloadUser commented out)");
        }

        Ok(())
    }

    /// Get a reference to the underlying alpm handle.
    pub fn handle(&self) -> &Alpm {
        &self.handle
    }

    /// Get a mutable reference to the underlying alpm handle.
    pub fn handle_mut(&mut self) -> &mut Alpm {
        &mut self.handle
    }

    fn find_package(&self, name: &str) -> Result<&alpm::Package> {
        for db in self.handle.syncdbs() {
            if let Ok(pkg) = db.pkg(name) {
                return Ok(pkg);
            }
        }
        bail!("package '{name}' not found in any repository")
    }

    fn setup_callbacks(&self) {
        self.handle
            .set_dl_cb((), |filename, event, _| match event.event() {
                DownloadEvent::Progress(p) => {
                    tracing::trace!(
                        target: "pacman.download",
                        file = filename,
                        downloaded = p.downloaded,
                        total = p.total,
                    );
                }
                DownloadEvent::Completed(c) => {
                    tracing::debug!(
                        target: "pacman.download",
                        file = filename,
                        result = ?c.result,
                        "download complete"
                    );
                }
                _ => {}
            });

        self.handle.set_log_cb((), |level, msg, _| {
            let msg = msg.trim();
            if msg.is_empty() {
                return;
            }
            match level {
                LogLevel::ERROR | LogLevel::WARNING => {
                    tracing::warn!(target: "pacman", "{msg}");
                }
                _ => tracing::trace!(target: "pacman", "{msg}"),
            }
        });

        self.handle.set_event_cb((), |event, _| {
            use alpm::Event;
            match event.event() {
                Event::TransactionStart => tracing::info!("transaction starting"),
                Event::TransactionDone => tracing::info!("transaction complete"),
                Event::PkgRetrieveStart(_) => {
                    tracing::info!("retrieving package signatures...")
                }
                Event::PkgRetrieveDone(_) => tracing::info!("package signatures verified"),
                Event::IntegrityStart => tracing::info!("checking package integrity..."),
                Event::IntegrityDone => tracing::info!("integrity check complete"),
                Event::KeyringStart => tracing::info!("checking keyring..."),
                Event::KeyringDone => tracing::info!("keyring check complete"),
                Event::PackageOperationStart(op) => {
                    use alpm::PackageOperation;
                    match op.operation() {
                        PackageOperation::Install(pkg) => {
                            tracing::info!("installing {} {}", pkg.name(), pkg.version());
                        }
                        PackageOperation::Upgrade(old, new) => {
                            tracing::info!(
                                "upgrading {} {} -> {}",
                                old.name(),
                                old.version(),
                                new.version()
                            );
                        }
                        PackageOperation::Remove(pkg) => {
                            tracing::info!("removing {} {}", pkg.name(), pkg.version());
                        }
                        _ => {}
                    }
                }
                Event::HookRunStart(hook) => {
                    tracing::debug!("running hook: {}", hook.name());
                }
                _ => {}
            }
        });
    }
}

// ── Target preparation ───────────────────────────────

fn prepare_target_dirs(target: &Path) -> Result<()> {
    let dirs = [
        "var/lib/pacman",
        "var/log",
        "var/cache/pacman/pkg",
        "etc/pacman.d",
    ];
    for dir in &dirs {
        fs::create_dir_all(target.join(dir)).wrap_err_with(|| format!("failed to create {dir}"))?;
    }

    // These need specific permissions
    for (dir, mode) in [
        ("run", 0o755),
        ("dev", 0o755),
        ("sys", 0o555),
        ("proc", 0o555),
    ] {
        let path = target.join(dir);
        fs::create_dir_all(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
        }
    }

    let tmp = target.join("tmp");
    fs::create_dir_all(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o1777))?;
    }

    tracing::debug!("created target directories");
    Ok(())
}

fn mount_api_filesystems(target: &Path) -> Result<Vec<PathBuf>> {
    use nix::mount::{MsFlags, mount};

    let mut mounts = Vec::new();

    // (source, dest, fstype, flags, data) — matches pacstrap's chroot_setup()
    let mount_points: Vec<(&str, &str, &str, MsFlags, Option<&str>)> = vec![
        (
            "proc",
            "proc",
            "proc",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
            None,
        ),
        (
            "sysfs",
            "sys",
            "sysfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV | MsFlags::MS_RDONLY,
            None,
        ),
        (
            "devtmpfs",
            "dev",
            "devtmpfs",
            MsFlags::MS_NOSUID,
            Some("mode=0755"),
        ),
        (
            "devpts",
            "dev/pts",
            "devpts",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
            Some("mode=0620,gid=5"),
        ),
        (
            "tmpfs",
            "dev/shm",
            "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some("mode=1777"),
        ),
        (
            "tmpfs",
            "tmp",
            "tmpfs",
            MsFlags::MS_STRICTATIME | MsFlags::MS_NODEV | MsFlags::MS_NOSUID,
            Some("mode=1777"),
        ),
    ];

    for (source, dest, fstype, flags, data) in &mount_points {
        let mount_path = target.join(dest);
        fs::create_dir_all(&mount_path)?;
        mount(Some(*source), &mount_path, Some(*fstype), *flags, *data)
            .wrap_err_with(|| format!("failed to mount {fstype} on {dest}"))?;
        mounts.push(mount_path);
    }

    // Conditionally mount efivarfs (matches pacstrap behavior)
    let efivars_path = target.join("sys/firmware/efi/efivars");
    if efivars_path.is_dir() {
        mount(
            Some("efivarfs"),
            &efivars_path,
            Some("efivarfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
            None::<&str>,
        )
        .wrap_err("failed to mount efivarfs")?;
        mounts.push(efivars_path);
    }

    // Bind mount /run
    let run_path = target.join("run");
    mount(
        Some("/run"),
        &run_path,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .wrap_err("failed to bind mount /run")?;
    mounts.push(run_path);

    tracing::debug!("mounted API filesystems in target");
    Ok(mounts)
}
