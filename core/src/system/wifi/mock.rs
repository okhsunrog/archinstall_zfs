//! Mock wifi backend.
//!
//! A hand-rolled in-memory backend that returns a fixed set of networks,
//! simulates realistic delays (scans take ~1.5 s, connects take ~2 s,
//! verify sleeps another 2 s), and tracks a tiny state machine so that
//! `connect` → `current_ssid` → `disconnect` round-trips behave like
//! real hardware. Intended for:
//!
//!   * CI / unit tests that exercise controllers without a live daemon
//!   * Desktop development where the dev's daily laptop runs something
//!     other than iwd or NetworkManager
//!   * Demos and screenshots with deterministic network lists
//!
//! Selected by the `wifi-mock` Cargo feature. Mutually exclusive with
//! `wifi-iwd` and `wifi-nm` — the `mod.rs` compile-error guards
//! enforce that at build time.
//!
//! Canned networks (signal from strongest to weakest, one open, one
//! enterprise, one known to exercise the "skip password prompt" path):
//!
//!   HomeNetwork    — WPA2, known, signal 90
//!   Neighbour 5G   — WPA2, not known, signal 72
//!   CoffeeShop     — open, signal 55
//!   CorpNet        — enterprise, signal 48
//!   DistantAP      — WPA2, signal 22
//!
//! "HomeNetwork" is in the known-network database with passphrase
//! "hunter2". Attempting to connect to it without a passphrase still
//! succeeds (simulating iwd auto-connect). Any other secured network
//! requires a passphrase; the mock treats any non-empty string as
//! correct and any empty or missing string as `PassphraseRequired`.

use std::sync::Mutex;
use std::time::Duration;

use futures::stream;

use super::{KnownNetworkInfo, Security, StationState, StationStateStream, WifiError, WifiNetwork};

// ─── in-memory state ────────────────────────────────────────────────────

/// Global mutable state for the mock backend. A single `Mutex<State>`
/// is fine because the installer has exactly one wifi client and we
/// never hit any lock contention worth measuring.
static STATE: Mutex<State> = Mutex::new(State::new());

struct State {
    connected_ssid: Option<String>,
    /// SSIDs that should be reported as "known" (saved profiles).
    /// Starts with one preseed so the UI can exercise the known path
    /// without having to connect first.
    known: Vec<String>,
}

impl State {
    const fn new() -> Self {
        Self {
            connected_ssid: None,
            known: Vec::new(),
        }
    }
}

fn canned_networks() -> Vec<WifiNetwork> {
    let known = {
        let g = STATE.lock().unwrap();
        g.known.clone()
    };
    // Preseed HomeNetwork as known on every scan so the first run
    // after process start already shows the Known badge without
    // needing a previous connect.
    let is_known = |ssid: &str| ssid == "HomeNetwork" || known.iter().any(|s| s == ssid);

    vec![
        WifiNetwork {
            ssid: "HomeNetwork".into(),
            signal_percent: 90,
            security: Security::Psk,
            known: is_known("HomeNetwork"),
        },
        WifiNetwork {
            ssid: "Neighbour 5G".into(),
            signal_percent: 72,
            security: Security::Psk,
            known: is_known("Neighbour 5G"),
        },
        WifiNetwork {
            ssid: "CoffeeShop".into(),
            signal_percent: 55,
            security: Security::Open,
            known: is_known("CoffeeShop"),
        },
        WifiNetwork {
            ssid: "CorpNet".into(),
            signal_percent: 48,
            security: Security::Enterprise,
            known: is_known("CorpNet"),
        },
        WifiNetwork {
            ssid: "DistantAP".into(),
            signal_percent: 22,
            security: Security::Psk,
            known: is_known("DistantAP"),
        },
    ]
}

// ─── public API ─────────────────────────────────────────────────────────

/// Always returns `true`: the mock backend has no external dependency
/// and is always "available" by construction.
pub async fn backend_available() -> bool {
    true
}

/// Compatibility alias. The welcome-view controller uses the generic
/// `iwd_available` name for the probe; the mock re-exports under that
/// name so the controller code is identical across backends.
pub use backend_available as iwd_available;

/// Simulate a fresh scan. Sleeps 1.5 s to give controllers a chance to
/// show their spinner state.
pub async fn scan_networks() -> Result<Vec<WifiNetwork>, WifiError> {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(canned_networks())
}

/// Simulate a connect attempt.
///
/// Success criteria:
///   * open networks always succeed
///   * known networks always succeed (profile covers the passphrase)
///   * unknown secured networks require a non-empty passphrase; any
///     non-empty string is accepted
///   * enterprise networks always fail with `ConnectFailed` (matches
///     the UI policy that enterprise is disabled)
///
/// Sleeps 2 s before returning so the "Connecting…" spinner is visible.
pub async fn connect(ssid: &str, passphrase: Option<String>) -> Result<(), WifiError> {
    tokio::time::sleep(Duration::from_secs(2)).await;

    let networks = canned_networks();
    let Some(net) = networks.iter().find(|n| n.ssid == ssid) else {
        return Err(WifiError::NetworkNotFound(ssid.to_string()));
    };

    match net.security {
        Security::Enterprise => {
            return Err(WifiError::ConnectFailed(
                "Enterprise networks are not supported by the mock backend".into(),
            ));
        }
        Security::Open => { /* no credentials needed */ }
        Security::Wep | Security::Psk => {
            if !net.known {
                match passphrase.as_deref() {
                    Some(p) if !p.is_empty() => { /* accepted */ }
                    _ => return Err(WifiError::PassphraseRequired(ssid.to_string())),
                }
            }
            // known networks skip the check — profile covers it
        }
    }

    let mut g = STATE.lock().unwrap();
    g.connected_ssid = Some(ssid.to_string());
    if !g.known.iter().any(|s| s == ssid) {
        g.known.push(ssid.to_string());
    }
    Ok(())
}

pub async fn connect_hidden(ssid: &str, passphrase: Option<String>) -> Result<(), WifiError> {
    // Fabricate a scan entry for the hidden SSID and delegate to
    // `connect` so the state-machine update path stays the same.
    tokio::time::sleep(Duration::from_secs(2)).await;
    if passphrase.as_deref().unwrap_or("").is_empty() {
        return Err(WifiError::PassphraseRequired(ssid.to_string()));
    }
    let mut g = STATE.lock().unwrap();
    g.connected_ssid = Some(ssid.to_string());
    if !g.known.iter().any(|s| s == ssid) {
        g.known.push(ssid.to_string());
    }
    Ok(())
}

pub async fn disconnect() -> Result<(), WifiError> {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut g = STATE.lock().unwrap();
    g.connected_ssid = None;
    Ok(())
}

pub async fn station_state() -> Result<StationState, WifiError> {
    let g = STATE.lock().unwrap();
    Ok(if g.connected_ssid.is_some() {
        StationState::Connected
    } else {
        StationState::Disconnected
    })
}

pub async fn check_connected() -> Result<bool, WifiError> {
    Ok(matches!(station_state().await?, StationState::Connected))
}

pub async fn current_ssid() -> Result<Option<String>, WifiError> {
    let g = STATE.lock().unwrap();
    Ok(g.connected_ssid.clone())
}

/// Produce an empty stream. The mock doesn't emit live state changes
/// because nothing external drives them — controllers that rely on
/// the stream will just never see an update, which is correct for a
/// test environment.
pub async fn watch_station_state() -> Result<StationStateStream, WifiError> {
    Ok(Box::pin(stream::empty()))
}

pub async fn list_known_networks() -> Result<Vec<KnownNetworkInfo>, WifiError> {
    let g = STATE.lock().unwrap();
    // Always include HomeNetwork (seeded) plus anything connected in
    // this process's lifetime.
    let mut out = vec![KnownNetworkInfo {
        ssid: "HomeNetwork".into(),
        security: Security::Psk,
        hidden: false,
    }];
    for ssid in &g.known {
        if ssid != "HomeNetwork" {
            out.push(KnownNetworkInfo {
                ssid: ssid.clone(),
                security: Security::Psk,
                hidden: false,
            });
        }
    }
    out.sort_by(|a, b| a.ssid.cmp(&b.ssid));
    Ok(out)
}

pub async fn forget_network(ssid: &str) -> Result<(), WifiError> {
    let mut g = STATE.lock().unwrap();
    g.known.retain(|s| s != ssid);
    // Forgetting the currently-connected network also disconnects,
    // matching iwd's behavior.
    if g.connected_ssid.as_deref() == Some(ssid) {
        g.connected_ssid = None;
    }
    Ok(())
}
