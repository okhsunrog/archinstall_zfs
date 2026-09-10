mod alongside;
mod bench;
mod iso;
mod qemu;
mod verify;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};

use qemu::QemuVm;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ZfsModeOpt {
    Precompiled,
    Dkms,
}

impl ZfsModeOpt {
    fn as_str(self) -> &'static str {
        match self {
            ZfsModeOpt::Precompiled => "precompiled",
            ZfsModeOpt::Dkms => "dkms",
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum EncryptionOpt {
    None,
    Pool,
    Dataset,
}

impl EncryptionOpt {
    fn as_str(self) -> &'static str {
        match self {
            EncryptionOpt::None => "none",
            EncryptionOpt::Pool => "pool",
            EncryptionOpt::Dataset => "dataset",
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum InitOpt {
    Dracut,
    Mkinitcpio,
}

impl InitOpt {
    fn as_str(self) -> &'static str {
        match self {
            InitOpt::Dracut => "dracut",
            InitOpt::Mkinitcpio => "mkinitcpio",
        }
    }
}

#[derive(Parser)]
#[command(name = "xtask", about = "Development tasks for archinstall-zfs-rs")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::enum_variant_names)]
enum Commands {
    /// Run full cycle: fresh disk, install, boot, verify
    TestVm {
        #[command(flatten)]
        opts: TestOpts,
    },

    /// Install only: fresh disk, run installer, verify exit code
    TestInstall {
        #[command(flatten)]
        opts: TestOpts,
    },

    /// Boot only: boot existing disk, SSH in, verify system health
    TestBoot {
        #[command(flatten)]
        opts: TestOpts,
    },

    /// Run installs at multiple concurrency levels and collect metrics JSONL files
    BenchDownloads {
        #[command(flatten)]
        opts: TestOpts,

        /// Comma-separated concurrency values to test (e.g. "1,3,5,10")
        #[arg(long, default_value = "1,3,5,10,20")]
        concurrency: String,

        /// Output directory for collected metrics files
        #[arg(long, default_value = "bench-results")]
        out_dir: PathBuf,

        /// Number of samples per concurrency level (median is reported)
        #[arg(long, default_value_t = 1)]
        samples: usize,
    },

    /// Parse metrics JSONL files from bench-downloads and print a markdown table
    AnalyzeMetrics {
        /// Directory containing metrics files (conc_N.jsonl naming)
        #[arg(long, default_value = "bench-results")]
        dir: PathBuf,
    },

    /// Render archiso profile Jinja2 templates for ISO building
    RenderProfile {
        /// Source profile directory containing .j2 templates
        #[arg(long)]
        profile_dir: PathBuf,

        /// Output directory for rendered profile
        #[arg(long)]
        out_dir: PathBuf,

        /// Kernel package (linux, linux-lts, linux-zen)
        #[arg(long, default_value = "linux-lts")]
        kernel: String,

        /// ZFS module mode (precompiled or dkms)
        #[arg(long, default_value = "precompiled")]
        zfs: String,

        /// Include kernel headers (auto, true, false)
        #[arg(long, default_value = "auto")]
        headers: String,

        /// Fast build mode (minimal packages, erofs)
        #[arg(long)]
        fast: bool,
    },
}

#[derive(Args, Clone)]
struct TestOpts {
    /// Install by shrinking an ext4 fixture in the disposable VM; verify retained files.
    #[arg(long)]
    alongside: bool,

    #[command(flatten)]
    paths: PathOpts,

    #[command(flatten)]
    cache: CacheOpts,

    #[command(flatten)]
    ssh: SshOpts,

    #[command(flatten)]
    overrides: OverrideOpts,
}

/// Inputs of a run and the scratch files it writes.
#[derive(Args, Clone)]
struct PathOpts {
    /// Testing ISO to boot. By default, use the newest *-testing-*.iso.
    /// Custom images must allow passwordless root SSH on the ISO VM.
    #[arg(long)]
    iso: Option<PathBuf>,

    /// Path to base JSON config file (overrides layered on top)
    #[arg(long, default_value = "xtask/configs/default.json")]
    config: PathBuf,

    /// Path to installer binary
    #[arg(long, default_value = "target/release/azfs-tui")]
    binary: PathBuf,

    /// Path to qcow2 disk image (overridden by --tmpfs)
    #[arg(long, default_value = "gen_iso/arch.qcow2")]
    disk: PathBuf,

    /// Path to UEFI vars file (overridden by --tmpfs)
    #[arg(long, default_value = "gen_iso/my_vars.fd")]
    vars: PathBuf,

    /// Place disk image and UEFI vars in /tmp (tmpfs) for faster I/O
    #[arg(long)]
    tmpfs: bool,
}

/// Host-side package and source cache shared into the guest.
#[derive(Args, Clone)]
struct CacheOpts {
    /// Directory on the host holding downloaded packages and sources, reused
    /// across runs. Created if missing; --no-cache turns it off.
    #[arg(long, default_value = "/var/tmp/archinstall-zfs-cache")]
    cache_dir: PathBuf,

    /// Download everything again instead of reusing the host cache.
    #[arg(long)]
    no_cache: bool,
}

/// How the VMs are reached.
#[derive(Args, Clone)]
struct SshOpts {
    /// SSH port for ISO VM
    #[arg(long, default_value_t = 2222)]
    iso_port: u16,

    /// SSH port for booted system VM
    #[arg(long, default_value_t = 2223)]
    boot_port: u16,

    /// SSH password for installed system
    #[arg(long, default_value = "test")]
    password: String,

    /// SSH/boot timeout in seconds
    #[arg(long, default_value_t = 120)]
    timeout: u64,
}

/// Config overrides (layered on --config).
#[derive(Args, Clone)]
struct OverrideOpts {
    /// Override zfs_module_mode
    #[arg(long, value_enum)]
    zfs_mode: Option<ZfsModeOpt>,

    /// Override zfs_encryption_mode
    #[arg(long, value_enum)]
    encryption: Option<EncryptionOpt>,

    /// Passphrase for --encryption pool|dataset (hardcoded test default)
    #[arg(long, default_value = "test12345")]
    encryption_password: String,

    /// Override init_system
    #[arg(long, value_enum)]
    init_system: Option<InitOpt>,

    /// Replace aur_packages (comma-separated)
    #[arg(long, value_delimiter = ',')]
    aur_packages: Option<Vec<String>>,

    /// Set profile_selection.profile (e.g. "kde")
    #[arg(long)]
    profile: Option<String>,
}

fn apply_tmpfs(mut opts: TestOpts) -> TestOpts {
    if opts.paths.tmpfs {
        opts.paths.disk = PathBuf::from("/tmp/archzfs-test.qcow2");
        opts.paths.vars = PathBuf::from("/tmp/archzfs-test-vars.fd");
        eprintln!(
            "Using tmpfs: disk={}, vars={}",
            opts.paths.disk.display(),
            opts.paths.vars.display()
        );
    }
    opts
}

/// Insert `key` when a value is given and remember that the config changed.
fn set_if_some(obj: &mut Map<String, Value>, changed: &mut bool, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        obj.insert(key.into(), value);
        *changed = true;
    }
}

/// Apply CLI overrides to the base config, write the merged JSON to a temp
/// file, and point opts.config at it. Downstream code then reads the merged
/// config transparently (including detect_init_system and scp_to).
fn materialize_config(mut opts: TestOpts) -> Result<TestOpts, String> {
    let content = fs::read_to_string(&opts.paths.config)
        .map_err(|e| format!("read {}: {e}", opts.paths.config.display()))?;
    let mut json: Value = serde_json::from_str(&content)
        .map_err(|e| format!("parse {}: {e}", opts.paths.config.display()))?;
    let obj = json
        .as_object_mut()
        .ok_or_else(|| "config JSON must be an object".to_string())?;
    let overrides = &opts.overrides;

    let mut changed = false;
    set_if_some(
        obj,
        &mut changed,
        "zfs_module_mode",
        overrides.zfs_mode.map(|m| json!(m.as_str())),
    );
    set_if_some(
        obj,
        &mut changed,
        "zfs_encryption_mode",
        overrides.encryption.map(|e| json!(e.as_str())),
    );
    set_if_some(
        obj,
        &mut changed,
        "zfs_encryption_password",
        overrides
            .encryption
            .filter(|e| *e != EncryptionOpt::None)
            .map(|_| json!(overrides.encryption_password)),
    );
    set_if_some(
        obj,
        &mut changed,
        "init_system",
        overrides.init_system.map(|i| json!(i.as_str())),
    );
    set_if_some(
        obj,
        &mut changed,
        "aur_packages",
        overrides.aur_packages.as_ref().map(|pkgs| json!(pkgs)),
    );
    set_if_some(
        obj,
        &mut changed,
        "profile_selection",
        overrides.profile.as_ref().map(|p| {
            json!({
                "profile": p,
                "optional_packages": [],
                "display_manager_override": null,
            })
        }),
    );

    if changed {
        let tmp = std::env::temp_dir().join("archzfs-xtask-merged.json");
        fs::write(&tmp, serde_json::to_string_pretty(&json).unwrap())
            .map_err(|e| format!("write merged config: {e}"))?;
        eprintln!("Merged config written to {}", tmp.display());
        opts.paths.config = tmp;
    }

    Ok(opts)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::TestVm { opts } => materialize_config(apply_tmpfs(opts)).and_then(cmd_test_vm),
        Commands::TestInstall { opts } => {
            materialize_config(apply_tmpfs(opts)).and_then(cmd_test_install)
        }
        Commands::TestBoot { opts } => {
            materialize_config(apply_tmpfs(opts)).and_then(cmd_test_boot)
        }
        Commands::BenchDownloads {
            opts,
            concurrency,
            out_dir,
            samples,
        } => materialize_config(apply_tmpfs(opts))
            .and_then(|o| bench::cmd_bench_downloads(o, &concurrency, &out_dir, samples)),
        Commands::AnalyzeMetrics { dir } => bench::cmd_analyze_metrics(&dir),
        Commands::RenderProfile {
            profile_dir,
            out_dir,
            kernel,
            zfs,
            headers,
            fast,
        } => iso::render_profile(&profile_dir, &out_dir, &kernel, &zfs, &headers, fast),
    };
    match result {
        Ok(()) => {
            eprintln!("PASS");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("FAIL: {e}");
            ExitCode::FAILURE
        }
    }
}

// ── Commands ───────────────────────────────────────────

fn cmd_test_vm(opts: TestOpts) -> Result<(), String> {
    cmd_test_install(opts.clone())?;
    cmd_test_boot(opts)?;
    Ok(())
}

/// Prepare the directory shared into the guest, or `None` when caching is off.
///
/// Holds two things: the packages the installer downloads, which is the
/// gigabyte that would otherwise be fetched again on every run, and the
/// sources makepkg needs for the AUR builds.
fn prepare_cache(opts: &CacheOpts) -> Result<Option<PathBuf>, String> {
    if opts.no_cache {
        return Ok(None);
    }

    let dir = &opts.cache_dir;
    for sub in ["pkg", "src"] {
        fs::create_dir_all(dir.join(sub))
            .map_err(|e| format!("cannot create cache {}: {e}", dir.join(sub).display()))?;
    }
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve cache {}: {e}", dir.display()))?;

    seed_aur_sources(&dir.join("src"));
    Ok(Some(dir))
}

/// Download the ZFSBootMenu tarball so makepkg does not have to.
///
/// Best effort: makepkg fetches it itself if this fails, and verifies the
/// checksum either way, so a stale or missing file costs a download rather
/// than a wrong build. Worth doing because this one download comes from
/// GitHub's archive service, which is what fails first when GitHub is
/// unwell — twice in one afternoon, in the run that prompted this.
fn seed_aur_sources(dir: &Path) {
    let pkgbuild = match Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "30",
            "https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h=zfsbootmenu",
        ])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => {
            eprintln!("  Could not read the zfsbootmenu PKGBUILD; makepkg will download its own");
            return;
        }
    };

    let Some(version) = pkgbuild.lines().find_map(|l| l.strip_prefix("pkgver=")) else {
        eprintln!("  No pkgver in the zfsbootmenu PKGBUILD; skipping the source cache");
        return;
    };
    let version = version.trim();

    // The name makepkg looks for, from the PKGBUILD's renamed source entry.
    let filename = format!("zfsbootmenu-v{version}.tar.gz");
    let target = dir.join(&filename);
    if target.exists() {
        return;
    }

    let url = format!("https://github.com/zbm-dev/zfsbootmenu/archive/v{version}.tar.gz");
    let partial = dir.join(format!("{filename}.part"));
    let fetched = Command::new("curl")
        .args(["-fsSL", "--max-time", "180", "-o"])
        .arg(&partial)
        .arg(&url)
        .status();

    match fetched {
        Ok(status) if status.success() => {
            // Renamed only once complete, so an interrupted download is not
            // mistaken for a cached one on the next run.
            if fs::rename(&partial, &target).is_ok() {
                eprintln!("  Cached {filename}");
            }
        }
        _ => {
            let _ = fs::remove_file(&partial);
            eprintln!("  Could not cache {filename}; makepkg will download it");
        }
    }
}

fn cmd_test_install(opts: TestOpts) -> Result<(), String> {
    check_prerequisites(&opts)?;
    let paths = &opts.paths;
    let timeout = Duration::from_secs(opts.ssh.timeout);
    let iso = match &paths.iso {
        Some(path) if path.is_file() => path.clone(),
        Some(path) => return Err(format!("ISO not found: {}", path.display())),
        None => qemu::find_latest_testing_iso()?,
    };

    eprintln!("=== test-install: Fresh disk + install ===");
    eprintln!("Using testing ISO: {}", iso.display());

    // Fresh environment
    eprintln!("[1/4] Creating fresh disk and UEFI vars");
    qemu::create_fresh_disk(&paths.disk);
    if opts.alongside {
        let status = Command::new("qemu-img")
            .args(["resize"])
            .arg(&paths.disk)
            .arg("80G")
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Cannot enlarge alongside fixture disk".into());
        }
    }
    qemu::reset_uefi_vars(&paths.vars);

    // Boot ISO
    eprintln!("[2/4] Booting ISO VM on port {}", opts.ssh.iso_port);
    let cache = prepare_cache(&opts.cache)?;
    let mut vm = QemuVm::boot_iso(
        &paths.disk,
        &paths.vars,
        &iso,
        opts.ssh.iso_port,
        cache.as_deref(),
    );
    if !vm.wait_for_ssh(timeout) {
        return Err(format!("ISO VM not SSH-accessible within {timeout:?}"));
    }

    // Upload and run installer
    eprintln!("[3/4] Running installer");
    vm.scp_to(&paths.binary, "/root/archinstall-zfs-rs");
    vm.scp_to(&paths.config, "/root/config.json");
    if opts.alongside {
        alongside::prepare(&vm, &paths.config)?;
    }
    vm.ssh_run("chmod +x /root/archinstall-zfs-rs")
        .map_err(|e| format!("chmod failed: {e}"))?;

    let installer_command = match &cache {
        Some(_) => {
            let tag = qemu::CACHE_MOUNT_TAG;
            let dir = qemu::CACHE_GUEST_DIR;
            eprintln!(
                "  Reusing the host cache at {}",
                opts.cache.cache_dir.display()
            );
            format!(
                "mkdir -p {dir} && \
                 mount -t 9p -o trans=virtio,version=9p2000.L {tag} {dir} && \
                 mkdir -p {dir}/pkg {dir}/src && \
                 ARCHINSTALL_ZFS_PKG_CACHE={dir}/pkg ARCHINSTALL_ZFS_SRCDEST={dir}/src \
                 /root/archinstall-zfs-rs --config /root/config.json --silent"
            )
        }
        None => "/root/archinstall-zfs-rs --config /root/config.json --silent".to_string(),
    };
    let output = vm
        .ssh_run(&installer_command)
        .map_err(|e| format!("installer failed to execute: {e}"))?;

    if opts.alongside && output.status.success() {
        alongside::verify(&vm)?;
    }

    // Pull installer logs from VM before shutdown (regardless of success/failure)
    let log_dest = PathBuf::from("test-install.log");
    if vm.scp_from("/tmp/archinstall-zfs.log", &log_dest) {
        eprintln!("  Logs saved to {}", log_dest.display());
    } else {
        eprintln!("  Warning: could not retrieve installer logs");
    }

    // Pull metrics JSONL (may not exist if install failed early)
    let metrics_dest = PathBuf::from("/tmp/archinstall-metrics.jsonl");
    if vm.scp_from("/tmp/archinstall-metrics.jsonl", &metrics_dest) {
        eprintln!("  Metrics saved to {}", metrics_dest.display());
    } else {
        eprintln!("  Warning: could not retrieve metrics file");
    }

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        vm.shutdown();
        return Err(format!(
            "Installer exited with {}:\nstdout: {stdout}\nstderr: {stderr}\nLogs: {}",
            output.status,
            log_dest.display()
        ));
    }

    // Shut down
    eprintln!("[4/4] Installation succeeded, shutting down");
    vm.shutdown();

    eprintln!("=== test-install: PASSED ===\n");
    Ok(())
}

fn cmd_test_boot(opts: TestOpts) -> Result<(), String> {
    let paths = &opts.paths;
    let timeout = Duration::from_secs(opts.ssh.timeout);

    if !paths.disk.exists() {
        return Err(format!(
            "Disk {} not found. Run 'cargo xtask test-install' first.",
            paths.disk.display()
        ));
    }

    eprintln!("=== test-boot: Boot from disk + verify ===");

    // Reset UEFI vars (clean slate, uses EFI/BOOT/BOOTX64.EFI fallback)
    eprintln!("[1/3] Resetting UEFI vars for clean boot");
    qemu::reset_uefi_vars(&paths.vars);

    // Boot from disk
    eprintln!(
        "[2/3] Booting installed system on port {}",
        opts.ssh.boot_port
    );
    let vm = QemuVm::boot_disk(&paths.disk, &paths.vars, opts.ssh.boot_port)
        .with_password(&opts.ssh.password);

    if !vm.wait_for_ssh(timeout) {
        return Err(format!(
            "Installed system not SSH-accessible within {timeout:?}. \
             Boot may have failed (ZFSBootMenu, initramfs, or network issue)."
        ));
    }

    // Verify
    eprintln!("[3/3] Verifying system health");
    verify::verify_system(&vm, &paths.config)?;

    eprintln!("=== test-boot: PASSED ===\n");
    Ok(())
}

// ── Helpers ────────────────────────────────────────────

fn check_prerequisites(opts: &TestOpts) -> Result<(), String> {
    if !opts.paths.binary.exists() {
        return Err(format!(
            "Binary not found: {}. Run 'cargo build --release' first.",
            opts.paths.binary.display()
        ));
    }
    if !opts.paths.config.exists() {
        return Err(format!("Config not found: {}", opts.paths.config.display()));
    }
    // Check KVM
    if !std::path::Path::new("/dev/kvm").exists() {
        return Err("KVM not available (/dev/kvm not found)".to_string());
    }
    // Check sshpass
    let sshpass = std::process::Command::new("which")
        .arg("sshpass")
        .stdout(std::process::Stdio::null())
        .status();
    if !sshpass.is_ok_and(|s| s.success()) {
        return Err("sshpass not found. Install it: pacman -S sshpass".to_string());
    }
    Ok(())
}
