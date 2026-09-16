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

use std::time::{Duration, Instant};

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
/// How many scans every listing is built from. One scan is one pass over
/// the channels, and a pass that misses a subset misses every network on
/// it — including, on the test laptop, the access point the user was
/// looking for. Two passes are also what tells a daemon that has nothing
/// to say once from one that is stuck saying it.
const SCANS_PER_LISTING: usize = 2;
/// How long to wait for iwd to put a station on the bus.
const STATION_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a network stays in the listing after the last scan that saw
/// it. Long enough to survive the scans that drop it, short enough that
/// a network really gone is not offered for the rest of the session.
const NETWORK_MEMORY: Duration = Duration::from_secs(120);
/// How long to keep trying to reach iwd again after restarting it.
const RESTART_REJOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How many times a connection that fails for a reason another attempt
/// could fix is tried.
const CONNECT_ATTEMPTS: usize = 3;
/// How long to leave the adapter alone between connection attempts.
const CONNECT_RETRY_DELAY: Duration = Duration::from_secs(2);

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
    let station = station_once_iwd_has_one(&session).await?;

    // Whatever iwd's own periodic scanning already turned up costs one
    // call to collect and is as good as anything this function asks for.
    let mut found = Listing::default();
    found.add(read_networks(&station).await.unwrap_or_default());

    for attempt in 1..=SCANS_PER_LISTING {
        found.add(scan_once(&station).await?);
        tracing::debug!(attempt, networks = found.len(), "scanned");
    }
    if !found.is_empty() {
        return Ok(remember(found));
    }

    // Leave a working connection alone: restarting iwd would drop it, and
    // a station that is connected has something better to say than an
    // empty list anyway.
    if !matches!(station.state().await, Ok(IwdState::Disconnected)) {
        tracing::info!("no networks found; leaving the connected station alone");
        return Ok(Vec::new());
    }

    tracing::warn!("no networks after {SCANS_PER_LISTING} scans; restarting iwd");
    let session = match restart_iwd().await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(%error, "could not restart iwd");
            return Ok(Vec::new());
        }
    };
    let station = station_once_iwd_has_one(&session).await?;
    for attempt in 1..=SCANS_PER_LISTING {
        found.add(scan_once(&station).await?);
        tracing::debug!(attempt, networks = found.len(), "scanned after the restart");
    }
    Ok(remember(found))
}

/// Add what earlier scans saw to this listing, and keep the result for
/// the scans that come after it.
fn remember(mut found: Listing) -> Vec<WifiNetwork> {
    let mut remembered = REMEMBERED.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = found.len();
    if let Some(previous) = remembered.as_ref() {
        found.carried_over(previous);
    }
    if found.len() > fresh {
        tracing::debug!(
            fresh,
            listed = found.len(),
            "some networks are from earlier scans"
        );
    }
    let networks = found.clone_networks();
    *remembered = Some(found);
    networks
}

/// The networks several scans found between them, strongest first.
///
/// One scan is not a complete picture on an adapter that drops a subset
/// of the channels, and the networks it misses are not the same ones
/// each time, so a listing is what the scans found together rather than
/// what the last one happened to return.
#[derive(Default)]
struct Listing(std::collections::HashMap<String, (WifiNetwork, Option<Instant>)>);

impl Listing {
    fn add(&mut self, networks: Vec<WifiNetwork>) {
        let now = Instant::now();
        for network in networks {
            match self.0.get(&network.ssid) {
                // The same network seen twice is kept at its best, which
                // is also the reading the user is nearest to.
                Some((seen, _)) if seen.signal_percent >= network.signal_percent => {
                    self.0
                        .entry(network.ssid)
                        .and_modify(|(_, at)| *at = Some(now));
                }
                _ => {
                    self.0.insert(network.ssid.clone(), (network, Some(now)));
                }
            }
        }
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn clone_networks(&self) -> Vec<WifiNetwork> {
        let mut networks: Vec<WifiNetwork> = self.0.values().map(|(n, _)| n.clone()).collect();
        networks.sort_by(|a, b| {
            b.signal_percent
                .cmp(&a.signal_percent)
                .then_with(|| a.ssid.cmp(&b.ssid))
        });
        networks
    }

    /// Everything still worth showing from an earlier listing, and
    /// everything this one found.
    fn carried_over(&mut self, previous: &Listing) {
        let now = Instant::now();
        for (ssid, (network, seen_at)) in &previous.0 {
            let fresh = seen_at.is_some_and(|at| now.duration_since(at) < NETWORK_MEMORY);
            if fresh && !self.0.contains_key(ssid) {
                self.0.insert(ssid.clone(), (network.clone(), *seen_at));
            }
        }
    }
}

/// The listing the last scan produced, kept so that a network this scan
/// missed is still offered.
///
/// An adapter that drops a subset of the channels drops different
/// networks each time, and a list rebuilt from nothing on every press of
/// Rescan loses the one the user is waiting for as readily as it finds
/// it. Remembering what the last minutes saw turns "press it again and
/// hope" into a list that only grows more complete.
static REMEMBERED: std::sync::Mutex<Option<Listing>> = std::sync::Mutex::new(None);

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

    read_networks(station).await
}

/// What iwd is holding as the networks it knows about, without asking it
/// to look again.
async fn read_networks(station: &Station) -> Result<Vec<WifiNetwork>, WifiError> {
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
    let station = station_once_iwd_has_one(&session).await?;

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

    connect_with_retries(&station, &network, ssid).await
}

/// Ask iwd to connect, and ask again when the answer is the kind of
/// failure that another attempt can fix.
///
/// An adapter whose firmware has just come up can time out on the
/// association itself, seconds into a connection to an access point a
/// metre away:
///
/// ```text
/// event: connect-info, ssid: …, signal: -35, load: 21/255
/// event: state, old: autoconnect_full, new: connecting
/// event: connect-timeout, reason: 2
/// event: connect-failed, status: 1
/// ```
///
/// The test laptop's RTL8723BE needed three goes at it after a boot,
/// with nothing to tell the user but "connect failed" in between. A
/// passphrase iwd rejects, and a request iwd cannot carry out at all,
/// are answered on the first attempt as before.
async fn connect_with_retries(
    station: &Station,
    network: &Network,
    ssid: &str,
) -> Result<(), WifiError> {
    let mut network = network.clone();
    for attempt in 1..=CONNECT_ATTEMPTS {
        let error = match network.connect().await {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };

        if attempt == CONNECT_ATTEMPTS || !worth_another_attempt(&error) {
            return Err(error.into());
        }
        tracing::warn!(attempt, ?error, "connect failed; trying again");
        tokio::time::sleep(CONNECT_RETRY_DELAY).await;

        // iwd falls back to autoconnect after a failed connection and may
        // have arrived where the user wanted while we waited.
        if network.connected().await.unwrap_or(false) {
            tracing::info!("iwd connected on its own");
            return Ok(());
        }

        // The failed attempt can take the network object with it: iwd
        // drops networks whose last BSS aged out of the scan results.
        match find_network_by_ssid(station, ssid).await {
            Ok(Some(found)) => network = found,
            Ok(None) => return Err(WifiError::NetworkNotFound(ssid.to_string())),
            Err(error) => return Err(error),
        }
    }
    unreachable!("the loop returns on its last attempt")
}

/// Whether a failed connection is worth another attempt: a timeout or an
/// aborted attempt is the adapter having a bad moment, while a missing
/// agent or an unsupported network will fail the same way every time.
fn worth_another_attempt(error: &IWDError<ConnectError>) -> bool {
    matches!(
        error,
        IWDError::OperationError(
            ConnectError::Failed
                | ConnectError::Aborted
                | ConnectError::Busy
                | ConnectError::InProgress
        )
    )
}

/// Connect to a hidden network — one that does not broadcast its SSID
/// and therefore doesn't appear in scan results.
pub async fn connect_hidden(ssid: &str, passphrase: Option<String>) -> Result<(), WifiError> {
    let session = Session::new().await.map_err(|_| WifiError::NotAvailable)?;
    let station = station_once_iwd_has_one(&session).await?;

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

/// The first station, giving iwd time to put one on the bus.
///
/// A daemon that has only just started — at boot, or after this module
/// restarted it — owns its bus name before it has enumerated the
/// adapter, and answers in between with no stations at all. Asking once
/// in that moment told the user their adapter may be powered off while
/// iwd was seconds away from offering it.
async fn station_once_iwd_has_one(session: &Session) -> Result<Station, WifiError> {
    let deadline = tokio::time::Instant::now() + STATION_TIMEOUT;
    loop {
        match first_station(session).await {
            Ok(station) => return Ok(station),
            Err(error) if tokio::time::Instant::now() >= deadline => return Err(error),
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
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
