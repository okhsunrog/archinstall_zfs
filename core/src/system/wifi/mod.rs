//! WiFi management.
//!
//! Exactly one backend is selected at compile time via a mutually-exclusive
//! Cargo feature:
//!
//!   * `wifi-iwd`  — talk to `iwd` over D-Bus via the `iwdrs` crate.
//!                   Used on the installer ISO where iwd is the wifi daemon.
//!   * `wifi-nm`   — talk to NetworkManager over D-Bus via `nmrs`.
//!                   Used during desktop development (most Arch desktops
//!                   run NetworkManager, not iwd directly).
//!   * `wifi-mock` — canned data with simulated delays. Used in tests/CI
//!                   and on hosts that have no wifi stack at all.
//!
//! The public free-function API is identical across backends. Whichever
//! module is gated on the active feature is re-exported with `pub use`,
//! so `wifi::scan_networks()` always means the right thing for the target
//! environment without the caller knowing which backend is compiled in.
//!
//! Common types (`WifiNetwork`, `Security`, `WifiError`, `StationState`,
//! etc.) live here in `mod.rs` so every backend speaks the same vocabulary.

use std::pin::Pin;

use futures::Stream;
use thiserror::Error;

// ─── Mutual-exclusion guards ─────────────────────────────────────────────

#[cfg(any(
    all(feature = "wifi-iwd", feature = "wifi-nm"),
    all(feature = "wifi-iwd", feature = "wifi-mock"),
    all(feature = "wifi-nm", feature = "wifi-mock"),
))]
compile_error!("wifi-iwd, wifi-nm, and wifi-mock are mutually exclusive; enable exactly one");

#[cfg(not(any(feature = "wifi-iwd", feature = "wifi-nm", feature = "wifi-mock")))]
compile_error!("no wifi backend selected; enable one of wifi-iwd, wifi-nm, wifi-mock");

// ─── Backend module selection ────────────────────────────────────────────

#[cfg(feature = "wifi-iwd")]
mod iwd;
#[cfg(feature = "wifi-iwd")]
pub use iwd::*;

// ─── Common types ────────────────────────────────────────────────────────

/// A wifi network, as seen by a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// Signal strength normalized to 0-100.
    pub signal_percent: u8,
    pub security: Security,
    /// True if the backend has a saved profile for this network (so we
    /// can skip the passphrase prompt when reconnecting).
    pub known: bool,
}

/// A saved network profile — a network the user previously connected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownNetworkInfo {
    pub ssid: String,
    pub security: Security,
    pub hidden: bool,
}

/// Wifi security mode exposed in the UI. Collapses every backend's
/// richer security taxonomy (iwd's `NetworkType`, NM's capability
/// flags, etc.) into the four cases we actually care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    Wep,
    Psk,
    /// 802.1x / EAP enterprise networks. We scan and display them but
    /// the installer does not currently implement the connection flow
    /// (EAP methods, certs, usernames) — the UI disables the connect
    /// action for these.
    Enterprise,
}

impl Security {
    pub fn requires_passphrase(self) -> bool {
        matches!(self, Security::Wep | Security::Psk)
    }
}

/// Current state of the wireless station, normalized across backends.
/// Mirrors iwd's state machine 1:1; NetworkManager's richer device
/// states are collapsed into the same five cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationState {
    Connected,
    Disconnected,
    Connecting,
    Disconnecting,
    Roaming,
}

/// Live stream of station state changes. Returned by
/// `watch_station_state()` so callers can subscribe to
/// `PropertiesChanged` signals without depending on any specific
/// backend's stream type.
pub type StationStateStream =
    Pin<Box<dyn Stream<Item = Result<StationState, WifiError>> + Send + 'static>>;

#[derive(Debug, Error)]
pub enum WifiError {
    #[error("wifi backend is not available (daemon not running?)")]
    NotAvailable,
    #[error("no wireless adapter found")]
    NoAdapter,
    #[error("no station exposed by the wifi backend (adapter may be powered off)")]
    NoStation,
    #[error("network {0:?} not found in scan results")]
    NetworkNotFound(String),
    #[error("passphrase required for secured network {0:?}")]
    PassphraseRequired(String),
    #[error("connect failed: {0}")]
    ConnectFailed(String),
    #[error("scan failed")]
    ScanFailed,
    #[error("d-bus error: {0}")]
    Dbus(#[from] zbus::Error),
}

// ─── Backend-agnostic helpers ────────────────────────────────────────────

/// Find all wireless network interface names present in `/sys/class/net`.
///
/// Synchronous `/sys` probe that doesn't depend on whether any wifi
/// daemon is running. Useful to distinguish "no wifi hardware" from
/// "hardware present but backend not running" at UI startup.
pub fn detect_wifi_interfaces() -> Vec<String> {
    let net_path = std::path::Path::new("/sys/class/net");
    let Ok(entries) = std::fs::read_dir(net_path) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().join("wireless").is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Map a raw signal strength in `dBm * 100` units to a 0-100 percentage.
///
/// Used by both the iwd backend (iwd reports `i16` in hundredths of a
/// dBm) and the upcoming NM backend wherever it converts NM's own
/// `Strength u8` to a unit test point. The classic mapping:
///
/// * `≥ -50 dBm` → 100
/// * `≤ -100 dBm` → 0
/// * linear interpolation in between
pub(crate) fn signal_to_percent(dbm_times_100: i16) -> u8 {
    let dbm = dbm_times_100 as i32 / 100;
    if dbm >= -50 {
        100
    } else if dbm <= -100 {
        0
    } else {
        ((dbm + 100) * 2).clamp(0, 100) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_wifi_interfaces_does_not_panic() {
        // Just verify it doesn't panic — result depends on the host machine.
        let _ = detect_wifi_interfaces();
    }

    #[test]
    fn test_signal_to_percent_strong() {
        assert_eq!(signal_to_percent(-4000), 100); // -40 dBm, saturates at -50
        assert_eq!(signal_to_percent(-5000), 100); // -50 dBm
    }

    #[test]
    fn test_signal_to_percent_weak() {
        assert_eq!(signal_to_percent(-10000), 0); // -100 dBm, zero
        assert_eq!(signal_to_percent(-11000), 0); // -110 dBm, clamped
    }

    #[test]
    fn test_signal_to_percent_mid() {
        assert_eq!(signal_to_percent(-7500), 50); // -75 dBm
        assert_eq!(signal_to_percent(-6000), 80); // -60 dBm
        assert_eq!(signal_to_percent(-9000), 20); // -90 dBm
    }
}
