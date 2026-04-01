use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};

pub fn list_disks_by_id() -> Result<Vec<PathBuf>> {
    let by_id = Path::new("/dev/disk/by-id");
    if !by_id.exists() {
        return Ok(Vec::new());
    }
    let mut disks = Vec::new();
    for entry in fs::read_dir(by_id).wrap_err("failed to read /dev/disk/by-id")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip partitions (they contain -partN)
        if name.contains("-part") {
            continue;
        }
        // Skip cd/dvd drives
        if name.starts_with("sr") || name.starts_with("usb-") {
            continue;
        }
        disks.push(entry.path());
    }
    disks.sort();
    Ok(disks)
}

pub fn list_partitions_by_id() -> Result<Vec<PathBuf>> {
    let by_id = Path::new("/dev/disk/by-id");
    if !by_id.exists() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    for entry in fs::read_dir(by_id).wrap_err("failed to read /dev/disk/by-id")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains("-part") {
            parts.push(entry.path());
        }
    }
    parts.sort();
    Ok(parts)
}

pub fn resolve_by_id(by_id_path: &Path) -> Result<PathBuf> {
    fs::canonicalize(by_id_path)
        .wrap_err_with(|| format!("failed to resolve: {}", by_id_path.display()))
}

pub fn get_disk_by_id_for_device(dev_path: &Path) -> Result<Option<PathBuf>> {
    let by_id = Path::new("/dev/disk/by-id");
    if !by_id.exists() {
        return Ok(None);
    }
    let canonical = fs::canonicalize(dev_path)?;
    for entry in fs::read_dir(by_id)? {
        let entry = entry?;
        if let Ok(target) = fs::canonicalize(entry.path()) {
            if target == canonical {
                return Ok(Some(entry.path()));
            }
        }
    }
    Ok(None)
}
