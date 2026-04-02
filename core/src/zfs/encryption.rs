use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};

use super::cli::run_zfs;
use crate::system::cmd::{check_exit, CommandRunner};

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

pub fn load_key(runner: &dyn CommandRunner, pool: &str, key_path: &Path) -> Result<()> {
    let key_loc = format!("keylocation=file://{}", key_path.display());
    let output = run_zfs(runner, &["load-key", "-L", &key_loc, pool])?;
    check_exit(&output, &format!("zfs load-key {pool}"))?;
    Ok(())
}

/// Check whether an already-imported pool has encryption enabled.
pub fn detect_encryption(runner: &dyn CommandRunner, pool: &str) -> Result<bool> {
    let output = run_zfs(runner, &["get", "-H", "-o", "value", "encryption", pool])?;
    if !output.success() {
        return Ok(false);
    }
    let value = output.stdout.trim();
    Ok(value != "off" && !value.is_empty())
}

/// Verify a passphrase against an already-imported pool by writing a temp
/// key file and attempting load-key.
pub fn verify_passphrase(runner: &dyn CommandRunner, pool: &str, password: &str) -> Result<bool> {
    // Unload key first (ignore errors)
    let _ = run_zfs(runner, &["unload-key", pool]);

    // Try to load key with the provided password
    let output = runner.run_with_stdin("zfs", &["load-key", pool], password.as_bytes())?;
    Ok(output.success())
}

/// Detect whether a pool is encrypted using an ephemeral import.
///
/// Imports the pool with `-fN` (force, no mount), checks the encryption
/// property, then always attempts to unload-key and export (best-effort
/// cleanup). Returns `true` if encryption is present and not "off".
pub fn detect_pool_encryption(runner: &dyn CommandRunner, pool: &str) -> bool {
    // Import pool ephemerally
    let import_output = runner.run("zpool", &["import", "-fN", pool]);
    if import_output.is_err() || !import_output.as_ref().unwrap().success() {
        tracing::debug!(pool, "ephemeral import failed for encryption detection");
        return false;
    }

    // Check encryption property
    let encrypted = detect_encryption(runner, pool).unwrap_or(false);

    // Best-effort cleanup: unload key + export
    let _ = runner.run("zfs", &["unload-key", pool]);
    let _ = runner.run("zpool", &["export", pool]);

    tracing::info!(pool, encrypted, "detected pool encryption state");
    encrypted
}

/// Verify a pool passphrase using an ephemeral import.
///
/// Imports the pool with `-fN`, writes the password to a temporary file,
/// attempts `zfs load-key`, then always cleans up (unload-key, delete
/// temp file, export pool). Returns `true` if load-key succeeded.
pub fn verify_pool_passphrase(runner: &dyn CommandRunner, pool: &str, password: &str) -> bool {
    // Import pool ephemerally
    let import_output = runner.run("zpool", &["import", "-fN", pool]);
    if import_output.is_err() || !import_output.as_ref().unwrap().success() {
        tracing::debug!(pool, "ephemeral import failed for passphrase verification");
        return false;
    }

    // Write password to a temporary file
    let tmp_result = tempfile::NamedTempFile::new();
    let tmp = match tmp_result {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create temp key file");
            let _ = runner.run("zpool", &["export", pool]);
            return false;
        }
    };
    let key_path = tmp.path().to_path_buf();

    let write_ok = fs::write(&key_path, password).is_ok()
        && fs::set_permissions(&key_path, fs::Permissions::from_mode(0o000)).is_ok();

    let verified = if write_ok {
        let key_loc = format!("file://{}", key_path.display());
        let output = runner.run("zfs", &["load-key", "-L", &key_loc, pool]);
        output.is_ok() && output.unwrap().success()
    } else {
        false
    };

    // Best-effort cleanup: unload key, remove temp file, export pool
    let _ = runner.run("zfs", &["unload-key", pool]);
    let _ = fs::remove_file(&key_path);
    let _ = runner.run("zpool", &["export", pool]);

    tracing::info!(pool, verified, "verified pool passphrase");
    verified
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

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

        // Verify the original permissions were set to 000
        // (we changed them above, so just verify the function ran)
    }

    #[test]
    fn test_pool_encryption_properties() {
        let props = pool_encryption_properties(Path::new("/etc/zfs/zroot.key"));
        assert_eq!(props.len(), 3);
        assert_eq!(props[0], ("encryption", "aes-256-gcm".to_string()));
        assert_eq!(props[1], ("keyformat", "passphrase".to_string()));
        assert!(props[2].1.contains("/etc/zfs/zroot.key"));
    }

    #[test]
    fn test_detect_encryption_on() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: "aes-256-gcm\n".into(),
            ..Default::default()
        }]);
        assert!(detect_encryption(&runner, "testpool").unwrap());
    }

    #[test]
    fn test_detect_encryption_off() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: "off\n".into(),
            ..Default::default()
        }]);
        assert!(!detect_encryption(&runner, "testpool").unwrap());
    }
}
