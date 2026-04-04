use std::path::Path;

use color_eyre::eyre::{Result, bail};

use super::cmd::CommandRunner;

pub fn wait_for_db_lock(_runner: &dyn CommandRunner) -> Result<()> {
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

const ARCHZFS_REPO_BLOCK: &str = "\n[archzfs]\nSigLevel = Never\nServer = https://github.com/archzfs/archzfs/releases/download/experimental\n";

const ARCHZFS_KEY_IDS: &[&str] = &[
    "3A9917BF0DED5C13F69AC68FABEC0A1208037BE9",
    "DDF7DB817396A49B2A2723F7403BD972F75D9D76",
];

const KEYSERVERS: &[&str] = &[
    "hkps://keyserver.ubuntu.com",
    "hkps://pgp.mit.edu",
    "hkps://pool.sks-keyservers.net",
    "hkps://keys.openpgp.org",
];

pub fn add_archzfs_repo(runner: &dyn CommandRunner, target: Option<&Path>) -> Result<()> {
    let pacman_conf = match target {
        Some(t) => t.join("etc/pacman.conf"),
        None => std::path::PathBuf::from("/etc/pacman.conf"),
    };

    // Rewrite or append [archzfs] block
    let content = std::fs::read_to_string(&pacman_conf)?;
    if content.contains("[archzfs]") {
        let new_content = rewrite_archzfs_block(&content);
        std::fs::write(&pacman_conf, new_content)?;
        tracing::info!("updated existing archzfs repo block");
    } else {
        let mut new_content = content;
        new_content.push_str(ARCHZFS_REPO_BLOCK);
        std::fs::write(&pacman_conf, new_content)?;
        tracing::info!(path = %pacman_conf.display(), "added archzfs repo to pacman.conf");
    }

    // Initialize keyring
    let init_result = if let Some(t) = target {
        let r = crate::system::cmd::chroot_cmd(runner, t, "pacman-key", &["--init"]);
        if let Ok(ref output) = r
            && output.success()
        {
            crate::system::cmd::chroot_cmd(runner, t, "pacman-key", &["--populate", "archlinux"])
        } else {
            r
        }
    } else {
        let r = runner.run("pacman-key", &["--init"]);
        if let Ok(ref output) = r
            && output.success()
        {
            runner.run("pacman-key", &["--populate", "archlinux"])
        } else {
            r
        }
    };
    if let Ok(ref output) = init_result
        && !output.success()
    {
        tracing::warn!(
            "pacman-key init/populate had issues: {}",
            output.stderr.trim()
        );
    }

    // Import archzfs signing keys
    for key_id in ARCHZFS_KEY_IDS {
        let mut received = false;
        for server in KEYSERVERS {
            let output = if let Some(t) = target {
                crate::system::cmd::chroot_cmd(
                    runner,
                    t,
                    "pacman-key",
                    &["--keyserver", server, "-r", key_id],
                )
            } else {
                runner.run("pacman-key", &["--keyserver", server, "-r", key_id])
            };
            if output.is_ok() && output.as_ref().unwrap().success() {
                tracing::info!(key = key_id, server, "received archzfs key");
                received = true;
                break;
            }
        }
        if !received {
            tracing::warn!(key = key_id, "failed to receive key from any keyserver");
        }

        // Locally sign the key
        let _ = if let Some(t) = target {
            crate::system::cmd::chroot_cmd(runner, t, "pacman-key", &["--lsign-key", key_id])
        } else {
            runner.run("pacman-key", &["--lsign-key", key_id])
        };
    }

    // Database sync is handled by the caller via AlpmContext::sync_databases()
    Ok(())
}

fn rewrite_archzfs_block(content: &str) -> String {
    let mut result = String::new();
    let mut in_archzfs_block = false;

    for line in content.lines() {
        if line.trim() == "[archzfs]" {
            in_archzfs_block = true;
            continue;
        }
        if in_archzfs_block {
            if line.starts_with('[') {
                in_archzfs_block = false;
                result.push_str(line);
                result.push('\n');
            }
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }

    result.push_str(ARCHZFS_REPO_BLOCK);
    result
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
