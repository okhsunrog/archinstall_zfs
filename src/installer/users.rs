use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{check_exit, chroot, CommandRunner};

pub fn set_root_password(runner: &dyn CommandRunner, target: &Path, password: &str) -> Result<()> {
    let cmd = format!("echo 'root:{password}' | chpasswd");
    let output = chroot(runner, target, &cmd)?;
    check_exit(&output, "set root password")?;
    tracing::info!("set root password");
    Ok(())
}

pub fn create_user(
    runner: &dyn CommandRunner,
    target: &Path,
    username: &str,
    password: Option<&str>,
    sudo: bool,
    shell: Option<&str>,
    groups: Option<&[String]>,
) -> Result<()> {
    // Create user
    let mut useradd_cmd = format!("useradd -m {username}");
    if let Some(sh) = shell {
        useradd_cmd = format!("useradd -m -s {sh} {username}");
    }
    let output = chroot(runner, target, &useradd_cmd)?;
    check_exit(&output, &format!("useradd {username}"))?;

    // Set password
    if let Some(pw) = password {
        let cmd = format!("echo '{username}:{pw}' | chpasswd");
        let output = chroot(runner, target, &cmd)?;
        check_exit(&output, &format!("set password for {username}"))?;
    }

    // Add to groups
    if let Some(grps) = groups {
        for group in grps {
            let cmd = format!("usermod -aG {group} {username}");
            let output = chroot(runner, target, &cmd)?;
            check_exit(&output, &format!("add {username} to {group}"))?;
        }
    }

    // Enable sudo
    if sudo {
        enable_sudo(target, username)?;
    }

    tracing::info!(username, "created user");
    Ok(())
}

fn enable_sudo(target: &Path, username: &str) -> Result<()> {
    let sudoers_dir = target.join("etc/sudoers.d");
    fs::create_dir_all(&sudoers_dir)?;
    let sudoers_file = sudoers_dir.join(format!("00_{username}"));
    fs::write(&sudoers_file, format!("{username} ALL=(ALL:ALL) ALL\n"))
        .wrap_err("failed to write sudoers")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_create_user_basic() {
        let responses: Vec<CannedResponse> = (0..3).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);
        let dir = tempfile::tempdir().unwrap();

        create_user(
            &runner,
            dir.path(),
            "testuser",
            Some("pass"),
            true,
            None,
            None,
        )
        .unwrap();

        let calls = runner.calls();
        // useradd + chpasswd
        assert_eq!(calls.len(), 2);
        assert!(calls[0].args.iter().any(|a| a.contains("useradd")));
        assert!(calls[1].args.iter().any(|a| a.contains("chpasswd")));

        // Check sudoers file was created
        let sudoers = dir.path().join("etc/sudoers.d/00_testuser");
        assert!(sudoers.exists());
        let content = fs::read_to_string(sudoers).unwrap();
        assert!(content.contains("testuser ALL=(ALL:ALL) ALL"));
    }

    #[test]
    fn test_enable_sudo() {
        let dir = tempfile::tempdir().unwrap();
        enable_sudo(dir.path(), "myuser").unwrap();

        let path = dir.path().join("etc/sudoers.d/00_myuser");
        assert!(path.exists());
    }
}
