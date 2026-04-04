pub mod aur;
pub mod base;
pub mod fstab;
pub mod initramfs;
pub mod locale;
pub mod mirrors;
pub mod network;
pub mod services;
pub mod users;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use alpm::SigLevel;
use color_eyre::eyre::{Result, bail};
use tokio_util::sync::CancellationToken;

use crate::config::types::{GlobalConfig, InitSystem, SwapMode, ZfsEncryptionMode};
use crate::system::alpm_pacman::{AlpmContext, TargetMounts};
use crate::system::async_download::DownloadProgress;
use crate::system::cmd::CommandRunner;

pub struct Installer {
    pub runner: Arc<dyn CommandRunner>,
    pub config: GlobalConfig,
    pub target: PathBuf,
    cancel: CancellationToken,
    download_progress_tx: Option<Arc<tokio::sync::watch::Sender<DownloadProgress>>>,
    /// Swap partition computed at runtime (e.g. from full-disk partitioning).
    /// Overrides `config.swap_partition_by_id` when set.
    swap_partition: Option<PathBuf>,
    _target_mounts: Option<TargetMounts>,
    alpm_ctx: Option<AlpmContext>,
}

impl Installer {
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        config: GlobalConfig,
        target: &Path,
        cancel: CancellationToken,
        download_progress_tx: Option<Arc<tokio::sync::watch::Sender<DownloadProgress>>>,
    ) -> Self {
        Self {
            runner,
            config,
            target: target.to_path_buf(),
            cancel,
            download_progress_tx,
            swap_partition: None,
            _target_mounts: None,
            alpm_ctx: None,
        }
    }

    /// Set the swap partition path (used when full-disk partitioning computed
    /// the path at runtime, since it's not in the static config).
    pub fn set_swap_partition(&mut self, partition: PathBuf) {
        self.swap_partition = Some(partition);
    }

    /// Run the full installation pipeline.
    pub fn perform_installation(&mut self) -> Result<()> {
        let errors = self.config.validate_for_install();
        if !errors.is_empty() {
            bail!("Config validation failed:\n  {}", errors.join("\n  "));
        }

        // Phase 4: install base system via libalpm
        tracing::info!("Phase 4: Installing base system...");
        let target_mounts =
            base::install_base(&*self.runner, &self.target, &self.config, &self.cancel)?;
        self._target_mounts = Some(target_mounts);

        // Create a reusable AlpmContext for all subsequent package installs.
        // The target now has pacman.conf, keyring, and mirrorlist from finalize_target().
        let target_conf = self.target.join("etc/pacman.conf");
        let mut ctx = AlpmContext::for_target(&self.target, &target_conf)?;
        ctx.sync_databases(false)?;
        self.alpm_ctx = Some(ctx);

        // Phase 5: System config
        tracing::info!("Phase 5: Configuring system...");
        self.configure_system()?;

        // Phase 6: archzfs repo on target + ZFS packages
        tracing::info!("Phase 6: Installing ZFS packages on target...");
        self.install_zfs_on_target()?;

        // Phase 7: Initramfs
        tracing::info!("Phase 7: Generating initramfs...");
        self.generate_initramfs()?;

        // Phase 8: Users + authentication
        tracing::info!("Phase 8: Configuring users...");
        self.configure_users()?;

        // Phase 9: Profile packages + services
        tracing::info!("Phase 9: Installing profile packages...");
        self.install_profile()?;

        // Phase 10: Additional packages + AUR
        tracing::info!("Phase 10: Installing additional packages...");
        self.install_additional_packages()?;

        // Phase 11: Swap configuration
        tracing::info!("Phase 11: Configuring swap...");
        self.configure_swap()?;

        // Phase 12: ZFS services + genfstab + misc files
        tracing::info!("Phase 12: Finalizing ZFS configuration...");
        self.finalize_zfs()?;

        tracing::info!("Installation complete.");
        Ok(())
    }

    /// Install packages into the target via libalpm (replaces pacstrap calls).
    fn install_target_packages(&mut self, packages: &[&str]) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        self.alpm_ctx
            .as_mut()
            .expect("alpm_ctx must be initialized before installing packages")
            .install_packages(packages, &self.cancel, self.download_progress_tx.clone())
    }

    fn configure_system(&self) -> Result<()> {
        if let Some(ref hostname) = self.config.hostname {
            locale::set_hostname(&self.target, hostname)?;
        }

        if let Some(ref locale) = self.config.locale {
            locale::set_locale(&*self.runner, &self.target, locale)?;
        }

        locale::set_keyboard(&*self.runner, &self.target, &self.config.keyboard_layout)?;
        locale::set_x11_keyboard(
            &self.target,
            &self.config.keyboard_layout,
            self.config.x11_variant.as_deref(),
        )?;

        if let Some(ref tz) = self.config.timezone {
            locale::set_timezone(&self.target, tz)?;
        }

        if self.config.ntp {
            services::enable_service(&*self.runner, &self.target, "systemd-timesyncd")?;
        }

        // Mirror config
        if let Some(ref regions) = self.config.mirror_regions {
            mirrors::configure_mirrors(&*self.runner, &self.target, regions)?;
        }

        // Network
        if self.config.network_copy_iso {
            network::copy_iso_network(&*self.runner, &self.target)?;
        }

        Ok(())
    }

    fn install_zfs_on_target(&mut self) -> Result<()> {
        // Edit pacman.conf and import GPG keys (still needs shell for pacman-key)
        crate::system::pacman::add_archzfs_repo(&*self.runner, Some(&self.target))?;

        // Register archzfs repo in the live alpm handle and sync
        let ctx = self
            .alpm_ctx
            .as_mut()
            .expect("alpm_ctx must be initialized");
        ctx.register_repo(
            "archzfs",
            &["https://github.com/archzfs/archzfs/releases/download/experimental"],
            SigLevel::PACKAGE_OPTIONAL | SigLevel::DATABASE_OPTIONAL,
        )?;
        ctx.sync_databases(true)?;

        // Install ZFS packages via libalpm
        let kernel = self.config.primary_kernel();
        let zfs_packages = crate::kernel::get_zfs_packages(kernel, self.config.zfs_module_mode);
        let pkg_refs: Vec<&str> = zfs_packages.iter().map(|s| s.as_str()).collect();
        ctx.install_packages(&pkg_refs, &self.cancel, self.download_progress_tx.clone())?;

        Ok(())
    }

    fn generate_initramfs(&self) -> Result<()> {
        let encryption = self.config.zfs_encryption_mode != ZfsEncryptionMode::None;

        match self.config.init_system {
            InitSystem::Dracut => {
                initramfs::dracut::configure(&*self.runner, &self.target, encryption)?;
                initramfs::dracut::generate(&*self.runner, &self.target)?;
            }
            InitSystem::Mkinitcpio => {
                initramfs::mkinitcpio::configure(&self.target, encryption)?;
                initramfs::mkinitcpio::generate(&*self.runner, &self.target)?;
            }
        }

        Ok(())
    }

    fn configure_users(&self) -> Result<()> {
        if let Some(ref pw) = self.config.root_password {
            users::set_root_password(&*self.runner, &self.target, pw)?;

            // If sshd is in extra_services, allow root login
            if self.config.extra_services.iter().any(|s| s == "sshd") {
                let sshd_dir = self.target.join("etc/ssh/sshd_config.d");
                std::fs::create_dir_all(&sshd_dir)?;
                std::fs::write(sshd_dir.join("10-root-login.conf"), "PermitRootLogin yes\n")?;
            }
        }

        if let Some(ref user_list) = self.config.users {
            for user in user_list {
                users::create_user(
                    &*self.runner,
                    &self.target,
                    &user.username,
                    user.password.as_deref(),
                    user.sudo,
                    user.shell.as_deref(),
                    user.groups.as_deref(),
                )?;
            }
        }

        Ok(())
    }

    fn install_profile(&mut self) -> Result<()> {
        if let Some(ref profile_name) = self.config.profile {
            let profile = crate::profile::get_profile(profile_name);
            if let Some(p) = profile {
                if !p.packages.is_empty() {
                    let pkg_refs: Vec<&str> = p.packages.to_vec();
                    self.install_target_packages(&pkg_refs)?;
                }
                for service in &p.services {
                    services::enable_service(&*self.runner, &self.target, service)?;
                }
            } else {
                tracing::warn!(profile = profile_name, "unknown profile, skipping");
            }
        }

        // Audio server
        if let Some(audio) = self.config.audio {
            let pkgs: Vec<&str> = match audio {
                crate::config::types::AudioServer::Pipewire => {
                    vec!["pipewire", "pipewire-alsa", "pipewire-pulse", "wireplumber"]
                }
                crate::config::types::AudioServer::Pulseaudio => {
                    vec!["pulseaudio", "pulseaudio-alsa"]
                }
            };
            self.install_target_packages(&pkgs)?;
        }

        // Bluetooth
        if self.config.bluetooth {
            self.install_target_packages(&["bluez", "bluez-utils"])?;
            services::enable_service(&*self.runner, &self.target, "bluetooth")?;
        }

        // GPU drivers
        if let Some(driver) = self.config.gfx_driver {
            let pkgs = driver.packages();
            self.install_target_packages(pkgs)?;
            tracing::info!(?driver, "installed GPU driver packages");
        }

        Ok(())
    }

    fn install_additional_packages(&mut self) -> Result<()> {
        let additional: Vec<String> = self.config.additional_packages.clone();
        if !additional.is_empty() {
            let pkg_refs: Vec<&str> = additional.iter().map(|s| s.as_str()).collect();
            self.install_target_packages(&pkg_refs)?;
        }

        let aur_pkgs = self.config.all_aur_packages();
        if !aur_pkgs.is_empty() {
            // AUR install is async (dependency resolution uses async raur).
            // Bridge via block_on — justified because Installer holds !Send AlpmContext.
            let rt = tokio::runtime::Handle::current();
            rt.block_on(aur::install_aur_packages(
                self.runner.clone(),
                &self.target,
                &aur_pkgs,
                &self.cancel,
            ))?;
        }

        // Enable extra services
        for service in &self.config.extra_services {
            services::enable_service(&*self.runner, &self.target, service)?;
        }

        Ok(())
    }

    fn configure_swap(&self) -> Result<()> {
        match self.config.swap_mode {
            SwapMode::Zram => {
                crate::swap::configure_zram(&self.target, self.config.zram_size_expr.as_deref())?;
            }
            SwapMode::ZswapPartition => {
                let part = self.effective_swap_partition();
                if let Some(part) = part {
                    crate::swap::setup_swap_partition(&*self.runner, &self.target, part, false)?;
                }
            }
            SwapMode::ZswapPartitionEncrypted => {
                let part = self.effective_swap_partition();
                if let Some(part) = part {
                    crate::swap::setup_swap_partition(&*self.runner, &self.target, part, true)?;
                }
            }
            SwapMode::None => {}
        }
        Ok(())
    }

    /// Return the swap partition path: runtime override first, then config.
    fn effective_swap_partition(&self) -> Option<&Path> {
        self.swap_partition
            .as_deref()
            .or(self.config.swap_partition_by_id.as_deref())
    }

    /// Set the right TRIM strategy for the pool based on the storage type.
    ///
    /// - NVMe  → `autotrim=on`:  ZFS issues TRIM continuously as blocks are
    ///   freed; NVMe's deep command queue absorbs this with no I/O penalty.
    /// - SATA SSD → `zfs-trim-weekly@<pool>.timer`: periodic `zpool trim`
    ///   avoids the SATA bus latency spikes caused by concurrent TRIM on
    ///   consumer drives.
    /// - HDD  → nothing: rotational media doesn't support TRIM.
    ///
    /// `fstrim.timer` is intentionally never enabled — it is a VFS-level tool
    /// unaware of ZFS internals and silently skips ZFS pools on every run.
    fn configure_zfs_trim(&self, pool_name: &str) -> Result<()> {
        use crate::config::types::InstallationMode;
        use crate::system::sysinfo::{StorageType, detect_storage_type};

        // Only configure TRIM when we created (or know) the disk.
        // ExistingPool mode leaves the pool's autotrim property untouched.
        let disk_path = match self.config.installation_mode {
            Some(InstallationMode::FullDisk) => self.config.disk_by_id.as_deref(),
            Some(InstallationMode::NewPool) => self.config.zfs_partition_by_id.as_deref(),
            _ => None,
        };

        let Some(disk_path) = disk_path else {
            tracing::debug!("no disk path available for TRIM detection, skipping");
            return Ok(());
        };

        match detect_storage_type(disk_path) {
            StorageType::Nvme => {
                tracing::info!(pool = pool_name, "NVMe detected — enabling autotrim");
                crate::zfs::pool::set_pool_property(&*self.runner, pool_name, "autotrim", "on")?;
            }
            StorageType::SataSsd => {
                let timer = format!("zfs-trim-weekly@{pool_name}.timer");
                tracing::info!(
                    pool = pool_name,
                    timer,
                    "SATA SSD detected — enabling periodic zpool trim timer"
                );
                services::enable_service(&*self.runner, &self.target, &timer)?;
            }
            StorageType::Hdd => {
                tracing::debug!(pool = pool_name, "HDD detected — no TRIM configured");
            }
        }

        Ok(())
    }

    fn finalize_zfs(&self) -> Result<()> {
        let pool_name = self.config.pool_name.as_deref().unwrap_or("zroot");
        let prefix = &self.config.dataset_prefix;

        // Enable ZFS services
        for service in crate::zfs::ZFS_SERVICES {
            services::enable_service(&*self.runner, &self.target, service)?;
        }

        // Configure TRIM strategy based on detected storage type.
        // fstrim/fstrim.timer is NOT used — it cannot reach ZFS pools.
        // See StorageType docs in sysinfo.rs for full rationale.
        self.configure_zfs_trim(pool_name)?;

        // genfstab
        fstab::generate_fstab(&*self.runner, &self.target, pool_name, prefix)?;

        // Copy misc files (hostid, zfs cache)
        crate::zfs::cache::copy_misc_files(
            &*self.runner,
            &self.target,
            pool_name,
            Path::new("/mnt"),
        )?;

        // Copy encryption key if needed
        if self.config.encryption_enabled() {
            let key_src = crate::zfs::encryption::key_file_path(Path::new("/"));
            let key_dst = crate::zfs::encryption::key_file_path(&self.target);
            if key_src.exists() {
                if let Some(parent) = key_dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&key_src, &key_dst)?;
            }
        }

        // zrepl
        if self.config.zrepl_enabled {
            crate::zrepl::setup_zrepl(&self.target, pool_name, prefix)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::GlobalConfig;
    use crate::system::cmd::tests::RecordingRunner;

    #[test]
    fn test_installer_validates_config() {
        let runner: Arc<dyn CommandRunner> = Arc::new(RecordingRunner::new(vec![]));
        let config = GlobalConfig::default(); // missing installation_mode
        let mut installer = Installer::new(
            runner,
            config,
            Path::new("/mnt"),
            CancellationToken::new(),
            None,
        );
        let result = installer.perform_installation();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("validation failed")
        );
    }
}
