use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};
use serde::Serialize;

use crate::boot_environment::BootEnvironment;
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
    prefix: String,
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
            image_dir: "/var/lib/zfsbootmenu".into(),
            versions: false,
            enabled: true,
        },
        kernel: ZbmKernel {
            // Keep the filename stable: the NVRAM entries and fallback copy
            // below deliberately point at this name.
            prefix: "vmlinuz".into(),
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

[Trigger]
Type = Package
Operation = Install
Operation = Upgrade
Target = zfsbootmenu
Target = zfs-utils

[Action]
Description = Regenerating ZFSBootMenu...
When = PostTransaction
Exec = /usr/local/sbin/azfs-update-zbm
Depends = zfsbootmenu
"#;

fn install_zbm_pacman_hook(target: &Path) -> Result<()> {
    for (path, contents) in [
        (
            "usr/local/libexec/azfs-install-zbm",
            include_str!("../assets/azfs-install-zbm"),
        ),
        (
            "usr/local/sbin/azfs-update-zbm",
            include_str!("../assets/azfs-update-zbm"),
        ),
    ] {
        let path = target.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, contents)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    let hooks_dir = target.join("etc/pacman.d/hooks");
    fs::create_dir_all(&hooks_dir)?;
    fs::write(hooks_dir.join("95-zfsbootmenu.hook"), ZBM_PACMAN_HOOK)?;
    tracing::info!("installed ZBM pacman hook");
    Ok(())
}

/// Install zfsbootmenu from AUR and run generate-zbm to build the EFI bundle.
/// This replaces the old pre-built download approach with a locally built ZBM
/// that uses the same kernel and ZFS modules as the installed system.
/// Build ZFSBootMenu from the AUR, for distributions that do not package it.
async fn install_zbm_from_aur(
    runner: std::sync::Arc<dyn CommandRunner>,
    target: &Path,
    cancel: &tokio_util::sync::CancellationToken,
    download_config: crate::system::async_download::DownloadConfig,
) -> Result<()> {
    crate::installer::aur::install_aur_packages(
        runner,
        target,
        &["zfsbootmenu"],
        cancel,
        download_config,
    )
    .await
}

pub async fn install_and_generate_zbm(
    runner: std::sync::Arc<dyn CommandRunner>,
    target: &Path,
    distro: &'static crate::distro::Distribution,
    init_system: InitSystem,
    cancel: &tokio_util::sync::CancellationToken,
    download_config: crate::system::async_download::DownloadConfig,
) -> Result<()> {
    // 1. Install ZFSBootMenu, from the distribution's repositories when it
    //    has it and from the AUR when it does not. Asking the AUR for a
    //    package the repositories already satisfy resolves to nothing to
    //    build, and nothing gets installed at all.
    if let Some(package) = distro.zfsbootmenu_package {
        tracing::info!(package, "installing ZFSBootMenu from the distribution");
        let target_owned = target.to_path_buf();
        let cancel_owned = cancel.clone();
        let config = download_config.clone();
        tokio::task::spawn_blocking(move || {
            crate::system::alpm_pacman::install_into_target(
                &target_owned,
                &[package],
                &cancel_owned,
                config,
            )
        })
        .await??;
    } else {
        tracing::info!("building ZFSBootMenu from the AUR");
        install_zbm_from_aur(runner.clone(), target, cancel, download_config.clone()).await?;
    }
    if cancel.is_cancelled() {
        color_eyre::eyre::bail!("installation cancelled");
    }

    // 2-5. Sync operations: config, hooks, generate-zbm, copy EFI
    let r = runner;
    let t = target.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        write_zbm_config(&t, init_system)?;
        install_zbm_pacman_hook(&t)?;

        tracing::info!("running generate-zbm to build EFI bundle");
        let output = chroot_cmd(&*r, &t, "/usr/local/sbin/azfs-update-zbm", &[])?;
        check_exit(&output, "generate and install ZFSBootMenu")?;

        tracing::info!("ZFSBootMenu built and installed locally");
        Ok(())
    })
    .await?
}

/// Create efibootmgr entries pointing to the locally-built ZBM EFI bundle.
/// Since the cmdline is already embedded in the EFI by generate-zbm,
/// we don't need to pass -u here.
struct EfiLocation {
    disk: PathBuf,
    partition: u32,
    partuuid: String,
}

fn resolve_efi_location(runner: &dyn CommandRunner, efi_partition: &Path) -> Result<EfiLocation> {
    let efi = efi_partition.to_string_lossy();
    let output = runner.run(
        "lsblk",
        &[
            "--noheadings",
            "--raw",
            "--paths",
            "--output",
            "PKNAME,PARTN,PARTUUID",
            &efi,
        ],
    )?;
    check_exit(&output, "resolve EFI disk and partition")?;
    let fields: Vec<_> = output.stdout.split_whitespace().collect();
    if fields.len() != 3 {
        color_eyre::eyre::bail!(
            "cannot resolve EFI disk, partition number and PARTUUID for {}",
            efi_partition.display()
        );
    }
    Ok(EfiLocation {
        disk: fields[0].into(),
        partition: fields[1].parse().wrap_err("invalid EFI partition number")?,
        partuuid: fields[2].to_ascii_lowercase(),
    })
}

fn boot_entry(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.strip_prefix("Boot")?;
    let number = rest.get(..4)?;
    if !number.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let rest = rest.get(4..)?.strip_prefix('*').unwrap_or(&rest[4..]);
    let device_start = rest.find("HD(")?;
    Some((number, rest[..device_start].trim(), &rest[device_start..]))
}

fn entry_matches(device_path: &str, location: &EfiLocation, loader: &str) -> bool {
    let normalized = device_path.to_ascii_lowercase().replace('/', "\\");
    normalized.contains(&format!(
        "hd({},gpt,{},",
        location.partition, location.partuuid
    )) && normalized.contains(&format!("file({})", loader.to_ascii_lowercase()))
}

fn ensure_efi_entry(
    runner: &dyn CommandRunner,
    existing: &str,
    location: &EfiLocation,
    label: &str,
    loader: &str,
    required: bool,
) -> Result<()> {
    let mut found = false;
    for (number, entry_label, device_path) in existing.lines().filter_map(boot_entry) {
        if entry_label != label {
            continue;
        }
        if !found && entry_matches(device_path, location, loader) {
            found = true;
            continue;
        }
        let output = runner.run("efibootmgr", &["-b", number, "-B"])?;
        if let Err(error) = check_exit(&output, "remove stale EFI boot entry") {
            if required {
                return Err(error);
            }
            tracing::warn!(%error, label, "failed to remove stale optional EFI entry");
        }
    }
    if found {
        return Ok(());
    }

    let disk = location.disk.to_string_lossy();
    let partition = location.partition.to_string();
    let output = runner.run(
        "efibootmgr",
        &[
            "-c", "-d", &disk, "-p", &partition, "-L", label, "-l", loader,
        ],
    )?;
    if required {
        check_exit(&output, "create ZFSBootMenu EFI entry")?;
    } else if !output.success() {
        tracing::warn!(label, "failed to create optional EFI boot entry");
    }
    Ok(())
}

pub fn create_efi_entries(
    runner: &dyn CommandRunner,
    efi_partition: &Path,
    target: &Path,
) -> Result<()> {
    let location = resolve_efi_location(runner, efi_partition)?;

    let existing = runner.run("efibootmgr", &["-v"])?;
    check_exit(&existing, "read EFI boot entries")?;
    ensure_efi_entry(
        runner,
        &existing.stdout,
        &location,
        "ZFSBootMenu",
        "\\EFI\\zbm\\vmlinuz.EFI",
        true,
    )?;
    if target.join("boot/efi/EFI/zbm/vmlinuz-backup.EFI").is_file() {
        ensure_efi_entry(
            runner,
            &existing.stdout,
            &location,
            "ZFSBootMenu (Backup)",
            "\\EFI\\zbm\\vmlinuz-backup.EFI",
            false,
        )?;
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
    be: &BootEnvironment,
    init_system: InitSystem,
    zswap_enabled: bool,
    set_bootfs: bool,
) -> Result<()> {
    let zfs = zfskit::Zfs::new();
    let root_ds = be.root();
    let cmdline = build_zbm_cmdline(zswap_enabled);
    let rootprefix = rootprefix_for(init_system);

    let root_handle = zfs.dataset(&root_ds)?;
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
        zfs.pool(be.pool())?
            .set_property("bootfs", &root_ds)
            .await?;
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
        assert!(config.contains("ImageDir: /var/lib/zfsbootmenu"));
        assert!(config.contains("Enabled: true"));
        assert!(config.contains("Prefix: vmlinuz"));
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
        assert!(content.contains("azfs-update-zbm"));
        assert!(content.contains("zfs.ko"));
        assert!(content.contains("pkgbase"));
        // A new ZBM or zfs-utils release changes neither the kernel nor
        // zfs.ko, so the path trigger alone would leave a stale EFI bundle.
        assert!(content.contains("Type = Package"));
        assert!(content.contains("Target = zfsbootmenu"));
        assert!(content.contains("Target = zfs-utils"));
    }

    fn target_with_zbm(backup: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let zbm = dir.path().join("boot/efi/EFI/zbm");
        fs::create_dir_all(&zbm).unwrap();
        fs::write(zbm.join("vmlinuz.EFI"), b"main").unwrap();
        if backup {
            fs::write(zbm.join("vmlinuz-backup.EFI"), b"backup").unwrap();
        }
        dir
    }

    #[test]
    fn test_create_efi_entries_uses_the_selected_disk_and_partition() {
        // With locally-built ZBM, cmdline is embedded - no -u needed
        let target = target_with_zbm(true);
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: "/dev/nvme0n1 7 AABB-CCDD\n".into(),
                ..Default::default()
            },
            CannedResponse {
                stdout: "BootCurrent: 0000\n".into(),
                ..Default::default()
            },
            CannedResponse::default(), // efibootmgr -c (main)
            CannedResponse::default(), // efibootmgr -c (backup)
        ]);

        create_efi_entries(
            &runner,
            Path::new("/dev/disk/by-id/disk-part7"),
            target.path(),
        )
        .unwrap();

        let calls = runner.calls();
        let main_call = &calls[2];
        assert!(!main_call.args.contains(&"-u".to_string()));
        assert!(
            main_call
                .args
                .windows(2)
                .any(|a| a == ["-d", "/dev/nvme0n1"])
        );
        assert!(main_call.args.windows(2).any(|a| a == ["-p", "7"]));
        assert!(main_call.args.iter().any(|a| a.contains("vmlinuz.EFI")));
    }

    #[test]
    fn test_matching_entries_are_kept() {
        let target = target_with_zbm(true);
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: "/dev/sda 1 aabb-ccdd\n".into(),
                ..Default::default()
            },
            CannedResponse {
                stdout: concat!(
                    "Boot0001* ZFSBootMenu\tHD(1,GPT,AABB-CCDD,0x800,0x1000)/File(\\EFI\\zbm\\vmlinuz.EFI)\n",
                    "Boot0002* ZFSBootMenu (Backup)\tHD(1,GPT,AABB-CCDD,0x800,0x1000)/File(\\EFI\\zbm\\vmlinuz-backup.EFI)\n"
                )
                .into(),
                ..Default::default()
            },
        ]);

        create_efi_entries(&runner, Path::new("/dev/sda1"), target.path()).unwrap();

        assert_eq!(runner.calls().len(), 2);
    }

    #[test]
    fn test_backup_entry_does_not_hide_a_missing_main_entry() {
        let target = target_with_zbm(true);
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: "/dev/sda 1 aabb-ccdd\n".into(),
                ..Default::default()
            },
            CannedResponse {
                stdout: "Boot0002* ZFSBootMenu (Backup)\tHD(1,GPT,AABB-CCDD,0x800,0x1000)/File(\\EFI\\zbm\\vmlinuz-backup.EFI)\n".into(),
                ..Default::default()
            },
            CannedResponse::default(),
        ]);

        create_efi_entries(&runner, Path::new("/dev/sda1"), target.path()).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls[2].args.windows(2).any(|a| a == ["-L", "ZFSBootMenu"]));
    }

    #[test]
    fn test_stale_same_name_entry_is_replaced() {
        let target = target_with_zbm(false);
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                stdout: "/dev/sda 3 aabb-ccdd\n".into(),
                ..Default::default()
            },
            CannedResponse {
                stdout: "Boot00AF* ZFSBootMenu\tHD(1,GPT,DEAD-BEEF,0x800,0x1000)/File(\\EFI\\zbm\\vmlinuz.EFI)\n".into(),
                ..Default::default()
            },
            CannedResponse::default(),
            CannedResponse::default(),
        ]);

        create_efi_entries(&runner, Path::new("/dev/sda3"), target.path()).unwrap();

        let calls = runner.calls();
        assert_eq!(calls[2].args, ["-b", "00AF", "-B"]);
        assert!(calls[3].args.windows(2).any(|a| a == ["-p", "3"]));
    }
}
