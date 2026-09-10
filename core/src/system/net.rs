use std::time::Duration;

/// Arch's connectivity endpoint, also used by its NetworkManager package.
pub const CONNECTIVITY_URL: &str = "http://ping.archlinux.org/nm-check.txt";
const EXPECTED_RESPONSE: &[u8] = b"NetworkManager is online\n";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
enum ProbeError {
    #[error("connectivity request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("connectivity endpoint returned HTTP {0}")]
    Status(reqwest::StatusCode),
    #[error("unexpected connectivity response (possible captive portal)")]
    UnexpectedResponse,
}

/// Verify Arch's HTTP response rather than access to an external DNS server.
/// DNS resolution and connection fallback support both IPv4 and IPv6. Plain
/// HTTP works before the live ISO has synchronized its clock; redirects and
/// captive-portal pages must not be mistaken for a successful check.
pub async fn check_internet() -> bool {
    match probe(CONNECTIVITY_URL, PROBE_TIMEOUT).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(url = CONNECTIVITY_URL, ?error, "Internet check failed");
            false
        }
    }
}

/// Allow address assignment, routes and DNS to settle after Wi-Fi association.
/// The budget covers requests and delays; dropping the future cancels retries.
pub async fn wait_for_internet(budget: Duration) -> bool {
    wait_for_probe(budget, check_internet).await
}

async fn wait_for_probe<F, Fut>(budget: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    tokio::time::timeout(budget, async {
        loop {
            if check().await {
                return true;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn probe(url: &str, timeout: Duration) -> Result<(), ProbeError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()?;
    probe_with_client(&client, url).await
}

async fn probe_with_client(client: &reqwest::Client, url: &str) -> Result<(), ProbeError> {
    let mut response = client.get(url).send().await?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(ProbeError::Status(response.status()));
    }

    // Bound memory even if a portal returns a large or endless HTML document.
    let mut body = Vec::with_capacity(EXPECTED_RESPONSE.len());
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > EXPECTED_RESPONSE.len() {
            return Err(ProbeError::UnexpectedResponse);
        }
        body.extend_from_slice(&chunk);
    }
    if body != EXPECTED_RESPONSE {
        return Err(ProbeError::UnexpectedResponse);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn server(
        address: &str,
        response: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(address).await.unwrap();
        let url = format!("http://{}/nm-check.txt", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let len = socket.read(&mut request).await.unwrap();
            assert!(request[..len].starts_with(b"GET /nm-check.txt HTTP/1.1\r\n"));
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (url, task)
    }

    const ONLINE: &str = "HTTP/1.1 200 OK\r\nContent-Length: 25\r\nConnection: close\r\n\r\nNetworkManager is online\n";

    #[tokio::test]
    async fn accepts_arch_response_over_ipv4() {
        let (url, task) = server("127.0.0.1:0", ONLINE).await;
        probe(&url, PROBE_TIMEOUT).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn accepts_arch_response_over_ipv6() {
        // Some CI runners and live systems explicitly disable IPv6.
        let availability = TcpListener::bind("[::1]:0").await;
        if let Err(error) = availability {
            eprintln!("IPv6 loopback unavailable, skipping IPv6 probe: {error}");
            return;
        }
        drop(availability);
        let (url, task) = server("[::1]:0", ONLINE).await;
        probe(&url, PROBE_TIMEOUT).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn falls_back_to_ipv4_when_ipv6_is_unreachable() {
        let (url, task) = server("127.0.0.1:0", ONLINE).await;
        let port = reqwest::Url::parse(&url).unwrap().port().unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(PROBE_TIMEOUT)
            .resolve_to_addrs(
                "probe.test",
                &[
                    std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)),
                    std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port)),
                ],
            )
            .build()
            .unwrap();
        probe_with_client(&client, &format!("http://probe.test:{port}/nm-check.txt"))
            .await
            .unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_portals_redirects_and_server_errors() {
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nLogin",
            "HTTP/1.1 200 OK\r\nContent-Length: 26\r\n\r\nNetworkManager is online\n!",
            "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 25\r\n\r\nNetworkManager is online\n",
            "HTTP/1.1 204 No Content\r\n\r\n",
        ] {
            let (url, task) = server("127.0.0.1:0", response).await;
            assert!(probe(&url, PROBE_TIMEOUT).await.is_err());
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn bounds_time_waiting_for_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/nm-check.txt", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 25\r\n\r\n")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let result = probe(&url, Duration::from_millis(100)).await;
        assert!(matches!(result, Err(ProbeError::Request(error)) if error.is_timeout()));
        task.abort();
    }

    #[tokio::test]
    async fn reports_connection_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/nm-check.txt", listener.local_addr().unwrap());
        drop(listener);
        assert!(matches!(
            probe(&url, PROBE_TIMEOUT).await,
            Err(ProbeError::Request(_))
        ));
    }

    #[tokio::test]
    async fn retries_until_network_is_ready() {
        let mut attempts = 0;
        let online = wait_for_probe(Duration::from_secs(3), || {
            attempts += 1;
            std::future::ready(attempts == 2)
        })
        .await;
        assert!(online);
        assert_eq!(attempts, 2);
    }

    #[tokio::test]
    async fn retry_budget_cancels_a_pending_probe() {
        let online = wait_for_probe(Duration::from_millis(20), std::future::pending::<bool>).await;
        assert!(!online);
    }
}
