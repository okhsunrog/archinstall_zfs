// QEMU integration test: install Arch Linux with ZFS and verify boot.
//
// Run with: cargo test --test qemu_integration -- --ignored --test-threads=1 --nocapture
// Or: just test-vm
//
// Prerequisites:
// - Testing ISO built (just build-test)
// - KVM available
// - sshpass installed

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const SSH_PORT_ISO: u16 = 2222;
const SSH_PORT_BOOT: u16 = 2223;
const SSH_TIMEOUT: Duration = Duration::from_secs(10);
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

struct QemuVm {
    pid: Option<u32>,
    port: u16,
    password: Option<String>,
}

impl QemuVm {
    fn find_ovmf_code() -> PathBuf {
        for dir in [
            "/usr/share/edk2/x64",
            "/usr/share/edk2-ovmf/x64",
            "/usr/share/OVMF",
        ] {
            let path = PathBuf::from(dir).join("OVMF_CODE.4m.fd");
            if path.exists() {
                return path;
            }
        }
        panic!("OVMF_CODE.4m.fd not found. Install edk2-ovmf.");
    }

    fn find_latest_testing_iso() -> PathBuf {
        let out_dir = PathBuf::from("gen_iso/out");
        let mut isos: Vec<PathBuf> = std::fs::read_dir(&out_dir)
            .expect("gen_iso/out not found. Run 'just build-test' first.")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "iso") && p.to_string_lossy().contains("testing")
            })
            .collect();
        isos.sort();
        isos.last()
            .cloned()
            .expect("No testing ISO found in gen_iso/out/. Run 'just build-test' first.")
    }

    fn boot_iso(disk: &Path, uefi_vars: &Path, port: u16) -> Self {
        let ovmf = Self::find_ovmf_code();
        let iso = Self::find_latest_testing_iso();

        let child = Command::new("qemu-system-x86_64")
            .args([
                "-enable-kvm",
                "-cpu",
                "host",
                "-m",
                "4096",
                "-smp",
                "2",
                "-boot",
                "order=d",
                "-display",
                "none",
                "-net",
                "nic",
                "-net",
                &format!("user,hostfwd=tcp::{port}-:22"),
                "-machine",
                "type=q35,smm=on,accel=kvm,usb=on",
                "-global",
                "ICH9-LPC.disable_s3=1",
                "-no-reboot",
                "-drive",
                &format!(
                    "if=pflash,format=raw,unit=0,file={},read-only=on",
                    ovmf.display()
                ),
                "-drive",
                &format!("if=pflash,format=raw,unit=1,file={}", uefi_vars.display()),
                "-cdrom",
                iso.to_str().unwrap(),
                "-drive",
                &format!("file={},format=qcow2,if=none,id=disk0", disk.display()),
                "-device",
                "virtio-blk-pci,drive=disk0,serial=archzfs-test-disk",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start QEMU");

        Self {
            pid: Some(child.id()),
            port,
            password: None,
        }
    }

    fn boot_disk(disk: &Path, uefi_vars: &Path, port: u16) -> Self {
        let ovmf = Self::find_ovmf_code();

        let child = Command::new("qemu-system-x86_64")
            .args([
                "-enable-kvm",
                "-cpu",
                "host",
                "-m",
                "4096",
                "-smp",
                "2",
                "-boot",
                "order=c",
                "-display",
                "none",
                "-net",
                "nic",
                "-net",
                &format!("user,hostfwd=tcp::{port}-:22"),
                "-machine",
                "type=q35,smm=on,accel=kvm,usb=on",
                "-global",
                "ICH9-LPC.disable_s3=1",
                "-drive",
                &format!(
                    "if=pflash,format=raw,unit=0,file={},read-only=on",
                    ovmf.display()
                ),
                "-drive",
                &format!("if=pflash,format=raw,unit=1,file={}", uefi_vars.display()),
                "-drive",
                &format!("file={},format=qcow2,if=none,id=disk0", disk.display()),
                "-device",
                "virtio-blk-pci,drive=disk0,serial=archzfs-test-disk",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start QEMU");

        Self {
            pid: Some(child.id()),
            port,
            password: None,
        }
    }

    fn with_password(mut self, password: &str) -> Self {
        self.password = Some(password.to_string());
        self
    }

    fn wait_for_ssh(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.ssh_run("echo ready").is_ok_and(|o| o.status.success()) {
                return true;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        false
    }

    fn ssh_run(&self, cmd: &str) -> std::io::Result<Output> {
        let port_str = self.port.to_string();

        if let Some(ref pw) = self.password {
            Command::new("sshpass")
                .args([
                    "-p",
                    pw,
                    "ssh",
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "UserKnownHostsFile=/dev/null",
                    "-o",
                    &format!("ConnectTimeout={}", SSH_TIMEOUT.as_secs()),
                    "-p",
                    &port_str,
                    "root@localhost",
                    cmd,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        } else {
            Command::new("ssh")
                .args([
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "UserKnownHostsFile=/dev/null",
                    "-o",
                    &format!("ConnectTimeout={}", SSH_TIMEOUT.as_secs()),
                    "-p",
                    &port_str,
                    "root@localhost",
                    cmd,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        }
    }

    fn ssh_stdout(&self, cmd: &str) -> String {
        let output = self.ssh_run(cmd).expect("SSH command failed");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn scp_to(&self, local: &Path, remote: &str) {
        let port_str = self.port.to_string();
        let status = Command::new("scp")
            .args([
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-P",
                &port_str,
                local.to_str().unwrap(),
                &format!("root@localhost:{remote}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("scp failed");
        assert!(status.success(), "scp failed");
    }

    fn kill(&mut self) {
        if let Some(pid) = self.pid.take() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            std::thread::sleep(Duration::from_secs(2));
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
    }
}

impl Drop for QemuVm {
    fn drop(&mut self) {
        self.kill();
    }
}

fn fresh_qemu_env() -> (PathBuf, PathBuf) {
    let disk = PathBuf::from("gen_iso/arch.qcow2");
    let vars = PathBuf::from("gen_iso/my_vars.fd");

    // Fresh disk
    let _ = std::fs::remove_file(&disk);
    let status = Command::new("qemu-img")
        .args(["create", "-f", "qcow2", disk.to_str().unwrap(), "20G"])
        .stdout(Stdio::null())
        .status()
        .expect("qemu-img failed");
    assert!(status.success());

    // Fresh UEFI vars
    let ovmf_vars = [
        "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        "/usr/share/edk2-ovmf/x64/OVMF_VARS.4m.fd",
        "/usr/share/OVMF/OVMF_VARS.4m.fd",
    ];
    let src = ovmf_vars
        .iter()
        .find(|p| Path::new(p).exists())
        .expect("OVMF_VARS.fd not found");
    std::fs::copy(src, &vars).expect("failed to copy UEFI vars");

    (disk, vars)
}

// ─── Tests ─────────────────────────────────────────────

#[test]
#[ignore] // Run with: cargo test --test qemu_integration -- --ignored
fn test_full_disk_install_and_boot() {
    let binary = PathBuf::from("target/release/archinstall-zfs-rs");
    assert!(
        binary.exists(),
        "Binary not found. Run 'cargo build --release' first."
    );

    let config = PathBuf::from("tests/qemu_full_disk.json");
    assert!(config.exists(), "Test config not found");

    // Phase 1: Fresh environment
    eprintln!("--- Phase 1: Creating fresh QEMU environment ---");
    let (disk, vars) = fresh_qemu_env();

    // Phase 2: Boot ISO and install
    eprintln!("--- Phase 2: Booting ISO and running installer ---");
    let mut iso_vm = QemuVm::boot_iso(&disk, &vars, SSH_PORT_ISO);
    assert!(
        iso_vm.wait_for_ssh(BOOT_TIMEOUT),
        "ISO VM did not become SSH-accessible"
    );

    iso_vm.scp_to(&binary, "/root/archinstall-zfs-rs");
    iso_vm.scp_to(&config, "/root/config.json");
    iso_vm.ssh_run("chmod +x /root/archinstall-zfs-rs").unwrap();

    let install_output = iso_vm
        .ssh_run("/root/archinstall-zfs-rs --config /root/config.json --silent")
        .expect("Failed to run installer");
    let install_stdout = String::from_utf8_lossy(&install_output.stdout);
    let install_stderr = String::from_utf8_lossy(&install_output.stderr);

    assert!(
        install_output.status.success(),
        "Installation failed:\nstdout: {install_stdout}\nstderr: {install_stderr}"
    );
    eprintln!("--- Installation completed successfully ---");

    // Shut down ISO VM
    let _ = iso_vm.ssh_run("poweroff");
    std::thread::sleep(Duration::from_secs(5));
    iso_vm.kill();

    // Phase 3: Reset UEFI vars for clean boot (uses fallback bootloader)
    eprintln!("--- Phase 3: Booting installed system ---");
    let ovmf_vars_src = [
        "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        "/usr/share/edk2-ovmf/x64/OVMF_VARS.4m.fd",
    ];
    let src = ovmf_vars_src
        .iter()
        .find(|p| Path::new(p).exists())
        .unwrap();
    std::fs::copy(src, &vars).unwrap();

    let booted_vm = QemuVm::boot_disk(&disk, &vars, SSH_PORT_BOOT).with_password("test");

    assert!(
        booted_vm.wait_for_ssh(BOOT_TIMEOUT),
        "Installed system did not become SSH-accessible within {:?}",
        BOOT_TIMEOUT
    );
    eprintln!("--- System booted and SSH accessible ---");

    // Phase 4: Verify installed system
    eprintln!("--- Phase 4: Verifying installed system ---");

    let kernel = booted_vm.ssh_stdout("uname -r");
    assert!(kernel.contains("lts"), "Expected LTS kernel, got: {kernel}");
    eprintln!("  kernel: {kernel}");

    let pool_status = booted_vm.ssh_stdout("zpool status testpool");
    assert!(
        pool_status.contains("ONLINE"),
        "Pool not ONLINE:\n{pool_status}"
    );
    eprintln!("  pool: ONLINE");

    let sshd = booted_vm.ssh_stdout("systemctl is-active sshd");
    assert_eq!(sshd, "active", "sshd not active: {sshd}");
    eprintln!("  sshd: active");

    let fstab = booted_vm.ssh_stdout("cat /etc/fstab");
    assert!(
        fstab.contains("testpool/arch0/root"),
        "fstab missing root dataset:\n{fstab}"
    );
    eprintln!("  fstab: OK");

    let dracut_conf = booted_vm.ssh_stdout("cat /etc/dracut.conf.d/zfs.conf");
    assert!(
        dracut_conf.contains("hostonly"),
        "dracut config missing:\n{dracut_conf}"
    );
    eprintln!("  dracut: configured");

    let zram =
        booted_vm.ssh_stdout("cat /etc/systemd/zram-generator.conf 2>/dev/null || echo missing");
    assert!(zram.contains("zram0"), "zram not configured:\n{zram}");
    eprintln!("  zram: configured");

    let mounts = booted_vm.ssh_stdout("zfs list -o name,mountpoint");
    assert!(mounts.contains("/home"), "Missing /home mount:\n{mounts}");
    eprintln!("  ZFS mounts: OK");

    eprintln!("--- ALL CHECKS PASSED ---");
}
