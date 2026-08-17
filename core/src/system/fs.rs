//! Filesystem helpers for files whose permissions matter.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use color_eyre::eyre::{Context, Result};

/// Create or replace `path` with `contents`, applying `mode` *before* any
/// content lands on disk.
///
/// The obvious `fs::write` + `set_permissions` pair leaves a window where the
/// file exists with the process umask's permissions — typically 0644 — which
/// matters when the content is an encryption passphrase or an SSH key. Opening
/// with the mode up front closes that window.
///
/// `O_NOFOLLOW` refuses to write through a symlink: these paths live in an
/// install target that may be a pre-existing filesystem, so a planted link
/// must not redirect the write elsewhere. The file is truncated only after a
/// successful open, so a failed call cannot destroy the previous content.
pub fn write_file_with_mode(
    path: &Path,
    contents: &[u8],
    mode: u32,
    description: &str,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(mode)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .wrap_err_with(|| format!("failed to open {description}: {}", path.display()))?;
    // An already-existing file keeps its old mode through open(2), so set it
    // explicitly rather than trusting the creation mode.
    file.set_permissions(fs::Permissions::from_mode(mode))
        .wrap_err_with(|| format!("failed to secure {description}: {}", path.display()))?;
    file.set_len(0)
        .wrap_err_with(|| format!("failed to truncate {description}: {}", path.display()))?;
    file.write_all(contents)
        .wrap_err_with(|| format!("failed to write {description}: {}", path.display()))?;
    file.sync_all()
        .wrap_err_with(|| format!("failed to sync {description}: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_is_never_world_readable_and_content_lands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");

        write_file_with_mode(&path, b"hunter2", 0o600, "test file").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "hunter2");
    }

    #[test]
    fn existing_file_is_truncated_and_re_secured() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        fs::write(&path, "a much longer previous secret").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_file_with_mode(&path, b"new", 0o600, "test file").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn refuses_to_write_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        let link = dir.path().join("link");
        fs::write(&victim, "do not overwrite").unwrap();
        std::os::unix::fs::symlink(&victim, &link).unwrap();

        let result = write_file_with_mode(&link, b"clobbered", 0o600, "test file");

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(victim).unwrap(), "do not overwrite");
    }
}
