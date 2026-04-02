use std::path::Path;

use color_eyre::eyre::Result;

use crate::config::types::GlobalConfig;
use crate::system::alpm_pacman::AlpmContext;
use crate::system::cmd::CommandRunner;
use crate::system::sysinfo;

pub fn install_base(
    _runner: &dyn CommandRunner,
    target: &Path,
    config: &GlobalConfig,
) -> Result<()> {
    let mut packages: Vec<&str> = vec![
        "base",
        "base-devel",
        "linux-firmware",
        "linux-firmware-marvell",
        "sof-firmware",
    ];

    // Add selected kernels (fall back to primary if none configured)
    let kernels = config.effective_kernels();
    if kernels.is_empty() {
        packages.push(config.primary_kernel());
    } else {
        packages.extend(kernels.iter().map(|s| s.as_str()));
    }

    // Add initramfs package
    let initramfs_pkg = match config.init_system {
        crate::config::types::InitSystem::Dracut => "dracut",
        crate::config::types::InitSystem::Mkinitcpio => "mkinitcpio",
    };
    packages.push(initramfs_pkg);

    // Microcode
    if let Some(ucode) = sysinfo::cpu_vendor().microcode_package() {
        packages.push(ucode);
    }

    // Set parallel downloads on host before installing
    crate::system::pacman::set_parallel_downloads(None, config.parallel_downloads)?;

    let pacman_conf = Path::new("/etc/pacman.conf");
    let mut ctx = AlpmContext::for_target(target, pacman_conf)?;
    ctx.sync_databases(false)?;
    ctx.install_packages(&packages)?;
    ctx.finalize_target()?;

    // Set parallel downloads on target too
    crate::system::pacman::set_parallel_downloads(Some(target), config.parallel_downloads)?;

    Ok(())
}

// Note: install_base now uses AlpmContext directly (libalpm) instead of
// shelling out to pacstrap. It can only be tested with a real pacman
// environment (QEMU). The package list construction logic is straightforward
// enough that unit testing the full flow is not necessary.
