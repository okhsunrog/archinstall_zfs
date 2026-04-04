use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{CommandRunner, check_exit};

pub fn enable_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_string_lossy();
    let output = runner.run("systemctl", &["--root", &target_str, "enable", service])?;
    check_exit(&output, &format!("systemctl enable {service}"))?;
    tracing::info!(service, "enabled service");
    Ok(())
}

/// Enable a user-level systemd unit globally for all users in the target.
///
/// Uses `systemctl --root <target> --global enable` which writes symlinks into
/// `<target>/etc/systemd/user/` — the same approach as `enable_service` but
/// targeting user units instead of system units.
pub fn enable_user_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_string_lossy();
    let output = runner.run(
        "systemctl",
        &["--root", &target_str, "--global", "enable", service],
    )?;
    check_exit(&output, &format!("systemctl --global enable {service}"))?;
    tracing::info!(service, "enabled user service globally");
    Ok(())
}

pub fn disable_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_string_lossy();
    let _ = runner.run("systemctl", &["--root", &target_str, "disable", service]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_enable_service() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        enable_service(&runner, Path::new("/mnt"), "sshd").unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "systemctl");
        assert!(calls[0].args.contains(&"enable".to_string()));
        assert!(calls[0].args.contains(&"sshd".to_string()));
        assert!(calls[0].args.contains(&"/mnt".to_string()));
    }
}
