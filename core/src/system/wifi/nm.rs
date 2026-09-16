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
//! * `connect_hidden()` is not implemented. nmrs has a lower-level
//!   connection builder for hidden SSIDs, but the common backend API is
//!   not wired to that workflow yet.
//! * `list_known_networks()` uses nmrs's lightweight saved-connection IDs
//!   and marks every entry as `Security::Psk`. Richer saved-profile data is
//!   available, but the UI only needs the "Known" badge today.
//! * WEP networks are mapped to `Psk` — nmrs's `Network` struct doesn't
//!   distinguish them and WEP is effectively extinct.

use std::time::Duration;

use futures::stream;
use nmrs::{
    ConnectionError as NmConnectionError, Network as NmNetwork, NetworkManager,
    WifiSecurity as NmWifiSecurity,
};

use super::{KnownNetworkInfo, Security, StationState, StationStateStream, WifiError, WifiNetwork};

/// How long to wait for the scan the listing asked for to finish.
const SCAN_TIMEOUT: Duration = Duration::from_secs(10);
/// How often to ask the adapter whether its scan has finished.
const SCAN_POLL: Duration = Duration::from_millis(250);
/// NM's device type for a wireless device.
const DEVICE_TYPE_WIFI: u32 = 2;
/// How many times a connection that fails for a reason another attempt
/// could fix is tried.
const CONNECT_ATTEMPTS: usize = 3;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManagerRoot {
    fn get_devices(&self) -> zbus::Result<Vec<zbus::zvariant::OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait NmDevice {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait NmWireless {
    /// When the device's last scan finished, in CLOCK_BOOTTIME
    /// milliseconds. -1 until it has ever scanned.
    #[zbus(property)]
    fn last_scan(&self) -> zbus::Result<i64>;
}

/// The wireless device's record of when it last finished scanning.
///
/// `RequestScan` returns as soon as NetworkManager accepts the request,
/// and the access points arrive seconds later, so this is what says
/// whether the list is the answer to the scan just asked for or the one
/// before it.
async fn last_scan_finished() -> Option<i64> {
    let conn = zbus::Connection::system().await.ok()?;
    let nm = NetworkManagerRootProxy::new(&conn).await.ok()?;
    for path in nm.get_devices().await.ok()? {
        let device = NmDeviceProxy::builder(&conn)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()?;
        if device.device_type().await.ok()? != DEVICE_TYPE_WIFI {
            continue;
        }
        let wireless = NmWirelessProxy::builder(&conn)
            .path(path)
            .ok()?
            .build()
            .await
            .ok()?;
        return wireless.last_scan().await.ok();
    }
    None
}

/// Wait for a scan that finished later than `before`, giving up after
/// `SCAN_TIMEOUT`.
async fn wait_for_scan(before: Option<i64>) {
    let deadline = tokio::time::Instant::now() + SCAN_TIMEOUT;
    loop {
        tokio::time::sleep(SCAN_POLL).await;
        match (last_scan_finished().await, before) {
            (Some(now), Some(before)) if now > before => return,
            (Some(now), None) if now >= 0 => return,
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::debug!("NetworkManager did not report a finished scan");
            return;
        }
    }
}

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

    // NetworkManager answers RequestScan as soon as it takes the request,
    // and the access points arrive seconds afterwards. Reading the list
    // straight after the call hands back the results of the scan before
    // this one — on the test laptop, a list without the network the user
    // was reaching for, which the connection then could not find.
    let before = last_scan_finished().await;
    nm.scan_networks(None).await?;
    wait_for_scan(before).await;

    // Pull saved connection names so we can flag known networks in
    // the scan result. `list_saved_connection_ids` returns the NM
    // connection IDs which equal the SSIDs for standard wifi profiles.
    let known: std::collections::HashSet<String> = nm
        .list_saved_connection_ids()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    let raw = nm.list_networks(None).await?;
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
    out.sort_by_key(|network| std::cmp::Reverse(network.signal_percent));
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
    //
    // The network the user picked can be missing from the list a moment
    // after they picked it: this adapter drops a different set of
    // channels on every pass, so a scan of its own is worth asking for
    // before taking "not there" as the answer.
    let security = match find_network(&nm, ssid).await? {
        Some(net) => network_to_security(&net),
        None => {
            let before = last_scan_finished().await;
            let _ = nm.scan_networks(None).await;
            wait_for_scan(before).await;
            let net = find_network(&nm, ssid)
                .await?
                .ok_or_else(|| WifiError::NetworkNotFound(ssid.to_string()))?;
            network_to_security(&net)
        }
    };

    let psk = match security {
        Security::Open => None,
        Security::Wep | Security::Psk => {
            let psk = passphrase.ok_or_else(|| WifiError::PassphraseRequired(ssid.to_string()))?;
            if psk.is_empty() {
                return Err(WifiError::PassphraseRequired(ssid.to_string()));
            }
            Some(psk)
        }
        Security::Enterprise => {
            return Err(WifiError::ConnectFailed(
                "Enterprise (802.1x) networks are not supported by the installer".into(),
            ));
        }
    };

    for attempt in 1..=CONNECT_ATTEMPTS {
        let creds = match &psk {
            None => NmWifiSecurity::Open,
            Some(psk) => NmWifiSecurity::WpaPsk { psk: psk.clone() },
        };
        let error = match nm.connect(ssid, None, creds).await {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };

        if attempt == CONNECT_ATTEMPTS || !worth_another_attempt(&error) {
            return Err(match error {
                // The generic conversion has no SSID to put in the message.
                NmConnectionError::NotFound => WifiError::NetworkNotFound(ssid.to_string()),
                other => other.into(),
            });
        }
        tracing::warn!(attempt, %error, "connect failed; trying again");

        // The access point the connection could not find may be back in
        // the next scan: this adapter drops a different set of them on
        // every pass.
        let before = last_scan_finished().await;
        let _ = nm.scan_networks(None).await;
        wait_for_scan(before).await;
    }
    unreachable!("the loop returns on its last attempt")
}

/// The access point NetworkManager is currently holding for `ssid`.
async fn find_network(nm: &NetworkManager, ssid: &str) -> Result<Option<NmNetwork>, WifiError> {
    let networks = nm.list_networks(None).await?;
    Ok(networks.into_iter().find(|n| n.ssid == ssid))
}

/// Whether a failed connection is worth another attempt.
///
/// NetworkManager says which part gave way, so a passphrase it rejected
/// is reported at once and only the failures another attempt could fix
/// are repeated: an access point that was not in the list this time, and
/// the association timeouts this adapter produces for the first minutes
/// after a boot.
fn worth_another_attempt(error: &NmConnectionError) -> bool {
    matches!(
        error,
        NmConnectionError::NotFound
            | NmConnectionError::Timeout
            | NmConnectionError::SupplicantTimeout
            | NmConnectionError::SupplicantConfigFailed
            | NmConnectionError::Stuck(_)
    )
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
    nm.disconnect(None).await?;
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
    let ssids = nm.list_saved_connection_ids().await?;
    let mut out: Vec<KnownNetworkInfo> = ssids
        .into_iter()
        .map(|ssid| KnownNetworkInfo {
            ssid,
            // The lightweight ID query omits security and the UI does not
            // branch on it. Default to Psk.
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
