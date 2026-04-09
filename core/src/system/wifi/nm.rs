//! NetworkManager-backed wifi implementation.
//!
//! Talks to NetworkManager via the [`nmrs`] crate — a high-level async
//! wrapper around NM's D-Bus API. Compared to the iwd backend, this is
//! primarily for desktop development: most Arch and Arch-derivative
//! desktops run NetworkManager (via Gnome, KDE, etc.), so enabling the
//! `wifi-nm` feature lets developers iterate on the Slint GUI with
//! live wifi on their daily laptop without booting the installer ISO.
//!
//! # Feature parity with the iwd backend
//!
//! The public function signatures are identical across backends. Where
//! NM's model is richer than iwd's (e.g. per-connection DNS, multiple
//! access points for the same SSID, enterprise EAP settings), we
//! collapse to the common vocabulary in `mod.rs`. Where NM's model is
//! leaner than iwd's (e.g. no explicit Roaming state), we pick the
//! closest common variant.
//!
//! # Known limitations vs iwd
//!
//! * `watch_station_state()` currently returns an empty stream.
//!   nmrs exposes `monitor_network_changes(callback)` which is push-
//!   based, but the current slint-ui controller doesn't consume the
//!   stream yet, so we defer the callback→stream adapter until there's
//!   a real consumer driving the requirements.
//! * `connect_hidden()` is not implemented — nmrs doesn't expose a
//!   direct "connect to hidden SSID" entry point. Calling it returns
//!   `WifiError::ConnectFailed` with a descriptive message. Revisit
//!   if a user needs it.
//! * `list_known_networks()` returns SSIDs from `list_saved_connections`
//!   but marks every entry as `Security::Psk` because NM doesn't give
//!   us the security type without an extra lookup. Good enough for
//!   the "Known" badge; the UI doesn't actually branch on the saved
//!   profile's security type.
//! * WEP networks are mapped to `Psk` — nmrs's `Network` struct doesn't
//!   distinguish them and WEP is effectively extinct.

use futures::stream;
use nmrs::{
    ConnectionError as NmConnectionError, Network as NmNetwork, NetworkManager,
    WifiSecurity as NmWifiSecurity,
};

use super::{KnownNetworkInfo, Security, StationState, StationStateStream, WifiError, WifiNetwork};

// ─── type conversions ───────────────────────────────────────────────────

impl From<NmConnectionError> for WifiError {
    fn from(err: NmConnectionError) -> Self {
        match err {
            NmConnectionError::Dbus(z) => WifiError::Dbus(z),
            NmConnectionError::NotFound => WifiError::NetworkNotFound(String::new()),
            NmConnectionError::NoWifiDevice => WifiError::NoAdapter,
            NmConnectionError::WifiNotReady => WifiError::NoStation,
            other => WifiError::ConnectFailed(other.to_string()),
        }
    }
}

/// Fold NM's four security flags (`secured`, `is_psk`, `is_eap`, and
/// the absence thereof) into our four-case enum.
fn network_to_security(n: &NmNetwork) -> Security {
    if !n.secured {
        Security::Open
    } else if n.is_eap {
        Security::Enterprise
    } else {
        // is_psk or a plain "secured" flag — NM doesn't expose WEP
        // separately and WEP is effectively extinct, so default to Psk.
        Security::Psk
    }
}

// ─── public API (re-exported as `wifi::*` from mod.rs) ──────────────────

/// Probe NM on the system bus with a one-shot session construction.
///
/// Returns `false` if NetworkManager isn't installed, isn't running,
/// or the D-Bus handshake fails. The GUI corner widget uses this as
/// the gate to decide whether to show the rich wifi popup or the
/// read-only fallback.
pub async fn backend_available() -> bool {
    NetworkManager::new().await.is_ok()
}

/// Compatibility alias. The welcome controller uses the generic
/// `iwd_available` name in property and log messages; aliasing here
/// means the backend swap is invisible to the caller.
pub use backend_available as iwd_available;

/// Trigger a fresh scan, wait for the results, and return them sorted
/// strongest-first. Known-network status is computed from NM's saved
/// connections list.
pub async fn scan_networks() -> Result<Vec<WifiNetwork>, WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;

    nm.scan_networks().await?;

    // Pull saved connection names so we can flag known networks in
    // the scan result. `list_saved_connections` returns the NM
    // connection IDs which equal the SSIDs for standard wifi profiles.
    let known: std::collections::HashSet<String> = nm
        .list_saved_connections()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    let raw = nm.list_networks().await?;
    let mut out: Vec<WifiNetwork> = raw
        .into_iter()
        .filter(|n| !n.ssid.is_empty())
        .map(|n| WifiNetwork {
            signal_percent: n.strength.unwrap_or(0),
            security: network_to_security(&n),
            known: known.contains(&n.ssid),
            ssid: n.ssid,
        })
        .collect();
    out.sort_by(|a, b| b.signal_percent.cmp(&a.signal_percent));
    Ok(out)
}

/// Connect to `ssid`. Open networks connect with no passphrase; PSK
/// networks use the supplied passphrase; enterprise networks aren't
/// supported by the installer flow and return `ConnectFailed`.
pub async fn connect(ssid: &str, passphrase: Option<String>) -> Result<(), WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;

    // We need to know the security type before deciding what to pass
    // to nmrs. List the current scan, find the matching SSID, infer
    // security from its flags.
    let networks = nm.list_networks().await?;
    let net = networks
        .iter()
        .find(|n| n.ssid == ssid)
        .ok_or_else(|| WifiError::NetworkNotFound(ssid.to_string()))?;

    let security = network_to_security(net);

    let creds = match security {
        Security::Open => NmWifiSecurity::Open,
        Security::Wep | Security::Psk => {
            let psk = passphrase.ok_or_else(|| WifiError::PassphraseRequired(ssid.to_string()))?;
            if psk.is_empty() {
                return Err(WifiError::PassphraseRequired(ssid.to_string()));
            }
            NmWifiSecurity::WpaPsk { psk }
        }
        Security::Enterprise => {
            return Err(WifiError::ConnectFailed(
                "Enterprise (802.1x) networks are not supported by the installer".into(),
            ));
        }
    };

    nm.connect(ssid, creds).await?;
    Ok(())
}

/// Hidden-SSID connect is not implemented on the NM backend — nmrs
/// doesn't currently expose a direct entry point for it. Returns a
/// descriptive `ConnectFailed` so the UI can show an actionable
/// error. Delete this stub and wire up real support if nmrs adds it
/// or if a user asks for it.
pub async fn connect_hidden(_ssid: &str, _passphrase: Option<String>) -> Result<(), WifiError> {
    Err(WifiError::ConnectFailed(
        "connect_hidden is not implemented on the NetworkManager backend yet".into(),
    ))
}

pub async fn disconnect() -> Result<(), WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;
    nm.disconnect().await?;
    Ok(())
}

/// Derive a best-effort `StationState` from NM's state helpers. NM
/// doesn't give us Roaming / Disconnecting directly at this API
/// level, so we collapse to the three common cases. The controller
/// only cares about Connected / Disconnected / Connecting today.
pub async fn station_state() -> Result<StationState, WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;
    if nm.is_connecting().await? {
        return Ok(StationState::Connecting);
    }
    if nm.current_ssid().await.is_some() {
        return Ok(StationState::Connected);
    }
    Ok(StationState::Disconnected)
}

pub async fn check_connected() -> Result<bool, WifiError> {
    Ok(matches!(station_state().await?, StationState::Connected))
}

pub async fn current_ssid() -> Result<Option<String>, WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;
    Ok(nm.current_ssid().await)
}

/// Empty stream — see the module-level docs for why the NM backend
/// doesn't currently push live updates. Kept as a valid `impl Stream`
/// so the public signature stays the same across backends and
/// callers never branch on which backend is compiled in.
pub async fn watch_station_state() -> Result<StationStateStream, WifiError> {
    Ok(Box::pin(stream::empty()))
}

pub async fn list_known_networks() -> Result<Vec<KnownNetworkInfo>, WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;
    let ssids = nm.list_saved_connections().await?;
    let mut out: Vec<KnownNetworkInfo> = ssids
        .into_iter()
        .map(|ssid| KnownNetworkInfo {
            ssid,
            // nmrs doesn't expose the saved security type without an
            // extra lookup and we don't actually branch on it in the
            // UI. Default to Psk.
            security: Security::Psk,
            hidden: false,
        })
        .collect();
    out.sort_by(|a, b| a.ssid.cmp(&b.ssid));
    Ok(out)
}

pub async fn forget_network(ssid: &str) -> Result<(), WifiError> {
    let nm = NetworkManager::new()
        .await
        .map_err(|_| WifiError::NotAvailable)?;
    nm.forget(ssid).await?;
    Ok(())
}
