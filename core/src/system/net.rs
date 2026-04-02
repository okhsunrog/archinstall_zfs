use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Check internet connectivity by attempting a TCP connection to a well-known
/// DNS server (Cloudflare 1.1.1.1:53). No TLS or HTTP involved — works on
/// minimal ISOs without root certificates.
pub fn check_internet() -> bool {
    let addr: SocketAddr = ([1, 1, 1, 1], 53).into();
    TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok()
}

pub fn is_uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_internet_does_not_panic() {
        let _ = check_internet();
    }
}
