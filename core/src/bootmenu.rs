use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};
use serde::Serialize;

use crate::config::types::InitSystem;
use crate::system::cmd::{CommandRunner, check_exit, chroot_cmd};

pub const HOSTID_VALUE: &str = "0x00bab10c";

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ZbmConfig {
    global: ZbmGlobal,
    components: ZbmComponents,
    #[serde(rename = "EFI")]
    efi: ZbmEfi,
    kernel: ZbmKernel,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ZbmGlobal {
    manage_images: bool,
    boot_mount_point: String,
    dracut_conf_dir: String,
    #[serde(rename = "InitCPIOConfig")]
    init_cpio_config: String,
    #[serde(rename = "InitCPIO")]
    init_cpio: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ZbmComponents {
    enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ZbmEfi {
    image_dir: String,
    versions: bool,
    enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ZbmKernel {
    command_line: String,
}

/// Write /etc/zfsbootmenu/config.yaml inside the target chroot.
fn write_zbm_config(target: &Path, init_system: InitSystem) -> Result<()> {
    let conf_dir = target.join("etc/zfsbootmenu");
    fs::create_dir_all(&conf_dir)?;

    let config = ZbmConfig {
        global: ZbmGlobal {
            manage_images: true,
            boot_mount_point: "/boot/efi".into(),
            dracut_conf_dir: "/etc/zfsbootmenu/dracut.conf.d".into(),
            init_cpio_config: "/etc/zfsbootmenu/mkinitcpio.conf".into(),
            init_cpio: matches!(init_system, InitSystem::Mkinitcpio),
        },
        components: ZbmComponents { enabled: false },
        efi: ZbmEfi {
            image_dir: "/boot/efi/EFI/zbm".into(),
            versions: false,
            enabled: true,
        },
        kernel: ZbmKernel {
            command_line: "zbm.import_policy=hostid zbm.timeout=10 ro quiet loglevel=0".into(),
        },
    };

    let yaml = serde_yaml_ng::to_string(&config).wrap_err("failed to serialize ZBM config")?;
    fs::write(conf_dir.join("config.yaml"), yaml).wrap_err("failed to write ZBM config.yaml")?;
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
pub async fn install_and_generate_zbm(
    runner: std::sync::Arc<dyn CommandRunner>,
    target: &Path,
    init_system: InitSystem,
    cancel: &tokio_util::sync::CancellationToken,
    download_config: crate::system::async_download::DownloadConfig,
) -> Result<()> {
    // 1. Install zfsbootmenu from AUR (async: AUR dependency resolution)
    tracing::info!("installing zfsbootmenu from AUR");
    crate::installer::aur::install_aur_packages(
        runner.clone(),
        target,
        &["zfsbootmenu"],
        cancel,
        download_config,
    )
    .await?;

    // 2-5. Sync operations: config, hooks, generate-zbm, copy EFI
    let r = runner;
    let t = target.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        write_zbm_config(&t, init_system)?;
        install_zbm_pacman_hook(&t)?;

        tracing::info!("running generate-zbm to build EFI bundle");
        let output = chroot_cmd(&*r, &t, "generate-zbm", &[])?;
        check_exit(&output, "generate-zbm")?;

        let efi_src = t.join("boot/efi/EFI/zbm/vmlinuz.EFI");
        let fallback_dir = t.join("boot/efi/EFI/BOOT");
        fs::create_dir_all(&fallback_dir)?;
        if efi_src.exists() {
            fs::copy(&efi_src, fallback_dir.join("BOOTX64.EFI"))
                .wrap_err("failed to copy ZBM EFI to fallback path")?;
            tracing::info!("copied ZBM EFI to EFI/BOOT/BOOTX64.EFI fallback");
        } else {
            tracing::warn!("generate-zbm output not found at expected path, checking alternatives");
            let zbm_dir = t.join("boot/efi/EFI/zbm");
            if zbm_dir.exists() {
                for entry in fs::read_dir(&zbm_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("efi"))
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
    })
    .await?
}

/// Create efibootmgr entries pointing to the locally-built ZBM EFI bundle.
/// Since the cmdline is already embedded in the EFI by generate-zbm,
/// we don't need to pass -u here.
pub fn create_efi_entries(runner: &dyn CommandRunner, efi_partition: &Path) -> Result<()> {
    let efi_str = efi_partition.to_string_lossy();

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
                &efi_str,
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
                &efi_str,
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
/// Compose the value of `org.zfsbootmenu:commandline`. ZBM injects `root=`
/// itself; we only contribute hostid + zswap toggle + rw.
fn build_zbm_cmdline(zswap_enabled: bool) -> String {
    let zswap = if zswap_enabled {
        "zswap.enabled=1"
    } else {
        "zswap.enabled=0"
    };
    format!("spl.spl_hostid={HOSTID_VALUE} {zswap} rw")
}

/// Map init system to ZBM's `rootprefix` property value (the prefix prepended
/// to the dataset name for the kernel cmdline).
fn rootprefix_for(init_system: InitSystem) -> &'static str {
    match init_system {
        InitSystem::Dracut => "root=ZFS=",
        InitSystem::Mkinitcpio => "zfs=",
    }
}

pub async fn set_zbm_properties(
    pool_name: &str,
    prefix: &str,
    init_system: InitSystem,
    zswap_enabled: bool,
    set_bootfs: bool,
) -> Result<()> {
    let zfs = palimpsest::Zfs::new();
    let root_ds = format!("{pool_name}/{prefix}/root");
    let cmdline = build_zbm_cmdline(zswap_enabled);
    let rootprefix = rootprefix_for(init_system);

    let root_handle = zfs.dataset(&root_ds);
    root_handle
        .set_property("org.zfsbootmenu:commandline", &cmdline)
        .await?;
    root_handle
        .set_property("org.zfsbootmenu:rootprefix", rootprefix)
        .await?;

    if set_bootfs {
        // Set bootfs so ZBM knows which BE to auto-boot after the timeout.
        // With zbm.timeout=10 in the EFI cmdline, ZBM shows the menu with a
        // 10-second countdown then boots the bootfs dataset. Users can press
        // any key during the countdown to browse/select other BEs.
        // Without bootfs, ZBM ignores zbm.timeout and always waits for input.
        zfs.pool(pool_name).set_property("bootfs", &root_ds).await?;
        tracing::info!(
            cmdline,
            rootprefix,
            bootfs = root_ds.as_str(),
            "set ZFSBootMenu properties"
        );
    } else {
        tracing::info!(
            cmdline,
            rootprefix,
            "set ZFSBootMenu properties (bootfs disabled)"
        );
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
        write_zbm_config(dir.path(), InitSystem::Dracut).unwrap();

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
        write_zbm_config(dir.path(), InitSystem::Mkinitcpio).unwrap();

        let config = fs::read_to_string(dir.path().join("etc/zfsbootmenu/config.yaml")).unwrap();
        assert!(config.contains("InitCPIO: true"));
        assert!(config.contains("zbm.timeout=10"));
    }

    #[test]
    fn test_build_zbm_cmdline() {
        assert_eq!(
            build_zbm_cmdline(false),
            "spl.spl_hostid=0x00bab10c zswap.enabled=0 rw"
        );
        assert_eq!(
            build_zbm_cmdline(true),
            "spl.spl_hostid=0x00bab10c zswap.enabled=1 rw"
        );
    }

    #[test]
    fn test_rootprefix_for() {
        assert_eq!(rootprefix_for(InitSystem::Dracut), "root=ZFS=");
        assert_eq!(rootprefix_for(InitSystem::Mkinitcpio), "zfs=");
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
