use crate::config::types::ZfsModuleMode;
use crate::system::cmd::CommandRunner;

#[derive(Debug, Clone)]
pub struct CompatibilityResult {
    pub kernel_name: String,
    pub dkms_compatible: bool,
    pub dkms_warnings: Vec<String>,
    pub precompiled_compatible: bool,
    pub precompiled_warnings: Vec<String>,
}

pub fn scan_kernel_compatibility(runner: &dyn CommandRunner, kernel: &str) -> CompatibilityResult {
    let precompiled_ok = check_precompiled(runner, kernel);
    let dkms_ok = check_dkms(runner, kernel);

    CompatibilityResult {
        kernel_name: kernel.to_string(),
        dkms_compatible: dkms_ok.0,
        dkms_warnings: dkms_ok.1,
        precompiled_compatible: precompiled_ok.0,
        precompiled_warnings: precompiled_ok.1,
    }
}

fn check_precompiled(runner: &dyn CommandRunner, kernel: &str) -> (bool, Vec<String>) {
    let info = match super::get_kernel_info(kernel) {
        Some(i) => i,
        None => return (false, vec![format!("Unknown kernel: {kernel}")]),
    };

    let pkg = match info.precompiled_package {
        Some(p) => p,
        None => return (false, vec![format!("No precompiled ZFS for {kernel}")]),
    };

    // Check if the package is available via pacman
    let output = runner.run("pacman", &["-Si", pkg]);
    match output {
        Ok(o) if o.success() => (true, vec![]),
        _ => (false, vec![format!("Package {pkg} not found in repos")]),
    }
}

fn check_dkms(runner: &dyn CommandRunner, kernel: &str) -> (bool, Vec<String>) {
    // Check if zfs-dkms is available
    let output = runner.run("pacman", &["-Si", "zfs-dkms"]);
    match output {
        Ok(o) if o.success() => (true, vec![]),
        _ => (false, vec!["zfs-dkms not found in repos".to_string()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_scan_precompiled_available() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(), // pacman -Si zfs-linux-lts
            CannedResponse::default(), // pacman -Si zfs-dkms
        ]);
        let result = scan_kernel_compatibility(&runner, "linux-lts");
        assert!(result.precompiled_compatible);
        assert!(result.dkms_compatible);
    }

    #[test]
    fn test_scan_precompiled_not_available() {
        let runner = RecordingRunner::new(vec![
            CannedResponse {
                exit_code: 1,
                stderr: "error: package not found".into(),
                ..Default::default()
            }, // pacman -Si fails
            CannedResponse::default(), // zfs-dkms available
        ]);
        let result = scan_kernel_compatibility(&runner, "linux-lts");
        assert!(!result.precompiled_compatible);
        assert!(result.dkms_compatible);
    }
}
