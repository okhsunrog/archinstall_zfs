mod iso;
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

#[derive(Parser, Clone)]
struct TestOpts {
    /// Path to JSON config file
    #[arg(long, default_value = "xtask/configs/qemu_full_disk.json")]
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
        eprintln!(
            "Using tmpfs: disk={}, vars={}",
            opts.disk.display(),
            opts.vars.display()
        );
    }
    opts
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::TestVm { opts } => cmd_test_vm(apply_tmpfs(opts)),
        Commands::TestInstall { opts } => cmd_test_install(apply_tmpfs(opts)),
        Commands::TestBoot { opts } => cmd_test_boot(apply_tmpfs(opts)),
        Commands::BenchDownloads {
            opts,
            concurrency,
            out_dir,
            samples,
        } => cmd_bench_downloads(apply_tmpfs(opts), &concurrency, &out_dir, samples),
        Commands::AnalyzeMetrics { dir } => cmd_analyze_metrics(&dir),
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
        let mkinitcpio = vm.ssh_stdout("cat /etc/mkinitcpio.conf 2>/dev/null || echo missing");
        if mkinitcpio.contains("zfs") {
            checks.push("  mkinitcpio: configured (has zfs)".to_string());
            passed += 1;
        } else {
            checks.push(format!("  mkinitcpio: FAIL\n{mkinitcpio}"));
        }
    } else {
        let dracut = vm.ssh_stdout("cat /etc/dracut.conf.d/zfs.conf 2>/dev/null || echo missing");
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
        checks.push(format!(
            "  bootfs: FAIL (expected testpool/arch0/root, got '{bootfs}')"
        ));
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
        checks.push(format!(
            "  ZBM local build: FAIL (config={}, bin={})",
            if zbm_config.contains("ManageImages") {
                "ok"
            } else {
                "missing"
            },
            if zbm_bin.contains("missing") {
                "missing"
            } else {
                "ok"
            }
        ));
    }

    // ZBM pacman hook
    let zbm_hook =
        vm.ssh_stdout("cat /etc/pacman.d/hooks/95-zfsbootmenu.hook 2>/dev/null || echo missing");
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

// ── bench-downloads ────────────────────────────────────

/// Patch `parallel_downloads` in a JSON config and write to a temp file.
fn patch_config_concurrency(
    config_path: &Path,
    concurrency: usize,
    dest: &Path,
) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path).map_err(|e| format!("read config: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("parse config: {e}"))?;
    v["parallel_downloads"] = serde_json::json!(concurrency);
    let patched = serde_json::to_string_pretty(&v).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(dest, patched).map_err(|e| format!("write patched config: {e}"))?;
    Ok(())
}

fn cmd_bench_downloads(
    opts: TestOpts,
    concurrency_spec: &str,
    out_dir: &Path,
    samples: usize,
) -> Result<(), String> {
    check_prerequisites(&opts)?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create out_dir: {e}"))?;

    let samples = samples.max(1);

    // Parse comma-separated concurrency values
    let concurrencies: Vec<usize> = concurrency_spec
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<usize>()
                .map_err(|_| format!("invalid concurrency value: '{s}'"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    eprintln!(
        "=== bench-downloads: {} concurrency levels × {} sample(s): {:?} ===",
        concurrencies.len(),
        samples,
        concurrencies
    );

    for &conc in &concurrencies {
        eprintln!("\n--- concurrency={conc} ({samples} sample(s)) ---");

        // Write patched config to a temp file (shared across all samples)
        let patched_config = out_dir.join(format!("config_conc{conc}.json"));
        patch_config_concurrency(&opts.config, conc, &patched_config)?;

        // Track phase4 wall time for each sample to pick the median
        let mut sample_files: Vec<PathBuf> = Vec::new();
        let mut phase4_times: Vec<u64> = Vec::new();

        for s in 1..=samples {
            eprintln!("  sample {s}/{samples}");

            let run_opts = TestOpts {
                config: patched_config.clone(),
                ..opts.clone()
            };
            cmd_test_install(run_opts)?;

            let local_metrics = PathBuf::from("/tmp/archinstall-metrics.jsonl");
            let sample_dest = out_dir.join(format!("conc_{conc}_s{s}.jsonl"));
            if local_metrics.exists() {
                std::fs::copy(&local_metrics, &sample_dest)
                    .map_err(|e| format!("copy metrics sample: {e}"))?;
                eprintln!("    saved {}", sample_dest.display());

                // Extract phase 4 wall time from this sample
                let content = std::fs::read_to_string(&sample_dest).unwrap_or_default();
                let ph4_time = phase4_wall_ms_from_jsonl(&content);
                eprintln!("    phase4 wall: {:.1}s", ph4_time as f64 / 1000.0);
                phase4_times.push(ph4_time);
                sample_files.push(sample_dest);
            } else {
                eprintln!("    Warning: metrics file not found");
                phase4_times.push(u64::MAX);
                sample_files.push(PathBuf::new());
            }
        }

        // Pick the median sample (by phase 4 wall time) as the canonical result
        let median_idx = median_index(&phase4_times);
        let canonical = out_dir.join(format!("conc_{conc}.jsonl"));
        if sample_files[median_idx].exists() {
            std::fs::copy(&sample_files[median_idx], &canonical)
                .map_err(|e| format!("copy median sample: {e}"))?;
            eprintln!(
                "  Median sample: s{} ({:.1}s) → {}",
                median_idx + 1,
                phase4_times[median_idx] as f64 / 1000.0,
                canonical.display()
            );
        }
        if samples > 1 {
            let mut sorted = phase4_times.clone();
            sorted.retain(|&t| t != u64::MAX);
            sorted.sort_unstable();
            let min_s = sorted.first().copied().unwrap_or(0) as f64 / 1000.0;
            let max_s = sorted.last().copied().unwrap_or(0) as f64 / 1000.0;
            eprintln!("  Phase4 range: {min_s:.1}s – {max_s:.1}s");
        }
    }

    eprintln!("\n=== bench-downloads: complete ===");
    eprintln!(
        "Run 'cargo xtask analyze-metrics --dir {}' to see results.",
        out_dir.display()
    );
    Ok(())
}

/// Extract the phase-4 wall-clock time (ms) from a JSONL string.
/// Returns phase_5.ts_ms - phase_4.ts_ms, or 0 if either is missing.
fn phase4_wall_ms_from_jsonl(content: &str) -> u64 {
    let mut ts4 = None;
    let mut ts5 = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("event").and_then(|e| e.as_str()) == Some("phase_start") {
            let num = v.get("num").and_then(|n| n.as_u64()).unwrap_or(0);
            let ts = v.get("ts_ms").and_then(|t| t.as_u64()).unwrap_or(0);
            match num {
                4 => ts4 = Some(ts),
                5 => ts5 = Some(ts),
                _ => {}
            }
        }
    }
    match (ts4, ts5) {
        (Some(t4), Some(t5)) if t5 > t4 => t5 - t4,
        _ => 0,
    }
}

/// Return the index of the median value in a slice.
fn median_index(values: &[u64]) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut indexed: Vec<(usize, u64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by_key(|&(_, v)| v);
    indexed[indexed.len() / 2].0
}

// ── analyze-metrics ────────────────────────────────────

#[derive(Default)]
struct RunStats {
    concurrency: usize,
    pkg_count: u64,
    total_bytes: u64,
    total_dl_ms: u64,
    max_speed_bps: u64,
    avg_speed_bps: u64,
    batch_install_ms: u64,
    phases: Vec<(u32, String, u64)>, // (num, name, ts_ms)
}

fn cmd_analyze_metrics(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("directory not found: {}", dir.display()));
    }

    // Collect canonical conc_N.jsonl files, sorted by N
    let mut entries: Vec<(usize, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| format!("read dir: {e}"))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".jsonl")?;
            // Only canonical files (conc_N, not conc_N_sM)
            let n = stem.strip_prefix("conc_")?;
            if n.contains('_') {
                return None;
            }
            let n = n.parse::<usize>().ok()?;
            Some((n, e.path()))
        })
        .collect();
    entries.sort_by_key(|(n, _)| *n);

    if entries.is_empty() {
        return Err(format!("no conc_N.jsonl files found in {}", dir.display()));
    }

    // Also check for per-sample files to compute scatter
    let mut sample_map: std::collections::HashMap<usize, Vec<u64>> = Default::default();
    for e in std::fs::read_dir(dir)
        .map_err(|e| format!("read dir: {e}"))?
        .filter_map(|e| e.ok())
    {
        let name = e.file_name().into_string().unwrap_or_default();
        let stem = name.strip_suffix(".jsonl").unwrap_or("");
        // matches conc_N_sM
        if let Some(rest) = stem.strip_prefix("conc_") {
            let parts: Vec<&str> = rest.splitn(2, '_').collect();
            if parts.len() == 2
                && let (Ok(n), Some(_s)) = (parts[0].parse::<usize>(), parts[1].strip_prefix('s'))
                && let Ok(content) = std::fs::read_to_string(e.path())
            {
                let t = phase4_wall_ms_from_jsonl(&content);
                sample_map.entry(n).or_default().push(t);
            }
        }
    }

    let mut rows: Vec<RunStats> = Vec::new();

    for (conc, path) in &entries {
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;

        let mut stats = RunStats {
            concurrency: *conc,
            ..Default::default()
        };

        let mut phase_ts: Vec<(u32, String, u64)> = Vec::new();
        let mut dl_speeds: Vec<u64> = Vec::new();

        for line in content.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let event = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
            let ts_ms = v.get("ts_ms").and_then(|t| t.as_u64()).unwrap_or(0);

            match event {
                "pkg_download" => {
                    let bytes = v.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
                    let speed_bps = v.get("speed_bps").and_then(|s| s.as_u64()).unwrap_or(0);
                    stats.pkg_count += 1;
                    stats.total_bytes += bytes;
                    dl_speeds.push(speed_bps);
                    if speed_bps > stats.max_speed_bps {
                        stats.max_speed_bps = speed_bps;
                    }
                }
                "batch_install" => {
                    stats.batch_install_ms +=
                        v.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0);
                }
                "phase_start" => {
                    let num = v.get("num").and_then(|n| n.as_u64()).unwrap_or(0) as u32;
                    let name = v
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    phase_ts.push((num, name, ts_ms));
                }
                _ => {}
            }
        }

        if !dl_speeds.is_empty() {
            stats.avg_speed_bps = dl_speeds.iter().sum::<u64>() / dl_speeds.len() as u64;
        }

        // Use phase4→phase5 timestamps for wall-clock download time
        if let Some((_, _, ts4)) = phase_ts.iter().find(|(n, _, _)| *n == 4)
            && let Some((_, _, ts5)) = phase_ts.iter().find(|(n, _, _)| *n == 5)
        {
            stats.total_dl_ms = ts5 - ts4;
        }
        // total_bytes must be measured from pkg_download events (set above, but reset by wall logic)
        // re-sum bytes properly
        stats.total_bytes = 0;
        for line in content.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("event").and_then(|e| e.as_str()) == Some("pkg_download") {
                stats.total_bytes += v.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
            }
        }

        stats.phases = phase_ts;
        rows.push(stats);
    }

    // Print markdown table
    println!("## Download Benchmark Results\n");

    // Check if we have multi-sample scatter
    let has_scatter = sample_map.values().any(|v| v.len() > 1);

    if has_scatter {
        println!(
            "| conc | pkgs | total_MB | dl_med_s | dl_min_s | dl_max_s | avg_MBps | max_MBps | install_s |"
        );
        println!(
            "|-----:|-----:|---------:|---------:|---------:|---------:|---------:|---------:|----------:|"
        );
    } else {
        println!("| conc | pkgs | total_MB | dl_wall_s | avg_MBps | max_MBps | install_s |");
        println!("|-----:|-----:|---------:|----------:|---------:|---------:|----------:|");
    }

    for r in &rows {
        let total_mb = r.total_bytes as f64 / 1_048_576.0;
        let dl_wall_s = r.total_dl_ms as f64 / 1000.0;
        let avg_mbps = r.avg_speed_bps as f64 / 1_048_576.0;
        let max_mbps = r.max_speed_bps as f64 / 1_048_576.0;
        let install_s = r.batch_install_ms as f64 / 1000.0;

        if has_scatter {
            let mut samples = sample_map.get(&r.concurrency).cloned().unwrap_or_default();
            samples.sort_unstable();
            let min_s = samples.first().copied().unwrap_or(0) as f64 / 1000.0;
            let max_s = samples.last().copied().unwrap_or(0) as f64 / 1000.0;
            println!(
                "| {:>4} | {:>4} | {:>8.1} | {:>8.1} | {:>8.1} | {:>8.1} | {:>8.2} | {:>8.2} | {:>9.1} |",
                r.concurrency,
                r.pkg_count,
                total_mb,
                dl_wall_s,
                min_s,
                max_s,
                avg_mbps,
                max_mbps,
                install_s
            );
        } else {
            println!(
                "| {:>4} | {:>4} | {:>8.1} | {:>9.1} | {:>8.2} | {:>8.2} | {:>9.1} |",
                r.concurrency, r.pkg_count, total_mb, dl_wall_s, avg_mbps, max_mbps, install_s
            );
        }
    }

    // Per-phase breakdown for each run
    println!("\n## Phase Timings (seconds)\n");
    let all_phases: Vec<u32> = {
        let mut nums: Vec<u32> = rows
            .iter()
            .flat_map(|r| r.phases.iter().map(|(n, _, _)| *n))
            .collect();
        nums.sort_unstable();
        nums.dedup();
        nums
    };

    if !all_phases.is_empty() {
        let header_phases: String = all_phases
            .iter()
            .map(|n| format!("| Ph{n:>2} "))
            .collect::<Vec<_>>()
            .join("");
        println!("| conc {header_phases}|");
        let sep: String = all_phases
            .iter()
            .map(|_| "|-----:")
            .collect::<Vec<_>>()
            .join("");
        println!("|-----:{sep}|");

        for r in &rows {
            let phase_map: std::collections::HashMap<u32, u64> = r
                .phases
                .windows(2)
                .map(|w| {
                    let (n1, _, ts1) = &w[0];
                    let (_, _, ts2) = &w[1];
                    (*n1, ts2 - ts1)
                })
                .collect();

            let cells: String = all_phases
                .iter()
                .map(|n| {
                    if let Some(&dur) = phase_map.get(n) {
                        format!("| {:>4.1}s", dur as f64 / 1000.0)
                    } else {
                        "|    - ".to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("| {:>4} {cells} |", r.concurrency);
        }
    }

    Ok(())
}
