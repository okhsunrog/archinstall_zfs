use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use alpm::{Alpm, DownloadEvent, LogLevel, TransFlag};
use color_eyre::eyre::{Context, Result, bail, eyre};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::async_download::{DownloadConfig, DownloadProgress, DownloadTask};

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
        prepare_pacman_dirs(target)?;
        Self::mount(target)
    }

    /// Mount the API filesystems alone, for a target that is not a pacman
    /// root.
    pub fn mount(target: &Path) -> Result<Self> {
        prepare_api_dirs(target)?;
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

/// When the live system's databases were last synced. Every handle that
/// syncs takes `db.lck`, and the package search opens a handle per
/// keystroke, so two searches typed a second apart used to fight over the
/// lock and one of them failed. One sync at a time, and none while the last
/// one is fresh.
static LIVE_SYNC: Mutex<Option<Instant>> = Mutex::new(None);
const LIVE_SYNC_FRESH: Duration = Duration::from_secs(600);

/// Sync the live system's databases through `handle`, unless another
/// handle did so recently.
pub fn sync_live_databases(handle: &mut Alpm, force: bool) -> Result<()> {
    let mut last = LIVE_SYNC
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !force && last.is_some_and(|at| at.elapsed() < LIVE_SYNC_FRESH) {
        return Ok(());
    }
    tracing::info!("syncing package databases");
    handle
        .syncdbs_mut()
        .update(force)
        .map_err(|e| eyre!("failed to sync databases: {e}"))?;
    *last = Some(Instant::now());
    Ok(())
}

/// What a transaction needs beyond the packages themselves: hooks, the
/// local database and the files pacman writes while unpacking.
const INSTALL_SLACK: u64 = 512 * 1024 * 1024;

/// Refuse a transaction that cannot fit, before anything is downloaded.
///
/// The estimate is the sum of the packages' installed sizes plus slack;
/// files an upgrade replaces are counted twice, which errs towards asking
/// for more room than the transaction will use.
fn check_free_space(root: &Path, installed_bytes: u64) -> Result<()> {
    let stat = match nix::sys::statvfs::statvfs(root) {
        Ok(stat) => stat,
        Err(error) => {
            tracing::warn!(%error, root = %root.display(), "cannot check free space");
            return Ok(());
        }
    };
    let free = stat.blocks_available() as u64 * stat.fragment_size() as u64;
    let needed = installed_bytes + INSTALL_SLACK;
    let gib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if free >= needed {
        tracing::info!(
            needed_gib = format!("{:.1}", gib(needed)),
            free_gib = format!("{:.1}", gib(free)),
            "the transaction fits"
        );
        return Ok(());
    }
    bail!(
        "Not enough space in {}: the packages need about {:.1} GiB and {:.1} GiB is free. Choose a larger allocation, or fewer packages, and start again.",
        root.display(),
        gib(needed),
        gib(free)
    )
}

/// The packages `name` stands for in `handle`'s sync databases: the package
/// of that name from the first repository that has it, every member of the
/// group of that name, or whatever provides it. A group is what the desktop
/// profiles list for their application suites; adding it as a package failed
/// the install. A provider is what `pacman -S` settles for when a package was
/// renamed upstream and keeps its old name in `provides`, or when the name is
/// virtual to begin with.
pub fn resolve_name<'a>(handle: &'a Alpm, name: &str) -> Result<Vec<&'a alpm::Package>> {
    for db in handle.syncdbs() {
        if let Ok(pkg) = db.pkg(name) {
            return Ok(vec![pkg]);
        }
    }
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for db in handle.syncdbs() {
        if let Ok(group) = db.group(name) {
            for pkg in group.packages() {
                if seen.insert(pkg.name().to_string()) {
                    members.push(pkg);
                }
            }
        }
    }
    if members.is_empty() {
        if let Some(pkg) = handle.syncdbs().find_satisfier(name) {
            tracing::info!(name, package = pkg.name(), "provided by a renamed package");
            return Ok(vec![pkg]);
        }
        bail!("package '{name}' not found in any repository")
    }
    tracing::info!(
        group = name,
        count = members.len(),
        "group expands to packages"
    );
    Ok(members)
}

/// Names that neither a package nor a group in `handle`'s sync databases
/// answers to, in the order given.
pub fn unknown_names<'a>(handle: &Alpm, names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| resolve_name(handle, name).is_err())
        .map(str::to_string)
        .collect()
}

/// Wraps an alpm handle for host or target installs.
/// Does NOT own API filesystem mounts — use `TargetMounts` for that.
pub struct AlpmContext {
    handle: Alpm,
    root: PathBuf,
    is_target: bool,
    download_config: DownloadConfig,
}

impl AlpmContext {
    /// Create a context for installing packages on the HOST system.
    pub fn for_host(pacman_conf_path: &Path, download_config: DownloadConfig) -> Result<Self> {
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
            download_config,
        };
        ctx.setup_callbacks();
        Ok(ctx)
    }

    /// Create a context for installing packages into a TARGET chroot.
    /// Requires that `TargetMounts::setup()` has already been called and
    /// the returned `TargetMounts` is kept alive for the duration.
    pub fn for_target(
        target: &Path,
        pacman_conf_path: &Path,
        download_config: DownloadConfig,
    ) -> Result<Self> {
        // A cache outside the target survives it, so a later installation can
        // reuse what this one downloaded.
        let cache_dir = match download_config.cache_dir.as_ref() {
            Some(shared) => {
                fs::create_dir_all(shared).wrap_err_with(|| {
                    format!("failed to create package cache: {}", shared.display())
                })?;
                format!("{}/", shared.display())
            }
            None => format!("{}/var/cache/pacman/pkg/", target.to_string_lossy()),
        };

        let mut handle = open_target_alpm(target, pacman_conf_path)?;

        // Override cache dir to target
        handle
            .set_cachedirs([cache_dir.as_str()].iter())
            .map_err(|e| eyre!("failed to set cache dir: {e}"))?;

        // Hook directories are host paths: libalpm does not prefix them
        // with the root, pacman's own front end does that. Given as bare
        // paths they named the live system's hooks, so every target
        // transaction ran archiso's mkinitcpio and dkms hooks (which failed
        // inside the chroot) and none of the target's own: no icon cache,
        // no MIME database, no GDK pixbuf loaders on the installed desktop.
        let hookdirs = [
            target.join("usr/share/libalpm/hooks/"),
            target.join("etc/pacman.d/hooks/"),
        ];
        handle
            .set_hookdirs(
                hookdirs
                    .iter()
                    .map(|dir| dir.to_string_lossy().into_owned()),
            )
            .map_err(|e| eyre!("failed to set hook dirs: {e}"))?;

        let ctx = Self {
            handle,
            root: target.to_path_buf(),
            is_target: true,
            download_config,
        };
        ctx.setup_callbacks();
        Ok(ctx)
    }

    /// Sync all registered databases (equivalent to `pacman -Sy`).
    pub fn sync_databases(&mut self, force: bool) -> Result<()> {
        if !self.is_target {
            return sync_live_databases(&mut self.handle, force);
        }
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
        self.install_packages_excluding(packages, &[], cancel, progress_tx)
    }

    /// Like `install_packages`, leaving out `excluded` members of any group
    /// among `packages`.
    pub fn install_packages_excluding(
        &mut self,
        packages: &[&str],
        excluded: &[&str],
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

        // Once the transaction is open it has to be released on every path.
        // Returning early through `?` used to leave it open and the database
        // locked, so a later install on the same handle failed with "a
        // transaction is already initialized" rather than the original error.
        let result = self.run_transaction(packages, excluded, cancel, progress_tx);
        let released = self
            .handle
            .trans_release()
            .map_err(|e| eyre!("failed to release transaction: {e}"));

        result.and(released)?;
        tracing::info!("packages installed successfully");
        Ok(())
    }

    /// The body of [`Self::install_packages`], between `trans_init` and
    /// `trans_release`.
    fn run_transaction(
        &mut self,
        packages: &[&str],
        excluded: &[&str],
        cancel: &CancellationToken,
        progress_tx: Option<std::sync::Arc<watch::Sender<DownloadProgress>>>,
    ) -> Result<()> {
        // Add each name from the sync databases: a package as itself, a
        // group (kde-applications, xfce4-goodies, gnome-extra) as its members
        // minus the excluded ones.
        for &name in packages {
            let members = self.resolve_name(name)?;
            let expanded = members.len() > 1;
            for pkg in members {
                if expanded && excluded.contains(&pkg.name()) {
                    tracing::debug!(group = name, package = pkg.name(), "left out of the group");
                    continue;
                }
                self.handle
                    .trans_add_pkg(pkg)
                    .map_err(|e| eyre!("failed to add package '{name}': {e}"))?;
            }
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
            return Ok(());
        }

        // pacman only notices a full filesystem when it commits, which is
        // after every package has been downloaded: a KDE install spent
        // 1.9 GiB of transfer before failing on a pool that could never
        // have held it. The resolved transaction knows its installed size,
        // so the same answer is available now.
        let installed_bytes: u64 = self
            .handle
            .trans_add()
            .iter()
            .map(|pkg| pkg.isize().max(0) as u64)
            .sum();
        check_free_space(&self.root, installed_bytes)?;

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
                self.download_config.concurrency,
                cancel.clone(),
                progress_tx.clone(),
            ))?;
        }

        tracing::info!("installing packages");
        let batch_start = std::time::Instant::now();
        let pkg_count = count;

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

        let batch_duration_ms = batch_start.elapsed().as_millis() as u64;
        tracing::info!(
            target: "metrics",
            event = "batch_install",
            count = pkg_count as u64,
            duration_ms = batch_duration_ms,
        );

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

    /// The packages `name` stands for: one, or every member of the group
    /// of that name across the repositories.
    fn resolve_name(&self, name: &str) -> Result<Vec<&alpm::Package>> {
        resolve_name(&self.handle, name)
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

/// Open a libalpm handle rooted at `target`, with the target's database and
/// the repositories from `pacman_conf_path` (the medium's own file while the
/// base system is being installed, the target's afterwards).
pub(crate) fn open_target_alpm(target: &Path, pacman_conf_path: &Path) -> Result<Alpm> {
    let conf =
        pacmanconf::Config::from_file(pacman_conf_path.to_str().unwrap_or("/etc/pacman.conf"))
            .wrap_err("failed to parse pacman.conf")?;

    let target_str = target.to_string_lossy();
    let db_path = format!("{}/var/lib/pacman", target_str);
    let mut handle = Alpm::new(target_str.as_ref(), &db_path)
        .map_err(|e| eyre!("failed to init alpm for target: {e}"))?;

    // Configure from the given config but with target paths.
    alpm_utils::configure_alpm(&mut handle, &conf)
        .map_err(|e| eyre!("failed to configure alpm for target: {e}"))?;
    Ok(handle)
}

/// Install packages from the target's own repositories, for the phases that
/// run outside the installer's own package handle.
pub fn install_into_target(
    target: &Path,
    packages: &[&str],
    cancel: &CancellationToken,
    download_config: DownloadConfig,
) -> Result<()> {
    let target_conf = target.join("etc/pacman.conf");
    let mut ctx = AlpmContext::for_target(target, &target_conf, download_config)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(packages, cancel, None)
}

// ── Target preparation ───────────────────────────────

fn prepare_pacman_dirs(target: &Path) -> Result<()> {
    let dirs = [
        "var/lib/pacman",
        "var/log",
        "var/cache/pacman/pkg",
        "etc/pacman.d",
    ];
    for dir in &dirs {
        fs::create_dir_all(target.join(dir)).wrap_err_with(|| format!("failed to create {dir}"))?;
    }
    Ok(())
}

fn prepare_api_dirs(target: &Path) -> Result<()> {
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

#[cfg(test)]
mod space_tests {
    use super::*;

    #[test]
    fn a_transaction_larger_than_the_filesystem_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = check_free_space(dir.path(), 1 << 60).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("Not enough space"), "{message}");
        assert!(message.contains("GiB is free"), "{message}");
    }

    #[test]
    fn a_small_transaction_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        check_free_space(dir.path(), 0).unwrap();
    }

    #[test]
    fn an_unreadable_path_does_not_stop_the_installation() {
        check_free_space(Path::new("/nonexistent-root-for-tests"), 1 << 30).unwrap();
    }
}
