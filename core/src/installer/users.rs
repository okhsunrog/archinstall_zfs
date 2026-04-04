use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{CommandRunner, check_exit, chroot_cmd};

pub fn set_root_password(runner: &dyn CommandRunner, target: &Path, password: &str) -> Result<()> {
    let target_str = target.to_string_lossy();
    let input = format!("root:{password}\n");
    let output =
        runner.run_with_stdin("arch-chroot", &[&target_str, "chpasswd"], input.as_bytes())?;
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
    // Create user — args passed directly, no shell interpretation
    let output = if let Some(sh) = shell {
        chroot_cmd(runner, target, "useradd", &["-m", "-s", sh, username])?
    } else {
        chroot_cmd(runner, target, "useradd", &["-m", username])?
    };
    check_exit(&output, &format!("useradd {username}"))?;

    // Set password via stdin (not visible in process args)
    if let Some(pw) = password {
        let target_str = target.to_string_lossy();
        let input = format!("{username}:{pw}\n");
        let output =
            runner.run_with_stdin("arch-chroot", &[&target_str, "chpasswd"], input.as_bytes())?;
        check_exit(&output, &format!("set password for {username}"))?;
    }

    // Add to groups — args passed directly, no shell interpretation
    if let Some(grps) = groups {
        for group in grps {
            let output = chroot_cmd(runner, target, "usermod", &["-aG", group, username])?;
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

/// Write SSH authorized_keys for a user in the target.
/// Sets correct ownership (uid/gid from the target's /etc/passwd) and
/// permissions (700 on .ssh, 600 on authorized_keys) via chroot commands.
pub fn setup_ssh_keys(
    runner: &dyn CommandRunner,
    target: &Path,
    username: &str,
    keys: &[String],
) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }

    let ssh_dir = format!("/home/{username}/.ssh");
    let auth_keys_path = format!("{ssh_dir}/authorized_keys");

    // Create .ssh directory with correct permissions inside the chroot
    let output = chroot_cmd(runner, target, "install", &["-d", "-m", "700", &ssh_dir])?;
    check_exit(&output, &format!("create .ssh dir for {username}"))?;

    // Write authorized_keys on the host side (simpler than heredoc in chroot)
    let auth_keys_file = target.join(format!("home/{username}/.ssh/authorized_keys"));
    let content = keys.join("\n") + "\n";
    fs::write(&auth_keys_file, content).wrap_err("failed to write authorized_keys")?;

    // Fix ownership and permissions via chroot
    let output = chroot_cmd(runner, target, "chmod", &["600", &auth_keys_path])?;
    check_exit(&output, &format!("chmod authorized_keys for {username}"))?;

    let owner = format!("{username}:{username}");
    let output = chroot_cmd(runner, target, "chown", &["-R", &owner, &ssh_dir])?;
    check_exit(&output, &format!("chown .ssh for {username}"))?;

    tracing::info!(username, "set up SSH authorized_keys");
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

    #[test]
    fn test_setup_ssh_keys_writes_file() {
        // 3 chroot commands: install -d, chmod, chown
        let responses: Vec<CannedResponse> = (0..3).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);
        let dir = tempfile::tempdir().unwrap();

        // Pre-create the home directory so the write succeeds without chroot
        fs::create_dir_all(dir.path().join("home/alice/.ssh")).unwrap();

        let keys = vec![
            "ssh-ed25519 AAAAC3NzaC1 user@host".to_string(),
            "ssh-rsa AAAAB3NzaC1 backup@host".to_string(),
        ];
        setup_ssh_keys(&runner, dir.path(), "alice", &keys).unwrap();

        let content =
            fs::read_to_string(dir.path().join("home/alice/.ssh/authorized_keys")).unwrap();
        assert!(content.contains("ssh-ed25519 AAAAC3NzaC1 user@host"));
        assert!(content.contains("ssh-rsa AAAAB3NzaC1 backup@host"));

        let calls = runner.calls();
        assert_eq!(calls.len(), 3);
        // First call: install -d .ssh
        assert!(calls[0].args.iter().any(|a| a.contains("install")));
        // Second: chmod
        assert!(calls[1].args.iter().any(|a| a.contains("chmod")));
        // Third: chown
        assert!(calls[2].args.iter().any(|a| a.contains("chown")));
    }

    #[test]
    fn test_setup_ssh_keys_empty_is_noop() {
        let runner = RecordingRunner::new(vec![]);
        let dir = tempfile::tempdir().unwrap();
        setup_ssh_keys(&runner, dir.path(), "alice", &[]).unwrap();
        assert!(runner.calls().is_empty());
    }
}
