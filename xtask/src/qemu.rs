use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const SSH_TIMEOUT_SECS: u64 = 10;

pub struct QemuVm {
    pid: Option<u32>,
    port: u16,
    password: Option<String>,
}

impl QemuVm {
    pub fn boot_iso(disk: &Path, uefi_vars: &Path, iso: &Path, port: u16) -> Self {
        let ovmf = find_ovmf_code();
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
            .expect("Failed to start QEMU. Is KVM available?");

        Self {
            pid: Some(child.id()),
            port,
            password: None,
        }
    }

    pub fn boot_disk(disk: &Path, uefi_vars: &Path, port: u16) -> Self {
        let ovmf = find_ovmf_code();
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
            .expect("Failed to start QEMU. Is KVM available?");

        Self {
            pid: Some(child.id()),
            port,
            password: None,
        }
    }

    pub fn with_password(mut self, password: &str) -> Self {
        self.password = Some(password.to_string());
        self
    }

    pub fn wait_for_ssh(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        eprintln!("  Waiting for SSH on port {}...", self.port);
        while start.elapsed() < timeout {
            if self.ssh_run("echo ready").is_ok_and(|o| o.status.success()) {
                eprintln!("  SSH ready ({:.0}s)", start.elapsed().as_secs_f64());
                return true;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        false
    }

    pub fn ssh_run(&self, cmd: &str) -> std::io::Result<Output> {
        let port_str = self.port.to_string();
        let timeout_str = format!("ConnectTimeout={SSH_TIMEOUT_SECS}");

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
                    &timeout_str,
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
                    &timeout_str,
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

    pub fn ssh_stdout(&self, cmd: &str) -> String {
        let output = self.ssh_run(cmd).expect("SSH command failed to execute");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn ssh_ok(&self, cmd: &str) -> bool {
        self.ssh_run(cmd).is_ok_and(|o| o.status.success())
    }

    pub fn scp_to(&self, local: &Path, remote: &str) {
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
            .expect("scp failed to execute");
        assert!(status.success(), "scp to {remote} failed");
    }

    pub fn shutdown(&mut self) {
        if self.pid.is_some() {
            let _ = self.ssh_run("poweroff");
            std::thread::sleep(Duration::from_secs(5));
        }
        self.kill();
    }

    pub fn kill(&mut self) {
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

// --- Environment helpers ---

pub fn find_ovmf_code() -> PathBuf {
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

fn find_ovmf_vars_template() -> PathBuf {
    for dir in [
        "/usr/share/edk2/x64",
        "/usr/share/edk2-ovmf/x64",
        "/usr/share/OVMF",
    ] {
        let path = PathBuf::from(dir).join("OVMF_VARS.4m.fd");
        if path.exists() {
            return path;
        }
    }
    panic!("OVMF_VARS.4m.fd not found. Install edk2-ovmf.");
}

pub fn find_latest_testing_iso() -> PathBuf {
    let out_dir = PathBuf::from("gen_iso/out");
    let mut isos: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .unwrap_or_else(|_| panic!("gen_iso/out not found. Run 'just build-test' first."))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "iso") && p.to_string_lossy().contains("testing")
        })
        .collect();
    isos.sort();
    isos.last()
        .cloned()
        .expect("No testing ISO found. Run 'just build-test' first.")
}

pub fn create_fresh_disk(path: &Path) {
    let _ = std::fs::remove_file(path);
    let status = Command::new("qemu-img")
        .args(["create", "-f", "qcow2", path.to_str().unwrap(), "20G"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("qemu-img not found");
    assert!(status.success(), "qemu-img create failed");
}

pub fn reset_uefi_vars(path: &Path) {
    let src = find_ovmf_vars_template();
    std::fs::copy(src, path).expect("failed to copy UEFI vars");
}
