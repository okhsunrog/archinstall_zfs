//! Health checks run over SSH against the freshly booted installation.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::qemu::QemuVm;

/// Detect the init system from the JSON config file.
pub fn detect_init_system(config: &Path) -> String {
    let content = std::fs::read_to_string(config).unwrap_or_default();
    if content.contains("\"mkinitcpio\"") {
        "mkinitcpio".to_string()
    } else {
        "dracut".to_string()
    }
}

/// One remote command and the condition its trimmed stdout must satisfy.
struct Check {
    label: &'static str,
    command: String,
    /// What a passing output looks like; shown next to the verdict.
    expect: String,
    predicate: Box<dyn Fn(&str) -> bool>,
}

impl Check {
    fn new(
        label: &'static str,
        command: impl Into<String>,
        expect: impl Into<String>,
        predicate: impl Fn(&str) -> bool + 'static,
    ) -> Self {
        Self {
            label,
            command: command.into(),
            expect: expect.into(),
            predicate: Box::new(predicate),
        }
    }
}

fn checks(config_path: &Path) -> Result<Vec<Check>, String> {
    let init_system = detect_init_system(config_path);
    let config: Value = serde_json::from_slice(&fs::read(config_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let swap_mode = config["swap_mode"].as_str().unwrap_or("none").to_string();

    let initramfs = if init_system == "mkinitcpio" {
        Check::new(
            "mkinitcpio",
            "cat /etc/mkinitcpio.conf 2>/dev/null || echo missing",
            "configured (has zfs)",
            |out| out.contains("zfs"),
        )
    } else {
        Check::new(
            "dracut",
            "cat /etc/dracut.conf.d/zfs.conf 2>/dev/null || echo missing",
            "configured (hostonly)",
            |out| out.contains("hostonly"),
        )
    };
    // ZBM rootprefix property
    let expected_prefix = if init_system == "dracut" {
        "root=ZFS="
    } else {
        "zfs="
    };

    Ok(vec![
        Check::new("kernel", "uname -r", "lts", |out| out.contains("lts")),
        Check::new("zpool", "zpool status testpool 2>&1", "ONLINE", |out| {
            out.contains("ONLINE")
        }),
        Check::new("sshd", "systemctl is-active sshd", "active", |out| {
            out == "active"
        }),
        // A network service is enabled whichever path configured it.
        Check::new(
            "network",
            "systemctl is-enabled NetworkManager systemd-networkd 2>/dev/null",
            "NetworkManager or systemd-networkd enabled",
            |out| out.lines().any(|line| line.trim() == "enabled"),
        ),
        Check::new("fstab", "cat /etc/fstab", "has root dataset", |out| {
            out.contains("testpool/arch0/root")
        }),
        initramfs,
        // The configured swap path, including activation after boot.
        Check::new(
            "swap",
            "swapon --show=NAME --noheadings",
            format!("configured and active as requested ({swap_mode})"),
            move |out| match swap_mode.as_str() {
                "none" => out.trim().is_empty(),
                "zram" => out.contains("/dev/zram"),
                "zswap_partition" => !out.trim().is_empty() && !out.contains("zram"),
                "zswap_partition_encrypted" => {
                    out.contains("cryptswap") || out.contains("/dev/dm-")
                }
                _ => false,
            },
        ),
        Check::new(
            "ZFS mounts",
            "zfs list -o name,mountpoint 2>&1",
            "/home, /root",
            |out| out.contains("/home") && out.contains("/root"),
        ),
        Check::new(
            "hostid",
            "od -A n -t x1 /etc/hostid 2>/dev/null | tr -d ' \\n'",
            "0x00bab10c",
            |out| out == "0cb1ba00",
        ),
        // ZED cache hook (boot-environment aware)
        Check::new(
            "ZED hook",
            "cat /etc/zfs/zed.d/history_event-zfs-list-cacher.sh 2>/dev/null || echo missing",
            "installed (boot environment aware)",
            |out| out.contains("boot environment aware"),
        ),
        // bootfs (needed for zbm.timeout auto-boot; users can still select other BEs)
        Check::new(
            "bootfs",
            "zpool get -H -o value bootfs testpool 2>/dev/null",
            "testpool/arch0/root",
            |out| out == "testpool/arch0/root",
        ),
        Check::new(
            "rootprefix",
            "zfs get -H -o value org.zfsbootmenu:rootprefix testpool/arch0/root 2>/dev/null",
            expected_prefix,
            move |out| out == expected_prefix,
        ),
        // ZBM locally built (generate-zbm available + config present)
        Check::new(
            "ZBM local build",
            "echo config=$(grep -q 'ManageImages: true' /etc/zfsbootmenu/config.yaml 2>/dev/null \
             && echo ok || echo missing) \
             bin=$(which generate-zbm >/dev/null 2>&1 && echo ok || echo missing)",
            "config=ok bin=ok",
            |out| out == "config=ok bin=ok",
        ),
        Check::new(
            "ZBM pacman hook",
            "grep -qF 'Exec = /usr/local/sbin/azfs-update-zbm' /etc/pacman.d/hooks/95-zfsbootmenu.hook 2>/dev/null \
             && test -x /usr/local/sbin/azfs-update-zbm \
             && test -x /usr/local/libexec/azfs-install-zbm \
             && echo ready",
            "installed",
            |out| out == "ready",
        ),
    ])
}

/// Checks for the first boot of a boot environment installed into a pool that
/// already held `first`. That boot runs on the mount cache the installer wrote,
/// before the target's own ZED hook has rewritten it.
fn second_boot_environment_checks(first: &Path, second: &Path) -> Result<Vec<Check>, String> {
    let read = |path: &Path| -> Result<Value, String> {
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    };
    let (first, second) = (read(first)?, read(second)?);
    let pool = second["pool_name"]
        .as_str()
        .unwrap_or("testpool")
        .to_string();
    let base = format!("{pool}/{}", second["dataset_prefix"].as_str().unwrap_or(""));
    let other = format!("{pool}/{}", first["dataset_prefix"].as_str().unwrap_or(""));
    let user = second["users"][0]["username"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let distro = second["distribution"]
        .as_str()
        .unwrap_or("arch")
        .to_string();

    let mount = |label: &'static str, path: &'static str, dataset: String| {
        Check::new(
            label,
            format!("findmnt -n -o SOURCE {path}"),
            dataset.clone(),
            move |out| out == dataset,
        )
    };
    Ok(vec![
        mount("root", "/", format!("{base}/root")),
        // The other environment's /home listed first in the cache won the mount.
        mount("home", "/home", format!("{base}/data/home")),
        Check::new(
            "user home",
            format!("stat -c %U /home/{user} 2>&1"),
            user.clone(),
            {
                let user = user.clone();
                move |out| out == user
            },
        ),
        // flatpak's environment generator used to create /root/.cache first.
        Check::new(
            "flatpak",
            "pacman -Q flatpak >/dev/null 2>&1 && echo installed",
            "installed",
            |out| out == "installed",
        ),
        mount("/root", "/root", format!("{base}/data/root")),
        Check::new(
            "mount generator",
            "journalctl -b -o cat | grep -c 'already exists. Skipping'",
            "no competing mount units",
            |out| out == "0",
        ),
        Check::new(
            "os-release",
            ". /etc/os-release && echo $ID",
            distro.clone(),
            move |out| out == distro,
        ),
        Check::new(
            "bootfs",
            format!("zpool get -H -o value bootfs {pool}"),
            format!("{base}/root"),
            {
                let root = format!("{base}/root");
                move |out| out == root
            },
        ),
        Check::new(
            "first environment",
            format!("zfs list -H -o name {other}/root {other}/data/home 2>&1 | wc -l"),
            "still present",
            |out| out == "2",
        ),
    ])
}

pub fn verify_second_boot_environment(
    vm: &QemuVm,
    first_config: &Path,
    second_config: &Path,
) -> Result<(), String> {
    run_checks(
        vm,
        second_boot_environment_checks(first_config, second_config)?,
    )
}

pub fn verify_system(vm: &QemuVm, config_path: &Path) -> Result<(), String> {
    run_checks(vm, checks(config_path)?)
}

fn run_checks(vm: &QemuVm, checks: Vec<Check>) -> Result<(), String> {
    let total = checks.len();
    let mut passed = 0;
    let mut lines = Vec::new();

    for check in &checks {
        let output = vm.ssh_stdout(&check.command);
        if (check.predicate)(&output) {
            lines.push(format!("  {}: OK ({})", check.label, check.expect));
            passed += 1;
        } else if output.contains('\n') {
            lines.push(format!(
                "  {}: FAIL (expected {})\n{output}",
                check.label, check.expect
            ));
        } else {
            lines.push(format!(
                "  {}: FAIL (expected {}, got '{output}')",
                check.label, check.expect
            ));
        }
    }

    for line in &lines {
        eprintln!("{line}");
    }
    eprintln!("  --- {passed}/{total} checks passed ---");

    if passed == total {
        Ok(())
    } else {
        Err(format!("{}/{total} checks failed", total - passed))
    }
}
