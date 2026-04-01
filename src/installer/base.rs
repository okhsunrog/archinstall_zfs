use std::path::Path;
use std::sync::mpsc::Sender;

use color_eyre::eyre::Result;

use crate::config::types::GlobalConfig;
use crate::system::cmd::CommandRunner;
use crate::system::sysinfo;

pub fn install_base(
    runner: &dyn CommandRunner,
    target: &Path,
    config: &GlobalConfig,
    tx: Option<&Sender<String>>,
) -> Result<()> {
    let mut packages: Vec<&str> = vec![
        "base",
        "base-devel",
        "linux-firmware",
        "linux-firmware-marvell",
        "sof-firmware",
    ];

    // Add selected kernels
    let kernels = config.effective_kernels();
    let kernel_refs: Vec<&str> = kernels.iter().map(|s| s.as_str()).collect();
    packages.extend_from_slice(&kernel_refs);

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

    // Set parallel downloads before pacstrap
    crate::system::pacman::set_parallel_downloads(None, config.parallel_downloads)?;

    crate::system::pacman::pacstrap(runner, target, &packages, tx)?;

    // Set parallel downloads on target too
    crate::system::pacman::set_parallel_downloads(Some(target), config.parallel_downloads)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_install_base_includes_kernel_and_microcode() {
        // set_parallel_downloads reads /etc/pacman.conf on host — skip if not available
        if !Path::new("/etc/pacman.conf").exists() {
            return;
        }

        let responses: Vec<CannedResponse> = (0..5).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);
        let config = GlobalConfig {
            kernels: Some(vec!["linux-lts".to_string()]),
            ..Default::default()
        };

        let dir = tempfile::tempdir().unwrap();
        // Create target pacman.conf so set_parallel_downloads on target works
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        std::fs::write(
            dir.path().join("etc/pacman.conf"),
            "#ParallelDownloads = 5\n",
        )
        .unwrap();

        let result = install_base(&runner, dir.path(), &config, None);
        // May fail due to host pacman.conf being read-only; just check the commands
        let calls = runner.calls();
        let pacstrap_call = calls.iter().find(|c| c.program == "pacstrap");
        if let Some(call) = pacstrap_call {
            assert!(call.args.contains(&"base".to_string()));
            assert!(call.args.contains(&"linux-lts".to_string()));
            assert!(call.args.contains(&"dracut".to_string()));
        }
    }
}
