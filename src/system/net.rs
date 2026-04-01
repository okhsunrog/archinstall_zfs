use color_eyre::eyre::Result;

use super::cmd::{check_exit, CommandRunner};

pub fn check_internet(runner: &dyn CommandRunner) -> Result<bool> {
    // Use IP address to avoid DNS delays; 1.1.1.1 is Cloudflare DNS
    let output = runner.run("ping", &["-c", "1", "-W", "5", "1.1.1.1"])?;
    Ok(output.success())
}

pub fn is_uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_check_internet_success() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 0,
            ..Default::default()
        }]);
        assert!(check_internet(&runner).unwrap());
    }

    #[test]
    fn test_check_internet_failure() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 1,
            stderr: "Network unreachable".into(),
            ..Default::default()
        }]);
        assert!(!check_internet(&runner).unwrap());
    }
}
