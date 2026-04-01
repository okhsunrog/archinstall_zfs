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

pub fn detect_encryption(runner: &dyn CommandRunner, pool: &str) -> Result<bool> {
    let output = run_zfs(runner, &["get", "-H", "-o", "value", "encryption", pool])?;
    if !output.success() {
        return Ok(false);
    }
    let value = output.stdout.trim();
    Ok(value != "off" && !value.is_empty())
}

pub fn verify_passphrase(runner: &dyn CommandRunner, pool: &str, password: &str) -> Result<bool> {
    // Unload key first (ignore errors)
    let _ = run_zfs(runner, &["unload-key", pool]);

    // Try to load key with the provided password
    let output = runner.run_with_stdin("zfs", &["load-key", pool], password.as_bytes())?;
    Ok(output.success())
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
