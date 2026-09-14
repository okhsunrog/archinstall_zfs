//! Writing the wizard's configuration to a file and reading it back.
//!
//! The passwords live in a second file beside the first, so a
//! configuration can be shared, copied to another machine or kept for a
//! reinstall without carrying them. Both halves are what `--config` and
//! `--secrets` already accept on the command line.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use archinstall_zfs_core::config::io::secrets_path_for;
use archinstall_zfs_core::config::types::GlobalConfig;

thread_local! {
    /// What the last save or load did, shown on the review screen. The
    /// footer's status line is taken by the validation summary there, and
    /// this is the one place where the outcome stays in view.
    static LAST_RESULT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The outcome of the last save or load, empty until one happens.
pub fn last_result() -> String {
    LAST_RESULT.with_borrow(Clone::clone)
}

fn remember(message: String) -> String {
    LAST_RESULT.with_borrow_mut(|last| *last = message.clone());
    message
}

/// Where the dialogs start from: the live session's home, which is in RAM,
/// so a saved configuration has to be copied somewhere lasting to survive
/// a reboot. That is the user's choice to make, not ours.
pub fn default_path() -> PathBuf {
    PathBuf::from("/root/azfs-config.json")
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Save `config` to `path`. The returned string is what the wizard shows.
pub fn export(config: &GlobalConfig, path: &str) -> String {
    let path = Path::new(path.trim());
    if path.as_os_str().is_empty() {
        return remember("Enter a path to save the configuration to.".into());
    }
    remember(match config.save_with_secrets(path) {
        Ok(written) if written.len() > 1 => format!(
            "Saved {} and its passwords in {}",
            path.display(),
            name(&written[1])
        ),
        Ok(_) => format!("Saved {}", path.display()),
        Err(error) => format!("Could not save {}: {error}", path.display()),
    })
}

/// Load a configuration from `path`, with the passwords beside it when
/// that file is there. On failure the message is the wizard's status line.
pub fn import(path: &str) -> Result<(GlobalConfig, String), String> {
    let path = Path::new(path.trim());
    if path.as_os_str().is_empty() {
        return Err(remember(
            "Enter the path of a configuration to load.".into(),
        ));
    }
    match GlobalConfig::load_with_secrets(path) {
        Ok(config) => {
            let status = if secrets_path_for(path).is_file() {
                format!("Loaded {} with its passwords", path.display())
            } else if config.has_secrets() {
                format!("Loaded {}", path.display())
            } else {
                format!(
                    "Loaded {}. It carries no passwords; set them before installing.",
                    path.display()
                )
            };
            Ok((config, remember(status)))
        }
        Err(error) => Err(remember(format!(
            "Could not load {}: {error}",
            path.display()
        ))),
    }
}
