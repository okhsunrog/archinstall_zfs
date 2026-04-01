use std::path::Path;
use std::sync::mpsc::Sender;

use color_eyre::eyre::{bail, Result};

use super::cmd::{check_exit, CmdOutput, CommandRunner};

pub fn pacstrap(
    runner: &dyn CommandRunner,
    target: &Path,
    packages: &[&str],
    tx: Option<&Sender<String>>,
) -> Result<CmdOutput> {
    let target_str = target.to_str().unwrap_or("/mnt");
    let mut args = vec!["-C", "/etc/pacman.conf", "-K", target_str];
    args.extend(packages);
    args.push("--noconfirm");
    args.push("--needed");

    let output = if let Some(tx) = tx {
        runner.run_streaming("pacstrap", &args, tx)?
    } else {
        runner.run("pacstrap", &args)?
    };

    check_exit(&output, "pacstrap")?;
    Ok(output)
}

pub fn sync_db(runner: &dyn CommandRunner) -> Result<()> {
    let output = runner.run("pacman", &["-Syy", "--noconfirm"])?;
    check_exit(&output, "pacman -Syy")?;
    Ok(())
}

pub fn wait_for_db_lock(runner: &dyn CommandRunner) -> Result<()> {
    let lock_path = Path::new("/var/lib/pacman/db.lck");
    for _ in 0..60 {
        if !lock_path.exists() {
            return Ok(());
        }
        tracing::info!("pacman db.lck exists, waiting...");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    bail!("pacman db.lck not released after 10 minutes");
}

const ARCHZFS_REPO_BLOCK: &str = r#"
[archzfs]
Server = https://archzfs.com/$repo/$arch
Server = https://zxcvfdsa.com/archzfs/$repo/$arch
"#;

const ARCHZFS_KEY_IDS: &[&str] = &["DDF7DB817396A49B2A2723F7403BD972F75D9D76"];

pub fn add_archzfs_repo(runner: &dyn CommandRunner, target: Option<&Path>) -> Result<()> {
    let pacman_conf = match target {
        Some(t) => t.join("etc/pacman.conf"),
        None => std::path::PathBuf::from("/etc/pacman.conf"),
    };

    let content = std::fs::read_to_string(&pacman_conf)?;
    if content.contains("[archzfs]") {
        tracing::info!("archzfs repo already present in pacman.conf");
    } else {
        let mut new_content = content;
        new_content.push_str(ARCHZFS_REPO_BLOCK);
        std::fs::write(&pacman_conf, new_content)?;
        tracing::info!(path = %pacman_conf.display(), "added archzfs repo to pacman.conf");
    }

    // Import archzfs signing key
    for key_id in ARCHZFS_KEY_IDS {
        let gpgdir = target.map(|t| format!("{}/etc/pacman.d/gnupg", t.display()));

        // Try multiple keyservers
        for server in ["keyserver.ubuntu.com", "keys.openpgp.org", "pgp.mit.edu"] {
            let mut args: Vec<&str> = Vec::new();
            if let Some(ref dir) = gpgdir {
                args.extend_from_slice(&["--gpgdir", dir]);
            }
            args.extend_from_slice(&["--recv-keys", key_id, "--keyserver", server]);

            let output = runner.run("pacman-key", &args);
            if output.is_ok() && output.as_ref().unwrap().success() {
                tracing::info!(key = key_id, server, "received archzfs key");
                break;
            }
        }

        let mut lsign_args: Vec<&str> = Vec::new();
        if let Some(ref dir) = gpgdir {
            lsign_args.extend_from_slice(&["--gpgdir", dir]);
        }
        lsign_args.extend_from_slice(&["--lsign-key", key_id]);
        let _ = runner.run("pacman-key", &lsign_args);
    }

    // Sync databases
    if target.is_none() {
        let output = runner.run("pacman", &["-Sy", "--noconfirm"])?;
        check_exit(&output, "pacman -Sy after adding archzfs")?;
    }

    Ok(())
}

pub fn set_parallel_downloads(target: Option<&Path>, count: u32) -> Result<()> {
    let pacman_conf = match target {
        Some(t) => t.join("etc/pacman.conf"),
        None => std::path::PathBuf::from("/etc/pacman.conf"),
    };

    let content = std::fs::read_to_string(&pacman_conf)?;
    let new_line = format!("ParallelDownloads = {count}");

    let new_content = if content.contains("ParallelDownloads") {
        content
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("ParallelDownloads")
                    || line.trim_start().starts_with("#ParallelDownloads")
                {
                    new_line.as_str()
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        format!("{content}\n{new_line}\n")
    };

    std::fs::write(&pacman_conf, new_content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_pacstrap_builds_correct_args() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        let _ = pacstrap(&runner, Path::new("/mnt"), &["base", "linux-lts"], None);

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, "pacstrap");
        assert!(calls[0].args.contains(&"-K".to_string()));
        assert!(calls[0].args.contains(&"/mnt".to_string()));
        assert!(calls[0].args.contains(&"base".to_string()));
        assert!(calls[0].args.contains(&"linux-lts".to_string()));
        assert!(calls[0].args.contains(&"--noconfirm".to_string()));
        assert!(calls[0].args.contains(&"--needed".to_string()));
    }

    #[test]
    fn test_sync_db_runs_pacman() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        sync_db(&runner).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "pacman");
        assert!(calls[0].args.contains(&"-Syy".to_string()));
    }

    #[test]
    fn test_set_parallel_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("etc/pacman.conf");
        std::fs::create_dir_all(conf_path.parent().unwrap()).unwrap();
        std::fs::write(&conf_path, "#ParallelDownloads = 5\n").unwrap();

        set_parallel_downloads(Some(dir.path()), 10).unwrap();

        let content = std::fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("ParallelDownloads = 10"));
        assert!(!content.contains("#ParallelDownloads"));
    }
}
