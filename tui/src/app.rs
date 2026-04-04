use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::eyre::{Result, bail};
use tokio_util::sync::CancellationToken;

use archinstall_zfs_core::config::types::{
    GlobalConfig, InstallationMode, SwapMode, ZfsEncryptionMode,
};
use archinstall_zfs_core::system::cmd::{CommandRunner, RealRunner};

use crate::Cli;

pub async fn run(cli: Cli) -> Result<()> {
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
        let runner: Arc<dyn CommandRunner> = Arc::new(RealRunner);
        let cancel = CancellationToken::new();
        return run_install(runner, config, cancel, None).await;
    }

    // Interactive TUI mode
    crate::tui::run_tui(config, cli.dry_run).await
}

/// Full installation pipeline — async orchestrator.
///
/// Sync operations (CommandRunner subprocess calls, Alpm FFI) run inside
/// `spawn_blocking`. Async operations (HTTP, AUR resolver, package downloads)
/// are awaited directly.
pub async fn run_install(
    runner: Arc<dyn CommandRunner>,
    config: GlobalConfig,
    cancel: CancellationToken,
    download_progress_tx: Option<
        Arc<
            tokio::sync::watch::Sender<
                archinstall_zfs_core::system::async_download::DownloadProgress,
            >,
        >,
    >,
) -> Result<()> {
    let mountpoint = PathBuf::from("/mnt");
    let mode = config.installation_mode.unwrap();
    let pool_name = config.pool_name.as_deref().unwrap().to_string();
    let prefix = config.dataset_prefix.clone();
    let compression = config.compression.to_string();
    let encryption = config.zfs_encryption_mode;
    let kernel = config.primary_kernel().to_string();

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

    // Async: HTTP calls to validate kernel/ZFS compatibility
    let warnings = archinstall_zfs_core::kernel::scanner::validate_kernel_zfs_plan(
        &kernel,
        config.zfs_module_mode,
    )
    .await;
    for w in &warnings {
        tracing::warn!("kernel compatibility: {w}");
    }

    // Sync: initialize ZFS on host (runner + alpm)
    {
        let r = runner.clone();
        let k = kernel.clone();
        let zfs_mode = config.zfs_module_mode;
        let c = cancel.clone();
        tokio::task::spawn_blocking(move || {
            archinstall_zfs_core::zfs::kmod::initialize_zfs(&*r, &k, zfs_mode, &c)
        })
        .await??;
    }
    tracing::info!("ZFS initialized on host");

    // ── Phase 1: Disk preparation (sync) ──────────────────────
    tracing::info!("Phase 1: Disk preparation");
    let (efi_partition, zfs_partition, swap_partition) = {
        let r = runner.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || -> Result<_> {
            match mode {
                InstallationMode::FullDisk => {
                    let disk = config.disk_by_id.as_ref().unwrap();
                    tracing::info!(disk = %disk.display(), "full disk mode");

                    archinstall_zfs_core::disk::partition::zap_disk(&*r, disk)?;

                    let swap_size = match config.swap_mode {
                        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted => {
                            config.swap_partition_size.as_deref()
                        }
                        _ => None,
                    };
                    let layout = archinstall_zfs_core::disk::partition::create_partitions(
                        &*r, disk, swap_size,
                    )?;
                    let parts = archinstall_zfs_core::disk::partition::wait_for_by_id_partitions(
                        disk, &layout,
                    );

                    let efi = parts[0].clone();
                    let zfs = parts[1].clone();
                    let swap = if parts.len() > 2 {
                        Some(parts[2].clone())
                    } else {
                        None
                    };
                    Ok((efi, zfs, swap))
                }
                InstallationMode::NewPool => {
                    let efi = config.efi_partition_by_id.clone().unwrap();
                    let zfs = config.zfs_partition_by_id.clone().unwrap();
                    let swap = config.swap_partition_by_id.clone();
                    Ok((efi, zfs, swap))
                }
                InstallationMode::ExistingPool => {
                    let efi = config.efi_partition_by_id.clone().unwrap();
                    let zfs = PathBuf::new();
                    let swap = config.swap_partition_by_id.clone();
                    Ok((efi, zfs, swap))
                }
            }
        })
        .await??
    };

    // ── Phase 2: ZFS pool + datasets + encryption (sync) ──────
    tracing::info!("Phase 2: ZFS pool and datasets");
    {
        let r = runner.clone();
        let pool_name = pool_name.clone();
        let prefix = prefix.clone();
        let compression = compression.clone();
        let zfs_partition = zfs_partition.clone();
        let efi_partition = efi_partition.clone();
        let mountpoint = mountpoint.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            archinstall_zfs_core::zfs::cache::create_hostid(&*r)?;
            archinstall_zfs_core::zfs::cache::prepare_zfs_cache(Path::new("/"), &pool_name)?;
            let _ = r.run("systemctl", &["enable", "--now", "zfs-zed.service"]);

            if encryption != ZfsEncryptionMode::None
                && let Some(ref pw) = config.zfs_encryption_password
            {
                archinstall_zfs_core::zfs::encryption::write_key_file(Path::new("/"), pw)?;
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
                        &*r, &pool_name, &zfs_partition, &mountpoint, &compression, &enc_refs,
                    )?;
                    tracing::info!("Created pool: {pool_name}");

                    archinstall_zfs_core::zfs::pool::set_pool_property(
                        &*r, &pool_name, "cachefile", "none",
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
                        &*r, &pool_name, &prefix, &base_refs,
                    )?;

                    let datasets = archinstall_zfs_core::zfs::dataset::default_datasets();
                    archinstall_zfs_core::zfs::dataset::create_child_datasets(
                        &*r, &pool_name, &prefix, &datasets,
                    )?;
                    tracing::info!("Created datasets");

                    archinstall_zfs_core::zfs::pool::export_pool(&*r, &pool_name)?;
                    archinstall_zfs_core::zfs::pool::import_pool_no_mount(&*r, &pool_name, &mountpoint)?;
                }
                InstallationMode::ExistingPool => {
                    archinstall_zfs_core::zfs::pool::import_pool_no_mount(&*r, &pool_name, &mountpoint)?;
                    if encryption != ZfsEncryptionMode::None {
                        archinstall_zfs_core::zfs::encryption::load_key(&*r, &pool_name, &key_path)?;
                    }
                }
            }

            let datasets = archinstall_zfs_core::zfs::dataset::default_datasets();
            archinstall_zfs_core::zfs::dataset::mount_datasets_ordered(
                &*r, &pool_name, &prefix, &datasets,
            )?;
            tracing::info!("Datasets mounted");

            // ── Phase 3: Mount EFI ─────────────────────────────────────
            tracing::info!("Phase 3: Mounting EFI partition");
            archinstall_zfs_core::disk::partition::mount_efi(&*r, &efi_partition, &mountpoint)?;

            Ok(())
        })
        .await??;
    }

    // ── Phases 4-12: Installer pipeline (sync — AlpmContext is !Send) ──
    tracing::info!("Phase 4-12: Running installer pipeline");
    {
        let r = runner.clone();
        let config = config.clone();
        let mountpoint = mountpoint.clone();
        let cancel = cancel.clone();
        let swap = swap_partition;
        let download_tx = download_progress_tx.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut installer = archinstall_zfs_core::installer::Installer::new(
                r,
                config,
                &mountpoint,
                cancel,
                download_tx,
            );
            if let Some(swap) = swap {
                installer.set_swap_partition(swap);
            }
            installer.perform_installation()
        })
        .await??;
    }

    // ── Phase 13: ZFSBootMenu ──────────────────────────────────
    tracing::info!("Phase 13: Setting up ZFSBootMenu");

    let zswap_on = matches!(
        config.swap_mode,
        SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
    );
    {
        let r = runner.clone();
        let pool_name = pool_name.clone();
        let prefix = prefix.clone();
        let init_system = config.init_system;
        let set_bootfs = config.set_bootfs;
        tokio::task::spawn_blocking(move || {
            archinstall_zfs_core::zfs::bootmenu::set_zbm_properties(
                &*r,
                &pool_name,
                &prefix,
                init_system,
                zswap_on,
                set_bootfs,
            )
        })
        .await??;
    }

    archinstall_zfs_core::zfs::bootmenu::install_and_generate_zbm(
        runner.clone(),
        &mountpoint,
        config.init_system,
        &cancel,
    )
    .await?;
    tracing::info!("ZFSBootMenu built and installed");

    {
        let r = runner.clone();
        let efi = efi_partition.clone();
        tokio::task::spawn_blocking(move || {
            archinstall_zfs_core::zfs::bootmenu::create_efi_entries(&*r, &efi)
        })
        .await??;
    }

    // ── Phase 14: Cleanup (sync) ──────────────────────────────
    tracing::info!("Phase 14: Cleanup");
    {
        let r = runner;
        let pool_name = pool_name.clone();
        let prefix = prefix.clone();
        let mountpoint = mountpoint.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            nix::unistd::sync();

            let root_ds = format!("{pool_name}/{prefix}/root");
            archinstall_zfs_core::disk::partition::umount_efi(&*r, &mountpoint)?;

            for attempt in 1..=4 {
                let _result = match attempt {
                    1 => r.run("zfs", &["umount", "-a"]),
                    2 => r.run("zfs", &["unmount", &root_ds]),
                    3 => r.run("zfs", &["umount", "-af"]),
                    4 => r.run("zfs", &["unmount", "-f", &root_ds]),
                    _ => unreachable!(),
                };
                std::thread::sleep(std::time::Duration::from_secs(1));
                nix::unistd::sync();
            }

            let output = r.run("zpool", &["export", &pool_name])?;
            if !output.success() {
                tracing::warn!("zpool export failed, trying force");
                let _ = r.run("zpool", &["export", "-f", &pool_name]);
            }

            tracing::info!("Installation complete!");
            Ok(())
        })
        .await??;
    }

    Ok(())
}
