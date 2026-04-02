pub fn cpu_vendor() -> CpuVendor {
    let info = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::nothing()),
    );
    let vendor = info
        .cpus()
        .first()
        .map(|c| c.vendor_id().to_lowercase())
        .unwrap_or_default();

    if vendor.contains("intel") {
        CpuVendor::Intel
    } else if vendor.contains("amd") || vendor.contains("authenticamd") {
        CpuVendor::Amd
    } else {
        CpuVendor::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

impl CpuVendor {
    pub fn microcode_package(&self) -> Option<&'static str> {
        match self {
            Self::Intel => Some("intel-ucode"),
            Self::Amd => Some("amd-ucode"),
            Self::Unknown => None,
        }
    }
}

pub fn has_uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_vendor_microcode() {
        assert_eq!(CpuVendor::Intel.microcode_package(), Some("intel-ucode"));
        assert_eq!(CpuVendor::Amd.microcode_package(), Some("amd-ucode"));
        assert_eq!(CpuVendor::Unknown.microcode_package(), None);
    }
}
