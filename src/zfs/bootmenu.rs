use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{check_exit, CommandRunner};

const ZBM_EFI_URL: &str = "https://get.zfsbootmenu.org/efi/sigs";

pub fn download_zbm_efi(runner: &dyn CommandRunner, efi_dir: &Path) -> Result<()> {
    let zbm_dir = efi_dir.join("EFI/ZBM");
    std::fs::create_dir_all(&zbm_dir)?;

    let target_file = zbm_dir.join("vmlinuz.EFI");
    let target_str = target_file.to_str().unwrap();

    // Download ZFSBootMenu EFI binary
    let output = runner.run(
        "curl",
        &["-L", "-o", target_str, "https://get.zfsbootmenu.org/efi"],
    )?;
    check_exit(&output, "download ZFSBootMenu EFI")?;

    tracing::info!(path = %target_file.display(), "downloaded ZFSBootMenu EFI");
    Ok(())
}

pub fn create_efi_entry(
    runner: &dyn CommandRunner,
    efi_partition: &Path,
    pool_name: &str,
    prefix: &str,
    hostid: &str,
    init_system: &str,
    zswap_enabled: bool,
) -> Result<()> {
    let efi_str = efi_partition.to_str().unwrap();

    // Build kernel command line for ZFSBootMenu
    let root_ds = format!("{pool_name}/{prefix}/root");
    let spl_hostid = format!("spl.spl_hostid={hostid}");

    let mut cmdline = match init_system {
        "dracut" => format!("root=ZFS={root_ds} {spl_hostid}"),
        _ => format!("zfs={root_ds} {spl_hostid}"),
    };

    if zswap_enabled {
        cmdline.push_str(" zswap.enabled=1");
    } else {
        cmdline.push_str(" zswap.enabled=0");
    }

    // Create EFI boot entry
    let output = runner.run(
        "efibootmgr",
        &[
            "-c",
            "-d",
            efi_str,
            "-p",
            "1",
            "-L",
            "ZFSBootMenu",
            "-l",
            "\\EFI\\ZBM\\vmlinuz.EFI",
        ],
    )?;
    check_exit(&output, "efibootmgr create entry")?;

    tracing::info!("created ZFSBootMenu EFI boot entry");
    Ok(())
}

pub fn set_zbm_commandline(
    runner: &dyn CommandRunner,
    pool_name: &str,
    prefix: &str,
    init_system: &str,
    hostid: &str,
    zswap_enabled: bool,
) -> Result<()> {
    let root_ds = format!("{pool_name}/{prefix}/root");
    let spl_hostid = format!("spl.spl_hostid={hostid}");

    let mut cmdline = match init_system {
        "dracut" => format!("root=ZFS={root_ds} {spl_hostid}"),
        _ => format!("zfs={root_ds} {spl_hostid}"),
    };

    if zswap_enabled {
        cmdline.push_str(" zswap.enabled=1");
    } else {
        cmdline.push_str(" zswap.enabled=0");
    }

    // Set org.zfsbootmenu:commandline on the root dataset
    let prop = format!("org.zfsbootmenu:commandline={cmdline}");
    let output = runner.run("zfs", &["set", &prop, &root_ds])?;
    check_exit(&output, "set ZBM commandline property")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_download_zbm_efi() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        download_zbm_efi(&runner, dir.path()).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "curl");
        assert!(calls[0].args.contains(&"-L".to_string()));

        // Verify directory was created
        assert!(dir.path().join("EFI/ZBM").exists());
    }

    #[test]
    fn test_set_zbm_commandline_dracut() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        set_zbm_commandline(&runner, "mypool", "arch0", "dracut", "00bab10c", false).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "zfs");
        let args_str = calls[0].args.join(" ");
        assert!(args_str.contains("org.zfsbootmenu:commandline="));
        assert!(args_str.contains("root=ZFS=mypool/arch0/root"));
        assert!(args_str.contains("spl.spl_hostid=00bab10c"));
        assert!(args_str.contains("zswap.enabled=0"));
    }

    #[test]
    fn test_set_zbm_commandline_mkinitcpio_zswap() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        set_zbm_commandline(&runner, "pool", "arch1", "mkinitcpio", "00bab10c", true).unwrap();

        let calls = runner.calls();
        let args_str = calls[0].args.join(" ");
        assert!(args_str.contains("zfs=pool/arch1/root"));
        assert!(args_str.contains("zswap.enabled=1"));
    }
}
