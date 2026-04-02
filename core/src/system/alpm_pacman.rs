use std::fs;
use std::path::{Path, PathBuf};

use alpm::{Alpm, AnyDownloadEvent, AnyEvent, DownloadEvent, LogLevel, SigLevel, TransFlag};
use color_eyre::eyre::{Context, Result, bail, eyre};

/// Wraps an alpm handle with lifecycle management for host or target installs.
/// For target installs, manages API filesystem mounts (proc, sys, dev, etc.).
pub struct AlpmContext {
    handle: Alpm,
    root: PathBuf,
    is_target: bool,
    mounts: Vec<PathBuf>, // mounted paths, unmounted on drop (reverse order)
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

        let mut ctx = Self {
            handle,
            root: PathBuf::from(&conf.root_dir),
            is_target: false,
            mounts: Vec::new(),
        };
        ctx.setup_callbacks();
        Ok(ctx)
    }

    /// Create a context for installing packages into a TARGET chroot.
    /// Prepares directories and mounts API filesystems.
    pub fn for_target(target: &Path, pacman_conf_path: &Path) -> Result<Self> {
        // Prepare target directories (what pacstrap does)
        prepare_target_dirs(target)?;

        // Mount API filesystems
        let mounts = mount_api_filesystems(target)?;

        // Parse host pacman.conf for mirror/repo info
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

        let mut ctx = Self {
            handle,
            root: target.to_path_buf(),
            is_target: true,
            mounts,
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
    pub fn install_packages(&mut self, packages: &[&str]) -> Result<()> {
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

        let to_install: Vec<String> = self
            .handle
            .trans_add()
            .iter()
            .map(|p| format!("{} {}", p.name(), p.version()))
            .collect();
        tracing::info!(count = to_install.len(), "transaction prepared, installing");

        // Commit (download + install)
        self.handle.trans_commit().map_err(|e| {
            let msg = format!("transaction commit failed: {e}");
            eyre!(msg)
        })?;

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

        // Copy GPG keyring
        let src_gpg = Path::new("/etc/pacman.d/gnupg");
        let dst_gpg = self.root.join("etc/pacman.d/gnupg");
        if src_gpg.exists() && !dst_gpg.exists() {
            let options = fs_extra::dir::CopyOptions::new().copy_inside(true);
            fs_extra::dir::copy(src_gpg, &dst_gpg, &options)
                .map_err(|e| eyre!("failed to copy keyring: {e}"))?;
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

        // Copy pacman.conf
        let src_conf = Path::new("/etc/pacman.conf");
        let dst_conf = self.root.join("etc/pacman.conf");
        if src_conf.exists() {
            fs::copy(src_conf, &dst_conf).wrap_err("failed to copy pacman.conf")?;
            tracing::info!("copied pacman.conf to target");
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
            .set_progress_cb((), |progress, pkgname, percent, howmany, current, _| {
                tracing::info!(
                    target: "pacman.progress",
                    kind = ?progress,
                    package = pkgname,
                    percent,
                    total = howmany,
                    current,
                    "{pkgname} ({current}/{howmany}) {percent}%"
                );
            });

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
                LogLevel::ERROR => tracing::error!(target: "pacman", "{msg}"),
                LogLevel::WARNING => tracing::warn!(target: "pacman", "{msg}"),
                _ => tracing::trace!(target: "pacman", "{msg}"),
            }
        });

        self.handle.set_event_cb((), |event, _| {
            use alpm::Event;
            match event.event() {
                Event::TransactionStart => tracing::info!("transaction starting"),
                Event::TransactionDone => tracing::info!("transaction complete"),
                Event::PkgRetrieveStart(_) => tracing::info!("downloading packages..."),
                Event::PkgRetrieveDone(_) => tracing::info!("all packages downloaded"),
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

impl Drop for AlpmContext {
    fn drop(&mut self) {
        if self.is_target {
            // Unmount in reverse order
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

    let mount_points: Vec<(&str, &str, &str, MsFlags)> = vec![
        (
            "proc",
            "proc",
            "proc",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        ),
        (
            "sysfs",
            "sys",
            "sysfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV | MsFlags::MS_RDONLY,
        ),
        ("devtmpfs", "dev", "devtmpfs", MsFlags::MS_NOSUID),
        (
            "devpts",
            "dev/pts",
            "devpts",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        ),
        (
            "tmpfs",
            "dev/shm",
            "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        ),
        (
            "tmpfs",
            "tmp",
            "tmpfs",
            MsFlags::MS_STRICTATIME | MsFlags::MS_NODEV | MsFlags::MS_NOSUID,
        ),
    ];

    for (source, dest, fstype, flags) in &mount_points {
        let mount_path = target.join(dest);
        fs::create_dir_all(&mount_path)?;
        mount(
            Some(*source),
            &mount_path,
            Some(*fstype),
            *flags,
            None::<&str>,
        )
        .wrap_err_with(|| format!("failed to mount {fstype} on {dest}"))?;
        mounts.push(mount_path);
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
