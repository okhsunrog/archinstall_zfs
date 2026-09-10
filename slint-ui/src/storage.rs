//! Read-only storage discovery and identity-based selection for the graphical wizard.
use crate::{
    refresh::refresh_items,
    ui::{App, StorageCandidate, StorageState},
};
use archinstall_zfs_core::{
    config::{
        edit::{DeviceSetting, apply_device},
        types::{GlobalConfig, InstallationMode},
    },
    disk::device::{self, DeviceChoice},
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, path::Path, rc::Rc};

#[derive(Default)]
struct Inventory {
    disks: Vec<DeviceChoice>,
    partitions: Vec<DeviceChoice>,
    pools: Vec<StorageCandidate>,
    datasets: Vec<String>,
}
thread_local! { static INVENTORY: RefCell<Inventory> = RefCell::default(); }

pub fn choices(role: &str) -> Vec<DeviceChoice> {
    if crate::preview::enabled() {
        return if role == "disk" {
            crate::preview::disks()
        } else {
            crate::preview::partitions()
        };
    }
    INVENTORY.with_borrow(|i| {
        if role == "disk" {
            i.disks.clone()
        } else {
            i.partitions.clone()
        }
    })
}
fn same_device(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap_or_else(|_| a.into())
        == std::fs::canonicalize(b).unwrap_or_else(|_| b.into())
}
pub fn unavailable(choice: &DeviceChoice, role: &str, c: &GlobalConfig) -> String {
    if choice.usage.in_use {
        return if choice.usage.mountpoints.is_empty() {
            "Device is in use".into()
        } else {
            format!("In use: {}", choice.usage.mountpoints.join(", "))
        };
    }
    if role == "disk" {
        return String::new();
    }
    for (other, selected, active) in [
        ("efi_partition", c.efi_partition.as_ref(), true),
        (
            "zfs_partition",
            c.zfs_partition.as_ref(),
            c.installation_mode == Some(InstallationMode::NewPool),
        ),
        (
            "swap_partition",
            c.swap_partition.as_ref(),
            c.swap_mode.uses_partition(),
        ),
    ] {
        if active && role != other && selected.is_some_and(|p| same_device(p, &choice.path)) {
            return format!("Assigned to {}", role_label(other));
        }
    }
    if role == "efi_partition"
        && (!matches!(choice.usage.filesystem.as_str(), "vfat" | "fat32")
            || !choice
                .usage
                .partition_type
                .eq_ignore_ascii_case("c12a7328-f81f-11d2-ba4b-00a0c93ec93b"))
    {
        return "Requires a FAT32 EFI system partition".into();
    }
    String::new()
}
pub fn role_label(role: &str) -> &str {
    match role {
        "disk" => "Disk",
        "efi_partition" => "EFI boot partition",
        "zfs_partition" => "New ZFS pool",
        "swap_partition" => "Swap",
        _ => "Existing pool",
    }
}
fn candidate(choice: DeviceChoice, role: &str, c: &GlobalConfig) -> StorageCandidate {
    let unavailable = unavailable(&choice, role, c);
    let hardware = format!(
        "{}  {}",
        choice.transport.to_uppercase(),
        if choice.removable {
            "Removable"
        } else {
            &choice.media
        }
    );
    let details = format!(
        "{}\nModel: {}\nSerial: {}\n{}",
        choice.label,
        choice.model,
        if choice.serial.is_empty() {
            "Unknown"
        } else {
            &choice.serial
        },
        choice.path.display()
    );
    StorageCandidate {
        key: choice.path.to_string_lossy().as_ref().into(),
        name: choice.label.clone().into(),
        group: if choice.group_label.is_empty() {
            "Disks".into()
        } else {
            format!("{}  —  {}", choice.group_model, choice.group_label).into()
        },
        hardware: if role == "disk" {
            "".into()
        } else {
            hardware.clone().into()
        },
        size: choice.size.into(),
        filesystem: if role == "disk" {
            hardware.into()
        } else if choice.usage.filesystem.is_empty() {
            "Unknown".into()
        } else {
            choice.usage.filesystem.into()
        },
        label: if role == "disk" {
            choice.model.into()
        } else if choice.usage.label.is_empty() {
            "—".into()
        } else {
            choice.usage.label.into()
        },
        details: details.into(),
        unavailable: unavailable.into(),
    }
}
fn current(c: &GlobalConfig, role: &str) -> String {
    if role == "pool" {
        return c.pool_name.clone().unwrap_or_default();
    }
    match role {
        "disk" => &c.disk,
        "efi_partition" => &c.efi_partition,
        "zfs_partition" => &c.zfs_partition,
        _ => &c.swap_partition,
    }
    .as_ref()
    .map(|p| p.display().to_string())
    .unwrap_or_default()
}
fn populate(app: &App, c: &GlobalConfig) {
    let state = app.global::<StorageState>();
    let role = state.get_role();
    let filter = state.get_filter().to_lowercase();
    let rows: Vec<_> = if role == "pool" {
        INVENTORY.with_borrow(|i| i.pools.clone())
    } else {
        choices(&role)
            .into_iter()
            .map(|d| candidate(d, &role, c))
            .collect()
    };
    let selected = state.get_selected();
    if let Some(row) = rows
        .iter()
        .find(|r| r.key == selected && r.unavailable.is_empty())
    {
        state.set_selected_name(row.name.clone());
        state.set_selected_details(row.details.clone());
    } else {
        state.set_selected("".into());
        state.set_selected_name("".into());
        state.set_selected_details("".into());
    }
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| {
            format!(
                "{} {} {} {} {}",
                r.name, r.group, r.filesystem, r.label, r.details
            )
            .to_lowercase()
            .contains(&filter)
        })
        .collect();
    if !rows.iter().any(|r| r.key == state.get_selected()) {
        state.set_selected("".into());
        state.set_selected_name("".into());
        state.set_selected_details("".into());
    }
    state.set_candidates(ModelRc::new(VecModel::from(rows)));
}
pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<StorageState>().on_open(move |role| {
        let Some(app) = weak.upgrade() else { return };
        let state = app.global::<StorageState>();
        state.set_role(role.clone());
        state.set_title(match role.as_str() {
            "disk" => "Choose a disk",
            "efi_partition" => "Choose an EFI partition",
            "zfs_partition" => "Choose a ZFS partition",
            "swap_partition" => "Choose a swap partition",
            _ => "Choose an existing pool",
        }.into());
        state.set_explanation(match role.as_str() {
            "disk" => "All partitions on this disk will be erased during installation.",
            "efi_partition" => "Reuse an existing EFI filesystem. Bootloader files will be written here.",
            "pool" => "Choose a pool for a new boot environment. Discovery does not import or modify pools.",
            _ => "The selected partition's contents will be replaced during installation.",
        }.into());
        state.set_filter("".into());
        state.set_selected(current(&cfg.borrow(), &role).into());
        state.set_visible(true);
        populate(&app, &cfg.borrow());
        scan(&app, role == "pool");
    });
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<StorageState>().on_inventory_changed(move || {
        if let Some(app) = weak.upgrade() {
            populate(&app, &cfg.borrow());
            refresh_items(&app, &cfg.borrow());
        }
    });
    let weak = app.as_weak();
    app.global::<StorageState>().on_refresh(move || {
        if let Some(app) = weak.upgrade() {
            scan(&app, app.global::<StorageState>().get_role() == "pool");
        }
    });
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<StorageState>().on_filter_changed(move || {
        if let Some(app) = weak.upgrade() {
            populate(&app, &cfg.borrow());
        }
    });
    let weak = app.as_weak();
    app.global::<StorageState>().on_choose(move |key| {
        if let Some(app) = weak.upgrade() {
            use slint::Model;
            let state = app.global::<StorageState>();
            if let Some(row) = state
                .get_candidates()
                .iter()
                .find(|r| r.key == key && r.unavailable.is_empty())
            {
                state.set_selected(key);
                state.set_selected_name(row.name);
                state.set_selected_details(row.details);
            }
        }
    });
    let weak = app.as_weak();
    app.global::<StorageState>().on_move(move |direction| {
        if let Some(app) = weak.upgrade() {
            use slint::Model;
            let state = app.global::<StorageState>();
            let rows: Vec<_> = state
                .get_candidates()
                .iter()
                .filter(|r| r.unavailable.is_empty())
                .collect();
            if rows.is_empty() {
                return;
            }
            let current = rows
                .iter()
                .position(|r| r.key == state.get_selected())
                .map(|i| i as i32)
                .unwrap_or(if direction > 0 { -1 } else { 0 });
            let next = (current + direction).rem_euclid(rows.len() as i32) as usize;
            state.invoke_choose(rows[next].key.clone());
        }
    });
    let weak = app.as_weak();
    app.global::<StorageState>().on_cancel(move || {
        if let Some(app) = weak.upgrade() {
            let state = app.global::<StorageState>();
            // A late scan must not rebuild the form after focus has returned.
            state.set_generation(state.get_generation() + 1);
            state.set_busy(false);
            state.set_visible(false);
            crate::refresh::focus_item(&app, &format!("storage:{}", state.get_role()));
        }
    });
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<StorageState>().on_confirm(move || {
        if let Some(app) = weak.upgrade() {
            use slint::Model;
            let state = app.global::<StorageState>();
            if state.get_busy() {
                return;
            }
            let Some(row) = state
                .get_candidates()
                .iter()
                .find(|r| r.key == state.get_selected() && r.unavailable.is_empty())
            else {
                return;
            };
            let mut c = cfg.borrow_mut();
            let role = state.get_role();
            if role == "pool" {
                c.pool_name = Some(row.key.to_string());
            } else if let Some(setting) = DeviceSetting::parse(&role) {
                apply_device(&mut c, setting, Path::new(row.key.as_str()));
            }
            refresh_items(&app, &c);
            state.set_visible(false);
            crate::refresh::focus_item(&app, &format!("storage:{role}"));
        }
    });
    if !crate::preview::enabled() {
        scan(app, false);
    }
}
fn scan(app: &App, pools: bool) {
    let state = app.global::<StorageState>();
    let generation = state.get_generation() + 1;
    state.set_generation(generation);
    state.set_busy(true);
    state.set_error("".into());
    let weak = app.as_weak();
    tokio::spawn(async move {
        // Only plain owned Rust data crosses the worker/event-loop boundary.
        let result = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            if pools {
                let (pools, datasets) = pool_inventory().await?;
                Ok::<_, String>((Vec::new(), Vec::new(), pools, datasets))
            } else {
                tokio::task::spawn_blocking(|| {
                    let disks = if crate::preview::enabled() {
                        crate::preview::disks()
                    } else {
                        device::disk_choices().map_err(|e| e.to_string())?
                    };
                    let parts = if crate::preview::enabled() {
                        crate::preview::partitions()
                    } else {
                        device::partition_choices().map_err(|e| e.to_string())?
                    };
                    Ok((disks, parts, Vec::new(), Vec::new()))
                })
                .await
                .map_err(|e| e.to_string())?
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err("Storage discovery timed out. Check the devices and retry.".into())
        });
        let _ = weak.upgrade_in_event_loop(move |app| {
            let state = app.global::<StorageState>();
            if state.get_generation() != generation {
                return;
            }
            state.set_busy(false);
            match result {
                Ok((disks, parts, found, datasets)) => INVENTORY.with_borrow_mut(|i| {
                    if pools {
                        i.datasets = datasets;
                        i.pools = found
                            .into_iter()
                            .map(|(name, status, details, blocked)| StorageCandidate {
                                key: name.clone().into(),
                                name: name.into(),
                                group: "ZFS pools".into(),
                                label: status.into(),
                                details: details.into(),
                                unavailable: blocked.into(),
                                ..Default::default()
                            })
                            .collect();
                    } else {
                        i.disks = disks;
                        i.partitions = parts;
                    }
                }),
                Err(e) => {
                    state.set_error(e.into());
                    INVENTORY.with_borrow_mut(|i| {
                        if pools {
                            i.pools.clear()
                        } else {
                            i.disks.clear();
                            i.partitions.clear();
                        }
                    });
                }
            }
            state.invoke_inventory_changed();
        });
    });
}
type PoolRow = (String, String, String, String);
async fn pool_inventory() -> Result<(Vec<PoolRow>, Vec<String>), String> {
    if crate::preview::enabled() {
        return Ok((vec![
            ("zroot".into(), "ONLINE — imported".into(), "Samsung SSD 990 PRO 2TB\nAvailable: 800 GiB\nExisting environment: zroot/previous".into(), "".into()),
            ("backup-workstation".into(), "ONLINE — available for import".into(), "Pool GUID: 12450716\nSpace and datasets cannot be inspected until import.\nImported during installation; discovery makes no changes.".into(), "".into()),
        ],vec!["zroot/previous".into()]));
    }
    let zfs = zfskit::Zfs::new();
    let imported = zfs
        .list_pools(&Default::default())
        .await
        .map_err(|e| e.to_string())?;
    let available = zfs
        .discover_importable_pools()
        .await
        .map_err(|e| e.to_string())?;
    let datasets = if imported.is_empty() {
        Vec::new()
    } else {
        zfs.list_datasets(&zfskit::dataset::ListOptions {
            recursive: true,
            ..Default::default()
        })
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|d| d.name)
        .collect::<Vec<_>>()
    };
    let mut rows = Vec::new();
    fn paths(v: &zfskit::models::VdevStatus, output: &mut Vec<String>) {
        if let Some(path) = &v.path {
            output.push(path.clone());
        }
        for child in v.vdevs.values() {
            paths(child, output);
        }
    }
    for p in imported {
        let status = zfs
            .pool(&p.name)
            .map_err(|e| e.to_string())?
            .status()
            .await
            .map_err(|e| e.to_string())?;
        let mut devices = Vec::new();
        for v in status.vdevs.values() {
            paths(v, &mut devices);
        }
        devices.sort();
        let free = p
            .properties
            .get("free")
            .and_then(|p| p.value.parse::<u64>().ok())
            .map(|bytes| format!("{:.1} GiB", bytes as f64 / 1073741824.0))
            .unwrap_or_else(|| "Unknown".into());
        rows.push((
            p.name.clone(),
            format!("{} — imported", p.state),
            format!(
                "Pool GUID: {}\nAvailable: {}\n{}",
                p.pool_guid,
                free,
                devices.join(", ")
            ),
            if p.state == "ONLINE" {
                "".into()
            } else {
                "Pool must be ONLINE before installation".into()
            },
        ));
    }
    for p in available {
        rows.push((
            p.name.clone(),
            format!("{} — available for import", p.state),
            format!(
                "Pool GUID: {}\n{}\nSpace and datasets cannot be inspected until import.",
                p.id,
                p.status
                    .unwrap_or_else(|| "Imported during installation".into())
            ),
            if p.state == "ONLINE" {
                "".into()
            } else {
                "Pool must be ONLINE before installation".into()
            },
        ));
    }
    let names: Vec<_> = rows.iter().map(|r| r.0.clone()).collect();
    for row in &mut rows {
        if names.iter().filter(|n| **n == row.0).count() > 1 {
            row.3 = "Multiple pools have this name; resolve the ambiguity first".into();
        }
    }
    Ok((rows, datasets))
}

/// Review uses the same eligibility rules as the picker, including loaded configs.
pub fn issues(c: &GlobalConfig) -> Vec<String> {
    if c.installation_mode == Some(InstallationMode::Alongside) {
        return Vec::new();
    }
    let roles = if c.installation_mode == Some(InstallationMode::FullDisk) {
        vec![("disk", c.disk.as_deref())]
    } else {
        let mut roles = vec![("efi_partition", c.efi_partition.as_deref())];
        if c.installation_mode == Some(InstallationMode::NewPool) {
            roles.push(("zfs_partition", c.zfs_partition.as_deref()));
        }
        if c.swap_mode.uses_partition() {
            roles.push(("swap_partition", c.swap_partition.as_deref()));
        }
        roles
    };
    let mut issues: Vec<String> = roles
        .into_iter()
        .filter_map(|(role, path)| {
            let path = path?;
            let choices = choices(role);
            Some(match choices.iter().find(|d| same_device(&d.path, path)) {
                Some(choice) => {
                    let reason = unavailable(choice, role, c);
                    if reason.is_empty() {
                        return None;
                    }
                    format!("{}: {reason}", role_label(role))
                }
                None => format!(
                    "{}: selected device is unavailable. Open the selector and refresh.",
                    role_label(role)
                ),
            })
        })
        .collect();
    if c.installation_mode == Some(InstallationMode::ExistingPool)
        && let Some(name) = &c.pool_name
    {
        INVENTORY.with_borrow(|i| {
            match i.pools.iter().find(|p| p.name.as_str() == name) {
                None => issues.push("Choose or refresh the existing pool before installing".into()),
                Some(p) if !p.unavailable.is_empty() => issues.push(p.unavailable.to_string()),
                _ => {}
            }
            let be = archinstall_zfs_core::boot_environment::BootEnvironment::new(
                name,
                &c.dataset_prefix,
            )
            .base();
            if i.datasets.contains(&be) {
                issues.push(format!(
                    "Boot environment {be} already exists. Choose a different name."
                ));
            }
        });
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picker_rejects_active_devices_and_conflicting_roles() {
        let parts = crate::preview::partitions();
        let efi = parts.iter().find(|p| p.label == "/dev/nvme0n1p1").unwrap();
        let data = parts.iter().find(|p| p.label == "/dev/nvme0n1p2").unwrap();
        let c = GlobalConfig {
            installation_mode: Some(InstallationMode::NewPool),
            efi_partition: Some(efi.path.clone()),
            zfs_partition: Some(data.path.clone()),
            ..Default::default()
        };
        assert!(unavailable(efi, "efi_partition", &c).is_empty());
        assert!(unavailable(efi, "zfs_partition", &c).contains("Assigned"));
        assert!(!unavailable(data, "efi_partition", &c).is_empty());
        assert!(unavailable(data, "zfs_partition", &c).is_empty());
        assert!(
            parts
                .iter()
                .filter(|p| p.removable)
                .all(|p| unavailable(p, "zfs_partition", &c).starts_with("In use:"))
        );
        assert!(
            crate::preview::disks()
                .iter()
                .filter(|p| p.removable)
                .all(|p| !unavailable(p, "disk", &c).is_empty())
        );
    }
    #[test]
    fn missing_filesystem_information_is_not_assumed_to_be_efi() {
        let mut part = crate::preview::partitions().remove(0);
        part.usage = Default::default();
        assert!(!unavailable(&part, "efi_partition", &GlobalConfig::default()).is_empty());
        assert!(unavailable(&part, "zfs_partition", &GlobalConfig::default()).is_empty());
    }
}
