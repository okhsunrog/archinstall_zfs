use std::time::Duration;

/// Check internet connectivity by making a HEAD request to a known endpoint.
/// Uses reqwest directly instead of shelling out to ping/curl.
pub fn check_internet() -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    client
        .head("https://archlinux.org/check_network_status.txt")
        .send()
        .is_ok_and(|r| r.status().is_success())
}

pub fn is_uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_internet_does_not_panic() {
        // Just verify it doesn't crash — actual connectivity depends on environment
        let _ = check_internet();
    }
}
