use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail};

use archinstall_zfs_core::config::types::{
    GlobalConfig, InstallationMode, SwapMode, ZfsEncryptionMode,
};
use archinstall_zfs_core::system::cmd::{CommandRunner, RealRunner};

use crate::Cli;

pub fn run(cli: Cli) -> Result<()> {
    let config = if let Some(ref path) = cli.config {
        tracing::info!(path = %path.display(), "loading config from file");
        GlobalConfig::load_from_file(path)?
    } else {
        GlobalConfig::default()
    };

    if cli.silent {
        if cli.config.is_none() {
            bail!("--silent requires --config");
        }
        let errors = config.validate_for_install();
        if !errors.is_empty() {
            bail!("Config validation failed:\n  {}", errors.join("\n  "));
        }
        tracing::info!("silent mode: config valid, starting installation");
        let runner = RealRunner;
        return run_install(&runner, &config);
    }

    // Interactive TUI mode
    crate::tui::run_tui(config, cli.dry_run)
}

/// Full installation pipeline — used by both headless and TUI modes.
/// All progress is reported via tracing; the TUI captures it with a
/// custom tracing layer that forwards events to the progress screen.
pub fn run_install(runner: &dyn CommandRunner, config: &GlobalConfig) -> Result<()> {
    let mountpoint = PathBuf::from("/mnt");
    let mode = config.installation_mode.unwrap();
    let pool_name = config.pool_name.as_deref().unwrap();
    let prefix = &config.dataset_prefix;
    let compression = config.compression.to_string();
    let encryption = config.zfs_encryption_mode;
    let kernel = config.primary_kernel();

    // ── Phase 0: Pre-installation checks ───────────────────────
    tracing::info!("Phase 0: Pre-installation checks");

    if !archinstall_zfs_core::system::net::check_internet() {
        bail!("No internet connectivity. Connect to the network and retry.");
    }
    tracing::info!("Internet connectivity OK");

    if !archinstall_zfs_core::system::sysinfo::has_uefi() {
        bail!("UEFI boot required. This installer only supports UEFI systems.");
    }
    tracing::info!("UEFI boot detected");

    // Validate kernel/ZFS compatibility before proceeding
    let warnings = archinstall_zfs_core::kernel::scanner::validate_kernel_zfs_plan(
        kernel,
        config.zfs_module_mode,
    );
    for w in &warnings {
        tracing::warn!("kernel compatibility: {w}");
    }

    archinstall_zfs_core::zfs::kmod::initialize_zfs(runner, kernel, config.zfs_module_mode)?;
    tracing::info!("ZFS initialized on host");

    // ── Phase 1: Disk preparation ──────────────────────────────
    tracing::info!("Phase 1: Disk preparation");
    let (efi_partition, zfs_partition, _swap_partition) = match mode {
        InstallationMode::FullDisk => {
            let disk = config.disk_by_id.as_ref().unwrap();
            tracing::info!(disk = %disk.display(), "full disk mode");

            archinstall_zfs_core::disk::partition::zap_disk(runner, disk)?;

            let swap_size = match config.swap_mode {
                SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted => {
                    config.swap_partition_size.as_deref()
                }
                _ => None,
            };
            let layout =
                archinstall_zfs_core::disk::partition::create_partitions(runner, disk, swap_size)?;
            let parts =
                archinstall_zfs_core::disk::partition::wait_for_by_id_partitions(disk, &layout);

            let efi = parts[0].clone();
            let zfs = parts[1].clone();
            let swap = if parts.len() > 2 {
                Some(parts[2].clone())
            } else {
                None
            };
            (efi, zfs, swap)
        }
        InstallationMode::NewPool => {
            let efi = config.efi_partition_by_id.clone().unwrap();
            let zfs = config.zfs_partition_by_id.clone().unwrap();
            let swap = config.swap_partition_by_id.clone();
            (efi, zfs, swap)
        }
        InstallationMode::ExistingPool => {
            let efi = config.efi_partition_by_id.clone().unwrap();
            let zfs = PathBuf::new();
            let swap = config.swap_partition_by_id.clone();
            (efi, zfs, swap)
        }
    };

    // ── Phase 2: ZFS pool + datasets + encryption ──────────────
    tracing::info!("Phase 2: ZFS pool and datasets");

    archinstall_zfs_core::zfs::cache::create_hostid(runner)?;
    archinstall_zfs_core::zfs::cache::prepare_zfs_cache(Path::new("/"), pool_name)?;
    let _ = runner.run("systemctl", &["enable", "--now", "zfs-zed.service"]);

    if encryption != ZfsEncryptionMode::None {
        if let Some(ref pw) = config.zfs_encryption_password {
            archinstall_zfs_core::zfs::encryption::write_key_file(Path::new("/"), pw)?;
        }
    }

    let key_path = archinstall_zfs_core::zfs::encryption::key_file_path(Path::new("/"));

    match mode {
        InstallationMode::FullDisk | InstallationMode::NewPool => {
            let enc_props: Vec<(&str, String)> = match encryption {
                ZfsEncryptionMode::Pool => {
                    archinstall_zfs_core::zfs::encryption::pool_encryption_properties(&key_path)
                }
                _ => Vec::new(),
            };
            let enc_refs: Vec<(&str, &str)> =
                enc_props.iter().map(|(k, v)| (*k, v.as_str())).collect();

            archinstall_zfs_core::zfs::pool::create_pool(
                runner,
                pool_name,
                &zfs_partition,
                &mountpoint,
                &compression,
                &enc_refs,
            )?;
            tracing::info!("Created pool: {pool_name}");

            archinstall_zfs_core::zfs::pool::set_pool_property(
                runner,
                pool_name,
                "cachefile",
                "none",
            )?;

            let base_props: Vec<(&str, String)> = match encryption {
                ZfsEncryptionMode::Dataset => {
                    let mut p =
                        archinstall_zfs_core::zfs::encryption::dataset_encryption_properties(
                            &key_path,
                        );
                    p.push(("mountpoint", "none".to_string()));
                    p.push(("compression", compression.clone()));
                    p
                }
                _ => {
                    vec![
                        ("mountpoint", "none".to_string()),
                        ("compression", compression.clone()),
                    ]
                }
            };
            let base_refs: Vec<(&str, &str)> =
                base_props.iter().map(|(k, v)| (*k, v.as_str())).collect();
            archinstall_zfs_core::zfs::dataset::create_base_dataset(
                runner, pool_name, prefix, &base_refs,
            )?;

            let datasets = archinstall_zfs_core::zfs::dataset::default_datasets();
            archinstall_zfs_core::zfs::dataset::create_child_datasets(
                runner, pool_name, prefix, &datasets,
            )?;
            tracing::info!("Created datasets");

            archinstall_zfs_core::zfs::pool::export_pool(runner, pool_name)?;
            archinstall_zfs_core::zfs::pool::import_pool_no_mount(runner, pool_name, &mountpoint)?;
        }
        InstallationMode::ExistingPool => {
            archinstall_zfs_core::zfs::pool::import_pool_no_mount(runner, pool_name, &mountpoint)?;
            if encryption != ZfsEncryptionMode::None {
                archinstall_zfs_core::zfs::encryption::load_key(runner, pool_name, &key_path)?;
            }
        }
    }

    let datasets = archinstall_zfs_core::zfs::dataset::default_datasets();
    archinstall_zfs_core::zfs::dataset::mount_datasets_ordered(
        runner, pool_name, prefix, &datasets,
    )?;
    tracing::info!("Datasets mounted");

    // ── Phase 3: Mount EFI ─────────────────────────────────────
    tracing::info!("Phase 3: Mounting EFI partition");
    archinstall_zfs_core::disk::partition::mount_efi(runner, &efi_partition, &mountpoint)?;

    // ── Phases 4-12: Installer pipeline ────────────────────────
    tracing::info!("Phase 4-12: Running installer pipeline");
    let mut installer = archinstall_zfs_core::installer::Installer::new(runner, config, &mountpoint);
    installer.perform_installation()?;

    // ── Phase 13: ZFSBootMenu ──────────────────────────────────
    tracing::info!("Phase 13: Setting up ZFSBootMenu");

    let zswap_on = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );
    archinstall_zfs_core::zfs::bootmenu::set_zbm_properties(
        runner,
        pool_name,
        prefix,
        config.init_system,
        zswap_on,
        config.set_bootfs,
    )?;

    archinstall_zfs_core::zfs::bootmenu::install_and_generate_zbm(
        runner,
        &mountpoint,
        config.init_system,
    )?;
    tracing::info!("ZFSBootMenu built and installed");

    archinstall_zfs_core::zfs::bootmenu::create_efi_entries(runner, &efi_partition)?;

    // ── Phase 14: Cleanup ──────────────────────────────────────
    tracing::info!("Phase 14: Cleanup");
    nix::unistd::sync();

    let root_ds = format!("{pool_name}/{prefix}/root");

    archinstall_zfs_core::disk::partition::umount_efi(runner, &mountpoint)?;

    for attempt in 1..=4 {
        let _result = match attempt {
            1 => runner.run("zfs", &["umount", "-a"]),
            2 => runner.run("zfs", &["unmount", &root_ds]),
            3 => runner.run("zfs", &["umount", "-af"]),
            4 => runner.run("zfs", &["unmount", "-f", &root_ds]),
            _ => unreachable!(),
        };
        std::thread::sleep(std::time::Duration::from_secs(1));
        nix::unistd::sync();
    }

    let output = runner.run("zpool", &["export", pool_name])?;
    if !output.success() {
        tracing::warn!("zpool export failed, trying force");
        let _ = runner.run("zpool", &["export", "-f", pool_name]);
    }

    tracing::info!("Installation complete!");
    Ok(())
}
