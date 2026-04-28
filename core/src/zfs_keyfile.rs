//! Encryption key file lifecycle on the install target.
//!
//! Owns the path convention (`/etc/zfs/zroot.key`), the permission policy
//! (mode 000 once written — only loaded via the keyfile, not opened by
//! anything else), and the property bundles that pin a pool/dataset to
//! that file via `keylocation=file://...`.

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
