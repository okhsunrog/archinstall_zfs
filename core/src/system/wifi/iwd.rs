//! iwd-backed wifi implementation.
//!
//! All operations go through the `iwdrs` crate, which wraps iwd's
//! `net.connman.iwd` system-bus interface. No subprocess spawning, no
//! text parsing, no stringly-typed output.
//!
//! # Multi-interface
//!
//! Every operation acts on the **first** station iwd exposes. Machines
//! with more than one wireless adapter are extremely rare in installer
//! scenarios, so we deliberately skip the interface picker.
//! TODO: expose a `station_by_name(&str)` variant if users actually
//! request it.
//!
//! # Agent pattern
//!
//! iwd doesn't take passphrases as method arguments. It requires clients
//! to register a D-Bus agent object, then call `Network::connect()`,
//! at which point iwd calls back into the agent to ask for the passphrase.
//! [`connect`] constructs a one-shot [`PasswordAgent`] holding the
//! user's passphrase, registers it for the duration of the connect
//! call, and drops it when the Session goes out of scope.
//!
//! # Profile persistence
//!
//! After a successful `Network::connect()`, iwd writes the profile to
//! `/var/lib/iwd/<SSID>.<type>` automatically. `copy_iso_network` in the
//! installer picks that file up and copies it to the target so the
//! installed system reconnects without re-prompting for credentials.

use std::time::Duration;

use futures::StreamExt;
use iwdrs::{
    agent::Agent,
    error::{IWDError, agent::Canceled, network::ConnectError},
    network::{Network, NetworkType},
    session::Session,
    station::{State as IwdState, Station},
};

use super::{KnownNetworkInfo, Security, StationState, StationStateStream, WifiError, WifiNetwork};

/// Map iwd's dBm*100 signal strength to a 0-100 percentage.
///
/// iwd reports signal strength in hundredths of a dBm, i.e. `-5000`
/// means `-50 dBm`. The classic mapping (also used by NetworkManager's
/// `nm_wifi_utils_level_to_quality`) is:
///
/// * `≥ -50 dBm` → 100
/// * `≤ -100 dBm` → 0
/// * linear interpolation in between
///
/// Lives in this backend module because iwd's input format is
/// backend-specific; NetworkManager already exposes strength as a
/// 0-100 `u8` and needs no conversion.
fn signal_to_percent(dbm_times_100: i16) -> u8 {
    let dbm = dbm_times_100 as i32 / 100;
    if dbm >= -50 {
        100
    } else if dbm <= -100 {
        0
    } else {
        ((dbm + 100) * 2).clamp(0, 100) as u8
    }
}

// ─── type conversions ───────────────────────────────────────────────────

impl From<NetworkType> for Security {
    fn from(nt: NetworkType) -> Self {
        match nt {
            NetworkType::Open => Security::Open,
            NetworkType::Wep => Security::Wep,
            NetworkType::Psk => Security::Psk,
            NetworkType::Eap => Security::Enterprise,
        }
    }
}

impl From<IwdState> for StationState {
    fn from(s: IwdState) -> Self {
        match s {
            IwdState::Connected => StationState::Connected,
            IwdState::Disconnected => StationState::Disconnected,
            IwdState::Connecting => StationState::Connecting,
            IwdState::Disconnecting => StationState::Disconnecting,
            IwdState::Roaming => StationState::Roaming,
        }
    }
}

/// Convert `iwdrs::error::IWDError<ConnectError>` into our backend-
/// agnostic error type. `OperationError` carries an iwd-specific
/// `ConnectError` enum whose `Display` impl we reuse for the message.
impl From<IWDError<ConnectError>> for WifiError {
    fn from(err: IWDError<ConnectError>) -> Self {
        match err {
            IWDError::OperationError(op) => WifiError::ConnectFailed(op.to_string()),
            IWDError::ZbusError(z) => WifiError::Dbus(z),
        }
    }
}

// ─── public API (re-exported as `wifi::*` from mod.rs) ──────────────────

/// Fast probe: is iwd running and reachable on the system bus?
///
/// Returns `false` on a system where iwd is not installed (the `--fast`
/// test ISO), where the daemon is masked, or where the D-Bus connection
/// otherwise fails. This is the gate the GUI corner widget uses to
/// decide whether to enable the rich wifi-management popup.
pub async fn backend_available() -> bool {
    Session::new().await.is_ok()
}

/// Compatibility alias. The welcome-view controller uses the generic
/// name `iwd_available` in log messages and state property names;
/// kept as an alias so the slint-ui layer doesn't need to change when
/// the backend is swapped.
pub use backend_available as iwd_available;

/// How long to wait for iwd to report that the scan it was asked for has
/// started. `Scan()` returns once the request is queued, and `Scanning`
/// turns true a moment later.
const SCAN_START_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a single scan may take before it is given up on.
const SCAN_COMPLETE_TIMEOUT: Duration = Duration::from_secs(30);
/// How many scans have to come back with nothing before the daemon
/// itself is suspected. The second one is what tells a one-off apart
/// from the stuck state, which answers every scan with nothing.
const EMPTY_SCAN_ATTEMPTS: usize = 2;
/// How long to keep trying to reach iwd again after restarting it.
const RESTART_REJOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Trigger a scan on the first station and return all discovered
/// networks, sorted roughly strongest-first by iwd itself. Networks
/// already in iwd's known-network database are flagged with
/// `known: true`.
///
/// An empty result is not taken at face value. iwd splits a scan over
/// subsets of the channels, and on some adapters a subset comes back
/// with nothing and iwd drops every network the earlier subsets found:
///
/// ```text
/// station_dbus_scan_triggered() Scan triggered for wlan0 subset 1
/// scan_notify() Scan notification New Scan Results
/// process_network() No remaining BSSs for SSID: … -- Removing network
/// ```
///
/// The station stays in that state — further scans, including iwd's own
/// periodic ones, keep answering with nothing — while the kernel still
/// scans perfectly well, so the installer used to offer an empty list on
/// hardware with networks all around it. Scanning again clears a one-off;
/// only restarting iwd clears the stuck state, and a station with no
/// connection to lose can be restarted under the user.
pub async fn scan_networks() -> Result<Vec<WifiNetwork>, WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let mut station = first_station(&session).await?;

    for attempt in 1..=EMPTY_SCAN_ATTEMPTS {
        let networks = scan_once(&station).await?;
        if !networks.is_empty() {
            return Ok(networks);
        }
        tracing::debug!(attempt, "the scan found no networks");
    }

    // Leave a working connection alone: restarting iwd would drop it, and
    // a station that is connected has something better to say than an
    // empty list anyway.
    if !matches!(station.state().await, Ok(IwdState::Disconnected)) {
        tracing::info!("no networks found; leaving the connected station alone");
        return Ok(Vec::new());
    }

    tracing::warn!("no networks after {EMPTY_SCAN_ATTEMPTS} scans; restarting iwd");
    let session = match restart_iwd().await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(%error, "could not restart iwd");
            return Ok(Vec::new());
        }
    };
    station = first_station(&session).await?;
    scan_once(&station).await
}

/// One scan: ask for it, wait for it to finish, read what it found.
async fn scan_once(station: &Station) -> Result<Vec<WifiNetwork>, WifiError> {
    // Trigger a fresh scan. iwd reports "already scanning" as a method
    // error — treat it as success and fall through to fetching results.
    if let Err(e) = station.scan().await {
        tracing::debug!(?e, "iwd scan() returned error (may already be scanning)");
    }

    // `Scanning` is still false for a moment after `Scan()` returns, and
    // waiting for "not scanning" in that moment is answered by the state
    // from before the scan — leaving the results of the previous one, or
    // nothing at all, to be read as this scan's answer.
    wait_for_scan_start(station).await;

    // Block until the scan finishes (iwd emits Scanning=false).
    match tokio::time::timeout(SCAN_COMPLETE_TIMEOUT, station.wait_for_scan_complete()).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => return Err(WifiError::ScanFailed),
        Err(_) => {
            tracing::warn!("iwd is still scanning after {SCAN_COMPLETE_TIMEOUT:?}");
            return Err(WifiError::ScanFailed);
        }
    }

    let discovered = station
        .discovered_networks()
        .await
        .map_err(|_| WifiError::ScanFailed)?;

    let mut out = Vec::with_capacity(discovered.len());
    for (network, signal) in discovered {
        let ssid = match network.name().await {
            Ok(n) => n,
            Err(_) => continue,
        };
        let security = network
            .network_type()
            .await
            .map(Security::from)
            .unwrap_or(Security::Open);
        let known = network
            .known_network()
            .await
            .map(|k| k.is_some())
            .unwrap_or(false);

        out.push(WifiNetwork {
            ssid,
            signal_percent: signal_to_percent(signal),
            security,
            known,
        });
    }

    Ok(out)
}

/// Wait for iwd to report the scan as running, giving up after
/// `SCAN_START_TIMEOUT`: a scan that finished that quickly is one whose
/// results are already there to read.
async fn wait_for_scan_start(station: &Station) {
    let deadline = tokio::time::Instant::now() + SCAN_START_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        match station.is_scanning().await {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) => return,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tracing::debug!("iwd did not report a scan starting");
}

/// Restart iwd and return a session on the daemon that comes back.
async fn restart_iwd() -> Result<Session, WifiError> {
    let status = tokio::process::Command::new("systemctl")
        .args(["restart", "iwd"])
        .status()
        .await
        .map_err(|_| WifiError::NotAvailable)?;
    if !status.success() {
        return Err(WifiError::NotAvailable);
    }

    // The daemon takes a moment to claim its bus name again.
    let deadline = tokio::time::Instant::now() + RESTART_REJOIN_TIMEOUT;
    loop {
        if let Ok(session) = Session::new().await {
            tracing::info!("iwd restarted");
            return Ok(session);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(WifiError::NotAvailable);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Connect to `ssid` by triggering iwd's connect flow and providing
/// `passphrase` via a registered one-shot agent.
///
/// Saved networks reuse iwd's credentials when `passphrase` is `None`.
/// Unknown secured networks require a passphrase. An already-connected
/// network is left connected, including when it was joined through iwctl.
///
/// Returns once iwd reports the connect call complete — success means
/// the station reached the Connected state at layer 2. Callers that
/// need to verify internet connectivity should follow up with
/// `crate::system::net::check_internet`.
pub async fn connect(ssid: &str, passphrase: Option<String>) -> Result<(), WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;

    let network = find_network_by_ssid(&station, ssid)
        .await?
        .ok_or_else(|| WifiError::NetworkNotFound(ssid.to_string()))?;

    if network.connected().await.map_err(WifiError::Dbus)? {
        return Ok(());
    }

    let security: Security = network
        .network_type()
        .await
        .map(Security::from)
        .unwrap_or(Security::Open);

    if security.requires_passphrase()
        && passphrase.is_none()
        && network
            .known_network()
            .await
            .map_err(WifiError::Dbus)?
            .is_none()
    {
        return Err(WifiError::PassphraseRequired(ssid.to_string()));
    }

    // Register the agent before calling connect(). The AgentManager is
    // held in `_agent_guard` for the duration of the connect call —
    // when it drops, iwd unregisters the agent.
    let agent = PasswordAgent::new(passphrase);
    let _agent_guard = session
        .register_agent(agent)
        .await
        .map_err(WifiError::Dbus)?;

    network.connect().await?;
    Ok(())
}

/// Connect to a hidden network — one that does not broadcast its SSID
/// and therefore doesn't appear in scan results.
pub async fn connect_hidden(ssid: &str, passphrase: Option<String>) -> Result<(), WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;

    let agent = PasswordAgent::new(passphrase);
    let _agent_guard = session
        .register_agent(agent)
        .await
        .map_err(WifiError::Dbus)?;

    station
        .connect_hidden_network(ssid.to_string())
        .await
        .map_err(|e| match e {
            IWDError::OperationError(op) => WifiError::ConnectFailed(op.to_string()),
            IWDError::ZbusError(z) => WifiError::Dbus(z),
        })?;
    Ok(())
}

/// Disconnect the active station, if any.
pub async fn disconnect() -> Result<(), WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;
    station.disconnect().await.map_err(|e| match e {
        IWDError::OperationError(op) => WifiError::ConnectFailed(op.to_string()),
        IWDError::ZbusError(z) => WifiError::Dbus(z),
    })?;
    Ok(())
}

/// Return the current station state, or an error if iwd is not reachable.
pub async fn station_state() -> Result<StationState, WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;
    let state = station.state().await.map_err(WifiError::Dbus)?;
    Ok(state.into())
}

/// `true` if the station is in the Connected state. Quick layer-2 check;
/// for "can reach the internet" use `system::net::check_internet`.
pub async fn check_connected() -> Result<bool, WifiError> {
    Ok(matches!(station_state().await?, StationState::Connected))
}

/// Return the SSID of the currently-connected network, if any.
pub async fn current_ssid() -> Result<Option<String>, WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;
    let Some(network) = station.connected_network().await.map_err(WifiError::Dbus)? else {
        return Ok(None);
    };
    Ok(Some(network.name().await.map_err(WifiError::Dbus)?))
}

/// Subscribe to station-state changes. The stream yields every time iwd
/// emits `PropertiesChanged` on the `State` property, starting with the
/// current value. Used by the GUI to keep the corner widget in sync
/// without polling.
pub async fn watch_station_state() -> Result<StationStateStream, WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;
    let stream = station.state_stream().await.map_err(WifiError::Dbus)?;

    // Map iwdrs's `Item = zbus::Result<IwdState>` onto our common
    // `Item = Result<StationState, WifiError>` so callers don't need to
    // care which backend produced the stream.
    let mapped = stream.map(|r| match r {
        Ok(s) => Ok(StationState::from(s)),
        Err(e) => Err(WifiError::Dbus(e)),
    });

    // The underlying Proxy inside the stream already holds its own
    // Connection clone, so dropping `session`/`station` is safe at the
    // D-Bus level. We leak them anyway because the 2024 capture rules
    // make the compiler conservative; the leak cost is a couple of
    // Arc-ish handles per subscriber (the GUI opens at most one).
    std::mem::forget(station);
    std::mem::forget(session);
    Ok(Box::pin(mapped))
}

/// Enumerate saved networks in iwd's known-network database.
pub async fn list_known_networks() -> Result<Vec<KnownNetworkInfo>, WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let known = session.known_networks().await.map_err(WifiError::Dbus)?;

    let mut out = Vec::with_capacity(known.len());
    for kn in known {
        let ssid = match kn.name().await {
            Ok(n) => n,
            Err(_) => continue,
        };
        let security = kn
            .network_type()
            .await
            .map(Security::from)
            .unwrap_or(Security::Open);
        let hidden = kn.hidden().await.unwrap_or(false);
        out.push(KnownNetworkInfo {
            ssid,
            security,
            hidden,
        });
    }
    out.sort_by(|a, b| a.ssid.cmp(&b.ssid));
    Ok(out)
}

/// Forget the saved iwd profile for `ssid`.
///
/// iwd's `KnownNetwork.Forget` deletes the profile file (e.g.
/// `/var/lib/iwd/Foo.psk`) and removes the object from the bus.
/// Returns `Ok(())` if no profile was present under that name to
/// begin with — idempotent on purpose.
pub async fn forget_network(ssid: &str) -> Result<(), WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let known = session.known_networks().await.map_err(WifiError::Dbus)?;
    for kn in known {
        if kn.name().await.ok().as_deref() == Some(ssid) {
            kn.forget().await.map_err(WifiError::Dbus)?;
            return Ok(());
        }
    }
    Ok(())
}

// ─── internal helpers ───────────────────────────────────────────────────

/// Return the first station exposed by iwd.
///
/// TODO: multi-adapter machines pick the first station arbitrarily.
/// Extend to take an interface name when a real user reports needing it.
async fn first_station(session: &Session) -> Result<Station, WifiError> {
    let mut stations = session.stations().await.map_err(WifiError::Dbus)?;
    if stations.is_empty() {
        // Distinguish "no wireless hardware" from "hardware present but
        // no station registered" (adapter powered off, etc.). We only
        // get here when iwd is running, so NoStation is the right
        // variant — NoAdapter would mean no hardware at all.
        return Err(WifiError::NoStation);
    }
    Ok(stations.swap_remove(0))
}

/// Walk the discovered networks of `station` looking for one whose SSID
/// matches. Does not trigger a new scan.
async fn find_network_by_ssid(station: &Station, ssid: &str) -> Result<Option<Network>, WifiError> {
    let discovered = station
        .discovered_networks()
        .await
        .map_err(WifiError::Dbus)?;
    for (network, _signal) in discovered {
        if network.name().await.ok().as_deref() == Some(ssid) {
            return Ok(Some(network));
        }
    }
    Ok(None)
}

/// One-shot passphrase agent: holds a single passphrase and returns it
/// exactly once when iwd asks. All other agent callbacks (private-key
/// passphrase, user+password, etc.) return `Canceled` since the
/// installer does not support enterprise authentication.
struct PasswordAgent {
    passphrase: Option<String>,
}

impl PasswordAgent {
    fn new(passphrase: Option<String>) -> Self {
        Self { passphrase }
    }
}

impl Agent for PasswordAgent {
    fn request_passphrase(
        &self,
        _network: &Network,
    ) -> impl std::future::Future<Output = Result<String, Canceled>> + Send {
        let result = match self.passphrase.clone() {
            Some(psk) => Ok(psk),
            None => Err(Canceled {}),
        };
        std::future::ready(result)
    }

    fn request_private_key_passphrase(
        &self,
        _network: &Network,
    ) -> impl std::future::Future<Output = Result<String, Canceled>> + Send {
        std::future::ready(Err(Canceled {}))
    }

    fn request_user_name_and_passphrase(
        &self,
        _network: &Network,
    ) -> impl std::future::Future<Output = Result<(String, String), Canceled>> + Send {
        std::future::ready(Err(Canceled {}))
    }

    fn request_user_password(
        &self,
        _network: &Network,
        _user_name: Option<&String>,
    ) -> impl std::future::Future<Output = Result<String, Canceled>> + Send {
        std::future::ready(Err(Canceled {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
