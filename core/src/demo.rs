//! Safe demo mode.
//!
//! Demo mode runs the full UI and every hardware backend for real — input,
//! Wi-Fi, disk enumeration, ZFS inventory — while refusing to start the
//! installation. It is a *gate at the choke points*, not a fake command
//! runner: the install pipeline reaches storage through subprocesses, zfskit,
//! direct filesystem writes and libalpm, so intercepting one of those four
//! would leave a "safe" mode that still creates pools and writes to the
//! target. Blocking the pipeline entrypoint is the only honest place to stop.
//!
//! Both UIs share this trigger so the ISO boot entry (`archinstall_zfs.demo=1`)
//! behaves identically whichever binary the user launches.

/// Whether the running kernel's command line asks for demo mode.
pub fn enabled_from_kernel_cmdline() -> bool {
    cmdline_enables_demo(&std::fs::read_to_string("/proc/cmdline").unwrap_or_default())
}

/// Whether `cmdline` contains the demo boot argument.
pub fn cmdline_enables_demo(cmdline: &str) -> bool {
    cmdline
        .split_whitespace()
        .any(|arg| matches!(arg, "archinstall_zfs.demo" | "archinstall_zfs.demo=1"))
}

#[cfg(test)]
mod tests {
    use super::cmdline_enables_demo;

    #[test]
    fn demo_kernel_argument_is_detected() {
        assert!(cmdline_enables_demo(
            "quiet archinstall_zfs.demo=1 console=tty1"
        ));
        assert!(cmdline_enables_demo("archinstall_zfs.demo"));
    }

    #[test]
    fn unrelated_or_disabled_arguments_do_not_enable_demo() {
        assert!(!cmdline_enables_demo("archinstall_zfs.demo=0"));
        assert!(!cmdline_enables_demo("quiet console=tty1"));
        assert!(!cmdline_enables_demo(""));
        // Must not match a prefix of some other future argument.
        assert!(!cmdline_enables_demo("archinstall_zfs.demonstrate=1"));
    }
}
