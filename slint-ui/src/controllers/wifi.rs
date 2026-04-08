//! WiFi management controller.
//!
//! Owns the async workflow behind the wifi popup:
//!   * initial hardware / iwd availability probe
//!   * ethernet status snapshot
//!   * scan → pick → auth → connect → verify → connected/error state machine
//!   * cancellation via `CancellationToken` so closing the popup aborts
//!     any in-flight scan / connect / verify task cleanly
//!
//! Callbacks flow Slint → Rust (via `on_*` hooks on `WifiState`), state
//! updates flow Rust → Slint (via `upgrade_in_event_loop` + property
//! setters). The Slint side never blocks.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tokio_util::sync::CancellationToken;

use archinstall_zfs_core::system::{
    net,
    wifi::{self, Security, WifiError, WifiNetwork},
};

use crate::ui::{App, PopupState, WelcomeState, WifiNetworkUi, WifiPhase, WifiSecurity, WifiState};

/// Shared cancellation handle for the currently-running scan / connect
/// task. Any new task takes a fresh token; closing the popup or starting
/// a rescan aborts whatever is running now.
type InFlight = Arc<Mutex<Option<CancellationToken>>>;

pub fn setup(app: &App) {
    let in_flight: InFlight = Arc::new(Mutex::new(None));

    run_initial_probe(app);
    setup_open(app, in_flight.clone());
    setup_close(app, in_flight.clone());
    setup_rescan(app, in_flight.clone());
    setup_pick(app, in_flight.clone());
    setup_cancel_pick(app);
    setup_submit_password(app, in_flight.clone());
    setup_forget(app);
    setup_disconnect(app);
    setup_connect_hidden(app, in_flight);
}

/// Probe at startup: does the kernel see any wireless NIC, and is iwd
/// reachable on the system bus? Also snapshot the ethernet state once.
/// Everything runs in a background task so app startup stays snappy.
fn run_initial_probe(app: &App) {
    let weak = app.as_weak();
    tokio::spawn(async move {
        let has_hardware = !wifi::detect_wifi_interfaces().is_empty();
        let iwd_running = wifi::iwd_available().await;
        let (eth_connected, eth_ip) = snapshot_ethernet().await;
        let current_ssid = if iwd_running {
            wifi::current_ssid()
                .await
                .ok()
                .flatten()
                .unwrap_or_default()
        } else {
            String::new()
        };

        let _ = weak.upgrade_in_event_loop(move |app| {
            let s = app.global::<WifiState>();
            s.set_has_hardware(has_hardware);
            s.set_iwd_running(iwd_running);
            s.set_ethernet_connected(eth_connected);
            s.set_ethernet_ipv4(SharedString::from(eth_ip));
            s.set_current_ssid(SharedString::from(current_ssid));
        });
    });
}

fn setup_open(app: &App, in_flight: InFlight) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_open(move || {
        let Some(app) = weak.upgrade() else { return };
        app.global::<PopupState>().set_wifi_visible(true);
        app.global::<WifiState>().set_visible(true);

        // Kick off the first scan immediately — users expect an
        // already-populated list when they open the popup.
        start_scan(&app, in_flight.clone());
    });
}

fn setup_close(app: &App, in_flight: InFlight) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_close(move || {
        abort_in_flight(&in_flight);
        let Some(app) = weak.upgrade() else { return };
        let s = app.global::<WifiState>();
        s.set_visible(false);
        s.set_phase(WifiPhase::Idle);
        s.set_status_text(SharedString::default());
        s.set_error_text(SharedString::default());
        s.set_password(SharedString::default());
        s.set_show_password(false);
        app.global::<PopupState>().set_wifi_visible(false);
    });
}

fn setup_rescan(app: &App, in_flight: InFlight) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_rescan(move || {
        let Some(app) = weak.upgrade() else { return };
        start_scan(&app, in_flight.clone());
    });
}

fn setup_pick(app: &App, in_flight: InFlight) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_pick(move |idx| {
        let Some(app) = weak.upgrade() else { return };
        let s = app.global::<WifiState>();
        let networks = s.get_networks();
        let Some(net) = networks.row_data(idx as usize) else {
            return;
        };

        // Enterprise networks: the core connect flow doesn't support
        // EAP credential entry, so we refuse and show a short error.
        if matches!(net.security, WifiSecurity::Enterprise) {
            s.set_phase(WifiPhase::Error);
            s.set_error_text(SharedString::from(
                "Enterprise (802.1x) networks are not supported by the installer.",
            ));
            return;
        }

        let needs_password =
            matches!(net.security, WifiSecurity::Psk | WifiSecurity::Wep) && !net.known;
        if needs_password {
            s.set_phase(WifiPhase::Auth);
            s.set_error_text(SharedString::default());
            s.set_password(SharedString::default());
            s.set_show_password(false);
        } else {
            // Open or already-known → jump straight to connecting.
            start_connect(&app, in_flight.clone(), net.ssid.to_string(), None);
        }
    });
}

fn setup_cancel_pick(app: &App) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_cancel_pick(move || {
        let Some(app) = weak.upgrade() else { return };
        let s = app.global::<WifiState>();
        s.set_phase(WifiPhase::Picking);
        s.set_password(SharedString::default());
        s.set_error_text(SharedString::default());
    });
}

fn setup_submit_password(app: &App, in_flight: InFlight) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_submit_password(move || {
        let Some(app) = weak.upgrade() else { return };
        let s = app.global::<WifiState>();
        let idx = s.get_selected_index();
        let networks = s.get_networks();
        let Some(net) = networks.row_data(idx as usize) else {
            return;
        };
        let password = s.get_password().to_string();
        start_connect(
            &app,
            in_flight.clone(),
            net.ssid.to_string(),
            Some(password),
        );
    });
}

fn setup_forget(app: &App) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_forget(move |idx| {
        let Some(app) = weak.upgrade() else { return };
        let s = app.global::<WifiState>();
        let networks = s.get_networks();
        let Some(net) = networks.row_data(idx as usize) else {
            return;
        };
        let ssid = net.ssid.to_string();
        let weak = app.as_weak();
        tokio::spawn(async move {
            let _ = wifi::forget_network(&ssid).await;
            // Refresh the list so the "Known" badge drops away.
            if let Ok(networks) = wifi::scan_networks().await {
                let ui = networks.into_iter().map(to_ui).collect::<Vec<_>>();
                let _ = weak.upgrade_in_event_loop(move |app| {
                    app.global::<WifiState>()
                        .set_networks(ModelRc::new(VecModel::from(ui)));
                });
            }
        });
    });
}

fn setup_disconnect(app: &App) {
    let weak = app.as_weak();
    app.global::<WifiState>().on_disconnect(move || {
        let weak2 = weak.clone();
        tokio::spawn(async move {
            let _ = wifi::disconnect().await;
            let _ = weak2.upgrade_in_event_loop(|app| {
                let s = app.global::<WifiState>();
                s.set_current_ssid(SharedString::default());
                s.set_phase(WifiPhase::Picking);
            });
        });
    });
}

fn setup_connect_hidden(app: &App, in_flight: InFlight) {
    let weak = app.as_weak();
    app.global::<WifiState>()
        .on_connect_hidden(move |ssid, password| {
            let Some(app) = weak.upgrade() else { return };
            let ssid = ssid.to_string();
            let password = if password.is_empty() {
                None
            } else {
                Some(password.to_string())
            };

            // Reuse the normal connect pipeline — it'll drive phase into
            // Connecting/Verifying/Connected just like a regular connect.
            // The difference is the underlying call uses `connect_hidden`.
            let token = fresh_token(&in_flight);
            let weak = app.as_weak();

            let s = app.global::<WifiState>();
            s.set_phase(WifiPhase::Connecting);
            s.set_status_text(SharedString::from(format!("Connecting to \"{ssid}\"…")));
            s.set_error_text(SharedString::default());

            tokio::spawn(async move {
                let result = tokio::select! {
                    _ = token.cancelled() => return,
                    r = wifi::connect_hidden(&ssid, password) => r,
                };
                drive_post_connect(&weak, ssid, result, token).await;
            });
        });
}

// ── async workers ────────────────────────────────────────────────────────

fn start_scan(app: &App, in_flight: InFlight) {
    let token = fresh_token(&in_flight);
    let weak = app.as_weak();

    let s = app.global::<WifiState>();
    s.set_phase(WifiPhase::Scanning);
    s.set_status_text(SharedString::from("Scanning for networks…"));
    s.set_error_text(SharedString::default());

    tokio::spawn(async move {
        let scan = tokio::select! {
            _ = token.cancelled() => return,
            r = wifi::scan_networks() => r,
        };

        match scan {
            Ok(mut networks) => {
                // Sort strongest-first.
                networks.sort_by(|a, b| b.signal_percent.cmp(&a.signal_percent));
                let ui = networks.into_iter().map(to_ui).collect::<Vec<_>>();
                let had_any = !ui.is_empty();
                let _ = weak.upgrade_in_event_loop(move |app| {
                    let s = app.global::<WifiState>();
                    s.set_networks(ModelRc::new(VecModel::from(ui)));
                    s.set_phase(WifiPhase::Picking);
                    s.set_status_text(SharedString::default());
                    // Pre-select the first (strongest) row so Enter
                    // works immediately without the user pressing
                    // Down first.
                    s.set_selected_index(if had_any { 0 } else { -1 });
                });
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = weak.upgrade_in_event_loop(move |app| {
                    let s = app.global::<WifiState>();
                    s.set_phase(WifiPhase::Error);
                    s.set_error_text(SharedString::from(msg));
                });
            }
        }
    });
}

fn start_connect(app: &App, in_flight: InFlight, ssid: String, passphrase: Option<String>) {
    let token = fresh_token(&in_flight);
    let weak = app.as_weak();

    let s = app.global::<WifiState>();
    s.set_phase(WifiPhase::Connecting);
    s.set_status_text(SharedString::from(format!("Connecting to \"{ssid}\"…")));
    s.set_error_text(SharedString::default());

    let ssid_clone = ssid.clone();
    tokio::spawn(async move {
        let result = tokio::select! {
            _ = token.cancelled() => return,
            r = wifi::connect(&ssid_clone, passphrase) => r,
        };
        drive_post_connect(&weak, ssid_clone, result, token).await;
    });
}

/// Shared post-connect verification: on success, move to Verifying,
/// wait ~4s for DHCP, then check the internet. On failure, move to
/// Error.
async fn drive_post_connect(
    weak: &slint::Weak<App>,
    ssid: String,
    result: Result<(), WifiError>,
    token: CancellationToken,
) {
    match result {
        Ok(()) => {
            let weak2 = weak.clone();
            let _ = weak2.upgrade_in_event_loop(|app| {
                let s = app.global::<WifiState>();
                s.set_phase(WifiPhase::Verifying);
                s.set_status_text(SharedString::from("Waiting for IP address…"));
            });

            // Wait for DHCP — cancellable.
            let sleep = tokio::time::sleep(Duration::from_secs(4));
            tokio::select! {
                _ = token.cancelled() => return,
                _ = sleep => {}
            }

            let online = tokio::task::spawn_blocking(net::check_internet)
                .await
                .unwrap_or(false);

            if online {
                let ssid_final = ssid.clone();
                let _ = weak.upgrade_in_event_loop(move |app| {
                    let s = app.global::<WifiState>();
                    s.set_phase(WifiPhase::Connected);
                    s.set_current_ssid(SharedString::from(ssid_final));
                    s.set_status_text(SharedString::default());

                    // Trigger the welcome-screen's existing check-internet
                    // handler so `net_ok` flips to true and the ZFS init /
                    // kernel scan paths (both gated behind net_ok) kick
                    // off automatically. The welcome controller already
                    // owns that logic; we just invoke its callback.
                    app.global::<WelcomeState>().invoke_check_internet();
                });
            } else {
                let _ = weak.upgrade_in_event_loop(|app| {
                    let s = app.global::<WifiState>();
                    s.set_phase(WifiPhase::Error);
                    s.set_error_text(SharedString::from(
                        "Connected to the network, but no IP address was assigned (DHCP timeout).",
                    ));
                });
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = weak.upgrade_in_event_loop(move |app| {
                let s = app.global::<WifiState>();
                s.set_phase(WifiPhase::Error);
                s.set_error_text(SharedString::from(msg));
            });
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────

/// Abort any currently-running task and replace the stored token with
/// a fresh one. Returns the new token so the caller can hand it to the
/// task it's about to spawn.
fn fresh_token(in_flight: &InFlight) -> CancellationToken {
    let mut guard = in_flight.lock().unwrap();
    if let Some(existing) = guard.take() {
        existing.cancel();
    }
    let token = CancellationToken::new();
    *guard = Some(token.clone());
    token
}

fn abort_in_flight(in_flight: &InFlight) {
    let mut guard = in_flight.lock().unwrap();
    if let Some(existing) = guard.take() {
        existing.cancel();
    }
}

/// Map `core::system::wifi::WifiNetwork` to the Slint-side UI struct.
fn to_ui(n: WifiNetwork) -> WifiNetworkUi {
    WifiNetworkUi {
        ssid: SharedString::from(n.ssid),
        signal_percent: n.signal_percent as i32,
        security: match n.security {
            Security::Open => WifiSecurity::Open,
            Security::Wep => WifiSecurity::Wep,
            Security::Psk => WifiSecurity::Psk,
            Security::Enterprise => WifiSecurity::Enterprise,
        },
        known: n.known,
    }
}

/// Best-effort ethernet snapshot: walks `/sys/class/net/*`, picks the
/// first non-loopback non-wireless interface whose `carrier` reads `1`,
/// and tries to pull an IPv4 address from `ip -4 addr show <iface>`.
/// Used by the welcome-screen corner widget's Ethernet row.
async fn snapshot_ethernet() -> (bool, String) {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return (false, String::new());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) if n != "lo" => n.to_string(),
            _ => continue,
        };
        // Skip wireless interfaces.
        if path.join("wireless").is_dir() {
            continue;
        }
        // Must have a carrier.
        let carrier = std::fs::read_to_string(path.join("carrier"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if carrier != "1" {
            continue;
        }
        // Pull IPv4 via `ip -4 addr show`.
        let output = tokio::process::Command::new("ip")
            .args(["-4", "addr", "show", &name])
            .output()
            .await;
        let ipv4 = match output {
            Ok(out) => parse_ipv4_from_ip_addr(&String::from_utf8_lossy(&out.stdout)),
            Err(_) => None,
        };
        return (true, ipv4.unwrap_or_default());
    }
    (false, String::new())
}

/// Extract the first non-loopback IPv4 address from `ip addr show`
/// output. Looks for `inet <addr>/<prefix>` lines and returns the
/// `<addr>` portion without the CIDR suffix.
fn parse_ipv4_from_ip_addr(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("inet ") {
            let addr = rest.split_whitespace().next()?;
            if addr.starts_with("127.") {
                continue;
            }
            if let Some(slash) = addr.find('/') {
                return Some(addr[..slash].to_string());
            }
            return Some(addr.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ipv4_basic() {
        let sample = "\
2: enp0s25: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP
    inet 192.168.1.42/24 brd 192.168.1.255 scope global dynamic enp0s25
       valid_lft 85000sec preferred_lft 85000sec
";
        assert_eq!(
            parse_ipv4_from_ip_addr(sample).as_deref(),
            Some("192.168.1.42")
        );
    }

    #[test]
    fn parse_ipv4_skips_loopback() {
        let sample = "\
1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN
    inet 127.0.0.1/8 scope host lo
";
        assert_eq!(parse_ipv4_from_ip_addr(sample), None);
    }

    #[test]
    fn parse_ipv4_no_inet() {
        assert_eq!(parse_ipv4_from_ip_addr(""), None);
        assert_eq!(parse_ipv4_from_ip_addr("no inet here\n"), None);
    }
}
