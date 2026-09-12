//! What an interrupted installation leaves behind for the next run to pick
//! up: the configuration as it was submitted and, once the disk has been
//! prepared, the partitions that now exist. A cancelled alongside or
//! full-disk run has already changed the partition table, so the natural
//! way to continue is on those partitions rather than by planning again.
//!
//! The state lives on the live system's tmpfs, readable by root only. It
//! carries the configuration's secrets, which is why it stays there and is
//! removed when an installation completes.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::types::{GlobalConfig, InstallationMode};
use crate::prepare::PreparedPartitions;

pub const STATE_DIR: &str = "/tmp/archinstall-zfs";
pub const STATE_FILE: &str = "/tmp/archinstall-zfs/interrupted.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interrupted {
    pub config: GlobalConfig,
    /// Set once the disk has been partitioned or resolved.
    pub prepared: Option<PreparedPartitions>,
    /// Unix time of the start of the run.
    pub started: u64,
}

impl Interrupted {
    /// The configuration to continue with. Partitions that were already
    /// created are used as they are: a run that had repartitioned the disk
    /// is continued in the existing-partitions mode on those partitions,
    /// since planning again would find the disk changed.
    pub fn config_to_continue(&self) -> GlobalConfig {
        let mut config = self.config.clone();
        if let Some(prepared) = &self.prepared
            && matches!(
                config.installation_mode,
                Some(InstallationMode::FullDisk | InstallationMode::Alongside)
            )
        {
            config.installation_mode = Some(InstallationMode::NewPool);
            config.efi_partition = Some(prepared.efi.clone());
            config.zfs_partition = prepared.zfs.clone();
            config.swap_partition = prepared.swap.clone();
            config.alongside = None;
            config.disk = None;
        }
        config
    }

    /// One line for the welcome screen.
    pub fn summary(&self, now: u64) -> String {
        let mode = match self.config.installation_mode {
            Some(InstallationMode::FullDisk) => "erase a disk",
            Some(InstallationMode::Alongside) => "install alongside",
            Some(InstallationMode::NewPool) => "use existing partitions",
            Some(InstallationMode::ExistingPool) => "use an existing pool",
            None => "unknown mode",
        };
        let ago = now.saturating_sub(self.started);
        let when = if ago < 120 {
            "moments ago".to_string()
        } else if ago < 7200 {
            format!("{} minutes ago", ago / 60)
        } else {
            format!("{} hours ago", ago / 3600)
        };
        let disk = self
            .prepared
            .as_ref()
            .map(|p| format!(", partitions already created on {}", p.efi.display()))
            .unwrap_or_default();
        format!("{mode}, started {when}{disk}")
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn write(path: &Path, state: &Interrupted) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(state)?)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// Remember a run that is about to start.
pub fn record_start(config: &GlobalConfig) {
    record_start_at(Path::new(STATE_FILE), config);
}

pub fn record_start_at(path: &Path, config: &GlobalConfig) {
    let state = Interrupted {
        config: config.clone(),
        prepared: None,
        started: now(),
    };
    if let Err(error) = write(path, &state) {
        tracing::warn!(%error, "cannot record the installation for a later resume");
    }
}

/// Remember the partitions the run has just created or resolved.
pub fn record_prepared(prepared: &PreparedPartitions) {
    record_prepared_at(Path::new(STATE_FILE), prepared);
}

pub fn record_prepared_at(path: &Path, prepared: &PreparedPartitions) {
    let Some(mut state) = load_from(path) else {
        return;
    };
    state.prepared = Some(prepared.clone());
    if let Err(error) = write(path, &state) {
        tracing::warn!(%error, "cannot record the prepared partitions");
    }
}

/// Forget the run: it completed, or the user discarded it.
pub fn clear() {
    let _ = std::fs::remove_file(STATE_FILE);
}

pub fn load() -> Option<Interrupted> {
    load_from(Path::new(STATE_FILE))
}

pub fn load_from(path: &Path) -> Option<Interrupted> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes)
        .wrap_err_with(|| format!("cannot read {}", path.display()))
        .map_err(|error| tracing::warn!(%error, "ignoring the interrupted-installation record"))
        .ok()
}

pub fn state_path() -> PathBuf {
    PathBuf::from(STATE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partitioned_run_continues_on_its_partitions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("interrupted.json");
        let config = GlobalConfig {
            installation_mode: Some(InstallationMode::FullDisk),
            disk: Some("/dev/disk/by-id/x".into()),
            hostname: Some("box".into()),
            ..GlobalConfig::default()
        };
        record_start_at(&path, &config);
        let before = load_from(&path).unwrap();
        assert!(before.prepared.is_none());
        assert_eq!(
            before.config_to_continue().installation_mode,
            Some(InstallationMode::FullDisk)
        );

        record_prepared_at(
            &path,
            &PreparedPartitions {
                efi: "/dev/disk/by-id/x-part1".into(),
                zfs: Some("/dev/disk/by-id/x-part2".into()),
                swap: Some("/dev/disk/by-id/x-part3".into()),
            },
        );
        let after = load_from(&path).unwrap();
        let cont = after.config_to_continue();
        assert_eq!(cont.installation_mode, Some(InstallationMode::NewPool));
        assert_eq!(
            cont.efi_partition.as_deref(),
            Some(Path::new("/dev/disk/by-id/x-part1"))
        );
        assert_eq!(
            cont.zfs_partition.as_deref(),
            Some(Path::new("/dev/disk/by-id/x-part2"))
        );
        assert_eq!(
            cont.swap_partition.as_deref(),
            Some(Path::new("/dev/disk/by-id/x-part3"))
        );
        assert_eq!(cont.hostname.as_deref(), Some("box"));
        assert!(cont.disk.is_none());
        assert!(
            after
                .summary(after.started + 30)
                .starts_with("erase a disk, started moments ago")
        );
        assert!(
            after
                .summary(after.started + 600)
                .contains("10 minutes ago")
        );
        assert!(after.summary(after.started).contains("x-part1"));
    }

    #[test]
    fn a_run_that_never_touched_the_disk_keeps_its_mode() {
        let config = GlobalConfig {
            installation_mode: Some(InstallationMode::Alongside),
            ..GlobalConfig::default()
        };
        let state = Interrupted {
            config,
            prepared: None,
            started: 0,
        };
        assert_eq!(
            state.config_to_continue().installation_mode,
            Some(InstallationMode::Alongside)
        );
        assert!(load_from(Path::new("/nonexistent/interrupted.json")).is_none());
    }
}
