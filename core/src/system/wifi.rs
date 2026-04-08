//! WiFi management via `iwd` / `iwctl`.
//!
//! The Arch ISO ships `iwd` as the WiFi backend. All operations here use
//! `iwctl` in non-interactive (argument) mode. After connecting, the iwd
//! profile is saved to `/var/lib/iwd/<SSID>.psk` and is automatically copied
//! to the target by `network::copy_iso_network` when `network_copy_iso` is
//! set, so the installed system reconnects on first boot without extra config.
//!
//! # Usage flow
//! 1. `detect_wifi_interfaces()` — find wireless NICs
//! 2. `scan_networks(iface)` — trigger scan and return available SSIDs
//! 3. `connect(iface, ssid, passphrase)` — connect to a network
//! 4. `check_connected(iface)` — verify the connection succeeded

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

/// A WiFi network discovered by a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// Signal strength 0–100.
    pub signal_percent: u8,
    /// Security type as reported by iwd (e.g. "psk", "open", "8021x").
    pub security: String,
}

/// Find all wireless network interface names present in `/sys/class/net`.
///
/// This is a synchronous kernel probe (reads `/sys/class/net`) and is cheap
/// enough to stay non-async — callers can invoke it without `.await`.
pub fn detect_wifi_interfaces() -> Vec<String> {
    let net_path = Path::new("/sys/class/net");
    let Ok(entries) = std::fs::read_dir(net_path) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().join("wireless").is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Trigger a WiFi scan on `iface` and return discovered networks.
///
/// The scan is asynchronous at the driver level; we wait `SCAN_WAIT` before
/// fetching results. Returns an empty Vec if `iface` is not managed by iwd
/// or if no networks are found.
pub async fn scan_networks(iface: &str) -> Vec<WifiNetwork> {
    const SCAN_WAIT: Duration = Duration::from_secs(3);

    // Trigger scan — ignore errors (e.g. already scanning)
    let _ = Command::new("iwctl")
        .args(["station", iface, "scan"])
        .output()
        .await;

    tokio::time::sleep(SCAN_WAIT).await;

    let Ok(out) = Command::new("iwctl")
        .args(["station", iface, "get-networks"])
        .output()
        .await
    else {
        return Vec::new();
    };

    parse_get_networks(&String::from_utf8_lossy(&out.stdout))
}

/// Connect to a WiFi network.
///
/// Pass `passphrase = None` for open networks.
/// Returns `Ok(())` if `iwctl` exits successfully; the caller should verify
/// the connection with `check_connected` afterwards.
pub async fn connect(iface: &str, ssid: &str, passphrase: Option<&str>) -> std::io::Result<()> {
    let mut cmd = Command::new("iwctl");
    if let Some(psk) = passphrase {
        cmd.args(["--passphrase", psk]);
    }
    cmd.args(["station", iface, "connect", ssid]);

    let status = cmd.status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "iwctl connect exited with {:?}",
            status.code()
        )))
    }
}

/// Return true if `iface` has an IP address (i.e. is connected).
///
/// Uses `/sys/class/net/<iface>/operstate` for a quick kernel-level check,
/// then falls back to checking for a non-loopback IP via `ip addr`.
pub async fn check_connected(iface: &str) -> bool {
    let operstate = tokio::fs::read_to_string(format!("/sys/class/net/{iface}/operstate"))
        .await
        .unwrap_or_default();
    if operstate.trim() != "up" {
        return false;
    }

    // Verify an IP is actually assigned
    let Ok(out) = Command::new("ip")
        .args(["addr", "show", iface])
        .output()
        .await
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.contains("inet ") && !text.contains("inet 127.")
}

/// Parse `iwctl station <iface> get-networks` output into `WifiNetwork` list.
///
/// Example output (ANSI colour codes are stripped by iwctl when non-interactive):
/// ```text
///                               Available networks
/// ──────────────────────────────────────────────────────────────────────────────
///       Network name                    Security            Signal
/// ──────────────────────────────────────────────────────────────────────────────
///   > * HomeNetwork                     psk                 ****
///       OtherNet                        psk                 ***
///       FreeWifi                        open                **
/// ```
///
/// Signal asterisks map to percentage: `*`=25, `**`=50, `***`=75, `****`=100.
pub fn parse_get_networks(output: &str) -> Vec<WifiNetwork> {
    let mut networks = Vec::new();

    for line in output.lines() {
        // Strip ANSI escape codes (iwctl sometimes emits them)
        let clean = strip_ansi(line);
        let trimmed = clean.trim();

        // Skip empty lines, header/separator lines
        if trimmed.is_empty()
            || trimmed.starts_with("Available")
            || trimmed.starts_with("Network name")
            || trimmed.chars().all(|c| c == '─' || c == '-' || c == ' ')
        {
            continue;
        }

        // Strip leading status markers: '>', '*', spaces
        let data = trimmed.trim_start_matches(['>', '*', ' ']);

        // Split on 2+ consecutive spaces to get fields
        let fields: Vec<&str> = data
            .splitn(3, |_| false) // placeholder — see below
            .collect();
        let _ = fields; // unused above

        // Better: split on runs of ≥2 spaces
        let parts: Vec<&str> = split_columns(data);

        if parts.len() < 3 {
            continue;
        }

        let ssid = parts[0].trim().to_string();
        if ssid.is_empty() {
            continue;
        }

        let security = parts[1].trim().to_string();
        let signal_stars = parts[2].trim().chars().filter(|&c| c == '*').count();
        let signal_percent = (signal_stars.min(4) as u8) * 25;

        networks.push(WifiNetwork {
            ssid,
            signal_percent,
            security,
        });
    }

    networks
}

/// Split a string on runs of 2 or more whitespace characters.
fn split_columns(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_gap = false;
    let mut gap_start = 0;

    for (i, c) in s.char_indices() {
        if c == ' ' {
            if !in_gap {
                in_gap = true;
                gap_start = i;
            }
        } else {
            if in_gap && i - gap_start >= 2 {
                parts.push(&s[start..gap_start]);
                start = i;
            }
            in_gap = false;
        }
    }

    let tail = s[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }

    parts
}

/// Strip ANSI/VT100 escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Consume the escape sequence: ESC [ ... <letter>
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OUTPUT: &str = "\
                              Available networks
────────────────────────────────────────────────────────────────────────────
      Network name                    Security            Signal
────────────────────────────────────────────────────────────────────────────
  > * HomeNetwork                     psk                 ****
      OtherNet                        psk                 ***
      FreeWifi                        open                **
      WeakSignal                      psk                 *
";

    #[test]
    fn test_parse_get_networks() {
        let networks = parse_get_networks(SAMPLE_OUTPUT);
        assert_eq!(networks.len(), 4);

        assert_eq!(networks[0].ssid, "HomeNetwork");
        assert_eq!(networks[0].security, "psk");
        assert_eq!(networks[0].signal_percent, 100);

        assert_eq!(networks[1].ssid, "OtherNet");
        assert_eq!(networks[1].signal_percent, 75);

        assert_eq!(networks[2].ssid, "FreeWifi");
        assert_eq!(networks[2].security, "open");
        assert_eq!(networks[2].signal_percent, 50);

        assert_eq!(networks[3].ssid, "WeakSignal");
        assert_eq!(networks[3].signal_percent, 25);
    }

    #[test]
    fn test_parse_empty_output() {
        assert!(parse_get_networks("").is_empty());
        assert!(parse_get_networks("No networks available\n").is_empty());
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[1;32mHello\x1b[0m"), "Hello");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn test_split_columns() {
        let parts = split_columns("HomeNetwork                     psk                 ****");
        assert_eq!(parts[0], "HomeNetwork");
        assert_eq!(parts[1], "psk");
        assert_eq!(parts[2], "****");
    }

    #[test]
    fn test_detect_wifi_interfaces_does_not_panic() {
        // Just verify it doesn't panic — result depends on the host machine
        let _ = detect_wifi_interfaces();
    }
}
