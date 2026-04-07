//! Welcome-screen controller: initial system probes (network / UEFI / ZFS),
//! the on_check_internet retry handler, the background ZFS-init job, and the
//! background kernel compatibility scan.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use archinstall_zfs_core::config::types::GlobalConfig;
use archinstall_zfs_core::kernel::scanner::CompatibilityResult;
use slint::{ComponentHandle, SharedString};

use crate::ui::{App, WelcomeState};

/// Cached results of the background kernel compatibility scan. The wizard's
/// "Kernel" item activation reads this to populate the kernel select popup
/// without re-scanning every time.
#[derive(Clone, Default)]
pub struct KernelScan {
    inner: Arc<Mutex<Option<Vec<CompatibilityResult>>>>,
}

impl KernelScan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_some(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    /// Borrow the cached results for the duration of `f`. `None` if the scan
    /// hasn't completed yet.
    pub fn with<R>(&self, f: impl FnOnce(Option<&[CompatibilityResult]>) -> R) -> R {
        let guard = self.inner.lock().unwrap();
        f(guard.as_deref())
    }

    pub(super) fn store(&self, results: Vec<CompatibilityResult>) {
        *self.inner.lock().unwrap() = Some(results);
    }
}

pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>, kernel_scan: &KernelScan) {
    run_initial_checks(app, config, kernel_scan);

    let weak = app.as_weak();
    let cfg = config.clone();
    let kscan = kernel_scan.clone();
    app.global::<WelcomeState>().on_check_internet(move || {
        let Some(app) = weak.upgrade() else { return };
        let net = archinstall_zfs_core::system::net::check_internet();
        app.global::<WelcomeState>().set_net_ok(net);
        if net {
            if !app.global::<WelcomeState>().get_zfs_ok()
                && !app.global::<WelcomeState>().get_zfs_installing()
            {
                start_zfs_init(&app, &cfg.borrow());
            }
            if !kscan.is_some() {
                start_kernel_scan(&kscan);
            }
        }
    });
}

fn run_initial_checks(app: &App, config: &Rc<RefCell<GlobalConfig>>, kernel_scan: &KernelScan) {
    let net = archinstall_zfs_core::system::net::check_internet();
    let uefi = archinstall_zfs_core::system::sysinfo::has_uefi();
    let zfs_mod = archinstall_zfs_core::zfs::kmod::check_zfs_module(
        &archinstall_zfs_core::system::cmd::RealRunner,
    )
    .unwrap_or(false);
    let zfs_utils = archinstall_zfs_core::zfs::kmod::check_zfs_utils(
        &archinstall_zfs_core::system::cmd::RealRunner,
    )
    .unwrap_or(false);

    let welcome = app.global::<WelcomeState>();
    welcome.set_app_version(env!("CARGO_PKG_VERSION").into());
    welcome.set_net_ok(net);
    welcome.set_uefi_ok(uefi);
    welcome.set_zfs_ok(zfs_mod && zfs_utils);

    if net {
        if !(zfs_mod && zfs_utils) {
            start_zfs_init(app, &config.borrow());
        }
        start_kernel_scan(kernel_scan);
    }
}

fn start_zfs_init(app: &App, config: &GlobalConfig) {
    app.global::<WelcomeState>().set_zfs_installing(true);
    app.global::<WelcomeState>()
        .set_zfs_install_status(SharedString::from("Initializing..."));

    let weak = app.as_weak();
    let kernel = config.primary_kernel().to_string();
    let zfs_mode = config.zfs_module_mode;

    tokio::task::spawn_blocking(move || {
        let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
            Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
        let cancel = tokio_util::sync::CancellationToken::new();

        let w = weak.clone();
        let _ = w.upgrade_in_event_loop(|app| {
            app.global::<WelcomeState>()
                .set_zfs_install_status(SharedString::from("Checking reflector..."));
        });

        archinstall_zfs_core::zfs::kmod::ensure_reflector_finished_and_stopped(&*runner).ok();
        archinstall_zfs_core::zfs::kmod::refresh_mirrors_if_stale(&*runner).ok();

        let w = weak.clone();
        let _ = w.upgrade_in_event_loop(|app| {
            app.global::<WelcomeState>()
                .set_zfs_install_status(SharedString::from("Installing ZFS packages..."));
            app.global::<WelcomeState>().set_zfs_install_pct(30);
        });

        let result = archinstall_zfs_core::zfs::kmod::initialize_zfs(
            &*runner,
            &kernel,
            zfs_mode,
            &cancel,
            archinstall_zfs_core::system::async_download::DownloadConfig::default(),
        );

        let _ = weak.upgrade_in_event_loop(move |app| {
            app.global::<WelcomeState>().set_zfs_installing(false);
            match result {
                Ok(()) => {
                    app.global::<WelcomeState>().set_zfs_ok(true);
                    app.global::<WelcomeState>().set_zfs_install_pct(100);
                }
                Err(e) => {
                    app.global::<WelcomeState>()
                        .set_zfs_install_status(SharedString::from(format!("Failed: {e}")));
                }
            }
        });
    });
}

/// Start kernel compatibility scan in background, store results in shared state.
fn start_kernel_scan(scan_cache: &KernelScan) {
    let cache = scan_cache.clone();
    tokio::task::spawn(async move {
        tracing::info!("scanning kernel compatibility...");
        let results = archinstall_zfs_core::kernel::scanner::scan_all_kernels().await;
        for (info, result) in archinstall_zfs_core::kernel::AVAILABLE_KERNELS
            .iter()
            .zip(&results)
        {
            let pre = if result.precompiled_compatible {
                "OK"
            } else {
                "NO"
            };
            let dkms = if result.dkms_compatible { "OK" } else { "NO" };
            tracing::info!(
                kernel = info.name,
                precompiled = pre,
                dkms = dkms,
                "kernel scan result"
            );
        }
        cache.store(results);
        tracing::info!("kernel compatibility scan complete");
    });
}
