pub mod aur;
pub mod base;
pub mod fstab;
pub mod initramfs;
pub mod locale;
pub mod network;
pub mod services;
pub mod users;

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use color_eyre::eyre::{bail, Context, Result};

use crate::config::types::{
    GlobalConfig, InitSystem, InstallationMode, SwapMode, ZfsEncryptionMode, ZfsModuleMode,
};
use crate::system::cmd::CommandRunner;

pub struct Installer<'a> {
    pub runner: &'a dyn CommandRunner,
    pub config: &'a GlobalConfig,
    pub target: PathBuf,
    pub tx: Option<&'a Sender<String>>,
}

impl<'a> Installer<'a> {
    pub fn new(
        runner: &'a dyn CommandRunner,
        config: &'a GlobalConfig,
        target: &Path,
        tx: Option<&'a Sender<String>>,
    ) -> Self {
        Self {
            runner,
            config,
            target: target.to_path_buf(),
            tx,
        }
    }

    fn log(&self, msg: &str) {
        tracing::info!("{msg}");
        if let Some(tx) = self.tx {
            let _ = tx.send(msg.to_string());
        }
    }

    /// Run the full installation pipeline.
    /// Matches `perform_installation()` from archinstall_zfs/main.py.
    pub fn perform_installation(&self) -> Result<()> {
        let errors = self.config.validate_for_install();
        if !errors.is_empty() {
            bail!("Config validation failed:\n  {}", errors.join("\n  "));
        }

        // Phase 4: pacstrap base system
        self.log("Phase 4: Installing base system...");
        base::install_base(self.runner, &self.target, self.config, self.tx)?;

        // Phase 5: System config
        self.log("Phase 5: Configuring system...");
        self.configure_system()?;

        // Phase 6: archzfs repo on target + ZFS packages
        self.log("Phase 6: Installing ZFS packages on target...");
        self.install_zfs_on_target()?;

        // Phase 7: Initramfs
        self.log("Phase 7: Generating initramfs...");
        self.generate_initramfs()?;

        // Phase 8: Users + authentication
        self.log("Phase 8: Configuring users...");
        self.configure_users()?;

        // Phase 9: Profile packages + services
        self.log("Phase 9: Installing profile packages...");
        self.install_profile()?;

        // Phase 10: Additional packages + AUR
        self.log("Phase 10: Installing additional packages...");
        self.install_additional_packages()?;

        // Phase 11: Swap configuration
        self.log("Phase 11: Configuring swap...");
        self.configure_swap()?;

        // Phase 12: ZFS services + genfstab + misc files
        self.log("Phase 12: Finalizing ZFS configuration...");
        self.finalize_zfs()?;

        self.log("Installation complete.");
        Ok(())
    }

    fn configure_system(&self) -> Result<()> {
        if let Some(ref hostname) = self.config.hostname {
            locale::set_hostname(&self.target, hostname)?;
        }

        if let Some(ref locale) = self.config.locale {
            locale::set_locale(self.runner, &self.target, locale)?;
        }

        locale::set_keyboard(self.runner, &self.target, &self.config.keyboard_layout)?;

        if let Some(ref tz) = self.config.timezone {
            locale::set_timezone(&self.target, tz)?;
        }

        if self.config.ntp {
            services::enable_service(self.runner, &self.target, "systemd-timesyncd")?;
        }

        // Mirror config
        if let Some(ref _regions) = self.config.mirror_regions {
            // TODO: set mirrors from region list
        }

        // Network
        if self.config.network_copy_iso {
            network::copy_iso_network(self.runner, &self.target)?;
        }

        Ok(())
    }

    fn install_zfs_on_target(&self) -> Result<()> {
        // Add archzfs repo to target
        crate::system::pacman::add_archzfs_repo(self.runner, Some(&self.target))?;

        // Install ZFS packages inside chroot (not pacstrap, since archzfs
        // repo is in the target's pacman.conf, not the host's)
        let kernel = self
            .config
            .effective_kernels()
            .first()
            .cloned()
            .unwrap_or_else(|| "linux-lts".to_string());

        let zfs_packages = crate::kernel::get_zfs_packages(&kernel, self.config.zfs_module_mode);
        let pkg_list = zfs_packages.join(" ");
        let cmd = format!("pacman --noconfirm --needed -S {pkg_list}");
        let output = crate::system::cmd::chroot(self.runner, &self.target, &cmd)?;
        crate::system::cmd::check_exit(&output, "install ZFS packages in chroot")?;

        Ok(())
    }

    fn generate_initramfs(&self) -> Result<()> {
        let encryption = self.config.zfs_encryption_mode != ZfsEncryptionMode::None;

        match self.config.init_system {
            InitSystem::Dracut => {
                initramfs::dracut::configure(self.runner, &self.target, encryption)?;
                initramfs::dracut::generate(self.runner, &self.target)?;
            }
            InitSystem::Mkinitcpio => {
                initramfs::mkinitcpio::configure(&self.target, encryption)?;
                initramfs::mkinitcpio::generate(self.runner, &self.target)?;
            }
        }

        Ok(())
    }

    fn configure_users(&self) -> Result<()> {
        if let Some(ref pw) = self.config.root_password {
            users::set_root_password(self.runner, &self.target, pw)?;

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
                    self.runner,
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

    fn install_profile(&self) -> Result<()> {
        if let Some(ref profile_name) = self.config.profile {
            let profile = crate::profile::get_profile(profile_name);
            if let Some(p) = profile {
                if !p.packages.is_empty() {
                    let pkg_refs: Vec<&str> = p.packages.iter().copied().collect();
                    crate::system::pacman::pacstrap(self.runner, &self.target, &pkg_refs, self.tx)?;
                }
                for service in &p.services {
                    services::enable_service(self.runner, &self.target, service)?;
                }
            } else {
                tracing::warn!(profile = profile_name, "unknown profile, skipping");
            }
        }

        // Audio server
        if let Some(audio) = self.config.audio {
            let pkgs = match audio {
                crate::config::types::AudioServer::Pipewire => {
                    vec!["pipewire", "pipewire-alsa", "pipewire-pulse", "wireplumber"]
                }
                crate::config::types::AudioServer::Pulseaudio => {
                    vec!["pulseaudio", "pulseaudio-alsa"]
                }
            };
            crate::system::pacman::pacstrap(self.runner, &self.target, &pkgs, self.tx)?;
        }

        // Bluetooth
        if self.config.bluetooth {
            crate::system::pacman::pacstrap(
                self.runner,
                &self.target,
                &["bluez", "bluez-utils"],
                self.tx,
            )?;
            services::enable_service(self.runner, &self.target, "bluetooth")?;
        }

        Ok(())
    }

    fn install_additional_packages(&self) -> Result<()> {
        if !self.config.additional_packages.is_empty() {
            let pkg_refs: Vec<&str> = self
                .config
                .additional_packages
                .iter()
                .map(|s| s.as_str())
                .collect();
            crate::system::pacman::pacstrap(self.runner, &self.target, &pkg_refs, self.tx)?;
        }

        let aur_pkgs = self.config.all_aur_packages();
        if !aur_pkgs.is_empty() {
            aur::install_aur_packages(self.runner, &self.target, &aur_pkgs)?;
        }

        // Enable extra services
        for service in &self.config.extra_services {
            services::enable_service(self.runner, &self.target, service)?;
        }

        Ok(())
    }

    fn configure_swap(&self) -> Result<()> {
        match self.config.swap_mode {
            SwapMode::Zram => {
                crate::swap::configure_zram(&self.target, self.config.zram_size_expr.as_deref())?;
            }
            SwapMode::ZswapPartition => {
                if let Some(ref part) = self.config.swap_partition_by_id {
                    crate::swap::setup_swap_partition(self.runner, &self.target, part, false)?;
                }
            }
            SwapMode::ZswapPartitionEncrypted => {
                if let Some(ref part) = self.config.swap_partition_by_id {
                    crate::swap::setup_swap_partition(self.runner, &self.target, part, true)?;
                }
            }
            SwapMode::None => {}
        }
        Ok(())
    }

    fn finalize_zfs(&self) -> Result<()> {
        let pool_name = self.config.pool_name.as_deref().unwrap_or("zroot");
        let prefix = &self.config.dataset_prefix;

        // Enable ZFS services
        for service in crate::zfs::ZFS_SERVICES {
            services::enable_service(self.runner, &self.target, service)?;
        }

        // genfstab
        let zswap_on = matches!(
            self.config.swap_mode,
            SwapMode::ZswapPartition | SwapMode::ZswapPartitionEncrypted
        );
        fstab::generate_fstab(self.runner, &self.target, pool_name, prefix)?;

        // Copy misc files (hostid, zfs cache)
        crate::zfs::cache::copy_misc_files(&self.target, pool_name)?;

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
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_installer_validates_config() {
        let runner = RecordingRunner::new(vec![]);
        let config = GlobalConfig::default(); // missing installation_mode
        let installer = Installer::new(&runner, &config, Path::new("/mnt"), None);
        let result = installer.perform_installation();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("validation failed"));
    }
}
