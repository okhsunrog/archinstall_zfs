use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const SSH_TIMEOUT_SECS: u64 = 10;

/// 9p tag the host cache is shared under, and where the guest mounts it.
///
/// Under /run on purpose: the installer bind-mounts the medium's /run into the
/// target, so a path there is reachable from inside the chroot where makepkg
/// runs, without the share having to know anything about the target.
pub const CACHE_MOUNT_TAG: &str = "archzfscache";
pub const CACHE_GUEST_DIR: &str = "/run/archzfs-cache";

pub struct QemuVm {
    pid: Option<u32>,
    port: u16,
    password: Option<String>,
}

/// OpenSSH client the VM is reached with; `scp` spells the port flag `-P`.
#[derive(Clone, Copy)]
enum RemoteTool {
    Ssh,
    Scp,
}

impl QemuVm {
    /// Boot the installation medium, with `cache` shared into the guest when
    /// one is given.
    ///
    /// The share carries the package cache and the pre-downloaded sources, so
    /// a run reuses what earlier runs fetched instead of pulling a gigabyte of
    /// packages again.
    pub fn boot_iso(
        disk: &Path,
        uefi_vars: &Path,
        iso: &Path,
        port: u16,
        cache: Option<&Path>,
    ) -> Self {
        let ovmf = find_ovmf_code();
        let share = cache.map(|path| {
            format!(
                "local,path={},mount_tag={CACHE_MOUNT_TAG},security_model=mapped-xattr",
                path.display()
            )
        });
        let mut child = Command::new("qemu-system-x86_64")
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
            .args(match &share {
                Some(spec) => vec!["-virtfs", spec.as_str()],
                None => vec![],
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start QEMU. Is KVM available?");

        let pid = child.id();
        // Detach: we manage the process via kill(pid), not wait()
        std::thread::spawn(move || {
            let _ = child.wait();
        });

        Self {
            pid: Some(pid),
            port,
            password: None,
        }
    }

    pub fn boot_disk(disk: &Path, uefi_vars: &Path, port: u16) -> Self {
        let ovmf = find_ovmf_code();
        let mut child = Command::new("qemu-system-x86_64")
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

        let pid = child.id();
        std::thread::spawn(move || {
            let _ = child.wait();
        });

        Self {
            pid: Some(pid),
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

    /// Build an `ssh`/`scp` command for the guest.
    ///
    /// Wraps the tool in `sshpass` when a password is set, disables host key
    /// checks and appends the port; `opts` go between the shared options and
    /// the port, so the caller adds only its own `-o` flags and operands.
    fn remote_cmd(&self, tool: RemoteTool, opts: &[&str]) -> Command {
        let (name, port_flag) = match tool {
            RemoteTool::Ssh => ("ssh", "-p"),
            RemoteTool::Scp => ("scp", "-P"),
        };
        let mut cmd = match &self.password {
            Some(pw) => {
                let mut cmd = Command::new("sshpass");
                cmd.args(["-p", pw, name]);
                cmd
            }
            None => Command::new(name),
        };
        cmd.args([
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
        ])
        .args(opts)
        .args([port_flag, &self.port.to_string()]);
        cmd
    }

    pub fn ssh_run(&self, cmd: &str) -> std::io::Result<Output> {
        let timeout_str = format!("ConnectTimeout={SSH_TIMEOUT_SECS}");
        self.remote_cmd(RemoteTool::Ssh, &["-o", &timeout_str])
            .args(["root@localhost", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    }

    pub fn ssh_stdout(&self, cmd: &str) -> String {
        let output = self.ssh_run(cmd).expect("SSH command failed to execute");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn scp_to(&self, local: &Path, remote: &str) {
        let local_str = local.to_str().unwrap();
        let remote_dest = format!("root@localhost:{remote}");
        let status = self
            .remote_cmd(RemoteTool::Scp, &[])
            .args([local_str, remote_dest.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("scp failed to execute");
        assert!(status.success(), "scp to {remote} failed");
    }

    /// Copy a file from the VM to a local path. Returns true on success.
    pub fn scp_from(&self, remote: &str, local: &Path) -> bool {
        let remote_src = format!("root@localhost:{remote}");
        let local_str = local.to_str().unwrap();
        let status = self
            .remote_cmd(RemoteTool::Scp, &[])
            .args([remote_src.as_str(), local_str])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        status.is_ok_and(|s| s.success())
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

/// Locate an OVMF firmware file by trying distro-specific layouts.
/// Arch ships `<name>.4m.fd`; Fedora/Debian ship `<name>.fd` (no 4m).
///
/// The justfile (`qemu-setup-uefi`) and `gen_iso/run-qemu.sh`
/// (`find_ovmf_file`) keep their own lists: the justfile searches the
/// `/usr/share/{edk2,edk2-ovmf,OVMF}` roots recursively and skips secboot
/// images, run-qemu.sh probes the x64 subdirectories without
/// `/usr/share/edk2/ovmf`. Update all three together.
fn find_ovmf(base: &str) -> PathBuf {
    let dirs = [
        "/usr/share/edk2/x64",
        "/usr/share/edk2-ovmf/x64",
        "/usr/share/edk2/ovmf",
        "/usr/share/OVMF",
    ];
    // Try the Arch-style 4m suffix first (smaller, newer), fall back to bare.
    for name in [format!("{base}.4m.fd"), format!("{base}.fd")] {
        for dir in dirs {
            let path = PathBuf::from(dir).join(&name);
            if path.exists() {
                return path;
            }
        }
    }
    panic!("{base}{{,.4m}}.fd not found. Install edk2-ovmf.");
}

pub fn find_ovmf_code() -> PathBuf {
    find_ovmf("OVMF_CODE")
}

fn find_ovmf_vars_template() -> PathBuf {
    find_ovmf("OVMF_VARS")
}

pub fn find_latest_testing_iso() -> Result<PathBuf, String> {
    let out_dir = PathBuf::from("gen_iso/out");
    let mut isos: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&out_dir)
        .map_err(|e| {
            format!(
                "cannot read {}: {e}. Run 'just iso-test' first.",
                out_dir.display()
            )
        })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_testing_iso(p))
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((mtime, p))
        })
        .collect();
    isos.sort_by_key(|(mtime, _)| *mtime);
    isos.pop().map(|(_, p)| p).ok_or_else(|| {
        "No testing ISO found in gen_iso/out. Run 'just iso-test' first, or pass --iso explicitly."
            .to_string()
    })
}

fn is_testing_iso(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "iso")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("archzfs-") && name.contains("-testing-"))
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

#[cfg(test)]
mod tests {
    use super::{QemuVm, RemoteTool, is_testing_iso};
    use std::path::Path;
    use std::process::Command;

    fn argv(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn remote_commands_keep_their_argv() {
        let vm = QemuVm {
            pid: None,
            port: 2222,
            password: None,
        };
        assert_eq!(
            argv(&vm.remote_cmd(RemoteTool::Ssh, &["-o", "ConnectTimeout=10"])),
            [
                "ssh",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "ConnectTimeout=10",
                "-p",
                "2222",
            ]
        );
        assert_eq!(
            argv(&vm.remote_cmd(RemoteTool::Scp, &[])),
            [
                "scp",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-P",
                "2222",
            ]
        );

        let vm = vm.with_password("test");
        assert_eq!(
            argv(&vm.remote_cmd(RemoteTool::Scp, &[])),
            [
                "sshpass",
                "-p",
                "test",
                "scp",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-P",
                "2222",
            ]
        );
    }

    #[test]
    fn integration_harness_accepts_only_testing_isos() {
        assert!(is_testing_iso(Path::new(
            "archzfs-linux-lts-dkms-testing-2026.09.09-x86_64.iso"
        )));
        assert!(!is_testing_iso(Path::new(
            "archzfs-linux-dkms-2026.09.09-x86_64.iso"
        )));
        assert!(!is_testing_iso(Path::new("other-testing-image.iso")));
        assert!(!is_testing_iso(Path::new(
            "archzfs-linux-lts-dkms-testing-2026.09.09-x86_64.img"
        )));
    }
}
