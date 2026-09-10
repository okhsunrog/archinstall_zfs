//! Tiny formatting helpers used by the install progress display and the
//! storage pages.

use archinstall_zfs_core::disk::alongside::{GIB, MIB};

pub fn format_speed(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.1} MB/s", bps as f64 / 1_000_000.0)
    } else if bps >= 1_000 {
        format!("{:.0} KB/s", bps as f64 / 1_000.0)
    } else if bps > 0 {
        format!("{bps} B/s")
    } else {
        "-- B/s".to_string()
    }
}

pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

pub fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        match s.char_indices().nth(max) {
            Some((idx, _)) => &s[..idx],
            None => s,
        }
    }
}

/// Bytes as fractional GiB for `{:.0}`/`{:.1}` display.
pub fn gib(bytes: u64) -> f64 {
    bytes as f64 / GIB as f64
}

/// A sector count as fractional GiB for display.
pub fn sectors_gib(sectors: u64, sectorsize: u64) -> f64 {
    sectors as f64 * sectorsize as f64 / GIB as f64
}

/// A sector count as fractional MiB for display.
pub fn sectors_mib(sectors: u64, sectorsize: u64) -> f64 {
    sectors as f64 * sectorsize as f64 / MIB as f64
}
