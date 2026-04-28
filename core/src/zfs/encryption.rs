use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};

const KEY_FILE_PATH: &str = "etc/zfs/zroot.key";

pub fn key_file_path(target: &Path) -> PathBuf {
    target.join(KEY_FILE_PATH)
}

pub fn write_key_file(target: &Path, password: &str) -> Result<()> {
    let key_path = key_file_path(target);
    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)
            .wrap_err_with(|| format!("failed to create dir: {}", parent.display()))?;
    }
    fs::write(&key_path, password).wrap_err("failed to write key file")?;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o000))
        .wrap_err("failed to set key file permissions")?;
    tracing::info!(path = %key_path.display(), "wrote encryption key file");
    Ok(())
}

pub fn pool_encryption_properties(key_path: &Path) -> Vec<(&'static str, String)> {
    vec![
        ("encryption", "aes-256-gcm".to_string()),
        ("keyformat", "passphrase".to_string()),
        ("keylocation", format!("file://{}", key_path.display())),
    ]
}

pub fn dataset_encryption_properties(key_path: &Path) -> Vec<(&'static str, String)> {
    pool_encryption_properties(key_path)
}

/// File-based load-key for the install pipeline. Loads from `key_path` via
/// `-L file://<key_path>`, overriding whatever the dataset's stored
/// `keylocation` property is. Used during prepare/mount where we know the
/// key file's path explicitly.
pub async fn load_key(zfs: &palimpsest::Zfs, dataset: &str, key_path: &Path) -> Result<()> {
    let key_loc = format!("file://{}", key_path.display());
    zfs.dataset(dataset)
        .load_key_with_keylocation(&key_loc)
        .await?;
    Ok(())
}

/// Detect whether a pool is encrypted using an ephemeral import. Public
/// wrapper that hides palimpsest from callers — UI crates call this with
/// just a pool name.
pub async fn detect_pool_encryption(pool_name: &str) -> bool {
    detect_pool_encryption_with_zfs(&palimpsest::Zfs::new(), pool_name).await
}

/// Inner orchestrator with an injectable `Zfs` handle. `pub(crate)` because
/// the only caller outside this file is the test module; production callers
/// go through `detect_pool_encryption(name)`.
///
/// Imports the pool with `-fN` (force, no mount), checks the encryption
/// property, then always attempts to unload-key and export (best-effort
/// cleanup). Returns `true` if encryption is present and not "off".
pub(crate) async fn detect_pool_encryption_with_zfs(
    zfs: &palimpsest::Zfs,
    pool_name: &str,
) -> bool {
    let pool = zfs.pool(pool_name);
    let dataset = zfs.dataset(pool_name);

    let opts = palimpsest::pool::ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    };
    if pool.import(&opts).await.is_err() {
        tracing::debug!(
            pool_name,
            "ephemeral import failed for encryption detection"
        );
        return false;
    }

    let encrypted = match dataset.get_property("encryption").await {
        Ok(p) => p.value != "off" && !p.value.is_empty(),
        Err(_) => false,
    };

    let _ = dataset.unload_key().await;
    let _ = pool
        .export(&palimpsest::pool::ExportOptions::default())
        .await;

    tracing::info!(pool_name, encrypted, "detected pool encryption state");
    encrypted
}

/// Verify a pool passphrase using an ephemeral import. Public wrapper.
pub async fn verify_pool_passphrase(pool_name: &str, password: &str) -> bool {
    verify_pool_passphrase_with_zfs(&palimpsest::Zfs::new(), pool_name, password).await
}

/// Inner orchestrator with injectable Zfs handle for testing.
///
/// Imports the pool with `-fN`, attempts `zfs load-key` with the passphrase
/// piped via stdin, then always cleans up (unload-key, export pool). No
/// temporary key file on disk — the passphrase never touches the filesystem.
pub(crate) async fn verify_pool_passphrase_with_zfs(
    zfs: &palimpsest::Zfs,
    pool_name: &str,
    password: &str,
) -> bool {
    let pool = zfs.pool(pool_name);
    let dataset = zfs.dataset(pool_name);

    let opts = palimpsest::pool::ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    };
    if pool.import(&opts).await.is_err() {
        tracing::debug!(
            pool_name,
            "ephemeral import failed for passphrase verification"
        );
        return false;
    }

    // Best-effort: ensure key is unloaded so load_key starts from a clean state.
    let _ = dataset.unload_key().await;

    let verified = dataset
        .load_key_with_passphrase(password.as_bytes())
        .await
        .is_ok();

    let _ = dataset.unload_key().await;
    let _ = pool
        .export(&palimpsest::pool::ExportOptions::default())
        .await;

    tracing::info!(pool_name, verified, "verified pool passphrase");
    verified
}

#[cfg(test)]
mod tests {
    use super::*;
    use palimpsest::{Cmd, RecordingRunner, Zfs};

    #[test]
    fn test_write_key_file() {
        let dir = tempfile::tempdir().unwrap();
        write_key_file(dir.path(), "testpassword").unwrap();

        let key_path = key_file_path(dir.path());
        assert!(key_path.exists());

        // File has mode 000, so restore read permission to verify content
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o400)).unwrap();
        let content = fs::read_to_string(&key_path).unwrap();
        assert_eq!(content, "testpassword");
    }

    #[test]
    fn test_pool_encryption_properties() {
        let props = pool_encryption_properties(Path::new("/etc/zfs/zroot.key"));
        assert_eq!(props.len(), 3);
        assert_eq!(props[0], ("encryption", "aes-256-gcm".to_string()));
        assert_eq!(props[1], ("keyformat", "passphrase".to_string()));
        assert!(props[2].1.contains("/etc/zfs/zroot.key"));
    }

    fn get_property_json(pool: &str, value: &str, source_kind: &str) -> Vec<u8> {
        format!(
            "{{\"output_version\":{{\"command\":\"zfs get\",\"vers_major\":0,\"vers_minor\":1}},\
             \"datasets\":{{\"{pool}\":{{\"name\":\"{pool}\",\"type\":\"FILESYSTEM\",\
             \"pool\":\"{pool}\",\"createtxg\":\"1\",\"properties\":{{\"encryption\":\
             {{\"value\":\"{value}\",\"source\":{{\"type\":\"{source_kind}\",\"data\":\"-\"}}}}}}}}}}}}"
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn test_detect_pool_encryption_encrypted() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "testpool"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "testpool"]),
                get_property_json("testpool", "aes-256-gcm", "LOCAL"),
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "testpool"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zpool").args(["export", "testpool"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        assert!(detect_pool_encryption_with_zfs(&zfs, "testpool").await);
    }

    #[tokio::test]
    async fn test_detect_pool_encryption_not_encrypted() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "testpool"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "testpool"]),
                get_property_json("testpool", "off", "DEFAULT"),
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "testpool"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zpool").args(["export", "testpool"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        assert!(!detect_pool_encryption_with_zfs(&zfs, "testpool").await);
    }

    #[tokio::test]
    async fn test_detect_pool_encryption_import_fails() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zpool").args(["import", "-f", "-N", "badpool"]),
            vec![],
            b"cannot import 'badpool': no such pool available\n".to_vec(),
            1,
        );
        let zfs = Zfs::with_runner(runner);
        assert!(!detect_pool_encryption_with_zfs(&zfs, "badpool").await);
    }

    #[tokio::test]
    async fn test_verify_pool_passphrase_correct() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "testpool"]),
                vec![],
                vec![],
                0,
            )
            // best-effort pre-clean unload, will say "Key already unloaded"
            .record(
                Cmd::new("zfs").args(["unload-key", "testpool"]),
                vec![],
                b"Key unload error: Key already unloaded for 'testpool'.\n".to_vec(),
                255,
            )
            .record(
                Cmd::new("zfs")
                    .args(["load-key", "testpool"])
                    .stdin_secret(b"correct".to_vec()),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zpool").args(["export", "testpool"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        assert!(verify_pool_passphrase_with_zfs(&zfs, "testpool", "correct").await);
    }

    #[tokio::test]
    async fn test_verify_pool_passphrase_wrong() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "testpool"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "testpool"]),
                vec![],
                b"Key unload error: Key already unloaded for 'testpool'.\n".to_vec(),
                255,
            )
            .record(
                Cmd::new("zfs")
                    .args(["load-key", "testpool"])
                    .stdin_secret(b"wrong".to_vec()),
                vec![],
                b"Key load error: Incorrect key provided for 'testpool'.\n".to_vec(),
                1,
            )
            .record(
                Cmd::new("zpool").args(["export", "testpool"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        assert!(!verify_pool_passphrase_with_zfs(&zfs, "testpool", "wrong").await);
    }

    #[tokio::test]
    async fn test_verify_pool_passphrase_import_fails() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zpool").args(["import", "-f", "-N", "badpool"]),
            vec![],
            b"cannot import 'badpool': no such pool available\n".to_vec(),
            1,
        );
        let zfs = Zfs::with_runner(runner);
        assert!(!verify_pool_passphrase_with_zfs(&zfs, "badpool", "pass").await);
    }
}
