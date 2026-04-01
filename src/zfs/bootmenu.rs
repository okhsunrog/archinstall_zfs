use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{check_exit, CommandRunner};

pub const HOSTID_VALUE: &str = "0x00bab10c";

pub fn download_zbm_efi(runner: &dyn CommandRunner, efi_dir: &Path) -> Result<()> {
    let zbm_dir = efi_dir.join("EFI/ZBM");
    std::fs::create_dir_all(&zbm_dir)?;

    let main_file = zbm_dir.join("VMLINUZ.EFI");
    let main_str = main_file.to_str().unwrap();

    // Download main ZBM image
    let output = runner.run(
        "curl",
        &["-L", "-o", main_str, "https://get.zfsbootmenu.org/efi"],
    )?;
    check_exit(&output, "download ZFSBootMenu EFI")?;

    // Download recovery image
    let recovery_file = zbm_dir.join("RECOVERY.EFI");
    let recovery_str = recovery_file.to_str().unwrap();
    let output = runner.run(
        "curl",
        &[
            "-L",
            "-o",
            recovery_str,
            "https://get.zfsbootmenu.org/efi/recovery",
        ],
    )?;
    if !output.success() {
        tracing::warn!("failed to download ZBM recovery image (non-fatal)");
    }

    // Install as UEFI fallback bootloader
    let fallback_dir = efi_dir.join("EFI/BOOT");
    std::fs::create_dir_all(&fallback_dir)?;
    std::fs::copy(&main_file, fallback_dir.join("BOOTX64.EFI"))?;

    tracing::info!(path = %main_file.display(), "downloaded ZFSBootMenu EFI");
    Ok(())
}

pub fn create_efi_entries(runner: &dyn CommandRunner, efi_partition: &Path) -> Result<()> {
    let efi_str = efi_partition.to_str().unwrap();

    // ZBM kernel cmdline: pass hostid so ZBM can import pools,
    // and timeout=10 for auto-boot countdown
    let zbm_cmdline = format!("spl_hostid={HOSTID_VALUE} zbm.timeout=10");

    // Check for existing entries
    let existing = runner.run("efibootmgr", &["-v"])?;
    let existing_text = existing.stdout.clone();

    // Add main entry if not exists
    if !existing_text.contains("ZFSBootMenu") {
        let output = runner.run(
            "efibootmgr",
            &[
                "-c",
                "-d",
                efi_str,
                "-L",
                "ZFSBootMenu",
                "-l",
                "\\EFI\\ZBM\\VMLINUZ.EFI",
                "-u",
                &zbm_cmdline,
            ],
        )?;
        check_exit(&output, "efibootmgr create ZFSBootMenu entry")?;
    }

    // Add recovery entry if not exists
    if !existing_text.contains("ZFSBootMenu-Recovery") {
        let output = runner.run(
            "efibootmgr",
            &[
                "-c",
                "-d",
                efi_str,
                "-L",
                "ZFSBootMenu-Recovery",
                "-l",
                "\\EFI\\ZBM\\RECOVERY.EFI",
                "-u",
                &zbm_cmdline,
            ],
        )?;
        if !output.success() {
            tracing::warn!("failed to create ZBM recovery boot entry (non-fatal)");
        }
    }

    tracing::info!("created ZFSBootMenu EFI boot entries");
    Ok(())
}

/// Set ZFSBootMenu properties on the root dataset.
/// Matches Python ZFSManager.finish() exactly.
///
/// org.zfsbootmenu:commandline does NOT include root= -- ZBM adds it.
/// It only contains: spl.spl_hostid, zswap, rw
pub fn set_zbm_properties(
    runner: &dyn CommandRunner,
    pool_name: &str,
    prefix: &str,
    init_system: &str,
    zswap_enabled: bool,
) -> Result<()> {
    let root_ds = format!("{pool_name}/{prefix}/root");

    // Build commandline: do NOT include root= -- ZBM injects it automatically
    let mut cmdline_parts = vec![format!("spl.spl_hostid={HOSTID_VALUE}")];
    cmdline_parts.push(if zswap_enabled {
        "zswap.enabled=1".to_string()
    } else {
        "zswap.enabled=0".to_string()
    });
    cmdline_parts.push("rw".to_string());
    let cmdline = cmdline_parts.join(" ");

    // Set commandline
    let prop = format!("org.zfsbootmenu:commandline={cmdline}");
    let output = runner.run("zfs", &["set", &prop, &root_ds])?;
    check_exit(&output, "set ZBM commandline")?;

    // Set rootprefix based on init system
    let rootprefix = match init_system {
        "dracut" => "root=ZFS=",
        _ => "zfs=",
    };
    let prop = format!("org.zfsbootmenu:rootprefix={rootprefix}");
    let output = runner.run("zfs", &["set", &prop, &root_ds])?;
    check_exit(&output, "set ZBM rootprefix")?;

    // Set bootfs on pool for auto-boot (ZBM skips menu if bootfs is set)
    let output = runner.run("zpool", &["set", &format!("bootfs={root_ds}"), pool_name])?;
    check_exit(&output, "set pool bootfs")?;

    tracing::info!(
        cmdline,
        rootprefix,
        bootfs = root_ds.as_str(),
        "set ZFSBootMenu properties"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_download_zbm_efi() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // curl main
            CannedResponse::default(), // curl recovery
        ]);
        // Create dummy EFI files since mock runner doesn't write files
        let zbm_dir = dir.path().join("EFI/ZBM");
        std::fs::create_dir_all(&zbm_dir).unwrap();
        std::fs::write(zbm_dir.join("VMLINUZ.EFI"), b"fake-efi").unwrap();

        download_zbm_efi(&runner, dir.path()).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "curl");
        assert!(dir.path().join("EFI/BOOT/BOOTX64.EFI").exists());
    }

    #[test]
    fn test_set_zbm_properties_dracut() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // set commandline
            CannedResponse::default(), // set rootprefix
            CannedResponse::default(), // set bootfs
        ]);
        set_zbm_properties(&runner, "mypool", "arch0", "dracut", false).unwrap();

        let calls = runner.calls();

        // Commandline: no root=, has spl.spl_hostid with 0x prefix, has rw
        let cmdline_args = calls[0].args.join(" ");
        assert!(cmdline_args.contains("org.zfsbootmenu:commandline="));
        assert!(!cmdline_args.contains("root="));
        assert!(cmdline_args.contains("spl.spl_hostid=0x00bab10c"));
        assert!(cmdline_args.contains("zswap.enabled=0"));
        assert!(cmdline_args.contains(" rw"));

        // Rootprefix for dracut
        let rootprefix_args = calls[1].args.join(" ");
        assert!(rootprefix_args.contains("root=ZFS="));

        // Bootfs
        let bootfs_args = calls[2].args.join(" ");
        assert!(bootfs_args.contains("bootfs=mypool/arch0/root"));
    }

    #[test]
    fn test_set_zbm_properties_mkinitcpio_zswap() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse::default(),
            CannedResponse::default(),
        ]);
        set_zbm_properties(&runner, "pool", "arch1", "mkinitcpio", true).unwrap();

        let calls = runner.calls();
        let cmdline_args = calls[0].args.join(" ");
        assert!(cmdline_args.contains("zswap.enabled=1"));
        let rootprefix_args = calls[1].args.join(" ");
        assert!(rootprefix_args.contains("rootprefix=zfs="));
    }
}
