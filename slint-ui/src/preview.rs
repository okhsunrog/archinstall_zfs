//! Deterministic UI fixtures. Enabled explicitly, and only in desktop-mock builds.
//! Installation, reboot, probes, device discovery and package searches are replaced
//! before their controllers can start real work. Editing still uses the real wizard.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use archinstall_zfs_core::config::types::{
    GlobalConfig, InstallationMode, ProfileSelection, UserConfig,
};
use archinstall_zfs_core::disk::alongside::GIB;
use archinstall_zfs_core::disk::device::{
    BlockDevice, BlockPartition, DeviceChoice, DevicePath, DevicePathKind,
};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::ui::{
    App, DownloadInfo, InstallState, LogMessage, PackageSearchResult, WelcomeState, WizardState,
};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn enable() {
    if !cfg!(feature = "desktop-mock") {
        panic!("preview requires a desktop-mock build");
    }
    ENABLED.store(true, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Scene {
    Welcome,
    Interrupted,
    Offline,
    NoUefi,
    ZfsPreparing,
    ZfsFailed,
    WifiEmpty,
    WifiUnavailable,
    WifiNoInternet,
    WifiVerifying,
    Cancelling,
    Disk,
    NewPool,
    Alongside,
    ExistingPool,
    Zfs,
    System,
    Users,
    Desktop,
    Review,
    Install,
    Done,
    Failed,
    Cancelled,
    Inspect,
    Invalid,
}

#[derive(Clone, Copy, Debug)]
pub struct Size(pub u32, pub u32);

impl std::str::FromStr for Size {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (w, h) = value.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
        let w = w.parse::<u32>().map_err(|_| "invalid width")?;
        let h = h.parse::<u32>().map_err(|_| "invalid height")?;
        if !(640..=7680).contains(&w) || !(480..=4320).contains(&h) {
            return Err("preview size must be between 640x480 and 7680x4320".into());
        }
        Ok(Self(w, h))
    }
}

fn devices() -> Vec<BlockDevice> {
    [
        (
            "nvme0n1",
            "Samsung SSD 990 PRO 2TB",
            "S7HENX0W123456A",
            2048,
            "nvme",
            false,
        ),
        (
            "sda",
            "KINGSTON RBUSNS8180S3128GJ",
            "50026B76825246C7",
            119,
            "sata",
            false,
        ),
        (
            "sdb",
            "Kingston DataTraveler 70",
            "1831BFBC9935E681298501A3",
            115,
            "usb",
            true,
        ),
    ]
    .into_iter()
    .map(|(node, model, serial, size, bus, removable)| BlockDevice {
        devnode: format!("/dev/{node}").into(),
        aliases: vec![DevicePath {
            path: format!(
                "/dev/disk/by-id/{bus}-{}_{}",
                model.replace(' ', "_"),
                serial
            )
            .into(),
            kind: DevicePathKind::ById,
        }],
        model: Some(model.into()),
        serial: Some(serial.into()),
        size_bytes: Some(size * GIB),
        transport: Some(bus.into()),
        rotational: Some(false),
        removable,
        usage: archinstall_zfs_core::disk::device::DeviceUsage {
            in_use: removable,
            ..Default::default()
        },
    })
    .collect()
}

pub fn disks() -> Vec<DeviceChoice> {
    if storage_fixture() == "empty" {
        return vec![];
    }
    devices().into_iter().map(Into::into).collect()
}

fn storage_fixture() -> String {
    std::env::var("AZFS_PREVIEW_STORAGE").unwrap_or_default()
}

pub fn partitions() -> Vec<DeviceChoice> {
    if storage_fixture() == "empty" {
        return vec![];
    }
    devices()
        .into_iter()
        .flat_map(|disk| {
            let count = if storage_fixture() == "many" {
                20
            } else if disk.transport.as_deref() == Some("sata") {
                4
            } else {
                2
            };
            (1..=count).map(move |number| {
                BlockPartition {
                    devnode: format!(
                        "{}{}{number}",
                        disk.devnode.display(),
                        if disk.transport.as_deref() == Some("nvme") {
                            "p"
                        } else {
                            ""
                        }
                    )
                    .into(),
                    aliases: vec![DevicePath {
                        path: format!("{}-part{number}", disk.aliases[0].path.display()).into(),
                        kind: DevicePathKind::ById,
                    }],
                    parent_devnode: Some(disk.devnode.clone()),
                    model: disk.model.clone(),
                    serial: disk.serial.clone(),
                    size_bytes: Some(if number == 1 {
                        GIB
                    } else {
                        (disk.size_bytes.unwrap() - GIB) / (count - 1)
                    }),
                    parent_size_bytes: disk.size_bytes,
                    transport: disk.transport.clone(),
                    rotational: disk.rotational,
                    removable: disk.removable,
                    usage: archinstall_zfs_core::disk::device::DeviceUsage {
                        filesystem: if storage_fixture() == "missing" {
                            ""
                        } else if number == 1 {
                            "vfat"
                        } else if number == 3 {
                            "ntfs"
                        } else {
                            "ext4"
                        }
                        .into(),
                        label: if storage_fixture() == "missing" {
                            "".into()
                        } else if number == 1 {
                            "EFI".into()
                        } else {
                            format!(
                                "{}-partition-{number}",
                                if number == 3 {
                                    "Windows"
                                } else {
                                    "Previous-Linux"
                                }
                            )
                        },
                        partition_type: if number == 1 {
                            "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
                        } else {
                            "0fc63daf-8483-4772-8e79-3d69d8477de4"
                        }
                        .into(),
                        in_use: disk.removable,
                        mountpoints: if disk.removable {
                            vec!["/run/archiso/bootmnt".into()]
                        } else {
                            vec![]
                        },
                    },
                }
                .into()
            })
        })
        .collect()
}

pub fn config(scene: Scene) -> GlobalConfig {
    if matches!(scene, Scene::Invalid) {
        return GlobalConfig::default();
    }
    GlobalConfig {
        installation_mode: Some(match scene {
            Scene::NewPool => InstallationMode::NewPool,
            Scene::Alongside => InstallationMode::Alongside,
            Scene::ExistingPool => InstallationMode::ExistingPool,
            _ => InstallationMode::FullDisk,
        }),
        disk: disks().first().map(|d| d.path.clone()),
        efi_partition: partitions().first().map(|d| d.path.clone()),
        zfs_partition: partitions().get(1).map(|d| d.path.clone()),
        pool_name: Some("zroot".into()),
        hostname: Some("arch-workstation".into()),
        locale: Some("en_US.UTF-8".into()),
        timezone: Some("Europe/Moscow".into()),
        kernels: Some(vec!["linux".into()]),
        root_password: Some("preview-only-password".into()),
        profile_selection: ProfileSelection::new("kde"),
        users: Some(vec![UserConfig {
            username: "alex".into(),
            password: Some("preview-only-password".into()),
            sudo: true,
            shell: Some("/bin/zsh".into()),
            groups: None,
            ssh_authorized_keys: vec![],
            autologin: false,
        }]),
        additional_packages: vec!["firefox".into(), "git".into(), "htop".into()],
        extra_services: vec!["sshd".into()],
        ..GlobalConfig::default()
    }
}

pub fn welcome(app: &App) {
    let state = app.global::<WelcomeState>();
    state.set_app_version(env!("CARGO_PKG_VERSION").into());
    state.set_uefi_ok(true);
    state.set_zfs_ok(true);
    state.set_net_ok(true);
    let weak = app.as_weak();
    state.on_check_internet(move || {
        if let Some(app) = weak.upgrade() {
            let wifi = app.global::<crate::ui::WifiState>();
            app.global::<WelcomeState>()
                .set_net_ok(wifi.get_ethernet_connected() || !wifi.get_current_ssid().is_empty());
        }
    });
}

pub fn packages(query: &str, aur: bool) -> Vec<PackageSearchResult> {
    [
        ("firefox", "Fast, private web browser"),
        ("git", "Distributed version control"),
        ("htop", "Interactive process viewer"),
        (
            "visual-studio-code-bin",
            "Code editor with integrated debugging",
        ),
    ]
    .into_iter()
    .filter(|(name, _)| name.contains(query))
    .map(|(name, description)| PackageSearchResult {
        name: name.into(),
        description: description.into(),
        repo: if aur { "aur" } else { "extra" }.into(),
    })
    .collect()
}

pub fn install(app: &App, config: &std::rc::Rc<std::cell::RefCell<GlobalConfig>>) {
    app.global::<InstallState>()
        .set_log_messages(ModelRc::new(VecModel::<LogMessage>::default()));
    let weak = app.as_weak();
    let config = config.clone();
    app.on_install_requested(move || {
        if let Some(app) = weak.upgrade() {
            if let Some(error) = config.borrow().validate_for_install().first() {
                app.global::<WizardState>()
                    .set_status_text(format!("Validation: {error}").into());
                return;
            }
            progress(&app, 1);
            advance(app.as_weak());
        }
    });
    let weak = app.as_weak();
    app.on_cancel_requested(move || {
        if let Some(app) = weak.upgrade() {
            progress(&app, 5);
        }
    });
}

fn advance(weak: slint::Weak<App>) {
    slint::Timer::single_shot(Duration::from_secs(1), move || {
        let Some(app) = weak.upgrade() else { return };
        let state = app.global::<InstallState>();
        if state.get_state() != 1 {
            return;
        }
        let phase = state.get_phase() + 1;
        if phase >= state.get_total_phases() {
            progress(&app, 2);
        } else {
            state.set_phase(phase);
            advance(app.as_weak());
        }
    });
}

fn progress(app: &App, state: i32) {
    let install = app.global::<InstallState>();
    install.set_state(state);
    install.set_phase(if state == 2 { 14 } else { 5 });
    install.set_phase_label("Installing base system".into());
    install.set_download_active(state == 1);
    install.set_download_pct(62);
    install.set_download_status("62% · 24.8 MiB/s · 18 seconds remaining".into());
    // The panel lists every transfer in flight; the fixture shows a full set
    // of parallel downloads including a stalled one and one being verified.
    let row = |filename: &str, pct: i32, speed: &str, state: i32| DownloadInfo {
        filename: filename.into(),
        pct,
        speed: speed.into(),
        state,
    };
    install.set_download_items(ModelRc::new(VecModel::from(vec![
        row(
            "linux-7.2.3.arch1-3-x86_64.pkg.tar.zst",
            62,
            "24.8 MiB/s",
            0,
        ),
        row(
            "linux-firmware-intel-20260810-2-any.pkg.tar.zst",
            41,
            "9.1 MiB/s",
            0,
        ),
        row(
            "gcc-16.2.1+r23+gd564253eb6c8-1-x86_64.pkg.tar.zst",
            88,
            "6.4 MiB/s",
            0,
        ),
        row("perl-5.42.2-2-x86_64.pkg.tar.zst", 17, "3.2 MiB/s", 0),
        row("systemd-261.2-1-x86_64.pkg.tar.zst", 100, "", 1),
        row(
            "glibc-2.44+r24+g16be1518495f-1-x86_64.pkg.tar.zst",
            0,
            "waiting for mirror",
            0,
        ),
    ])));
    // Enough realistic output to review scrolling, wrapping and persistent actions.
    // All messages describe the preview fixture; no commands are executed here.
    let mut messages = vec![
        "[INFO] Preview mode: simulated installation; no disks are modified.",
        "[INFO] Internet connectivity OK",
        "[INFO] UEFI boot detected",
        "[INFO] ZFS initialized on host",
        "[INFO] Preparing /dev/nvme0n1 and EFI system partition",
        "[INFO] Creating pool zroot",
        "[INFO] Creating zroot/arch0/root",
        "[INFO] Creating zroot/arch0/data/home",
        "[INFO] Creating zroot/arch0/data/root",
        "[INFO] Creating zroot/arch0/vm",
        "[INFO] Mounting EFI partition at /mnt/boot/efi",
        "[INFO] Refreshing package databases",
        "[INFO] Resolving dependencies for base, linux, linux-firmware and zfs-utils",
        "[INFO] Downloading packages",
    ];
    if state == 2 {
        messages.extend([
            "[INFO] Package integrity checks passed",
            "[INFO] Base system installed",
            "[INFO] Configuring hostname, locale and timezone",
            "[INFO] Creating user account previewuser",
            "[INFO] Enabling NetworkManager.service",
            "[INFO] Building initramfs with ZFS support",
            "[INFO] Installing ZFSBootMenu to the EFI system partition",
            "[INFO] Creating UEFI boot entry",
            "[INFO] Unmounting installation filesystems",
            "[INFO] Exporting pool zroot",
            "[INFO] Installation complete!",
        ]);
    } else if state == 3 {
        messages.push("[ERROR] Download failed: connection interrupted while retrieving linux-firmware. Check your network connection before retrying.");
    } else if state == 5 {
        messages.extend([
            "[INFO] Cancellation requested",
            "[INFO] Cleaning up the simulated installation",
            "[INFO] Installation cancelled",
        ]);
    }
    let messages: Vec<_> = messages
        .into_iter()
        .map(|text| LogMessage {
            text: text.into(),
            level: if text.starts_with("[ERROR]") { 4 } else { 2 },
        })
        .collect();
    install.set_log_messages(ModelRc::new(VecModel::from(messages)));
}

pub fn show(app: &App, scene: Scene, size: Size) {
    app.set_preview_mode(true);
    // The headless backend does not consume SLINT_SCALE_FACTOR. Deliver the
    // same event a desktop backend emits so screenshot tests exercise real DPI.
    if let Ok(value) = std::env::var("SLINT_SCALE_FACTOR")
        && let Ok(scale_factor) = value.parse::<f32>()
        && scale_factor.is_finite()
        && scale_factor > 0.0
    {
        app.window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor });
    }
    app.window()
        .set_size(slint::PhysicalSize::new(size.0, size.1));
    let step = match scene {
        Scene::Welcome
        | Scene::Interrupted
        | Scene::Offline
        | Scene::NoUefi
        | Scene::ZfsPreparing
        | Scene::ZfsFailed
        | Scene::WifiEmpty
        | Scene::WifiUnavailable
        | Scene::WifiNoInternet
        | Scene::WifiVerifying => 0,
        Scene::Disk | Scene::NewPool | Scene::ExistingPool | Scene::Alongside => 1,
        Scene::Zfs => 2,
        Scene::System => 3,
        Scene::Users => 4,
        Scene::Desktop => 5,
        _ => 6,
    };
    app.global::<WizardState>().set_max_visited(6);
    app.global::<WizardState>().invoke_go_to(step);
    if matches!(scene, Scene::Offline) {
        app.global::<WelcomeState>().set_net_ok(false);
        app.global::<crate::ui::WifiState>()
            .set_ethernet_connected(false);
    }
    match scene {
        Scene::NoUefi => app.global::<WelcomeState>().set_uefi_ok(false),
        Scene::Interrupted => {
            // A record left by a cancelled alongside run: partitions exist,
            // so continuing reuses them instead of resizing again.
            let state = app.global::<WelcomeState>();
            state.set_interrupted_summary(
                "install alongside, started 12 minutes ago, partitions already created on /dev/nvme0n1p5"
                    .into(),
            );
            state.set_interrupted_available(true);
            let weak = app.as_weak();
            state.on_continue_interrupted(move || {
                if let Some(app) = weak.upgrade() {
                    app.global::<WelcomeState>()
                        .set_interrupted_available(false);
                    app.global::<WizardState>().invoke_go_to(1);
                }
            });
            let weak = app.as_weak();
            state.on_discard_interrupted(move || {
                if let Some(app) = weak.upgrade() {
                    app.global::<WelcomeState>()
                        .set_interrupted_available(false);
                }
            });
        }
        Scene::ZfsPreparing | Scene::ZfsFailed => {
            let state = app.global::<WelcomeState>();
            state.set_zfs_ok(false);
            let preparing = matches!(scene, Scene::ZfsPreparing);
            state.set_zfs_installing(preparing);
            state.set_zfs_install_pct(45);
            state.set_zfs_install_status(if preparing {
                "Building the ZFS module for the running kernel…".into()
            } else {
                "Failed: could not retrieve ZFS packages. Check your network connection and try again.".into()
            });
        }
        Scene::WifiEmpty
        | Scene::WifiUnavailable
        | Scene::WifiNoInternet
        | Scene::WifiVerifying => {
            use crate::ui::{WifiPhase, WifiState};
            let wifi = app.global::<WifiState>();
            app.global::<crate::ui::PopupState>().set_wifi_visible(true);
            wifi.set_visible(true);
            app.global::<WelcomeState>().set_net_ok(false);
            wifi.set_networks(ModelRc::new(VecModel::default()));
            wifi.set_ethernet_connected(false);
            wifi.set_iwd_running(!matches!(scene, Scene::WifiUnavailable));
            wifi.set_phase(match scene {
                Scene::WifiNoInternet => WifiPhase::NoInternet,
                Scene::WifiVerifying => WifiPhase::Verifying,
                _ => WifiPhase::Picking,
            });
            wifi.set_status_text("Checking internet access…".into());
            if matches!(scene, Scene::WifiNoInternet) {
                wifi.set_current_ssid("HomeNetwork".into());
                wifi.set_error_text("Connected to HomeNetwork, but the internet check did not succeed. Check the router or choose another network.".into());
            }
        }
        Scene::Cancelling => progress(app, 4),
        Scene::Install => progress(app, 1),
        Scene::Done => {
            progress(app, 2);
            app.global::<InstallState>().set_shell_available(true);
        }
        Scene::Failed => progress(app, 3),
        Scene::Cancelled => progress(app, 5),
        Scene::Inspect => inspect(app),
        _ => {}
    }
}

fn inspect(app: &App) {
    use crate::ui::{DemoDataset, DemoPool, DemoState};
    use slint::Model;
    let state = app.global::<DemoState>();
    let pools = std::rc::Rc::new(VecModel::from(vec![
        DemoPool {
            name: "zroot".into(),
            state: "ONLINE".into(),
            imported: true,
            owned: false,
        },
        DemoPool {
            name: "backup-workstation".into(),
            state: "ONLINE".into(),
            imported: false,
            owned: false,
        },
    ]));
    state.set_pools(pools.clone().into());
    state.set_datasets(ModelRc::new(VecModel::from(vec![
        DemoDataset {
            name: "zroot/ROOT/arch0".into(),
            kind: "filesystem".into(),
        },
        DemoDataset {
            name: "zroot/ROOT/arch0/home".into(),
            kind: "filesystem".into(),
        },
    ])));
    state.set_status("Simulated pools and datasets".into());
    state.set_popup_visible(true);
    let import_pools = pools.clone();
    state.on_import_readonly(move |name| {
        for index in 0..import_pools.row_count() {
            let mut pool = import_pools.row_data(index).unwrap();
            if pool.name == name {
                pool.imported = true;
                pool.owned = true;
                import_pools.set_row_data(index, pool);
            }
        }
    });
    state.on_export_pool(move |name| {
        for index in 0..pools.row_count() {
            let mut pool = pools.row_data(index).unwrap();
            if pool.name == name {
                pool.imported = false;
                pool.owned = false;
                pools.set_row_data(index, pool);
            }
        }
    });
    let weak = app.as_weak();
    state.on_select_pool(move |name| {
        if let Some(app) = weak.upgrade() {
            app.invoke_text_confirmed("pool_name".into(), name);
            app.global::<DemoState>().set_popup_visible(false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_fixtures_are_valid_and_select_visible_devices() {
        let disks = disks();
        let partitions = partitions();
        for scene in [Scene::Disk, Scene::NewPool, Scene::ExistingPool] {
            let config = config(scene);
            assert!(config.validate_for_install().is_empty());
            assert!(disks.iter().any(|d| Some(&d.path) == config.disk.as_ref()));
            assert!(
                partitions
                    .iter()
                    .any(|p| Some(&p.path) == config.efi_partition.as_ref())
            );
            assert!(
                partitions
                    .iter()
                    .any(|p| Some(&p.path) == config.zfs_partition.as_ref())
            );
        }
        assert!(disks.iter().any(|disk| disk.removable));
    }
}
