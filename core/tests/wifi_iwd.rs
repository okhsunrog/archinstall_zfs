//! Exercise the real iwd backend on a private D-Bus, without host Wi-Fi changes.
#![cfg(feature = "wifi-iwd")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archinstall_zfs_core::system::wifi::{self, WifiError};
use zbus::zvariant::OwnedObjectPath;

fn path(name: &str) -> OwnedObjectPath {
    format!("/net/connman/iwd/{name}").try_into().unwrap()
}

struct Station;

#[zbus::interface(name = "net.connman.iwd.Station")]
impl Station {
    fn get_ordered_networks(&self) -> Vec<(OwnedObjectPath, i16)> {
        ["Saved", "Unknown", "Open", "Connected"]
            .into_iter()
            .map(|name| (path(name), -5000))
            .collect()
    }
}

struct Network {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}

#[zbus::interface(name = "net.connman.iwd.Network")]
impl Network {
    fn connect(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
    }

    #[zbus(property)]
    fn name(&self) -> &str {
        self.name
    }

    #[zbus(property, name = "Type")]
    fn security(&self) -> &str {
        if self.name == "Open" { "open" } else { "psk" }
    }

    #[zbus(property)]
    fn connected(&self) -> bool {
        self.name == "Connected"
    }

    #[zbus(property)]
    fn known_network(&self) -> zbus::fdo::Result<OwnedObjectPath> {
        if self.name == "Saved" || self.name == "Connected" {
            Ok(path("profile"))
        } else {
            Err(zbus::fdo::Error::UnknownProperty("KnownNetwork".into()))
        }
    }
}

struct KnownNetwork;

#[zbus::interface(name = "net.connman.iwd.KnownNetwork")]
impl KnownNetwork {
    #[zbus(property)]
    fn name(&self) -> &str {
        "Saved"
    }
}

struct AgentManager;

#[zbus::interface(name = "net.connman.iwd.AgentManager")]
impl AgentManager {
    fn register_agent(&self, _agent: OwnedObjectPath) {}
    fn unregister_agent(&self, _agent: OwnedObjectPath) {}
}

#[test]
fn saved_and_connected_networks_do_not_require_a_new_password() {
    // Set the bus address only in a child process. Changing the process-wide
    // environment inside a concurrent Rust test runner would be unsafe.
    let output = std::process::Command::new("dbus-run-session")
        .args([
            "--",
            "sh",
            "-c",
            "DBUS_SYSTEM_BUS_ADDRESS=\"$DBUS_SESSION_BUS_ADDRESS\" exec \"$@\"",
            "sh",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "private_bus_worker", "--ignored", "--nocapture"])
        .env("AZFS_TEST_PRIVATE_BUS", "1")
        .output()
        .expect("dbus-run-session is required for iwd integration tests");
    assert!(
        output.status.success(),
        "private iwd test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "run by the parent test in a private D-Bus session"]
async fn private_bus_worker() {
    assert_eq!(std::env::var("AZFS_TEST_PRIVATE_BUS").as_deref(), Ok("1"));
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let calls: Vec<_> = (0..4).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        let mut builder = zbus::connection::Builder::system()
            .unwrap()
            .name("net.connman.iwd")
            .unwrap()
            .serve_at("/", zbus::fdo::ObjectManager)
            .unwrap()
            .serve_at(path("station"), Station)
            .unwrap()
            .serve_at("/net/connman/iwd", AgentManager)
            .unwrap()
            .serve_at(path("profile"), KnownNetwork)
            .unwrap();
        for (name, calls) in ["Saved", "Unknown", "Open", "Connected"]
            .into_iter()
            .zip(&calls)
        {
            builder = builder
                .serve_at(
                    path(name),
                    Network {
                        name,
                        calls: calls.clone(),
                    },
                )
                .unwrap();
        }
        let _service = builder.build().await.unwrap();

        wifi::connect("Saved", None).await.unwrap();
        assert_eq!(calls[0].load(Ordering::SeqCst), 1);
        assert!(matches!(
            wifi::connect("Unknown", None).await,
            Err(WifiError::PassphraseRequired(_))
        ));
        assert_eq!(calls[1].load(Ordering::SeqCst), 0);
        wifi::connect("Open", None).await.unwrap();
        assert_eq!(calls[2].load(Ordering::SeqCst), 1);
        wifi::connect("Connected", None).await.unwrap();
        assert_eq!(calls[3].load(Ordering::SeqCst), 0);
    })
    .await
    .expect("iwd test timed out");
}
