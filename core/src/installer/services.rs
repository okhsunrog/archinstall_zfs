use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::CommandRunner;

/// Enable a system-level service in the target via `systemctl --root`.
///
/// A non-zero exit from systemctl is logged as a warning rather than aborting
/// the installation — the unit file may not be present yet (e.g. a DM whose
/// package install scripts run after the initial pacstrap), and the service
/// can be enabled manually post-install. The install progress screen displays
/// WARN entries in yellow so the user will see the failure.
pub fn enable_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_string_lossy();
    let output = runner.run("systemctl", &["--root", &target_str, "enable", service])?;
    if output.success() {
        tracing::info!(service, "enabled service");
    } else {
        tracing::warn!(
            service,
            exit_code = output.exit_code,
            stderr = %output.stderr,
            "failed to enable service — continuing (can be enabled manually)"
        );
    }
    Ok(())
}

/// Enable a user-level systemd unit globally for all users in the target.
///
/// Uses `systemctl --root <target> --global enable` which writes symlinks into
/// `<target>/etc/systemd/user/`. Non-zero exit is treated as a warning for the
/// same reasons as `enable_service`.
pub fn enable_user_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_string_lossy();
    let output = runner.run(
        "systemctl",
        &["--root", &target_str, "--global", "enable", service],
    )?;
    if output.success() {
        tracing::info!(service, "enabled user service globally");
    } else {
        tracing::warn!(
            service,
            exit_code = output.exit_code,
            stderr = %output.stderr,
            "failed to enable user service — continuing (can be enabled manually)"
        );
    }
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
    fn test_enable_service_success() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        enable_service(&runner, Path::new("/mnt"), "sshd").unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "systemctl");
        assert!(calls[0].args.contains(&"enable".to_string()));
        assert!(calls[0].args.contains(&"sshd".to_string()));
        assert!(calls[0].args.contains(&"/mnt".to_string()));
    }

    #[test]
    fn test_enable_service_failure_is_non_fatal() {
        // A failing systemctl exit code should not abort the installation.
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 1,
            stderr: "Unit not found".into(),
            ..Default::default()
        }]);
        // Must return Ok, not Err
        let result = enable_service(&runner, Path::new("/mnt"), "nonexistent.service");
        assert!(result.is_ok(), "service enable failure should be non-fatal");
    }
}
