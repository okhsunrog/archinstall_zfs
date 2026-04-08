use archinstall_zfs_core::system::{net, wifi};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::theme;

use super::edit::run_edit;
use super::select::run_select;

/// Check internet and, if absent, offer a WiFi connection flow.
///
/// Returns `true` if the user connected to WiFi (so the caller can auto-enable
/// `network_copy_iso` in the config — the iwd profile will already be saved at
/// `/var/lib/iwd/<ssid>.psk` and will be copied to the target).
///
/// Returns `false` if already connected, no WiFi hardware found, or user skipped.
pub async fn run_wifi_setup(
    terminal: &mut ratatui::DefaultTerminal,
) -> color_eyre::eyre::Result<bool> {
    // ── 1. Already online ───────────────────────────────────────────────────
    terminal.draw(render_checking)?;
    // net::check_internet is still sync (quick TCP probe); keep spawn_blocking
    // so the TUI event loop isn't stalled waiting on DNS / connect timeouts.
    let online = tokio::task::spawn_blocking(net::check_internet).await?;
    if online {
        return Ok(false);
    }

    // ── 2. No WiFi hardware ─────────────────────────────────────────────────
    let ifaces = wifi::detect_wifi_interfaces();
    if ifaces.is_empty() {
        return Ok(false);
    }

    // ── 3. Ask user ─────────────────────────────────────────────────────────
    let result = run_select(
        terminal,
        "No network connection detected",
        &["Connect to WiFi", "Skip (continue without network)"],
        0,
    )?;
    if result.selected != Some(0) {
        return Ok(false);
    }

    // ── 4. Pick interface (skip picker when there's only one) ────────────────
    let iface = if ifaces.len() == 1 {
        ifaces[0].clone()
    } else {
        let iface_refs: Vec<&str> = ifaces.iter().map(|s| s.as_str()).collect();
        let result = run_select(terminal, "Select wireless interface", &iface_refs, 0)?;
        match result.selected {
            Some(idx) => ifaces[idx].clone(),
            None => return Ok(false),
        }
    };

    // ── 5. Scan → pick → connect loop ───────────────────────────────────────
    loop {
        // Scan (takes ~3 s — show status while waiting)
        terminal.draw(|frame| render_status(frame, &format!("Scanning on {iface}…")))?;
        let mut networks = wifi::scan_networks(&iface).await;

        if networks.is_empty() {
            let result = run_select(terminal, "No WiFi networks found", &["Rescan", "Skip"], 0)?;
            match result.selected {
                Some(0) => continue,
                _ => return Ok(false),
            }
        }

        // Sort strongest-first
        networks.sort_by(|a, b| b.signal_percent.cmp(&a.signal_percent));

        // Build menu options
        let mut options: Vec<String> = networks
            .iter()
            .map(|n| {
                let bars = signal_bars(n.signal_percent);
                let security = if n.security == "open" { "open" } else { "🔒" };
                format!("{:<34} {security:<5}  {bars}", n.ssid)
            })
            .collect();
        options.push("↻  Rescan".to_string());
        options.push("✕  Skip WiFi setup".to_string());
        let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();

        let result = run_select(terminal, "Select WiFi network", &opt_refs, 0)?;
        let Some(idx) = result.selected else {
            return Ok(false);
        };

        // Rescan
        if idx == options.len() - 2 {
            continue;
        }
        // Skip
        if idx == options.len() - 1 {
            return Ok(false);
        }

        let network = networks[idx].clone();

        // ── 6. Password prompt for secured networks ──────────────────────────
        let passphrase = if network.security != "open" {
            let result = run_edit(
                terminal,
                &format!("Password for \"{}\"", network.ssid),
                "",
                true,
            )?;
            match result.value {
                Some(pw) if !pw.is_empty() => Some(pw),
                // Cancelled — go back to network list
                _ => continue,
            }
        } else {
            None
        };

        // ── 7. Connect ───────────────────────────────────────────────────────
        terminal
            .draw(|frame| render_status(frame, &format!("Connecting to \"{}\"…", network.ssid)))?;

        let connect_ok = wifi::connect(&iface, &network.ssid, passphrase.as_deref())
            .await
            .is_ok();

        if !connect_ok {
            let result = run_select(
                terminal,
                "Connection failed",
                &["Try another network", "Rescan", "Skip"],
                0,
            )?;
            match result.selected {
                Some(1) => continue, // rescan
                Some(0) => continue, // pick again (same network list still in scope but loop restarts)
                _ => return Ok(false),
            }
        }

        // ── 8. Verify — wait for DHCP / IP assignment ───────────────────────
        terminal.draw(|frame| render_status(frame, "Waiting for IP address…"))?;
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;

        if wifi::check_connected(&iface).await {
            let _ = run_select(
                terminal,
                &format!("Connected to \"{}\"", network.ssid),
                &["Continue"],
                0,
            )?;
            // iwd saved the profile to /var/lib/iwd/<ssid>.psk
            return Ok(true);
        }

        // IP not assigned yet — let the user decide
        let result = run_select(
            terminal,
            "Connected but no IP address assigned yet",
            &["Wait and retry", "Continue anyway", "Skip WiFi"],
            0,
        )?;
        match result.selected {
            Some(0) => {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if wifi::check_connected(&iface).await {
                    return Ok(true);
                }
                continue;
            }
            Some(1) => return Ok(true), // user's choice to proceed
            _ => return Ok(false),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn signal_bars(percent: u8) -> &'static str {
    match percent {
        76..=100 => "████",
        51..=75 => "███░",
        26..=50 => "██░░",
        _ => "█░░░",
    }
}

fn render_checking(frame: &mut Frame) {
    render_status(frame, "Checking network connectivity…");
}

fn render_status(frame: &mut Frame, msg: &str) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme::BG_STYLE), area);

    let [_, center, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .areas(area);

    let para = Paragraph::new(Line::from(vec![
        Span::styled(" ⟳ ", theme::ACCENT_STYLE),
        Span::styled(msg, theme::NORMAL_STYLE),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::BORDER_STYLE),
    );
    frame.render_widget(para, center);
}
