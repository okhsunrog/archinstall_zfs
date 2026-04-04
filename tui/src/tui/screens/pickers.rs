use std::path::PathBuf;

use color_eyre::eyre::Result;

use archinstall_zfs_core::config::types::{
    AudioServer, CompressionAlgo, GlobalConfig, InitSystem, InstallationMode, SwapMode, UserConfig,
    ZfsEncryptionMode, ZfsModuleMode,
};

use super::edit::run_edit;
use super::select::run_select;

// ── Pickers ─────────────────────────────────────────

pub fn pick_timezone(terminal: &mut ratatui::DefaultTerminal) -> Result<Option<String>> {
    use archinstall_zfs_core::installer::locale;

    let regions = locale::list_timezone_regions();
    let region_strs: Vec<&str> = regions.to_vec();
    let result = run_select(terminal, "Timezone region", &region_strs, 0)?;
    let Some(region_idx) = result.selected else {
        return Ok(None);
    };
    let region = regions[region_idx];

    let cities = locale::list_timezone_cities(region);
    let city_strs: Vec<&str> = cities.iter().map(|s| s.as_str()).collect();
    let result = run_select(terminal, &format!("{region} /"), &city_strs, 0)?;
    let Some(city_idx) = result.selected else {
        return Ok(None);
    };

    Ok(Some(format!("{region}/{}", cities[city_idx])))
}

pub fn pick_locale(terminal: &mut ratatui::DefaultTerminal) -> Result<Option<String>> {
    use archinstall_zfs_core::installer::locale;

    let locales = locale::list_locales();
    let locale_strs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
    let result = run_select(terminal, "Locale", &locale_strs, 0)?;
    match result.selected {
        Some(idx) => Ok(Some(locales[idx].clone())),
        None => Ok(None),
    }
}

pub fn pick_disk(terminal: &mut ratatui::DefaultTerminal) -> Result<Option<PathBuf>> {
    let disks = archinstall_zfs_core::disk::by_id::list_disks_by_id()?;
    if disks.is_empty() {
        return Ok(None);
    }
    let disk_strs: Vec<String> = disks.iter().map(|p| p.display().to_string()).collect();
    let disk_refs: Vec<&str> = disk_strs.iter().map(|s| s.as_str()).collect();
    let result = run_select(terminal, "Select disk", &disk_refs, 0)?;
    match result.selected {
        Some(idx) => Ok(Some(disks[idx].clone())),
        None => Ok(None),
    }
}

pub fn pick_partition(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
) -> Result<Option<PathBuf>> {
    let parts = archinstall_zfs_core::disk::by_id::list_partitions_by_id()?;
    if parts.is_empty() {
        return Ok(None);
    }
    let part_strs: Vec<String> = parts.iter().map(|p| p.display().to_string()).collect();
    let part_refs: Vec<&str> = part_strs.iter().map(|s| s.as_str()).collect();
    let result = run_select(terminal, title, &part_refs, 0)?;
    match result.selected {
        Some(idx) => Ok(Some(parts[idx].clone())),
        None => Ok(None),
    }
}

pub fn pick_existing_pool(
    config: &mut GlobalConfig,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<()> {
    use archinstall_zfs_core::system::cmd::RealRunner;
    use archinstall_zfs_core::zfs::{encryption, pool};

    let runner = RealRunner;

    let pool_name = loop {
        let mut pools = pool::discover_importable_pools(&runner);
        let mut options: Vec<String> = pools.to_vec();
        options.push("Refresh".into());
        options.push("Enter manually".into());
        let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();

        let current = if let Some(ref name) = config.pool_name {
            pools.iter().position(|p| p == name).unwrap_or(0)
        } else {
            0
        };

        let result = run_select(terminal, "Select importable ZFS pool", &opt_refs, current)?;
        let Some(idx) = result.selected else {
            return Ok(());
        };

        if idx == options.len() - 2 {
            continue;
        } else if idx == options.len() - 1 {
            let result = run_edit(terminal, "Pool name", "", false)?;
            match result.value {
                Some(name) if !name.is_empty() => break name,
                _ => return Ok(()),
            }
        } else {
            break pools.swap_remove(idx);
        }
    };

    config.pool_name = Some(pool_name.clone());

    if encryption::detect_pool_encryption(&runner, &pool_name) {
        loop {
            let result = run_edit(terminal, "Enter pool passphrase", "", true)?;
            let Some(pw) = result.value else {
                break;
            };
            if pw.is_empty() {
                break;
            }

            if encryption::verify_pool_passphrase(&runner, &pool_name, &pw) {
                config.zfs_encryption_mode = ZfsEncryptionMode::Pool;
                config.zfs_encryption_password = Some(pw);
                break;
            } else {
                let _ = run_select(
                    terminal,
                    "Passphrase verification failed. Try again.",
                    &["OK"],
                    0,
                );
            }
        }
    } else {
        let result = run_select(
            terminal,
            "Encrypt the new base dataset?",
            &["No - Skip encryption", "Yes - Encrypt new base dataset"],
            0,
        )?;
        if result.selected == Some(1) {
            loop {
                let pw1 = run_edit(terminal, "Encryption password (min 8 chars)", "", true)?;
                let Some(pw1) = pw1.value.filter(|p| !p.is_empty()) else {
                    break;
                };
                let pw2 = run_edit(terminal, "Verify password", "", true)?;
                let Some(pw2) = pw2.value else {
                    break;
                };
                if pw1 == pw2 {
                    config.zfs_encryption_mode = ZfsEncryptionMode::Dataset;
                    config.zfs_encryption_password = Some(pw1);
                    break;
                } else {
                    let _ = run_select(terminal, "Passwords do not match. Try again.", &["OK"], 0);
                }
            }
        } else {
            config.zfs_encryption_mode = ZfsEncryptionMode::None;
            config.zfs_encryption_password = None;
        }
    }

    Ok(())
}

pub async fn pick_kernel(
    config: &GlobalConfig,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<Option<Vec<String>>> {
    use archinstall_zfs_core::kernel::AVAILABLE_KERNELS;
    use archinstall_zfs_core::kernel::scanner::scan_all_kernels;

    let results = scan_all_kernels().await;

    let mut options = Vec::new();
    let mut kernel_names = Vec::new();
    for (info, result) in AVAILABLE_KERNELS.iter().zip(&results) {
        let compat = match config.zfs_module_mode {
            ZfsModuleMode::Precompiled => {
                if result.precompiled_compatible {
                    "OK"
                } else {
                    "INCOMPATIBLE"
                }
            }
            ZfsModuleMode::Dkms => {
                if result.dkms_compatible {
                    "OK"
                } else {
                    "INCOMPATIBLE"
                }
            }
        };
        let ver = result.kernel_version.as_deref().unwrap_or("?");
        options.push(format!("{} ({ver}) [{compat}]", info.display_name));
        kernel_names.push(info.name);
    }

    let option_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let current_kernel = config.primary_kernel();
    let current_idx = kernel_names
        .iter()
        .position(|&n| n == current_kernel)
        .unwrap_or(0);

    let result = run_select(terminal, "Kernel", &option_refs, current_idx)?;
    match result.selected {
        Some(idx) => Ok(Some(vec![kernel_names[idx].to_string()])),
        None => Ok(None),
    }
}

pub fn pick_profile(terminal: &mut ratatui::DefaultTerminal) -> Result<Option<String>> {
    let profiles = archinstall_zfs_core::profile::all_profiles();
    let mut names: Vec<&str> = vec!["None"];
    names.extend(profiles.iter().map(|p| p.name));
    let result = run_select(terminal, "Profile", &names, 0)?;
    match result.selected {
        Some(0) => Ok(Some(String::new())), // "None" selected
        Some(idx) => Ok(Some(names[idx].to_string())),
        None => Ok(None),
    }
}

pub fn pick_parallel_downloads(
    config: &GlobalConfig,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<Option<u32>> {
    let options: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
    let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let current = (config.parallel_downloads as usize).saturating_sub(1);
    let result = run_select(terminal, "Parallel downloads", &opt_refs, current)?;
    match result.selected {
        Some(idx) => Ok(Some((idx + 1) as u32)),
        None => Ok(None),
    }
}

pub fn manage_users(
    config: &mut GlobalConfig,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<()> {
    loop {
        let users = config.users.clone().unwrap_or_default();
        let mut options: Vec<String> = users
            .iter()
            .map(|u| {
                let sudo = if u.sudo { " [sudo]" } else { "" };
                format!("{}{sudo}", u.username)
            })
            .collect();
        options.push("+ Add user".to_string());
        if !users.is_empty() {
            options.push("- Remove user".to_string());
        }
        options.push("Done".to_string());

        let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        let result = run_select(terminal, "User accounts", &opt_refs, 0)?;
        let Some(idx) = result.selected else {
            break;
        };

        if idx < users.len() {
            if let Some(ref mut user_list) = config.users
                && let Some(user) = user_list.get_mut(idx)
            {
                user.sudo = !user.sudo;
            }
        } else if options[idx] == "+ Add user" {
            let result = run_edit(terminal, "Username", "", false)?;
            if let Some(username) = result.value
                && !username.is_empty()
            {
                let pw_result = run_edit(terminal, "Password (empty=no password)", "", true)?;
                let password = pw_result.value.filter(|p| !p.is_empty());

                let sudo_opts = ["No", "Yes"];
                let sudo_result = run_select(terminal, "Enable sudo?", &sudo_opts, 1)?;
                let sudo = sudo_result.selected == Some(1);

                let user = UserConfig {
                    username,
                    password,
                    sudo,
                    shell: None,
                    groups: None,
                    ssh_authorized_keys: Vec::new(),
                    autologin: false,
                };
                config.users.get_or_insert_with(Vec::new).push(user);
            }
        } else if options[idx].starts_with("- Remove") {
            let user_names: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
            let result = run_select(terminal, "Remove user", &user_names, 0)?;
            if let Some(rm_idx) = result.selected
                && let Some(ref mut user_list) = config.users
            {
                user_list.remove(rm_idx);
                if user_list.is_empty() {
                    config.users = None;
                }
            }
        } else {
            break;
        }
    }
    Ok(())
}

// ── Apply handlers ──────────────────────────────────

pub fn apply_toggle(config: &mut GlobalConfig, key: &str) {
    match key {
        "ntp" => config.ntp = !config.ntp,
        "bluetooth" => config.bluetooth = !config.bluetooth,
        "zrepl" => config.zrepl_enabled = !config.zrepl_enabled,
        _ => {}
    }
}

pub fn apply_select(
    config: &mut GlobalConfig,
    key: &str,
    idx: usize,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<()> {
    match key {
        "installation_mode" => {
            let new_mode = match idx {
                0 => InstallationMode::FullDisk,
                1 => InstallationMode::NewPool,
                2 => InstallationMode::ExistingPool,
                _ => return Ok(()),
            };
            if config.installation_mode != Some(new_mode) {
                config.disk_by_id = None;
                config.efi_partition_by_id = None;
                config.zfs_partition_by_id = None;
                config.swap_partition_by_id = None;
            }
            config.installation_mode = Some(new_mode);
        }
        "encryption" => {
            let new_mode = match idx {
                0 => ZfsEncryptionMode::None,
                1 => ZfsEncryptionMode::Pool,
                2 => ZfsEncryptionMode::Dataset,
                _ => return Ok(()),
            };
            config.zfs_encryption_mode = new_mode;
            if new_mode != ZfsEncryptionMode::None && config.zfs_encryption_password.is_none() {
                let result = run_edit(terminal, "Encryption password (min 8 chars)", "", true)?;
                if let Some(pw) = result.value
                    && !pw.is_empty()
                {
                    config.zfs_encryption_password = Some(pw);
                }
            }
            if new_mode == ZfsEncryptionMode::None {
                config.zfs_encryption_password = None;
            }
        }
        "compression" => {
            config.compression = match idx {
                0 => CompressionAlgo::Lz4,
                1 => CompressionAlgo::Zstd,
                2 => CompressionAlgo::Zstd5,
                3 => CompressionAlgo::Zstd10,
                4 => CompressionAlgo::Off,
                _ => return Ok(()),
            };
        }
        "swap_mode" => {
            config.swap_mode = match idx {
                0 => SwapMode::None,
                1 => SwapMode::Zram,
                2 => SwapMode::ZswapPartition,
                3 => SwapMode::ZswapPartitionEncrypted,
                _ => return Ok(()),
            };
        }
        "init_system" => {
            config.init_system = match idx {
                0 => InitSystem::Dracut,
                1 => InitSystem::Mkinitcpio,
                _ => return Ok(()),
            };
        }
        "zfs_module_mode" => {
            config.zfs_module_mode = match idx {
                0 => ZfsModuleMode::Precompiled,
                1 => ZfsModuleMode::Dkms,
                _ => return Ok(()),
            };
        }
        "audio" => {
            config.audio = match idx {
                0 => None,
                1 => Some(AudioServer::Pipewire),
                2 => Some(AudioServer::Pulseaudio),
                _ => return Ok(()),
            };
        }
        "network" => {
            config.network_copy_iso = idx == 0;
        }
        _ => {}
    }
    Ok(())
}

pub fn apply_text(config: &mut GlobalConfig, key: &str, val: &str) {
    let val_opt = if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    };
    match key {
        "pool_name" => config.pool_name = val_opt,
        "dataset_prefix" => {
            if !val.is_empty() {
                config.dataset_prefix = val.to_string();
            }
        }
        "hostname" => config.hostname = val_opt,
        "locale" => config.locale = val_opt,
        "timezone" => config.timezone = val_opt,
        "keyboard" => {
            if !val.is_empty() {
                config.keyboard_layout = val.to_string();
            }
        }
        "root_password" => config.root_password = val_opt,
        "encryption_password" => config.zfs_encryption_password = val_opt,
        "swap_partition_size" => config.swap_partition_size = val_opt,
        "additional_packages" => {
            config.additional_packages = val
                .split_whitespace()
                .map(|s| s.trim_matches(',').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        "aur_packages" => {
            config.aur_packages = val
                .split_whitespace()
                .map(|s| s.trim_matches(',').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        "extra_services" => {
            config.extra_services = val
                .split_whitespace()
                .map(|s| s.trim_matches(',').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        _ => {}
    }
}
