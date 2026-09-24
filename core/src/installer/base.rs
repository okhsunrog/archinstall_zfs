use std::path::Path;
use std::sync::Arc;

use color_eyre::eyre::Result;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::types::GlobalConfig;
use crate::distro::Apt;
use crate::system::alpm_pacman::{AlpmContext, TargetMounts};
use crate::system::apt::AptTarget;
use crate::system::async_download::DownloadProgress;
use crate::system::cmd::CommandRunner;
use crate::system::sysinfo;

/// What a base installation installs: the distribution's base, the kernels,
/// the initramfs generator and the processor's microcode.
fn base_packages(config: &GlobalConfig) -> Result<Vec<&str>> {
    let distro = config.distribution();
    let mut packages: Vec<&str> = distro.base_packages.to_vec();

    // Add selected kernels
    packages.extend(config.effective_kernels());

    let Some(initramfs) = distro.packages.initramfs(config.init_system) else {
        color_eyre::eyre::bail!(
            "{} does not offer {:?} for the initramfs",
            distro.display_name,
            config.init_system
        );
    };
    packages.extend_from_slice(initramfs);

    if let Some(ucode) = distro.packages.microcode(sysinfo::cpu_vendor()) {
        packages.push(ucode);
    }
    Ok(packages)
}

/// Install base system packages into target.
/// Returns `TargetMounts` which must be kept alive for the duration of the
/// installation — dropping it unmounts API filesystems (proc, sys, dev, etc.).
pub fn install_base(
    target: &Path,
    config: &GlobalConfig,
    cancel: &CancellationToken,
    progress_tx: Option<
        std::sync::Arc<tokio::sync::watch::Sender<crate::system::async_download::DownloadProgress>>,
    >,
) -> Result<TargetMounts> {
    let packages = base_packages(config)?;

    // Set parallel downloads on host before installing
    crate::system::conf::set_parallel_downloads(None, config.parallel_downloads)?;

    // Mount API filesystems — returned to caller to keep alive
    let target_mounts = TargetMounts::setup(target)?;

    let pacman_conf = Path::new("/etc/pacman.conf");
    let mut ctx = AlpmContext::for_target(target, pacman_conf, config.download_config())?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&packages, cancel, progress_tx)?;
    ctx.finalize_target()?;
    // ctx (AlpmContext) drops here — that's fine, just releases the alpm handle.
    // target_mounts stays alive via the return value.

    // Set parallel downloads on target too
    crate::system::conf::set_parallel_downloads(Some(target), config.parallel_downloads)?;

    Ok(target_mounts)
}

// Note: install_base now uses AlpmContext directly (libalpm) instead of
// shelling out to pacstrap. It can only be tested with a real pacman
// environment (QEMU). The package list construction logic is straightforward
// enough that unit testing the full flow is not necessary.

/// Install a Debian base system: debootstrap, then everything else the base
/// phase installs with apt inside the target.
///
/// Returns the mounts to keep alive and the apt handle the remaining phases
/// install through.
pub fn install_base_apt(
    runner: Arc<dyn CommandRunner>,
    target: &Path,
    config: &GlobalConfig,
    apt: &Apt,
    cancel: &CancellationToken,
    progress_tx: Option<Arc<watch::Sender<DownloadProgress>>>,
) -> Result<(TargetMounts, AptTarget)> {
    let mut packages = base_packages(config)?;
    // The ZFS module can only be built with DKMS here, so each kernel's
    // headers go in with it: DKMS then builds for every kernel it finds as
    // soon as the ZFS phase installs the module.
    let distro = config.distribution();
    for kernel in config.effective_kernels() {
        if let Some(info) = crate::kernel::get_kernel_info(distro, kernel) {
            packages.push(info.headers_package);
        }
    }

    crate::system::apt::bootstrap(&*runner, target, apt, cancel)?;
    let mounts = TargetMounts::mount(target)?;
    let apt_target = AptTarget::new(runner, target);
    apt_target.update(cancel)?;
    apt_target.install(&packages, cancel, progress_tx.as_ref())?;
    Ok((mounts, apt_target))
}
