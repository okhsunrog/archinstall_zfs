use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{check_exit, chroot, CommandRunner};

pub const HOSTID_VALUE: &str = "0x00bab10c";

/// Write /etc/zfsbootmenu/config.yaml inside the target chroot.
/// This configures generate-zbm to build a unified EFI bundle using the same
/// init system (dracut or mkinitcpio) and kernel as the installed system.
fn write_zbm_config(target: &Path, init_system: &str) -> Result<()> {
    let conf_dir = target.join("etc/zfsbootmenu");
    fs::create_dir_all(&conf_dir)?;

    let initcpio_line = if init_system == "mkinitcpio" {
        "  InitCPIO: true"
    } else {
        "  InitCPIO: false"
    };

    // zbm.timeout=10: auto-boot after 10s countdown
    // zbm.import_policy=hostid: adopt hostid from pool if needed
    let config = format!(
        r#"Global:
  ManageImages: true
  BootMountPoint: /boot/efi
  DracutConfDir: /etc/zfsbootmenu/dracut.conf.d
  InitCPIOConfig: /etc/zfsbootmenu/mkinitcpio.conf
{initcpio_line}
Components:
  Enabled: false
EFI:
  ImageDir: /boot/efi/EFI/zbm
  Versions: false
  Enabled: true
Kernel:
  CommandLine: zbm.import_policy=hostid zbm.timeout=10 ro quiet loglevel=0
"#
    );

    fs::write(conf_dir.join("config.yaml"), config)
        .wrap_err("failed to write ZBM config.yaml")?;
    tracing::info!("wrote /etc/zfsbootmenu/config.yaml (init_system={init_system})");
    Ok(())
}

/// Write a pacman hook that regenerates ZBM images whenever the kernel
/// or ZFS packages are updated. This ensures the bootloader stays in sync.
const ZBM_PACMAN_HOOK: &str = r#"[Trigger]
Type = Path
Operation = Install
Operation = Upgrade
Target = usr/lib/modules/*/pkgbase
Target = usr/lib/modules/*/extramodules/zfs.ko*

[Action]
Description = Regenerating ZFSBootMenu...
When = PostTransaction
Exec = /usr/bin/generate-zbm
Depends = zfsbootmenu
"#;

fn install_zbm_pacman_hook(target: &Path) -> Result<()> {
    let hooks_dir = target.join("etc/pacman.d/hooks");
    fs::create_dir_all(&hooks_dir)?;
    fs::write(hooks_dir.join("95-zfsbootmenu.hook"), ZBM_PACMAN_HOOK)?;
    tracing::info!("installed ZBM pacman hook");
    Ok(())
}

/// Install zfsbootmenu from AUR and run generate-zbm to build the EFI bundle.
/// This replaces the old pre-built download approach with a locally built ZBM
/// that uses the same kernel and ZFS modules as the installed system.
pub fn install_and_generate_zbm(
    runner: &dyn CommandRunner,
    target: &Path,
    init_system: &str,
) -> Result<()> {
    // 1. Install zfsbootmenu from AUR
    tracing::info!("installing zfsbootmenu from AUR");
    crate::installer::aur::install_aur_packages(
        runner,
        target,
        &["zfsbootmenu".to_string()],
    )?;

    // 2. Write config.yaml
    write_zbm_config(target, init_system)?;

    // 3. Install pacman hook for automatic regeneration
    install_zbm_pacman_hook(target)?;

    // 4. Run generate-zbm inside chroot
    tracing::info!("running generate-zbm to build EFI bundle");
    let output = chroot(runner, target, "generate-zbm")?;
    check_exit(&output, "generate-zbm")?;

    // 5. Copy main EFI to UEFI fallback path (EFI/BOOT/BOOTX64.EFI)
    // generate-zbm with Versions: false creates vmlinuz.EFI in ImageDir
    let efi_src = target.join("boot/efi/EFI/zbm/vmlinuz.EFI");
    let fallback_dir = target.join("boot/efi/EFI/BOOT");
    fs::create_dir_all(&fallback_dir)?;
    if efi_src.exists() {
        fs::copy(&efi_src, fallback_dir.join("BOOTX64.EFI"))
            .wrap_err("failed to copy ZBM EFI to fallback path")?;
        tracing::info!("copied ZBM EFI to EFI/BOOT/BOOTX64.EFI fallback");
    } else {
        tracing::warn!("generate-zbm output not found at expected path, checking alternatives");
        // generate-zbm may use the kernel prefix; look for any .EFI in the dir
        let zbm_dir = target.join("boot/efi/EFI/zbm");
        if zbm_dir.exists() {
            for entry in fs::read_dir(&zbm_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("efi"))
                    && !path.to_string_lossy().contains("backup")
                {
                    fs::copy(&path, fallback_dir.join("BOOTX64.EFI"))
                        .wrap_err("failed to copy ZBM EFI to fallback path")?;
                    tracing::info!(
                        src = %path.display(),
                        "copied ZBM EFI to EFI/BOOT/BOOTX64.EFI fallback"
                    );
                    break;
                }
            }
        }
    }

    tracing::info!("ZFSBootMenu built and installed locally");
    Ok(())
}

/// Create efibootmgr entries pointing to the locally-built ZBM EFI bundle.
/// Since the cmdline is already embedded in the EFI by generate-zbm,
/// we don't need to pass -u here.
pub fn create_efi_entries(runner: &dyn CommandRunner, efi_partition: &Path) -> Result<()> {
    let efi_str = efi_partition.to_str().unwrap();

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
                "\\EFI\\zbm\\vmlinuz.EFI",
            ],
        )?;
        check_exit(&output, "efibootmgr create ZFSBootMenu entry")?;
    }

    // Add backup entry if backup exists
    if !existing_text.contains("ZFSBootMenu (Backup)") {
        let output = runner.run(
            "efibootmgr",
            &[
                "-c",
                "-d",
                efi_str,
                "-L",
                "ZFSBootMenu (Backup)",
                "-l",
                "\\EFI\\zbm\\vmlinuz-backup.EFI",
            ],
        )?;
        if !output.success() {
            tracing::warn!("failed to create ZBM backup boot entry (non-fatal)");
        }
    }

    tracing::info!("created ZFSBootMenu EFI boot entries");
    Ok(())
}

/// Set ZFSBootMenu properties on the root dataset.
///
/// org.zfsbootmenu:commandline does NOT include root= -- ZBM adds it.
/// It only contains: spl.spl_hostid, zswap, rw
pub fn set_zbm_properties(
    runner: &dyn CommandRunner,
    pool_name: &str,
    prefix: &str,
    init_system: &str,
    zswap_enabled: bool,
    set_bootfs: bool,
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

    if set_bootfs {
        // Set bootfs so ZBM knows which BE to auto-boot after the timeout.
        // With zbm.timeout=10 in the EFI cmdline, ZBM shows the menu with a
        // 10-second countdown then boots the bootfs dataset. Users can press
        // any key during the countdown to browse/select other BEs.
        // Without bootfs, ZBM ignores zbm.timeout and always waits for input.
        let output =
            runner.run("zpool", &["set", &format!("bootfs={root_ds}"), pool_name])?;
        check_exit(&output, "set pool bootfs")?;
        tracing::info!(cmdline, rootprefix, bootfs = root_ds.as_str(), "set ZFSBootMenu properties");
    } else {
        tracing::info!(cmdline, rootprefix, "set ZFSBootMenu properties (bootfs disabled)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_write_zbm_config_dracut() {
        let dir = tempfile::tempdir().unwrap();
        write_zbm_config(dir.path(), "dracut").unwrap();

        let config = fs::read_to_string(dir.path().join("etc/zfsbootmenu/config.yaml")).unwrap();
        assert!(config.contains("ManageImages: true"));
        assert!(config.contains("InitCPIO: false"));
        assert!(config.contains("zbm.timeout=10"));
        assert!(config.contains("Versions: false"));
        assert!(config.contains("Enabled: true"));
    }

    #[test]
    fn test_write_zbm_config_mkinitcpio() {
        let dir = tempfile::tempdir().unwrap();
        write_zbm_config(dir.path(), "mkinitcpio").unwrap();

        let config = fs::read_to_string(dir.path().join("etc/zfsbootmenu/config.yaml")).unwrap();
        assert!(config.contains("InitCPIO: true"));
        assert!(config.contains("zbm.timeout=10"));
    }

    #[test]
    fn test_install_zbm_pacman_hook() {
        let dir = tempfile::tempdir().unwrap();
        install_zbm_pacman_hook(dir.path()).unwrap();

        let hook_path = dir.path().join("etc/pacman.d/hooks/95-zfsbootmenu.hook");
        assert!(hook_path.exists());
        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("generate-zbm"));
        assert!(content.contains("zfs.ko"));
        assert!(content.contains("pkgbase"));
    }

    #[test]
    fn test_set_zbm_properties_dracut_with_bootfs() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // set commandline
            CannedResponse::default(), // set rootprefix
            CannedResponse::default(), // set bootfs
        ]);
        set_zbm_properties(&runner, "mypool", "arch0", "dracut", false, true).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 3);

        let cmdline_args = calls[0].args.join(" ");
        assert!(cmdline_args.contains("org.zfsbootmenu:commandline="));
        assert!(!cmdline_args.contains("root="));
        assert!(cmdline_args.contains("spl.spl_hostid=0x00bab10c"));
        assert!(cmdline_args.contains("zswap.enabled=0"));
        assert!(cmdline_args.contains(" rw"));

        let rootprefix_args = calls[1].args.join(" ");
        assert!(rootprefix_args.contains("root=ZFS="));

        let bootfs_args = calls[2].args.join(" ");
        assert!(bootfs_args.contains("bootfs=mypool/arch0/root"));
    }

    #[test]
    fn test_set_zbm_properties_without_bootfs() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // set commandline
            CannedResponse::default(), // set rootprefix
            // no bootfs call
        ]);
        set_zbm_properties(&runner, "mypool", "arch0", "dracut", false, false).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "should not set bootfs when disabled");
    }

    #[test]
    fn test_set_zbm_properties_mkinitcpio_zswap() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // set commandline
            CannedResponse::default(), // set rootprefix
            CannedResponse::default(), // set bootfs
        ]);
        set_zbm_properties(&runner, "pool", "arch1", "mkinitcpio", true, true).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 3);
        let cmdline_args = calls[0].args.join(" ");
        assert!(cmdline_args.contains("zswap.enabled=1"));
        let rootprefix_args = calls[1].args.join(" ");
        assert!(rootprefix_args.contains("rootprefix=zfs="));
    }

    #[test]
    fn test_create_efi_entries_no_u_flag() {
        // With locally-built ZBM, cmdline is embedded - no -u needed
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: "BootCurrent: 0000\n".into(), // no existing ZFSBootMenu entries
                ..Default::default()
            },
            CannedResponse::default(), // efibootmgr -c (main)
            CannedResponse::default(), // efibootmgr -c (backup)
        ]);

        create_efi_entries(&runner, Path::new("/dev/sda1")).unwrap();

        let calls = runner.calls();
        // Main entry should NOT have -u flag (cmdline embedded in EFI)
        let main_call = &calls[1];
        assert!(!main_call.args.contains(&"-u".to_string()));
        assert!(main_call.args.contains(&"ZFSBootMenu".to_string()));
        assert!(main_call.args.iter().any(|a| a.contains("vmlinuz.EFI")));
    }
}
