use std::path::Path;

use color_eyre::eyre::Result;

use super::cli::{run_zpool, run_zpool_json};
use super::models::{ZpoolListOutput, ZpoolStatusOutput};
use crate::system::cmd::{check_exit, CommandRunner};

pub const DEFAULT_POOL_OPTIONS: &[&str] = &[
    "-o",
    "ashift=12",
    "-O",
    "acltype=posixacl",
    "-O",
    "relatime=on",
    "-O",
    "xattr=sa",
    "-o",
    "autotrim=on",
    "-O",
    "dnodesize=auto",
    "-O",
    "normalization=formD",
    "-O",
    "devices=off",
    "-m",
    "none",
];

pub fn create_pool(
    runner: &dyn CommandRunner,
    name: &str,
    device: &Path,
    mountpoint: &Path,
    compression: &str,
    extra_props: &[(&str, &str)],
) -> Result<()> {
    let device_str = device.to_str().unwrap();
    let mount_str = mountpoint.to_str().unwrap();

    let mut args: Vec<&str> = vec!["create", "-f"];
    args.extend_from_slice(DEFAULT_POOL_OPTIONS);
    args.extend_from_slice(&["-R", mount_str]);
    let compression_opt = format!("compression={compression}");
    args.extend_from_slice(&["-O", &compression_opt]);

    // leak-safe: we only build short-lived owned strings and keep refs to them
    let owned_props: Vec<String> = extra_props
        .iter()
        .flat_map(|(k, v)| vec![format!("-O"), format!("{k}={v}")])
        .collect();
    let prop_refs: Vec<&str> = owned_props.iter().map(|s| s.as_str()).collect();
    args.extend_from_slice(&prop_refs);

    args.push(name);
    args.push(device_str);

    let output = run_zpool(runner, &args)?;
    check_exit(&output, "zpool create")?;
    Ok(())
}

pub fn import_pool(runner: &dyn CommandRunner, name: &str, mountpoint: &Path) -> Result<()> {
    let mount_str = mountpoint.to_str().unwrap();
    let output = run_zpool(runner, &["import", "-f", "-R", mount_str, name])?;
    check_exit(&output, "zpool import")?;
    Ok(())
}

pub fn import_pool_no_mount(
    runner: &dyn CommandRunner,
    name: &str,
    mountpoint: &Path,
) -> Result<()> {
    let mount_str = mountpoint.to_str().unwrap();
    let output = run_zpool(runner, &["import", "-N", "-R", mount_str, name])?;
    check_exit(&output, "zpool import -N")?;
    Ok(())
}

pub fn export_pool(runner: &dyn CommandRunner, name: &str) -> Result<()> {
    nix::unistd::sync();
    // Try umount all first
    let _ = super::cli::run_zfs(runner, &["umount", "-a"]);
    let output = run_zpool(runner, &["export", name])?;
    check_exit(&output, "zpool export")?;
    Ok(())
}

pub fn set_pool_property(
    runner: &dyn CommandRunner,
    pool: &str,
    property: &str,
    value: &str,
) -> Result<()> {
    let prop_val = format!("{property}={value}");
    let output = run_zpool(runner, &["set", &prop_val, pool])?;
    check_exit(&output, &format!("zpool set {prop_val}"))?;
    Ok(())
}

pub fn list_pools(runner: &dyn CommandRunner) -> Result<ZpoolListOutput> {
    run_zpool_json(runner, &["list"])
}

pub fn pool_status(runner: &dyn CommandRunner, pool: &str) -> Result<ZpoolStatusOutput> {
    run_zpool_json(runner, &["status", pool])
}

pub fn pool_exists(runner: &dyn CommandRunner, name: &str) -> Result<bool> {
    let output = run_zpool(runner, &["list", name])?;
    Ok(output.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_create_pool_command() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        create_pool(
            &runner,
            "testpool",
            Path::new("/dev/disk/by-id/test-part2"),
            Path::new("/mnt"),
            "lz4",
            &[],
        )
        .unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "zpool");
        assert!(calls[0].args.contains(&"create".to_string()));
        assert!(calls[0].args.contains(&"testpool".to_string()));
        assert!(calls[0].args.contains(&"ashift=12".to_string()));
    }

    #[test]
    fn test_create_pool_with_encryption_props() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        create_pool(
            &runner,
            "encpool",
            Path::new("/dev/disk/by-id/test-part2"),
            Path::new("/mnt"),
            "zstd",
            &[
                ("encryption", "aes-256-gcm"),
                ("keyformat", "passphrase"),
                ("keylocation", "file:///etc/zfs/zroot.key"),
            ],
        )
        .unwrap();

        let calls = runner.calls();
        let args_str = calls[0].args.join(" ");
        assert!(args_str.contains("encryption=aes-256-gcm"));
        assert!(args_str.contains("keyformat=passphrase"));
    }

    #[test]
    fn test_import_pool_command() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        import_pool(&runner, "mypool", Path::new("/mnt")).unwrap();

        let calls = runner.calls();
        assert!(calls[0].args.contains(&"import".to_string()));
        assert!(calls[0].args.contains(&"mypool".to_string()));
        assert!(calls[0].args.contains(&"-f".to_string()));
    }

    #[test]
    fn test_export_pool_command() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // zfs umount -a
            CannedResponse::default(), // zpool export
        ]);
        export_pool(&runner, "mypool").unwrap();

        let calls = runner.calls();
        // First call: zfs umount -a
        assert_eq!(calls[0].program, "zfs");
        // Second call: zpool export
        assert_eq!(calls[1].program, "zpool");
        assert!(calls[1].args.contains(&"export".to_string()));
        assert!(calls[1].args.contains(&"mypool".to_string()));
    }
}
