use std::path::{Path, PathBuf};

use color_eyre::eyre::{bail, Context, Result};

use crate::config::types::{
    GlobalConfig, InstallationMode, SwapMode, ZfsEncryptionMode, ZfsModuleMode,
};
use crate::system::cmd::{check_exit, CommandRunner, RealRunner};
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
        return run_headless_install(&runner, &config);
    }

    // Interactive TUI mode
    crate::tui::run_tui(config, cli.dry_run)
}

/// Full headless installation — matches archinstall_zfs perform_installation()
fn run_headless_install(runner: &dyn CommandRunner, config: &GlobalConfig) -> Result<()> {
    let mountpoint = PathBuf::from("/mnt");
    let mode = config.installation_mode.unwrap();
    let pool_name = config.pool_name.as_deref().unwrap();
    let prefix = &config.dataset_prefix;
    let compression = config.compression.to_string();
    let encryption = config.zfs_encryption_mode;
    let kernel = config
        .effective_kernels()
        .first()
        .cloned()
        .unwrap_or_else(|| "linux-lts".to_string());

    // ── Phase 0: Pre-installation checks ───────────────────────
    tracing::info!("Phase 0: Pre-installation checks");

    // Check internet
    let has_internet = crate::system::net::check_internet(runner)?;
    if !has_internet {
        bail!("No internet connectivity. Connect to the network and retry.");
    }
    tracing::info!("internet connectivity OK");

    // Check UEFI
    if !crate::system::sysinfo::has_uefi() {
        bail!("UEFI boot required. This installer only supports UEFI systems.");
    }
    tracing::info!("UEFI boot detected");

    // Initialize ZFS on host (handles reflector, archzfs repo, ZFS packages, module loading)
    crate::zfs::kmod::initialize_zfs(runner, &kernel, config.zfs_module_mode)?;

    // ── Phase 1: Disk preparation ──────────────────────────────
    tracing::info!("Phase 1: Disk preparation");
    let (efi_partition, zfs_partition, swap_partition) = match mode {
        InstallationMode::FullDisk => {
            let disk = config.disk_by_id.as_ref().unwrap();
            tracing::info!(disk = %disk.display(), "full disk mode");

            crate::disk::partition::zap_disk(runner, disk)?;

            let swap_size = match config.swap_mode {
                SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted => {
                    config.swap_partition_size.as_deref()
                }
                _ => None,
            };
            let layout = crate::disk::partition::create_partitions(runner, disk, swap_size)?;
            let parts = crate::disk::partition::wait_for_by_id_partitions(disk, &layout);

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
            // No ZFS partition needed — pool already exists
            let zfs = PathBuf::new(); // unused
            let swap = config.swap_partition_by_id.clone();
            (efi, zfs, swap)
        }
    };

    // ── Phase 2: ZFS pool + datasets + encryption ──────────────
    tracing::info!("Phase 2: ZFS pool and datasets");

    // Create hostid
    crate::zfs::cache::create_hostid(runner)?;

    // Prepare ZFS cache on host
    crate::zfs::cache::prepare_zfs_cache(Path::new("/"), pool_name)?;

    // Enable ZED on host
    let _ = runner.run("systemctl", &["enable", "--now", "zfs-zed.service"]);

    // Encryption key setup
    if encryption != ZfsEncryptionMode::None {
        if let Some(ref pw) = config.zfs_encryption_password {
            crate::zfs::encryption::write_key_file(Path::new("/"), pw)?;
        }
    }

    let key_path = crate::zfs::encryption::key_file_path(Path::new("/"));

    match mode {
        InstallationMode::FullDisk | InstallationMode::NewPool => {
            // Build encryption properties
            let enc_props: Vec<(&str, String)> = match encryption {
                ZfsEncryptionMode::Pool => {
                    crate::zfs::encryption::pool_encryption_properties(&key_path)
                }
                _ => Vec::new(),
            };
            let enc_refs: Vec<(&str, &str)> =
                enc_props.iter().map(|(k, v)| (*k, v.as_str())).collect();

            // Create pool
            crate::zfs::pool::create_pool(
                runner,
                pool_name,
                &zfs_partition,
                &mountpoint,
                &compression,
                &enc_refs,
            )?;

            // Set cachefile=none (use scan-based import)
            crate::zfs::pool::set_pool_property(runner, pool_name, "cachefile", "none")?;

            // Create base dataset
            let base_enc_props: Vec<(&str, &str)> = match encryption {
                ZfsEncryptionMode::Dataset => {
                    let props = crate::zfs::encryption::dataset_encryption_properties(&key_path);
                    // Need to leak these into the right lifetime — build owned vec
                    // and return refs from it. We'll handle this inline.
                    vec![] // handled below
                }
                _ => vec![],
            };
            let base_props: Vec<(&str, String)> = match encryption {
                ZfsEncryptionMode::Dataset => {
                    let mut p = crate::zfs::encryption::dataset_encryption_properties(&key_path);
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
            crate::zfs::dataset::create_base_dataset(runner, pool_name, prefix, &base_refs)?;

            // Create child datasets
            let datasets = crate::zfs::dataset::default_datasets();
            crate::zfs::dataset::create_child_datasets(runner, pool_name, prefix, &datasets)?;

            // Export and reimport (needed for ZFS to properly set up mount hierarchy)
            crate::zfs::pool::export_pool(runner, pool_name)?;
            crate::zfs::pool::import_pool_no_mount(runner, pool_name, &mountpoint)?;
        }
        InstallationMode::ExistingPool => {
            // Import existing pool
            crate::zfs::pool::import_pool_no_mount(runner, pool_name, &mountpoint)?;

            // Load encryption key if pool is encrypted
            if encryption != ZfsEncryptionMode::None {
                crate::zfs::encryption::load_key(runner, pool_name, &key_path)?;
            }
        }
    }

    // Mount datasets
    let datasets = crate::zfs::dataset::default_datasets();
    crate::zfs::dataset::mount_datasets_ordered(runner, pool_name, prefix, &datasets)?;

    // ── Phase 3: Mount EFI ─────────────────────────────────────
    tracing::info!("Phase 3: Mounting EFI partition");
    crate::disk::partition::mount_efi(runner, &efi_partition, &mountpoint)?;

    // ── Phases 4-12: Installer pipeline ────────────────────────
    tracing::info!("Phases 4-12: Running installer pipeline");
    let installer = crate::installer::Installer::new(runner, config, &mountpoint, None);
    installer.perform_installation()?;

    // ── Phase 13: ZFSBootMenu ──────────────────────────────────
    tracing::info!("Phase 13: Setting up ZFSBootMenu");
    let efi_mount = mountpoint.join("boot/efi");
    crate::zfs::bootmenu::download_zbm_efi(runner, &efi_mount)?;
    crate::zfs::bootmenu::create_efi_entries(runner, &efi_partition)?;

    // Set ZBM properties: commandline (no root=), rootprefix, bootfs
    let zswap_on = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );
    crate::zfs::bootmenu::set_zbm_properties(
        runner,
        pool_name,
        prefix,
        &config.init_system.to_string(),
        zswap_on,
    )?;

    // ── Phase 14: Cleanup ──────────────────────────────────────
    tracing::info!("Phase 14: Cleanup");
    nix::unistd::sync();

    let root_ds = format!("{pool_name}/{prefix}/root");

    // Unmount EFI
    crate::disk::partition::umount_efi(runner, &mountpoint)?;

    // Unmount ZFS (multiple strategies, matching Python)
    for attempt in 1..=4 {
        let result = match attempt {
            1 => runner.run("zfs", &["umount", "-a"]),
            2 => runner.run("zfs", &["unmount", &root_ds]),
            3 => runner.run("zfs", &["umount", "-af"]),
            4 => runner.run("zfs", &["unmount", "-f", &root_ds]),
            _ => unreachable!(),
        };
        std::thread::sleep(std::time::Duration::from_secs(1));
        nix::unistd::sync();
    }

    // Export pool
    let output = runner.run("zpool", &["export", pool_name])?;
    if !output.success() {
        tracing::warn!("zpool export failed, trying force");
        let _ = runner.run("zpool", &["export", "-f", pool_name]);
    }

    tracing::info!("Installation complete!");
    Ok(())
}
