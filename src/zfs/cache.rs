use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{check_exit, CommandRunner};

const HOSTID_VALUE: &str = "00bab10c";

pub fn create_hostid(runner: &dyn CommandRunner) -> Result<()> {
    let output = runner.run("zgenhostid", &["-f", &format!("0x{HOSTID_VALUE}")])?;
    check_exit(&output, "zgenhostid")?;
    Ok(())
}

pub fn prepare_zfs_cache(target: &Path, pool_name: &str) -> Result<()> {
    let cache_dir = target.join("etc/zfs/zfs-list.cache");
    fs::create_dir_all(&cache_dir)
        .wrap_err_with(|| format!("failed to create cache dir: {}", cache_dir.display()))?;

    let cache_file = cache_dir.join(pool_name);
    if !cache_file.exists() {
        fs::write(&cache_file, "")
            .wrap_err_with(|| format!("failed to create cache file: {}", cache_file.display()))?;
    }
    Ok(())
}

pub fn copy_hostid(target: &Path) -> Result<()> {
    let src = Path::new("/etc/hostid");
    let dst = target.join("etc/hostid");
    if src.exists() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, &dst).wrap_err("failed to copy hostid to target")?;
    }
    Ok(())
}

pub fn copy_zfs_cache(target: &Path, pool_name: &str) -> Result<()> {
    let src_cache = Path::new("/etc/zfs/zfs-list.cache").join(pool_name);
    let dst_cache = target.join("etc/zfs/zfs-list.cache").join(pool_name);
    if src_cache.exists() {
        if let Some(parent) = dst_cache.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&src_cache, &dst_cache).wrap_err("failed to copy ZFS cache to target")?;
    }
    Ok(())
}

pub fn copy_misc_files(target: &Path, pool_name: &str) -> Result<()> {
    copy_hostid(target)?;
    copy_zfs_cache(target, pool_name)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_create_hostid() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        create_hostid(&runner).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "zgenhostid");
        assert!(calls[0].args.contains(&format!("0x{HOSTID_VALUE}")));
    }

    #[test]
    fn test_prepare_zfs_cache() {
        let dir = tempfile::tempdir().unwrap();
        prepare_zfs_cache(dir.path(), "testpool").unwrap();

        let cache_file = dir.path().join("etc/zfs/zfs-list.cache/testpool");
        assert!(cache_file.exists());
    }
}
