mod qemu;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use qemu::QemuVm;

#[derive(Parser)]
#[command(name = "xtask", about = "Development tasks for archinstall-zfs-rs")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
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
}

#[derive(Parser, Clone)]
struct TestOpts {
    /// Path to JSON config file
    #[arg(long, default_value = "tests/qemu_full_disk.json")]
    config: PathBuf,

    /// Path to installer binary
    #[arg(long, default_value = "target/release/archinstall-zfs-rs")]
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

fn apply_tmpfs(mut opts: TestOpts) -> TestOpts {
    if opts.tmpfs {
        opts.disk = PathBuf::from("/tmp/archzfs-test.qcow2");
        opts.vars = PathBuf::from("/tmp/archzfs-test-vars.fd");
        eprintln!("Using tmpfs: disk={}, vars={}", opts.disk.display(), opts.vars.display());
    }
    opts
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::TestVm { opts } => cmd_test_vm(apply_tmpfs(opts)),
        Commands::TestInstall { opts } => cmd_test_install(apply_tmpfs(opts)),
        Commands::TestBoot { opts } => cmd_test_boot(apply_tmpfs(opts)),
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

/// Detect the init system from the JSON config file.
fn detect_init_system(config: &Path) -> String {
    let content = std::fs::read_to_string(config).unwrap_or_default();
    if content.contains("\"mkinitcpio\"") {
        "mkinitcpio".to_string()
    } else {
        "dracut".to_string()
    }
}

fn cmd_test_install(opts: TestOpts) -> Result<(), String> {
    check_prerequisites(&opts)?;
    let timeout = Duration::from_secs(opts.timeout);

    eprintln!("=== test-install: Fresh disk + install ===");

    // Fresh environment
    eprintln!("[1/4] Creating fresh disk and UEFI vars");
    qemu::create_fresh_disk(&opts.disk);
    qemu::reset_uefi_vars(&opts.vars);

    // Boot ISO
    eprintln!("[2/4] Booting ISO VM on port {}", opts.iso_port);
    let iso = qemu::find_latest_testing_iso();
    let mut vm = QemuVm::boot_iso(&opts.disk, &opts.vars, &iso, opts.iso_port);
    if !vm.wait_for_ssh(timeout) {
        return Err(format!("ISO VM not SSH-accessible within {timeout:?}"));
    }

    // Upload and run installer
    eprintln!("[3/4] Running installer");
    vm.scp_to(&opts.binary, "/root/archinstall-zfs-rs");
    vm.scp_to(&opts.config, "/root/config.json");
    vm.ssh_run("chmod +x /root/archinstall-zfs-rs")
        .map_err(|e| format!("chmod failed: {e}"))?;

    let output = vm
        .ssh_run("/root/archinstall-zfs-rs --config /root/config.json --silent")
        .map_err(|e| format!("installer failed to execute: {e}"))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Installer exited with {}:\nstdout: {stdout}\nstderr: {stderr}",
            output.status
        ));
    }

    // Shut down
    eprintln!("[4/4] Installation succeeded, shutting down");
    vm.shutdown();

    eprintln!("=== test-install: PASSED ===\n");
    Ok(())
}

fn cmd_test_boot(opts: TestOpts) -> Result<(), String> {
    let timeout = Duration::from_secs(opts.timeout);

    if !opts.disk.exists() {
        return Err(format!(
            "Disk {} not found. Run 'cargo xtask test-install' first.",
            opts.disk.display()
        ));
    }

    eprintln!("=== test-boot: Boot from disk + verify ===");

    // Reset UEFI vars (clean slate, uses EFI/BOOT/BOOTX64.EFI fallback)
    eprintln!("[1/3] Resetting UEFI vars for clean boot");
    qemu::reset_uefi_vars(&opts.vars);

    // Boot from disk
    eprintln!("[2/3] Booting installed system on port {}", opts.boot_port);
    let vm =
        QemuVm::boot_disk(&opts.disk, &opts.vars, opts.boot_port).with_password(&opts.password);

    if !vm.wait_for_ssh(timeout) {
        return Err(format!(
            "Installed system not SSH-accessible within {timeout:?}. \
             Boot may have failed (ZFSBootMenu, initramfs, or network issue)."
        ));
    }

    // Verify
    eprintln!("[3/3] Verifying system health");
    let init_system = detect_init_system(&opts.config);
    verify_system(&vm, &init_system)?;

    eprintln!("=== test-boot: PASSED ===\n");
    Ok(())
}

// ── Verification ───────────────────────────────────────

fn verify_system(vm: &QemuVm, init_system: &str) -> Result<(), String> {
    let mut passed = 0;
    let mut checks = Vec::new();

    // Kernel
    let kernel = vm.ssh_stdout("uname -r");
    if kernel.contains("lts") {
        checks.push(format!("  kernel: {kernel}"));
        passed += 1;
    } else {
        checks.push(format!("  kernel: FAIL (expected lts, got '{kernel}')"));
    }

    // ZFS pool
    let pool = vm.ssh_stdout("zpool status testpool 2>&1");
    if pool.contains("ONLINE") {
        checks.push("  zpool: ONLINE".to_string());
        passed += 1;
    } else {
        checks.push(format!("  zpool: FAIL\n{pool}"));
    }

    // sshd
    let sshd = vm.ssh_stdout("systemctl is-active sshd");
    if sshd == "active" {
        checks.push("  sshd: active".to_string());
        passed += 1;
    } else {
        checks.push(format!("  sshd: FAIL ({sshd})"));
    }

    // fstab
    let fstab = vm.ssh_stdout("cat /etc/fstab");
    if fstab.contains("testpool/arch0/root") {
        checks.push("  fstab: OK (has root dataset)".to_string());
        passed += 1;
    } else {
        checks.push(format!("  fstab: FAIL\n{fstab}"));
    }

    // initramfs config
    if init_system == "mkinitcpio" {
        let mkinitcpio =
            vm.ssh_stdout("cat /etc/mkinitcpio.conf 2>/dev/null || echo missing");
        if mkinitcpio.contains("zfs") {
            checks.push("  mkinitcpio: configured (has zfs)".to_string());
            passed += 1;
        } else {
            checks.push(format!("  mkinitcpio: FAIL\n{mkinitcpio}"));
        }
    } else {
        let dracut =
            vm.ssh_stdout("cat /etc/dracut.conf.d/zfs.conf 2>/dev/null || echo missing");
        if dracut.contains("hostonly") {
            checks.push("  dracut: configured".to_string());
            passed += 1;
        } else {
            checks.push(format!("  dracut: FAIL ({dracut})"));
        }
    }

    // zram
    let zram = vm.ssh_stdout("cat /etc/systemd/zram-generator.conf 2>/dev/null || echo missing");
    if zram.contains("zram0") {
        checks.push("  zram: configured".to_string());
        passed += 1;
    } else {
        checks.push(format!("  zram: FAIL ({zram})"));
    }

    // ZFS mounts
    let mounts = vm.ssh_stdout("zfs list -o name,mountpoint 2>&1");
    if mounts.contains("/home") && mounts.contains("/root") {
        checks.push("  ZFS mounts: OK (/home, /root)".to_string());
        passed += 1;
    } else {
        checks.push(format!("  ZFS mounts: FAIL\n{mounts}"));
    }

    // hostid
    let hostid = vm.ssh_stdout("od -A n -t x1 /etc/hostid 2>/dev/null | tr -d ' \\n'");
    if hostid == "0cb1ba00" {
        checks.push("  hostid: 0x00bab10c".to_string());
        passed += 1;
    } else {
        checks.push(format!("  hostid: FAIL (got '{hostid}')"));
    }

    // ZED cache hook (boot-environment aware)
    let zed_hook = vm.ssh_stdout(
        "cat /etc/zfs/zed.d/history_event-zfs-list-cacher.sh 2>/dev/null || echo missing",
    );
    if zed_hook.contains("boot environment aware") {
        checks.push("  ZED hook: installed".to_string());
        passed += 1;
    } else {
        checks.push("  ZED hook: FAIL (custom hook not found)".to_string());
    }

    // bootfs (needed for zbm.timeout auto-boot; users can still select other BEs)
    let bootfs = vm.ssh_stdout("zpool get -H -o value bootfs testpool 2>/dev/null");
    if bootfs == "testpool/arch0/root" {
        checks.push("  bootfs: testpool/arch0/root".to_string());
        passed += 1;
    } else {
        checks.push(format!("  bootfs: FAIL (expected testpool/arch0/root, got '{bootfs}')"));
    }

    // ZBM rootprefix property
    let rootprefix = vm.ssh_stdout(
        "zfs get -H -o value org.zfsbootmenu:rootprefix testpool/arch0/root 2>/dev/null",
    );
    let expected_prefix = if init_system == "dracut" {
        "root=ZFS="
    } else {
        "zfs="
    };
    if rootprefix == expected_prefix {
        checks.push(format!("  rootprefix: {rootprefix}").to_string());
        passed += 1;
    } else {
        checks.push(format!(
            "  rootprefix: FAIL (expected '{expected_prefix}', got '{rootprefix}')"
        ));
    }

    // ZBM locally built (generate-zbm available + config present)
    let zbm_config = vm.ssh_stdout("cat /etc/zfsbootmenu/config.yaml 2>/dev/null || echo missing");
    let zbm_bin = vm.ssh_stdout("which generate-zbm 2>/dev/null || echo missing");
    if zbm_config.contains("ManageImages: true") && !zbm_bin.contains("missing") {
        checks.push("  ZBM local build: OK".to_string());
        passed += 1;
    } else {
        checks.push(format!("  ZBM local build: FAIL (config={}, bin={})",
            if zbm_config.contains("ManageImages") { "ok" } else { "missing" },
            if zbm_bin.contains("missing") { "missing" } else { "ok" }
        ));
    }

    // ZBM pacman hook
    let zbm_hook = vm.ssh_stdout(
        "cat /etc/pacman.d/hooks/95-zfsbootmenu.hook 2>/dev/null || echo missing",
    );
    if zbm_hook.contains("generate-zbm") {
        checks.push("  ZBM pacman hook: installed".to_string());
        passed += 1;
    } else {
        checks.push("  ZBM pacman hook: FAIL".to_string());
    }

    let total = 13;
    for line in &checks {
        eprintln!("{line}");
    }
    eprintln!("  --- {passed}/{total} checks passed ---");

    if passed == total {
        Ok(())
    } else {
        Err(format!("{}/{total} checks failed", total - passed))
    }
}

// ── Helpers ────────────────────────────────────────────

fn check_prerequisites(opts: &TestOpts) -> Result<(), String> {
    if !opts.binary.exists() {
        return Err(format!(
            "Binary not found: {}. Run 'cargo build --release' first.",
            opts.binary.display()
        ));
    }
    if !opts.config.exists() {
        return Err(format!("Config not found: {}", opts.config.display()));
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
