use std::path::Path;

use color_eyre::eyre::Result;

use super::cmd::CommandRunner;
use crate::distro::{Distribution, Repository};

/// Tried in order until one answers with the key.
const KEYSERVERS: &[&str] = &[
    "hkps://keyserver.ubuntu.com",
    "hkps://pgp.mit.edu",
    "hkps://keys.openpgp.org",
];

/// Add a distribution's repositories to pacman.conf and trust their keys.
///
/// `target` selects which pacman.conf: the medium's own when `None`, the
/// installed system's when given.
pub fn add_repositories(
    runner: &dyn CommandRunner,
    target: Option<&Path>,
    distro: &Distribution,
) -> Result<()> {
    let pacman_conf = match target {
        Some(t) => t.join("etc/pacman.conf"),
        None => std::path::PathBuf::from("/etc/pacman.conf"),
    };

    let mut content = std::fs::read_to_string(&pacman_conf)?;
    for repo in distro.repositories {
        // Rewritten rather than appended when already present, so re-running
        // an installation does not stack duplicate blocks.
        content = replace_repo_block(&content, repo);
        tracing::info!(repo = repo.name, path = %pacman_conf.display(), "repository configured");
    }
    std::fs::write(&pacman_conf, content)?;

    init_keyring(runner, target, distro.keyring);
    for repo in distro.repositories {
        trust_repository_keys(runner, target, repo);
    }

    // Database sync is handled by the caller via AlpmContext::sync_databases()
    Ok(())
}

/// Populate the keyring the distribution's packages are signed against.
fn init_keyring(runner: &dyn CommandRunner, target: Option<&Path>, keyring: &str) {
    let run = |args: &[&str]| match target {
        Some(t) => crate::system::cmd::chroot_cmd(runner, t, "pacman-key", args),
        None => runner.run("pacman-key", args),
    };

    let initialised = run(&["--init"]);
    let result = match &initialised {
        Ok(output) if output.success() => run(&["--populate", keyring]),
        _ => initialised,
    };
    if let Ok(output) = &result
        && !output.success()
    {
        tracing::warn!(
            keyring,
            "pacman-key init/populate had issues: {}",
            output.stderr.trim()
        );
    }
}

/// Receive and locally sign the keys a repository's packages are signed with.
fn trust_repository_keys(runner: &dyn CommandRunner, target: Option<&Path>, repo: &Repository) {
    let run = |args: &[&str]| match target {
        Some(t) => crate::system::cmd::chroot_cmd(runner, t, "pacman-key", args),
        None => runner.run("pacman-key", args),
    };

    for key_id in repo.key_ids {
        let received =
            KEYSERVERS
                .iter()
                .any(|server| match run(&["--keyserver", server, "-r", key_id]) {
                    Ok(output) if output.success() => {
                        tracing::info!(repo = repo.name, key = key_id, server, "received key");
                        true
                    }
                    _ => false,
                });
        if !received {
            tracing::warn!(
                repo = repo.name,
                key = key_id,
                "failed to receive key from any keyserver"
            );
            continue;
        }
        let _ = run(&["--lsign-key", key_id]);
    }
}

/// Replace a repository's block, or append it when the file has none.
fn replace_repo_block(content: &str, repo: &Repository) -> String {
    let header = format!("[{}]", repo.name);
    let mut result = String::new();
    let mut inside = false;

    for line in content.lines() {
        if line.trim() == header {
            inside = true;
            continue;
        }
        if inside {
            // The block runs until the next section header.
            if line.starts_with('[') {
                inside = false;
            } else {
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    // Appended either way: an existing block was dropped above, and where a
    // repository sits only matters against ones offering the same packages.
    result.push_str(&repo.pacman_conf_block());
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
    use crate::distro::Signatures;

    const REPO: Repository = Repository {
        name: "archzfs",
        servers: &["https://example.invalid/archzfs"],
        mirrorlist: None,
        key_ids: &[],
        signatures: Signatures::Never,
    };

    #[test]
    fn a_repository_is_added_once() {
        let conf = "[options]\nHoldPkg = pacman\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n";

        let once = replace_repo_block(conf, &REPO);
        let twice = replace_repo_block(&once, &REPO);

        assert_eq!(once.matches("[archzfs]").count(), 1);
        assert_eq!(
            twice.matches("[archzfs]").count(),
            1,
            "re-running an installation must not stack blocks: {twice}"
        );
    }

    #[test]
    fn rewriting_replaces_the_old_settings() {
        // What a previous version of this installer left behind.
        let conf = "[options]\n\n[archzfs]\nSigLevel = Optional\nServer = https://old.invalid\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n";

        let result = replace_repo_block(conf, &REPO);

        assert!(!result.contains("https://old.invalid"), "got: {result}");
        assert!(result.contains("https://example.invalid/archzfs"));
        assert!(result.contains("SigLevel = Never"));
        // Untouched sections survive.
        assert!(result.contains("[core]"));
        assert!(result.contains("Include = /etc/pacman.d/mirrorlist"));
    }

    #[test]
    fn other_sections_keep_their_settings() {
        let conf = "[options]\nParallelDownloads = 5\n\n[extra]\nSigLevel = Required\n";

        let result = replace_repo_block(conf, &REPO);

        assert!(result.contains("ParallelDownloads = 5"));
        assert!(result.contains("[extra]\nSigLevel = Required"));
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
