//! Exercise the installer's own Wi-Fi calls against the machine it runs on.
//!
//! The wizard's list and its connection come from `wifi::scan_networks` and
//! `wifi::connect`, and an adapter that answers badly is only visible
//! through them — `iw` and `nmcli` ask the hardware a different question.
//! This runs the same two calls and prints what they returned, so a test
//! image can be measured without driving the graphical wizard by hand.
//!
//! ```sh
//! azfs-wifi-probe scan                       # one listing
//! azfs-wifi-probe connect <ssid> [passphrase] # one cold connection
//! ```
//!
//! The output is one line of `key=value` per run, for a script to collect.

use std::time::Instant;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "scan".to_string());
    let ssid = args.next();
    let passphrase = args.next();

    match command.as_str() {
        "scan" => scan(ssid.as_deref()).await,
        "connect" => {
            let Some(ssid) = ssid else {
                eprintln!("connect needs an SSID");
                std::process::exit(2);
            };
            connect(&ssid, passphrase).await;
        }
        other => {
            eprintln!("unknown command {other:?}; expected scan or connect");
            std::process::exit(2);
        }
    }
}

async fn scan(target: Option<&str>) {
    let started = Instant::now();
    match archinstall_zfs_core::system::wifi::scan_networks().await {
        Ok(networks) => {
            let found = target.is_none_or(|t| networks.iter().any(|n| n.ssid == t));
            println!(
                "result=ok networks={} target={} seconds={:.2}",
                networks.len(),
                found,
                started.elapsed().as_secs_f64()
            );
            for network in networks {
                println!("  {:>3}%  {}", network.signal_percent, network.ssid);
            }
        }
        Err(error) => println!(
            "result=error seconds={:.2} error={error}",
            started.elapsed().as_secs_f64()
        ),
    }
}

async fn connect(ssid: &str, passphrase: Option<String>) {
    let started = Instant::now();
    match archinstall_zfs_core::system::wifi::connect(ssid, passphrase).await {
        Ok(()) => println!("result=ok seconds={:.2}", started.elapsed().as_secs_f64()),
        Err(error) => println!(
            "result=error seconds={:.2} error={error}",
            started.elapsed().as_secs_f64()
        ),
    }
}
