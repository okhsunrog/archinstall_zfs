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

/// Detect whether a pool is encrypted using an ephemeral import. Hides
/// palimpsest from UI crates and collapses any failure to `false` (the
/// installer flow treats "can't tell" the same as "not encrypted" — the
/// passphrase prompt simply isn't shown).
pub async fn detect_pool_encryption(pool_name: &str) -> bool {
    palimpsest::Zfs::new()
        .pool(pool_name)
        .is_encrypted()
        .await
        .unwrap_or(false)
}

/// Verify a pool passphrase using an ephemeral import. Same wrapper
/// rationale as [`detect_pool_encryption`]: import errors collapse to
/// `false` so callers don't have to distinguish between "wrong password"
/// and "couldn't import".
pub async fn verify_pool_passphrase(pool_name: &str, password: &str) -> bool {
    palimpsest::Zfs::new()
        .pool(pool_name)
        .verify_passphrase(password.as_bytes())
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
