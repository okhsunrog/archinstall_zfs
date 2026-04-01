use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{check_exit, CommandRunner};

pub fn enable_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_str().unwrap();
    let output = runner.run("systemctl", &["--root", target_str, "enable", service])?;
    check_exit(&output, &format!("systemctl enable {service}"))?;
    tracing::info!(service, "enabled service");
    Ok(())
}

pub fn disable_service(runner: &dyn CommandRunner, target: &Path, service: &str) -> Result<()> {
    let target_str = target.to_str().unwrap();
    let _ = runner.run("systemctl", &["--root", target_str, "disable", service]);
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
