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

use futures::StreamExt;
use iwdrs::{
    agent::Agent,
    error::{IWDError, agent::Canceled, network::ConnectError},
    network::{Network, NetworkType},
    session::Session,
    station::{State as IwdState, Station},
};

use super::{
    KnownNetworkInfo, Security, StationState, StationStateStream, WifiError, WifiNetwork,
    signal_to_percent,
};

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

/// Trigger a scan on the first station and return all discovered
/// networks, sorted roughly strongest-first by iwd itself. Networks
/// already in iwd's known-network database are flagged with
/// `known: true`.
pub async fn scan_networks() -> Result<Vec<WifiNetwork>, WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = first_station(&session).await?;

    // Trigger a fresh scan. iwd reports "already scanning" as a method
    // error — treat it as success and fall through to fetching results.
    if let Err(e) = station.scan().await {
        tracing::debug!(?e, "iwd scan() returned error (may already be scanning)");
    }

    // Block until the scan finishes (iwd emits Scanning=false).
    if station.wait_for_scan_complete().await.is_err() {
        return Err(WifiError::ScanFailed);
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

/// Connect to `ssid` by triggering iwd's connect flow and providing
/// `passphrase` via a registered one-shot agent.
///
/// For open networks `passphrase` may be `None`. For secured networks
/// `passphrase` must be `Some(...)`; omitting it returns
/// `WifiError::PassphraseRequired`.
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

    let security: Security = network
        .network_type()
        .await
        .map(Security::from)
        .unwrap_or(Security::Open);

    if security.requires_passphrase() && passphrase.is_none() {
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
